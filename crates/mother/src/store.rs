use crate::{
    Error, Result,
    config::private_file,
    protocol::{Operation, PROTOCOL_VERSION, Request},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 10;
mod continuity;
mod github;
mod personas;
mod tasks;
mod workers;

pub struct Store {
    dir: std::path::PathBuf,
    connection: Connection,
}

impl Store {
    /// Caller must hold the runtime lock for this connection's lifetime.
    pub fn open(dir: &Path) -> Result<Self> {
        private_file(&dir.join("mother.db"))?;
        let mut connection = Connection::open(dir.join("mother.db"))?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "database schema {version} is newer than supported schema {SCHEMA_VERSION}"
            )));
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        if version < SCHEMA_VERSION {
            let transaction = connection.transaction()?;
            if version == 0 {
                transaction.execute_batch(include_str!("../migrations/001_foundation.sql"))?;
            }
            if version < 2 {
                transaction.execute_batch(include_str!("../migrations/002_continuity.sql"))?;
            }
            if version < 3 {
                transaction.execute_batch(include_str!("../migrations/003_tasks.sql"))?;
            }
            if version < 4 {
                transaction.execute_batch(include_str!("../migrations/004_personas.sql"))?;
            }
            if version < 5 {
                transaction
                    .execute_batch(include_str!("../migrations/005_github_identities.sql"))?;
            }
            if version < 6 {
                transaction.execute_batch(include_str!("../migrations/006_worker_leases.sql"))?;
            }
            if version < 7 {
                transaction
                    .execute_batch(include_str!("../migrations/007_worker_processes.sql"))?;
            }
            if version < 8 {
                transaction
                    .execute_batch(include_str!("../migrations/008_worker_process_loss.sql"))?;
            }
            if version < 9 {
                transaction
                    .execute_batch(include_str!("../migrations/009_provider_adapter.sql"))?;
            }
            if version < 10 {
                transaction.execute_batch(include_str!("../migrations/010_codex_adapter.sql"))?;
            }
            transaction.commit()?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            connection,
        })
    }

    pub fn execute(&mut self, request: &Request) -> Result<Value> {
        if request.version != PROTOCOL_VERSION {
            return Err(Error::Invalid("unsupported IPC protocol version".into()));
        }
        text(&request.request_id, "request_id", 128)?;
        let transaction = self.connection.transaction()?;
        let operation = serde_json::to_string(&request.operation)?;
        if request.operation.mutates() {
            let cached: Option<(String, String)> = transaction
                .query_row(
                    "SELECT operation, response FROM idempotency WHERE request_id = ?1",
                    [&request.request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((previous, response)) = cached {
                if previous != operation {
                    return Err(Error::Conflict(
                        "idempotency key was already used for a different operation".into(),
                    ));
                }
                return Ok(serde_json::from_str(&response)?);
            }
        }
        let result = apply(&transaction, request, Some(&self.dir))?;
        if request.operation.mutates() {
            transaction.execute(
                "INSERT INTO idempotency VALUES (?1, ?2, ?3)",
                params![
                    request.request_id,
                    operation,
                    serde_json::to_string(&result)?
                ],
            )?;
        }
        transaction.commit()?;
        Ok(result)
    }
}

fn text(value: &str, name: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        return Err(Error::Invalid(format!(
            "{name} must contain 1..={max} UTF-8 bytes and no NUL"
        )));
    }
    Ok(())
}

fn worker_dir(dir: Option<&Path>) -> Result<&Path> {
    dir.ok_or_else(|| Error::Unavailable("worker operation requires runtime context".into()))
}

fn pagination(after: u64, limit: u32) -> Result<()> {
    if !(1..=100).contains(&limit) || after > i64::MAX as u64 {
        return Err(Error::Invalid(
            "limit must be in 1..=100 and cursor must fit a signed 64-bit integer".into(),
        ));
    }
    Ok(())
}

fn revision(tx: &Transaction<'_>, id: &str) -> Result<i64> {
    tx.query_row("SELECT revision FROM projects WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("project {id} does not exist")))
}

fn event(
    tx: &Transaction<'_>,
    request: &Request,
    project_id: &str,
    object_id: &str,
    action: &str,
    source: &str,
    revision: i64,
) -> Result<()> {
    tx.execute("INSERT INTO events (id,project_id,actor,source,action,object_id,correlation_id,revision,result) VALUES (?1,?2,'local_operator',?3,?4,?5,?6,?7,'committed')",
        params![Uuid::new_v4().to_string(), project_id, source, action, object_id, request.request_id, revision])?;
    Ok(())
}

fn page(mut items: Vec<Value>, limit: u32) -> Value {
    let mut included = 0;
    let mut bytes = 0;
    for item in items.iter().take(limit as usize) {
        let size = serde_json::to_vec(item)
            .expect("JSON values serialize")
            .len();
        if included > 0 && bytes + size > 1024 * 1024 {
            break;
        }
        included += 1;
        bytes += size;
    }
    let more = items.len() > included;
    items.truncate(included);
    let next = if more {
        items.last().and_then(|v| v.get("sequence")).cloned()
    } else {
        None
    };
    json!({"items": items, "next_cursor": next})
}

fn apply(tx: &Transaction<'_>, request: &Request, dir: Option<&Path>) -> Result<Value> {
    match &request.operation {
        Operation::Health => {
            let projects: i64 = tx.query_row("SELECT count(*) FROM projects", [], |r| r.get(0))?;
            Ok(
                json!({"status":"ok", "schema_version":SCHEMA_VERSION, "protocol_version":PROTOCOL_VERSION,
                "sqlite_version":rusqlite::version(), "projects":projects, "workers_supported":true, "worker_adapter":"mock_codex_and_opencode_cli", "provider_workers_supported":true, "provider_credentials_stored":false, "provider_credentials_brokered":false, "provider_budget_enforcement":true, "provider_budget_basis":"provider_reported_cost_and_steps", "provider_spend_independently_metered":false, "worker_process_supervision":true, "worker_supervision_scope":"owner_run_cli", "worker_worktree_isolation":true, "worker_project_repository_mutation":"not_guaranteed", "execution_leases_supported":true, "mcp_supported":true,"host_tracked_tasks_supported":true,"personas_supported":true,"persona_enforcement":"optional_assertion","github_identity_records_supported":true,"github_credential_storage":false,"github_operations":"read_only_verification"}),
            )
        }
        Operation::ProjectAttach { name, repository } => {
            text(name, "name", 256)?;
            text(repository, "repository", 4096)?;
            // Resolve on the trusted service, never accept a claimed canonical path.
            let path = std::fs::canonicalize(repository)?;
            if !path.is_dir() {
                return Err(Error::Invalid("repository must be a directory".into()));
            }
            let path = path
                .to_str()
                .ok_or_else(|| Error::Invalid("repository path must be UTF-8".into()))?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE repository=?1)",
                [path],
                |r| r.get(0),
            )?;
            if exists {
                return Err(Error::Conflict(
                    "repository is already attached; use projects list".into(),
                ));
            }
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO projects (id,name,repository,revision) VALUES (?1,?2,?3,1)",
                params![id, name, path],
            )?;
            event(tx, request, &id, &id, "project.attached", "local_ipc", 1)?;
            Ok(json!({"id":id,"name":name,"repository":path,"revision":1}))
        }
        Operation::ProjectsList { after, limit } => {
            pagination(*after, *limit)?;
            let mut stmt = tx.prepare("SELECT sequence,id,name,repository,revision,created_at FROM projects WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?;
            let items = stmt.query_map(params![*after as i64, limit+1], |r| Ok(json!({
                "sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,
                "repository":r.get::<_,String>(3)?,"revision":r.get::<_,i64>(4)?,"created_at":r.get::<_,String>(5)?
            })))?.collect::<std::result::Result<Vec<_>,_>>()?;
            Ok(page(items, *limit))
        }
        Operation::ProjectGet { project_id } => {
            let project = tx.query_row("SELECT id,name,repository,revision,created_at FROM projects WHERE id=?1", [project_id], |r| Ok(json!({
                "id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"repository":r.get::<_,String>(2)?,
                "revision":r.get::<_,i64>(3)?,"created_at":r.get::<_,String>(4)?
            }))).optional()?.ok_or_else(|| Error::NotFound(format!("project {project_id} does not exist")))?;
            Ok(project)
        }
        Operation::ContextRecord {
            project_id,
            kind,
            content,
            source,
            expected_revision,
        } => {
            text(content, "content", 4096)?;
            text(source, "source", 256)?;
            let current = revision(tx, project_id)?;
            if expected_revision.is_some_and(|expected| expected != current as u64) {
                return Err(Error::Conflict(format!(
                    "stale project revision; current revision is {current}"
                )));
            }
            let next = current
                .checked_add(1)
                .ok_or_else(|| Error::Conflict("project revision exhausted".into()))?;
            let kind = serde_json::to_value(kind)?.as_str().unwrap().to_owned();
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "UPDATE projects SET revision=?1 WHERE id=?2",
                params![next, project_id],
            )?;
            tx.execute("INSERT INTO context_records (id,project_id,kind,content,source,revision) VALUES (?1,?2,?3,?4,?5,?6)", params![id,project_id,kind,content,source,next])?;
            event(
                tx,
                request,
                project_id,
                &id,
                "context.recorded",
                source,
                next,
            )?;
            Ok(
                json!({"id":id,"project_id":project_id,"revision":next,"kind":kind,"content":content,"source":source,"trust":"unverified_submission"}),
            )
        }
        Operation::ContextGet {
            project_id,
            after,
            limit,
        } => {
            pagination(*after, *limit)?;
            let current = revision(tx, project_id)?;
            let mut stmt = tx.prepare("SELECT sequence,id,kind,content,source,revision,created_at FROM context_records WHERE project_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
            let items = stmt.query_map(params![project_id,*after as i64,limit+1],|r| Ok(json!({
                "sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,
                "content":r.get::<_,String>(3)?,"source":r.get::<_,String>(4)?,"revision":r.get::<_,i64>(5)?,
                "created_at":r.get::<_,String>(6)?,"trust":"unverified_submission"
            })))?.collect::<std::result::Result<Vec<_>,_>>()?;
            let mut result = page(items, *limit);
            result["project_id"] = json!(project_id);
            result["revision"] = json!(current);
            result["active_decisions"] = continuity::decisions(tx, project_id, false, 0, 20)?;
            Ok(result)
        }
        Operation::ActivityGet {
            project_id,
            after,
            limit,
        } => {
            pagination(*after, *limit)?;
            let current = project_id
                .as_deref()
                .map(|id| revision(tx, id))
                .transpose()?;
            let mut stmt = tx.prepare("SELECT sequence,id,project_id,actor,source,action,object_id,correlation_id,revision,result,created_at FROM events WHERE (?1 IS NULL OR project_id=?1) AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
            let items = stmt.query_map(params![project_id,*after as i64,limit+1], |r| Ok(json!({
                "sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"project_id":r.get::<_,String>(2)?,
                "actor":r.get::<_,String>(3)?,"source":r.get::<_,String>(4)?,"action":r.get::<_,String>(5)?,
                "object_id":r.get::<_,String>(6)?,"correlation_id":r.get::<_,String>(7)?,"revision":r.get::<_,i64>(8)?,
                "result":r.get::<_,String>(9)?,"created_at":r.get::<_,String>(10)?
            })))?.collect::<std::result::Result<Vec<_>,_>>()?;
            let mut result = page(items, *limit);
            result["revision"] = json!(current);
            Ok(result)
        }
        Operation::DecisionRecord(input) => continuity::record_decision(tx, request, input),
        Operation::DecisionsList {
            project_id,
            include_superseded,
            after,
            limit,
        } => continuity::decisions(tx, project_id, *include_superseded, *after, *limit),
        Operation::DecisionGet { decision_id } => continuity::decision(tx, decision_id),
        Operation::HandoffCreate(input) => continuity::create_handoff(tx, request, input),
        Operation::HandoffsList {
            project_id,
            after,
            limit,
        } => continuity::handoffs(tx, project_id, *after, *limit),
        Operation::HandoffGet { handoff_id } => continuity::handoff(tx, handoff_id),
        Operation::HandoffResume {
            handoff_id,
            recipient,
            expected_revision,
        } => continuity::resume(tx, request, handoff_id, recipient, *expected_revision),
        Operation::TaskCreate(input) => tasks::create(tx, request, input),
        Operation::TasksList {
            project_id,
            state,
            after,
            limit,
        } => tasks::list(tx, project_id, *state, *after, *limit),
        Operation::TaskGet { task_id } => tasks::get(tx, task_id),
        Operation::TaskTransition(input) => tasks::transition(tx, request, input),
        Operation::TaskCheckpoint(input) => tasks::checkpoint(tx, request, input),
        Operation::TaskCheckpointsList {
            task_id,
            after,
            limit,
        } => tasks::checkpoints(tx, task_id, *after, *limit),
        Operation::TaskTransitionsList {
            task_id,
            after,
            limit,
        } => tasks::transitions(tx, task_id, *after, *limit),
        Operation::PersonaCreate(input) => personas::create(tx, &request.request_id, input),
        Operation::PersonaUpdate(update) => personas::update(tx, &request.request_id, update),
        Operation::PersonaSetStatus(change) => {
            personas::set_status(tx, &request.request_id, change)
        }
        Operation::PersonaGet { persona_id } => personas::get(tx, persona_id),
        Operation::PersonasList {
            status,
            project_id,
            after,
            limit,
        } => personas::list(tx, *status, project_id.as_deref(), *after, *limit),
        Operation::PersonaHistory {
            persona_id,
            after,
            limit,
        } => personas::history(tx, persona_id, *after, *limit),
        Operation::GithubIdentityAttach(input) => github::attach(tx, &request.request_id, input),
        Operation::GithubIdentityRevoke(change) => github::revoke(tx, &request.request_id, change),
        Operation::GithubIdentityGet { identity_id } => github::get(tx, identity_id),
        Operation::GithubIdentitiesList {
            status,
            persona_id,
            after,
            limit,
        } => github::list(tx, *status, persona_id.as_deref(), *after, *limit),
        Operation::GithubIdentityHistory {
            identity_id,
            after,
            limit,
        } => github::history(tx, identity_id, *after, *limit),
        Operation::WorkerEligible { project_id, limit } => {
            workers::eligible(tx, project_id.as_deref(), *limit)
        }
        Operation::WorkerExecutionsList {
            task_id,
            project_id,
            status,
            after,
            limit,
        } => workers::list(
            tx,
            task_id.as_deref(),
            project_id.as_deref(),
            *status,
            *after,
            *limit,
        ),
        Operation::WorkerExecutionGet { execution_id } => workers::get(tx, execution_id),
        Operation::WorkerExecutionHistory {
            execution_id,
            after,
            limit,
        } => workers::history(tx, execution_id, *after, *limit),
        Operation::WorkerLease(input) => workers::lease(tx, request, input),
        Operation::WorkerHeartbeat(input) => workers::heartbeat(tx, request, input),
        Operation::WorkerMockRun(input) => workers::run_mock(tx, request, input),
        Operation::WorkerRelease(input) => workers::release(tx, request, input),
        Operation::WorkerWorkspaceStart(input) => {
            workers::workspace_start(tx, worker_dir(dir)?, request, input)
        }
        Operation::WorkerProcessAttach(input) => {
            workers::process_attach(tx, worker_dir(dir)?, request, input)
        }
        Operation::WorkerProcessFinish(input) => {
            workers::process_finish(tx, worker_dir(dir)?, request, input)
        }
        Operation::WorkerAbandon(input) => workers::abandon(tx, request, input),
        Operation::WorkerWorkspaceCleanup(input) => {
            workers::workspace_cleanup(tx, worker_dir(dir)?, request, input)
        }
        Operation::WorkerRecover => workers::recover(tx, request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config, protocol::ContextKind};

    fn fixture() -> (tempfile::TempDir, std::fs::File, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            dir.path(),
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        config::initialize(dir.path()).unwrap();
        let lock = config::lock(dir.path()).unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, lock, store)
    }

    fn request(operation: Operation) -> Request {
        Request {
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation,
        }
    }

    #[test]
    fn failed_audit_insert_rolls_back_state_revision_and_retry_key() {
        let (dir, _lock, mut store) = fixture();
        let attached = store
            .execute(&request(Operation::ProjectAttach {
                name: "test".into(),
                repository: dir.path().to_str().unwrap().into(),
            }))
            .unwrap();
        let id = attached["id"].as_str().unwrap().to_owned();
        store.connection.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
        let write = request(Operation::ContextRecord {
            project_id: id.clone(),
            kind: ContextKind::Failure,
            content: "failed test".into(),
            source: "unit-test".into(),
            expected_revision: Some(1),
        });
        assert!(store.execute(&write).is_err());
        let state = store
            .execute(&request(Operation::ContextGet {
                project_id: id.clone(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(state["revision"], 1);
        assert_eq!(state["items"], json!([]));
        let retry_count: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM idempotency WHERE request_id=?1",
                [&write.request_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retry_count, 0);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_event")
            .unwrap();
        assert_eq!(store.execute(&write).unwrap()["revision"], 2);
        assert!(
            store
                .connection
                .execute("UPDATE events SET result='forged'", [])
                .is_err()
        );
        assert!(store.connection.execute("DELETE FROM events", []).is_err());
    }

    #[test]
    fn rejects_future_schema_without_downgrading() {
        let (dir, _lock, store) = fixture();
        store
            .connection
            .pragma_update(None, "user_version", 999)
            .unwrap();
        drop(store);
        assert!(matches!(Store::open(dir.path()), Err(Error::Invalid(_))));
        let connection = Connection::open(dir.path().join("mother.db")).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 999);
    }

    #[test]
    fn validates_bounds_and_unknown_projects() {
        let (_dir, _lock, mut store) = fixture();
        for (after, limit) in [(0, 0), (0, 101), (u64::MAX, 20)] {
            assert!(matches!(
                store.execute(&request(Operation::ProjectsList { after, limit })),
                Err(Error::Invalid(_))
            ));
        }
        assert!(matches!(
            store.execute(&request(Operation::ContextGet {
                project_id: "missing".into(),
                after: 0,
                limit: 20
            })),
            Err(Error::NotFound(_))
        ));
        let mut invalid = request(Operation::Health);
        invalid.version = 99;
        assert!(matches!(store.execute(&invalid), Err(Error::Invalid(_))));
        invalid.version = PROTOCOL_VERSION;
        invalid.request_id.clear();
        assert!(matches!(store.execute(&invalid), Err(Error::Invalid(_))));
    }

    fn attach(store: &mut Store, dir: &Path) -> String {
        store
            .execute(&request(Operation::ProjectAttach {
                name: "continuity-test".into(),
                repository: dir.to_str().unwrap().into(),
            }))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }

    fn decision_input(
        project: &str,
        supersedes: Option<String>,
    ) -> crate::continuity::DecisionInput {
        crate::continuity::DecisionInput {
            project_id: project.into(),
            title: "Storage".into(),
            decision: "Use SQLite".into(),
            rationale: "Local-first operation".into(),
            alternatives: vec!["Remote SQL".into()],
            source: "test".into(),
            supersedes,
            expected_revision: None,
        }
    }

    fn handoff_input(project: &str) -> crate::continuity::HandoffInput {
        serde_json::from_value(json!({"project_id":project,"objective":"Continue implementation","source":"test","next_action":"Run tests"})).unwrap()
    }

    #[test]
    fn supersession_is_scoped_and_transactional() {
        let (dir, _lock, mut store) = fixture();
        let project = attach(&mut store, dir.path());
        let old = store
            .execute(&request(Operation::DecisionRecord(decision_input(
                &project, None,
            ))))
            .unwrap();
        let old_id = old["id"].as_str().unwrap();
        let replacement = request(Operation::DecisionRecord(decision_input(
            &project,
            Some(old_id.into()),
        )));
        store.connection.execute_batch("CREATE TRIGGER fail_decision_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(store.execute(&replacement).is_err());
        assert_eq!(
            store
                .execute(&request(Operation::DecisionGet {
                    decision_id: old_id.into()
                }))
                .unwrap()["status"],
            "active"
        );
        assert_eq!(
            store
                .execute(&request(Operation::ProjectGet {
                    project_id: project.clone()
                }))
                .unwrap()["revision"],
            2
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_decision_event")
            .unwrap();
        let newer = store.execute(&replacement).unwrap();
        assert_eq!(newer["revision"], 3);
        assert_eq!(store.execute(&replacement).unwrap(), newer);
        let old = store
            .execute(&request(Operation::DecisionGet {
                decision_id: old_id.into(),
            }))
            .unwrap();
        assert_eq!(old["status"], "superseded");
        assert_eq!(old["superseded_by"], newer["id"]);
        let current = store
            .execute(&request(Operation::ContextGet {
                project_id: project.clone(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(
            current["active_decisions"]["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(current["active_decisions"]["items"][0]["id"], newer["id"]);
        assert!(matches!(
            store.execute(&request(replacement.operation.clone())),
            Err(Error::Conflict(_))
        ));
        let another_dir = dir.path().join("second");
        std::fs::create_dir(&another_dir).unwrap();
        let second = attach(&mut store, &another_dir);
        assert!(matches!(
            store.execute(&request(Operation::DecisionRecord(decision_input(
                &second,
                Some(newer["id"].as_str().unwrap().into())
            )))),
            Err(Error::Conflict(_))
        ));
        let history = store
            .execute(&request(Operation::DecisionsList {
                project_id: project,
                include_superseded: true,
                after: 0,
                limit: 1,
            }))
            .unwrap();
        assert_eq!(history["items"][0]["id"], old_id);
        assert!(history["next_cursor"].is_number());
    }

    fn legacy_database() -> (tempfile::TempDir, std::fs::File, Connection) {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let lock = crate::config::lock(dir.path()).unwrap();
        crate::config::private_file(&dir.path().join("mother.db")).unwrap();
        let connection = Connection::open(dir.path().join("mother.db")).unwrap();
        connection
            .execute_batch(include_str!("../migrations/001_foundation.sql"))
            .unwrap();
        (dir, lock, connection)
    }

    #[test]
    fn migrates_v1_without_losing_records_or_retry_responses() {
        let (dir, _lock, connection) = legacy_database();
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','old','/legacy',2)",[]).unwrap();
        connection.execute("INSERT INTO context_records(id,project_id,kind,content,source,revision) VALUES('old-context','legacy','fact','preserved','test',2)",[]).unwrap();
        let old = request(Operation::ContextRecord {
            project_id: "legacy".into(),
            kind: ContextKind::Fact,
            content: "preserved".into(),
            source: "test".into(),
            expected_revision: Some(1),
        });
        connection
            .execute(
                "INSERT INTO idempotency VALUES(?1,?2,?3)",
                params![
                    old.request_id,
                    serde_json::to_string(&old.operation).unwrap(),
                    r#"{"id":"old-context","revision":2}"#
                ],
            )
            .unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.execute(&old).unwrap(),
            json!({"id":"old-context","revision":2})
        );
        let context = store
            .execute(&request(Operation::ContextGet {
                project_id: "legacy".into(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(context["items"][0]["content"], "preserved");
        assert_eq!(context["revision"], 2);
        let version: u32 = store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn failed_migration_leaves_old_schema_intact() {
        let (dir, _lock, connection) = legacy_database();
        connection
            .execute("CREATE TABLE handoffs(conflicting TEXT)", [])
            .unwrap();
        drop(connection);
        assert!(Store::open(dir.path()).is_err());
        let connection = Connection::open(dir.path().join("mother.db")).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let decisions_exist: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='decisions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
        assert!(!decisions_exist);
    }

    #[test]
    fn handoff_bounds_immutability_integrity_and_audit_rollback() {
        let (dir, _lock, mut store) = fixture();
        let project = attach(&mut store, dir.path());
        for _ in 0..25 {
            store
                .execute(&request(Operation::ContextRecord {
                    project_id: project.clone(),
                    kind: ContextKind::Observation,
                    content: "\u{1}".repeat(4096),
                    source: "test".into(),
                    expected_revision: None,
                }))
                .unwrap();
        }
        let create = request(Operation::HandoffCreate(Box::new(handoff_input(&project))));
        store.connection.execute_batch("CREATE TRIGGER fail_handoff_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(store.execute(&create).is_err());
        let count: i64 = store
            .connection
            .query_row("SELECT count(*) FROM handoffs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            store
                .execute(&request(Operation::ProjectGet {
                    project_id: project.clone()
                }))
                .unwrap()["revision"],
            26
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_handoff_event")
            .unwrap();
        let created = store.execute(&create).unwrap();
        assert_eq!(created["snapshot_revision"], 26);
        assert_eq!(created["revision"], 27);
        let snapshot = &created["package"]["snapshot"];
        assert!(serde_json::to_vec(snapshot).unwrap().len() <= 192 * 1024);
        let kept = snapshot["context_records"].as_array().unwrap().len();
        assert!(kept > 0 && kept < 20);
        assert_eq!(snapshot["omitted_context_records"], 25 - kept);
        assert_eq!(created["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(
            serde_json::from_str::<Value>(created["package_json"].as_str().unwrap()).unwrap(),
            created["package"]
        );
        assert!(
            store
                .connection
                .execute("UPDATE handoffs SET source='modified'", [])
                .is_err()
        );
        assert!(
            store
                .connection
                .execute("DELETE FROM handoffs", [])
                .is_err()
        );
        let id = created["id"].as_str().unwrap().to_owned();
        let resume = request(Operation::HandoffResume {
            handoff_id: id.clone(),
            recipient: "new-host".into(),
            expected_revision: 27,
        });
        store.connection.execute_batch("CREATE TRIGGER fail_claim_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(store.execute(&resume).is_err());
        assert!(
            store
                .execute(&request(Operation::HandoffGet {
                    handoff_id: id.clone()
                }))
                .unwrap()["claim"]
                .is_null()
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_claim_event")
            .unwrap();
        let claim = store.execute(&resume).unwrap();
        assert_eq!(claim["claim"]["revision"], 28);
        assert!(
            store
                .connection
                .execute("DELETE FROM handoff_claims", [])
                .is_err()
        );
        store
            .connection
            .execute_batch("DROP TRIGGER handoffs_no_update; UPDATE handoffs SET sha256='corrupt'")
            .unwrap();
        assert!(matches!(
            store.execute(&request(Operation::HandoffGet { handoff_id: id })),
            Err(Error::Integrity(_))
        ));
    }

    #[test]
    fn schema_two_upgrade_preserves_old_handoff_bytes_and_retry_keys() {
        let (dir, _lock, connection) = legacy_database();
        connection
            .execute_batch(include_str!("../migrations/002_continuity.sql"))
            .unwrap();
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',2)",[]).unwrap();
        let encoded = r#"{"format_version":1,"id":"old-handoff","snapshot":{"unavailable_scopes":["task","persona"]}}"#;
        connection.execute("INSERT INTO handoffs(id,project_id,snapshot_revision,revision,package_json,sha256,source,created_at) VALUES('old-handoff','legacy',1,2,?1,?2,'old-client','2026-11-20T00:00:00Z')",params![encoded,crate::evidence::sha256(encoded.as_bytes())]).unwrap();
        let mut input = handoff_input("legacy");
        input.expected_revision = Some(1);
        let old_request = request(Operation::HandoffCreate(Box::new(input)));
        let operation = serde_json::to_string(&old_request.operation).unwrap();
        assert!(
            !operation.contains("task_id"),
            "new optional task scope must not change legacy retry payloads"
        );
        connection
            .execute(
                "INSERT INTO idempotency VALUES(?1,?2,?3)",
                params![
                    old_request.request_id,
                    operation,
                    r#"{"id":"old-handoff","revision":2}"#
                ],
            )
            .unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        let handoff = store
            .execute(&request(Operation::HandoffGet {
                handoff_id: "old-handoff".into(),
            }))
            .unwrap();
        assert_eq!(handoff["package_json"], encoded);
        assert_eq!(
            handoff["package"]["snapshot"]["unavailable_scopes"][0],
            "task"
        );
        assert_eq!(
            store.execute(&old_request).unwrap(),
            json!({"id":"old-handoff","revision":2})
        );
        let tasks = store
            .execute(&request(Operation::TasksList {
                project_id: "legacy".into(),
                state: None,
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(tasks["items"], json!([]));
    }

    #[test]
    fn schema_three_failure_rolls_back_new_tables_without_changing_schema_two() {
        let (dir, _lock, connection) = legacy_database();
        connection
            .execute_batch(include_str!("../migrations/002_continuity.sql"))
            .unwrap();
        connection
            .execute("CREATE TABLE task_checkpoints(collision TEXT)", [])
            .unwrap();
        drop(connection);
        assert!(Store::open(dir.path()).is_err());
        let connection = Connection::open(dir.path().join("mother.db")).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        let tasks_exist: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='tasks')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
        assert!(!tasks_exist);
    }

    #[test]
    fn byte_bounded_context_pages_preserve_the_complete_cursor_sequence() {
        let (dir, _lock, mut store) = fixture();
        let project = attach(&mut store, dir.path());
        for _ in 0..60 {
            store
                .execute(&request(Operation::ContextRecord {
                    project_id: project.clone(),
                    kind: ContextKind::Observation,
                    content: "\u{1}".repeat(4096),
                    source: "test".into(),
                    expected_revision: None,
                }))
                .unwrap();
        }
        let first = store
            .execute(&request(Operation::ContextGet {
                project_id: project.clone(),
                after: 0,
                limit: 100,
            }))
            .unwrap();
        assert!(serde_json::to_vec(&first).unwrap().len() < 1100 * 1024);
        let count = first["items"].as_array().unwrap().len();
        assert!(count > 0 && count < 60);
        let second = store
            .execute(&request(Operation::ContextGet {
                project_id: project,
                after: first["next_cursor"].as_u64().unwrap(),
                limit: 100,
            }))
            .unwrap();
        assert_eq!(count + second["items"].as_array().unwrap().len(), 60);
        assert!(second["next_cursor"].is_null());
        assert!(
            first["items"].as_array().unwrap().last().unwrap()["sequence"]
                .as_u64()
                .unwrap()
                < second["items"][0]["sequence"].as_u64().unwrap()
        );
    }

    #[test]
    fn schema_three_upgrade_preserves_tasks_and_adds_personas_and_github_identities() {
        let (dir, _lock, connection) = legacy_database();
        connection
            .execute_batch(include_str!("../migrations/002_continuity.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../migrations/003_tasks.sql"))
            .unwrap();
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',2)",[]).unwrap();
        connection.execute("INSERT INTO tasks(id,project_id,title,objective,source,acceptance_criteria,review_required,state,revision,project_revision) VALUES('task','legacy','Old task','Preserve me','old-client','[]',0,'created',1,2)",[]).unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        let health = store.execute(&request(Operation::Health)).unwrap();
        assert_eq!(health["schema_version"], SCHEMA_VERSION);
        assert_eq!(health["execution_leases_supported"], true);
        assert_eq!(health["personas_supported"], true);
        assert_eq!(health["github_identity_records_supported"], true);
        assert_eq!(health["github_credential_storage"], false);
        let task = store
            .execute(&request(Operation::TaskGet {
                task_id: "task".into(),
            }))
            .unwrap();
        assert_eq!(task["title"], "Old task");
        assert_eq!(task["latest_transition"]["persona_id"], Value::Null);
        let personas = store
            .execute(&request(Operation::PersonasList {
                status: None,
                project_id: None,
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(personas["items"], json!([]));
        let created = store
            .execute(&request(Operation::PersonaCreate(
                crate::personas::PersonaInput {
                    name: "review-engineer".into(),
                    role: "review_engineer".into(),
                    description: "Reviews changes independently".into(),
                    responsibilities: vec![],
                    prohibitions: vec![],
                    capabilities: vec![],
                    project_id: None,
                    preferred_runtime: None,
                    github_identity: None,
                    context_retrieval: None,
                    review_required: true,
                    max_concurrency: 1,
                    budget_units: None,
                },
            )))
            .unwrap();
        assert_eq!(created["revision"], 1);
    }

    #[test]
    fn schema_four_upgrade_preserves_personas_and_adds_github_identities() {
        let (dir, _lock, connection) = legacy_database();
        connection
            .execute_batch(include_str!("../migrations/002_continuity.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../migrations/003_tasks.sql"))
            .unwrap();
        connection
            .execute_batch(include_str!("../migrations/004_personas.sql"))
            .unwrap();
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',1)",[]).unwrap();
        connection.execute("INSERT INTO personas(id,name,role,description,responsibilities,prohibitions,capabilities,review_required,max_concurrency,status,revision) VALUES('persona','reviewer','review_engineer','Reviews changes','[]','[]','[\"github.read\"]',1,1,'active',1)",[]).unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.execute(&request(Operation::Health)).unwrap()["schema_version"],
            SCHEMA_VERSION
        );
        let persona = store
            .execute(&request(Operation::PersonaGet {
                persona_id: "persona".into(),
            }))
            .unwrap();
        assert_eq!(persona["name"], "reviewer");
        let identities = store
            .execute(&request(Operation::GithubIdentitiesList {
                status: None,
                persona_id: None,
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(identities["items"], json!([]));
    }

    #[test]
    fn schema_six_upgrade_preserves_worker_leases_and_adds_process_tables() {
        let (dir, _lock, connection) = legacy_database();
        for migration in [
            include_str!("../migrations/002_continuity.sql"),
            include_str!("../migrations/003_tasks.sql"),
            include_str!("../migrations/004_personas.sql"),
            include_str!("../migrations/005_github_identities.sql"),
            include_str!("../migrations/006_worker_leases.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',3)",[]).unwrap();
        connection.execute("INSERT INTO tasks(id,project_id,title,objective,source,acceptance_criteria,review_required,state,owner_label,revision,project_revision) VALUES('task','legacy','Leased task','Preserve lease','old-client','[{\"path\":\"artifact\"}]',0,'assigned','worker-a',1,3)",[]).unwrap();
        connection.execute("INSERT INTO worker_executions(id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at) VALUES('execution','task','legacy','worker-a','mock','leased',1,1,120,'2099-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z')",[]).unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        let health = store.execute(&request(Operation::Health)).unwrap();
        assert_eq!(health["schema_version"], SCHEMA_VERSION);
        let execution = store
            .execute(&request(Operation::WorkerExecutionGet {
                execution_id: "execution".into(),
            }))
            .unwrap();
        assert_eq!(execution["status"], "leased");
        assert_eq!(execution["workspace"], Value::Null);
        assert_eq!(execution["process"], Value::Null);
        assert_eq!(execution["task_state"], "assigned");
    }

    #[test]
    fn schema_seven_upgrade_preserves_processes_and_allows_lost_status() {
        let (dir, _lock, connection) = legacy_database();
        for migration in [
            include_str!("../migrations/002_continuity.sql"),
            include_str!("../migrations/003_tasks.sql"),
            include_str!("../migrations/004_personas.sql"),
            include_str!("../migrations/005_github_identities.sql"),
            include_str!("../migrations/006_worker_leases.sql"),
            include_str!("../migrations/007_worker_processes.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',3)",[]).unwrap();
        connection.execute("INSERT INTO tasks(id,project_id,title,objective,source,acceptance_criteria,review_required,state,owner_label,revision,project_revision) VALUES('task','legacy','Leased task','Preserve process','old-client','[{\"path\":\"artifact\"}]',0,'assigned','worker-a',1,3)",[]).unwrap();
        connection.execute("INSERT INTO worker_executions(id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at) VALUES('execution','task','legacy','worker-a','mock','running',3,1,120,'2099-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z')",[]).unwrap();
        connection.execute("INSERT INTO worker_processes(id,execution_id,pid,status,log_path) VALUES('process','execution',42,'running','/legacy/worker.log')",[]).unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        let health = store.execute(&request(Operation::Health)).unwrap();
        assert_eq!(health["schema_version"], SCHEMA_VERSION);
        let execution = store
            .execute(&request(Operation::WorkerExecutionGet {
                execution_id: "execution".into(),
            }))
            .unwrap();
        assert_eq!(execution["process"]["status"], "running");
        let abandoned = store
            .execute(&request(Operation::WorkerAbandon(
                crate::workers::WorkerAbandonInput {
                    execution_id: "execution".into(),
                    expected_revision: 3,
                },
            )))
            .unwrap();
        assert_eq!(abandoned["process"]["status"], "lost");
        assert_eq!(abandoned["task_state"], "blocked");
    }

    #[test]
    fn schema_eight_and_nine_upgrades_preserve_execution_graph_and_add_codex_adapter() {
        for version in [8, 9] {
            let (dir, _lock, connection) = legacy_database();
            for migration in [
                include_str!("../migrations/002_continuity.sql"),
                include_str!("../migrations/003_tasks.sql"),
                include_str!("../migrations/004_personas.sql"),
                include_str!("../migrations/005_github_identities.sql"),
                include_str!("../migrations/006_worker_leases.sql"),
                include_str!("../migrations/007_worker_processes.sql"),
                include_str!("../migrations/008_worker_process_loss.sql"),
            ] {
                connection.execute_batch(migration).unwrap();
            }
            if version == 9 {
                connection
                    .execute_batch(include_str!("../migrations/009_provider_adapter.sql"))
                    .unwrap();
            }
            connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',3)",[]).unwrap();
            connection.execute("INSERT INTO tasks(id,project_id,title,objective,source,acceptance_criteria,review_required,state,owner_label,revision,project_revision) VALUES('task','legacy','Provider task','Preserve graph','old-client','[{\"path\":\"artifact\"}]',0,'ready','worker-a',1,3)",[]).unwrap();
            connection.execute("INSERT INTO worker_executions(id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at,result) VALUES('execution','task','legacy','worker-a','mock','released',4,1,120,'2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z','{\"ok\":true}')",[]).unwrap();
            connection.execute("INSERT INTO worker_execution_events(id,execution_id,action,revision,details) VALUES('event','execution','released',4,'{\"source\":\"legacy\"}')",[]).unwrap();
            connection.execute("INSERT INTO worker_workspaces(id,execution_id,path,base_commit,status) VALUES('workspace','execution','/legacy/worktree','abc','ready')",[]).unwrap();
            connection.execute("INSERT INTO worker_processes(id,execution_id,pid,status,exit_code,log_path,evidence) VALUES('process','execution',42,'exited',0,'/legacy/worker.log','{\"legacy\":true}')",[]).unwrap();
            drop(connection);
            let mut store = Store::open(dir.path()).unwrap();
            let health = store.execute(&request(Operation::Health)).unwrap();
            assert_eq!(health["schema_version"], SCHEMA_VERSION);
            assert_eq!(health["worker_adapter"], "mock_codex_and_opencode_cli");
            assert_eq!(health["provider_workers_supported"], true);
            assert_eq!(health["provider_credentials_stored"], false);
            assert_eq!(health["provider_credentials_brokered"], false);
            assert_eq!(
                health["provider_budget_basis"],
                "provider_reported_cost_and_steps"
            );
            assert_eq!(health["provider_spend_independently_metered"], false);
            assert_eq!(
                health["worker_project_repository_mutation"],
                "not_guaranteed"
            );
            let execution = store
                .execute(&request(Operation::WorkerExecutionGet {
                    execution_id: "execution".into(),
                }))
                .unwrap();
            assert_eq!(execution["adapter"], "mock");
            assert_eq!(execution["result"], json!({"ok": true}));
            assert_eq!(execution["workspace"]["path"], "/legacy/worktree");
            assert_eq!(execution["process"]["status"], "exited");
            assert_eq!(execution["process"]["evidence"], json!({"legacy": true}));
            let leased = store
                .execute(&request(Operation::WorkerLease(
                    crate::workers::WorkerLeaseInput {
                        task_id: "task".into(),
                        worker_label: "worker-b".into(),
                        adapter: crate::workers::WorkerAdapter::Codex,
                        lease_seconds: 120,
                        max_active: 1,
                        expected_task_revision: None,
                        expected_project_revision: None,
                    },
                )))
                .unwrap();
            assert_eq!(leased["adapter"], "codex");
        }
    }

    #[test]
    fn schema_five_upgrade_preserves_identities_and_adds_worker_leases() {
        let (dir, _lock, connection) = legacy_database();
        for migration in [
            include_str!("../migrations/002_continuity.sql"),
            include_str!("../migrations/003_tasks.sql"),
            include_str!("../migrations/004_personas.sql"),
            include_str!("../migrations/005_github_identities.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection.execute("INSERT INTO projects(id,name,repository,revision) VALUES('legacy','legacy','/legacy',1)",[]).unwrap();
        connection.execute("INSERT INTO personas(id,name,role,description,responsibilities,prohibitions,capabilities,review_required,max_concurrency,status,revision) VALUES('persona','reviewer','review_engineer','Reviews changes','[]','[]','[\"github.read\"]',1,1,'active',1)",[]).unwrap();
        connection.execute("INSERT INTO github_identities(id,host,account_id,login,profile_id,persona_id,persona_revision,operations,verification_method,status,revision,verified_at) VALUES('identity','github.com',42,'reviewer','profile','persona',1,'[\"read\"]','gh_api_user','active',1,'2026-11-20T00:00:00Z')",[]).unwrap();
        drop(connection);
        let mut store = Store::open(dir.path()).unwrap();
        let health = store.execute(&request(Operation::Health)).unwrap();
        assert_eq!(health["schema_version"], SCHEMA_VERSION);
        assert_eq!(health["workers_supported"], true);
        assert_eq!(health["worker_process_supervision"], true);
        assert_eq!(health["worker_worktree_isolation"], true);
        assert_eq!(health["execution_leases_supported"], true);
        let identities = store
            .execute(&request(Operation::GithubIdentitiesList {
                status: None,
                persona_id: None,
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(identities["items"][0]["id"], "identity");
        let eligible = store
            .execute(&request(Operation::WorkerEligible {
                project_id: Some("legacy".into()),
                limit: 20,
            }))
            .unwrap();
        assert_eq!(eligible["items"], json!([]));
    }
}
