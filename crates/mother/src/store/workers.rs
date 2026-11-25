use super::{event, page, pagination, tasks};
use crate::{
    Error, Result,
    protocol::Request,
    tasks::{CheckpointInput, TaskState, TaskTransition},
    workers::{
        WorkerAbandonInput, WorkerAdapter, WorkerExecutionStatus, WorkerHeartbeatInput,
        WorkerLeaseInput, WorkerMockInput, WorkerProcessAttachInput, WorkerProcessFinishInput,
        WorkerProcessOutcome, WorkerReleaseInput, WorkerReleaseOutcome,
        WorkerWorkspaceCleanupInput, WorkerWorkspaceInput,
    },
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const EXECUTION_COLUMNS: &str = "e.sequence,e.id,e.task_id,e.project_id,e.worker_label,e.adapter,e.status,e.revision,e.attempt,e.lease_seconds,e.lease_expires_at,e.heartbeat_at,e.result,e.created_at,e.updated_at,t.state,t.revision,t.project_revision,t.owner_label,(e.status='expired' OR (e.status IN ('leased','running') AND e.lease_expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now'))),w.path,w.base_commit,w.status,p.pid,p.status,p.exit_code,p.log_path,p.evidence,p.started_at,p.ended_at,p.updated_at";
const EXECUTION_FROM: &str = "FROM worker_executions e JOIN tasks t ON t.id=e.task_id LEFT JOIN worker_workspaces w ON w.execution_id=e.id LEFT JOIN worker_processes p ON p.execution_id=e.id";

fn parsed_json(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn execution_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let status = r.get::<_, String>(6)?;
    let expired = r.get::<_, bool>(19)?;
    let active = matches!(status.as_str(), "leased" | "running") && !expired;
    let workspace_path = r.get::<_, Option<String>>(20)?;
    let workspace = match workspace_path {
        Some(path) => json!({
            "path":path,
            "base_commit":r.get::<_,String>(21)?,
            "status":r.get::<_,String>(22)?,
        }),
        None => Value::Null,
    };
    let pid = r.get::<_, Option<i64>>(23)?;
    let process = match pid {
        Some(pid) => json!({
            "pid":pid,
            "status":r.get::<_,String>(24)?,
            "exit_code":r.get::<_,Option<i64>>(25)?,
            "log_path":r.get::<_,String>(26)?,
            "evidence":r.get::<_,Option<String>>(27)?.map(parsed_json).transpose()?,
            "started_at":r.get::<_,String>(28)?,
            "ended_at":r.get::<_,Option<String>>(29)?,
            "updated_at":r.get::<_,String>(30)?,
            "trust":"owner_run_cli_process_observation",
        }),
        None => Value::Null,
    };
    let adapter = r.get::<_, String>(5)?;
    let result = r
        .get::<_, Option<String>>(12)?
        .map(parsed_json)
        .transpose()?;
    let mock = adapter == "mock";
    let repository_mutation = if mock {
        json!(false)
    } else {
        result
            .as_ref()
            .and_then(|value| value.get("repository_mutation"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let worktree_mutation = if mock {
        json!(pid.is_some())
    } else {
        result
            .as_ref()
            .and_then(|value| value.get("worktree_mutation"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    Ok(json!({
        "sequence":r.get::<_,i64>(0)?,
        "id":r.get::<_,String>(1)?,
        "task_id":r.get::<_,String>(2)?,
        "project_id":r.get::<_,String>(3)?,
        "worker_label":r.get::<_,String>(4)?,
        "adapter":adapter,
        "status":status,
        "revision":r.get::<_,i64>(7)?,
        "attempt":r.get::<_,i64>(8)?,
        "lease_seconds":r.get::<_,i64>(9)?,
        "lease_expires_at":r.get::<_,String>(10)?,
        "heartbeat_at":r.get::<_,String>(11)?,
        "result":result,
        "created_at":r.get::<_,String>(13)?,
        "updated_at":r.get::<_,String>(14)?,
        "task_state":r.get::<_,String>(15)?,
        "task_revision":r.get::<_,i64>(16)?,
        "project_revision":r.get::<_,i64>(17)?,
        "task_owner_label":r.get::<_,Option<String>>(18)?,
        "expired":expired,
        "active":active,
        "lease_trust":"coordination_only",
        "workspace":workspace,
        "process":process,
        "process_supervised":pid.is_some(),
        "repository_mutation":repository_mutation,
        "worktree_mutation":worktree_mutation
    }))
}

fn execution(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    tx.query_row(
        &format!("SELECT {EXECUTION_COLUMNS} {EXECUTION_FROM} WHERE e.id=?1"),
        [id],
        execution_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("worker execution {id} does not exist")))
}

pub(super) fn for_task(tx: &Transaction<'_>, id: &str) -> Result<Option<Value>> {
    Ok(tx
        .query_row(
            &format!("SELECT {EXECUTION_COLUMNS} {EXECUTION_FROM} WHERE e.task_id=?1 AND e.status IN ('leased','running') ORDER BY e.sequence DESC LIMIT 1"),
            [id],
            execution_row,
        )
        .optional()?)
}

fn current(tx: &Transaction<'_>) -> Result<String> {
    Ok(
        tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
            r.get(0)
        })?,
    )
}

fn expire_modifier(seconds: u32) -> String {
    format!("+{seconds} seconds")
}

fn validate_seconds(seconds: u32) -> Result<()> {
    if !(1..=3600).contains(&seconds) {
        return Err(Error::Invalid("lease_seconds must be in 1..=3600".into()));
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<()> {
    super::text(label, "worker_label", 128)
}

fn validate_summary(summary: &str, name: &str) -> Result<()> {
    super::text(summary, name, 4096)
}

fn valid_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn worker_root(dir: &Path, execution_id: &str) -> Result<PathBuf> {
    let valid = Uuid::parse_str(execution_id).is_ok_and(|id| id.to_string() == execution_id);
    if !valid {
        return Err(Error::Invalid(
            "execution_id must be a canonical lowercase UUID".into(),
        ));
    }
    let dir = dir.canonicalize()?;
    let root = dir.join("workers").join(execution_id);
    crate::config::check_private_dir(&root)?;
    Ok(root)
}

fn worktree_path(dir: &Path, execution_id: &str) -> Result<PathBuf> {
    let root = worker_root(dir, execution_id)?;
    let path = root.join("worktree");
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Invalid(
            "worker worktree must be a real directory".into(),
        ));
    }
    let canonical = path.canonicalize()?;
    if canonical.parent() != Some(root.as_path()) {
        return Err(Error::Invalid(
            "worker worktree escapes the execution directory".into(),
        ));
    }
    Ok(canonical)
}

fn process_status(tx: &Transaction<'_>, execution_id: &str) -> Result<Option<String>> {
    Ok(tx
        .query_row(
            "SELECT status FROM worker_processes WHERE execution_id=?1",
            [execution_id],
            |r| r.get(0),
        )
        .optional()?)
}

fn require_process_owner(value: &Value) -> Result<()> {
    if value["task_owner_label"].as_str() != value["worker_label"].as_str() {
        return Err(Error::Conflict(
            "task owner no longer matches the worker lease".into(),
        ));
    }
    Ok(())
}

fn extend_lease(tx: &Transaction<'_>, execution_id: &str, seconds: u32) -> Result<()> {
    tx.execute(
        "UPDATE worker_executions SET lease_seconds=?1,lease_expires_at=strftime('%Y-%m-%dT%H:%M:%fZ','now',?2),heartbeat_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![seconds, expire_modifier(seconds), execution_id],
    )?;
    Ok(())
}

fn record_event(
    tx: &Transaction<'_>,
    request: &Request,
    execution: &Value,
    action: &str,
    details: Value,
) -> Result<()> {
    let id = execution["id"].as_str().unwrap();
    let revision = execution["revision"].as_i64().unwrap();
    let mut details = details;
    details["request_id"] = json!(request.request_id);
    details["execution"] = execution.clone();
    tx.execute(
        "INSERT INTO worker_execution_events(id,execution_id,action,revision,details) VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            id,
            action,
            revision,
            serde_json::to_string(&details)?
        ],
    )?;
    let project = execution["project_id"].as_str().unwrap();
    let project_revision = execution["project_revision"].as_i64().unwrap();
    event(
        tx,
        request,
        project,
        id,
        action,
        "worker-lease",
        project_revision,
    )?;
    Ok(())
}

fn expire_internal(tx: &Transaction<'_>, request: &Request, id: &str) -> Result<Value> {
    let value = execution(tx, id)?;
    if !matches!(value["status"].as_str(), Some("leased" | "running")) {
        return Err(Error::Conflict(
            "only an active worker execution can expire".into(),
        ));
    }
    let next = value["revision"].as_i64().unwrap() + 1;
    tx.execute(
        "UPDATE worker_executions SET status='expired',revision=?1,result=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            next,
            serde_json::to_string(&json!({"reason":"lease_expired","expired_at":current(tx)?}))?,
            id
        ],
    )?;
    let process = process_status(tx, id)?;
    let mut task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    let mut requeued = false;
    let mut blocked = false;
    if process.as_deref() == Some("running") {
        if matches!(task["state"].as_str(), Some("assigned" | "running")) {
            task = tasks::transition(
                tx,
                request,
                &TaskTransition {
                    task_id: task["id"].as_str().unwrap().into(),
                    state: TaskState::Blocked,
                    source: "worker-recovery".into(),
                    reason:
                        "worker lease expired while the supervised process may still be running"
                            .into(),
                    owner_label: None,
                    persona: None,
                    expected_task_revision: task["task_revision"].as_u64().unwrap(),
                    expected_project_revision: None,
                },
            )?;
            blocked = true;
        }
    } else if task["state"] == "assigned" {
        task = tasks::transition(
            tx,
            request,
            &TaskTransition {
                task_id: task["id"].as_str().unwrap().into(),
                state: TaskState::Ready,
                source: "worker-recovery".into(),
                reason: "worker lease expired before execution started".into(),
                owner_label: None,
                persona: None,
                expected_task_revision: task["task_revision"].as_u64().unwrap(),
                expected_project_revision: None,
            },
        )?;
        requeued = true;
    } else if task["state"] == "running" {
        task = tasks::transition(
            tx,
            request,
            &TaskTransition {
                task_id: task["id"].as_str().unwrap().into(),
                state: TaskState::AwaitingInput,
                source: "worker-recovery".into(),
                reason: "worker lease expired during execution".into(),
                owner_label: None,
                persona: None,
                expected_task_revision: task["task_revision"].as_u64().unwrap(),
                expected_project_revision: None,
            },
        )?;
        task = tasks::transition(
            tx,
            request,
            &TaskTransition {
                task_id: task["id"].as_str().unwrap().into(),
                state: TaskState::Ready,
                source: "worker-recovery".into(),
                reason: "expired execution returned to the scheduler queue".into(),
                owner_label: None,
                persona: None,
                expected_task_revision: task["task_revision"].as_u64().unwrap(),
                expected_project_revision: None,
            },
        )?;
        requeued = true;
    }
    let current = execution(tx, id)?;
    record_event(
        tx,
        request,
        &current,
        "worker.expired",
        json!({"reason":"lease_expired","task_requeued":requeued,"task_blocked":blocked,"task_state":task["state"]}),
    )?;
    Ok(current)
}

pub(super) fn lease(
    tx: &Transaction<'_>,
    request: &Request,
    input: &WorkerLeaseInput,
) -> Result<Value> {
    validate_label(&input.worker_label)?;
    validate_seconds(input.lease_seconds)?;
    if !(1..=64).contains(&input.max_active) {
        return Err(Error::Invalid("max_active must be in 1..=64".into()));
    }
    let mut task = tasks::get(tx, &input.task_id)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM worker_executions WHERE task_id=?1 AND status IN ('leased','running') ORDER BY sequence DESC LIMIT 1",
            [&input.task_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        let value = execution(tx, &id)?;
        if value["expired"] == true {
            expire_internal(tx, request, &id)?;
            task = tasks::get(tx, &input.task_id)?;
        } else {
            return Err(Error::Conflict(
                "task already has an unexpired worker lease".into(),
            ));
        }
    }
    if task["state"] != "ready" {
        return Err(Error::Conflict("worker leases require a ready task".into()));
    }
    let active: i64 = tx.query_row(
        "SELECT count(*) FROM worker_executions WHERE status IN ('leased','running') AND lease_expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        [],
        |r| r.get(0),
    )?;
    if active >= input.max_active as i64 {
        return Err(Error::Conflict(format!(
            "worker concurrency limit reached; {active} active leases"
        )));
    }
    let project = task["project_id"].as_str().unwrap();
    let assigned = tasks::transition(
        tx,
        request,
        &TaskTransition {
            task_id: input.task_id.clone(),
            state: TaskState::Assigned,
            source: "worker-lease".into(),
            reason: format!("execution lease acquired by {}", input.worker_label),
            owner_label: Some(input.worker_label.clone()),
            persona: None,
            expected_task_revision: input
                .expected_task_revision
                .unwrap_or_else(|| task["task_revision"].as_u64().unwrap()),
            expected_project_revision: input.expected_project_revision,
        },
    )?;
    let attempt: i64 = tx.query_row(
        "SELECT count(*) + 1 FROM worker_executions WHERE task_id=?1",
        [&input.task_id],
        |r| r.get(0),
    )?;
    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO worker_executions(id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at) VALUES(?1,?2,?3,?4,?5,'leased',1,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now',?8),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            id,
            input.task_id,
            project,
            input.worker_label,
            input.adapter.as_str(),
            attempt,
            input.lease_seconds,
            expire_modifier(input.lease_seconds)
        ],
    )?;
    let value = execution(tx, &id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.leased",
        json!({"task_revision":assigned["task_revision"],"project_revision":assigned["project_revision"],"adapter":input.adapter.as_str()}),
    )?;
    execution(tx, &id)
}

pub(super) fn heartbeat(
    tx: &Transaction<'_>,
    request: &Request,
    input: &WorkerHeartbeatInput,
) -> Result<Value> {
    validate_seconds(input.lease_seconds)?;
    let value = execution(tx, &input.execution_id)?;
    if !matches!(value["status"].as_str(), Some("leased" | "running")) {
        return Err(Error::Conflict("worker execution is not active".into()));
    }
    if value["expired"] == true {
        return Err(Error::Conflict(
            "worker lease expired; recover it before retrying".into(),
        ));
    }
    let current = value["revision"].as_u64().unwrap();
    if input.expected_revision != current {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {current}"
        )));
    }
    tx.execute(
        "UPDATE worker_executions SET revision=revision+1,lease_seconds=?1,lease_expires_at=strftime('%Y-%m-%dT%H:%M:%fZ','now',?2),heartbeat_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            input.lease_seconds,
            expire_modifier(input.lease_seconds),
            input.execution_id
        ],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(tx, request, &value, "worker.heartbeat", json!({}))?;
    execution(tx, &input.execution_id)
}

pub(super) fn run_mock(
    tx: &Transaction<'_>,
    request: &Request,
    input: &WorkerMockInput,
) -> Result<Value> {
    validate_seconds(input.lease_seconds)?;
    let value = execution(tx, &input.execution_id)?;
    if value["adapter"] != WorkerAdapter::Mock.as_str() {
        return Err(Error::Conflict(
            "the checkpoint mock operation requires the mock adapter".into(),
        ));
    }
    if !matches!(value["status"].as_str(), Some("leased" | "running")) {
        return Err(Error::Conflict("worker execution is not active".into()));
    }
    if value["expired"] == true {
        return Err(Error::Conflict(
            "worker lease expired; recover it before running".into(),
        ));
    }
    if !value["process"].is_null() {
        return Err(Error::Conflict(
            "worker execution is supervised by a process; use process operations".into(),
        ));
    }
    let current = value["revision"].as_u64().unwrap();
    if input.expected_revision != current {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {current}"
        )));
    }
    validate_summary(&input.summary, "summary")?;
    if input.work_remaining.len() > 16 || input.blockers.len() > 16 {
        return Err(Error::Invalid(
            "work_remaining and blockers allow at most 16 entries".into(),
        ));
    }
    for item in input.work_remaining.iter().chain(&input.blockers) {
        super::text(item, "work item", 1024)?;
    }
    let mut task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    if task["owner_label"].as_str() != value["worker_label"].as_str() {
        return Err(Error::Conflict(
            "task owner no longer matches the worker lease".into(),
        ));
    }
    match task["state"].as_str() {
        Some("assigned") => {
            task = tasks::transition(
                tx,
                request,
                &TaskTransition {
                    task_id: task["id"].as_str().unwrap().into(),
                    state: TaskState::Running,
                    source: "mock-worker".into(),
                    reason: "deterministic mock runner started".into(),
                    owner_label: None,
                    persona: None,
                    expected_task_revision: task["task_revision"].as_u64().unwrap(),
                    expected_project_revision: None,
                },
            )?;
        }
        Some("running") => {}
        _ => {
            return Err(Error::Conflict(
                "mock runner requires an assigned or running task".into(),
            ));
        }
    }
    let checkpoint = tasks::checkpoint(
        tx,
        request,
        &CheckpointInput {
            task_id: task["id"].as_str().unwrap().into(),
            summary: input.summary.clone(),
            source: "mock-worker".into(),
            work_remaining: input.work_remaining.clone(),
            blockers: input.blockers.clone(),
            reported_test_results: vec![
                "No tests were executed or verified by the mock runner".into(),
            ],
            persona: None,
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        },
    )?;
    tx.execute(
        "UPDATE worker_executions SET status='running',revision=revision+1,lease_seconds=?1,lease_expires_at=strftime('%Y-%m-%dT%H:%M:%fZ','now',?2),heartbeat_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            input.lease_seconds,
            expire_modifier(input.lease_seconds),
            input.execution_id
        ],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.mock_checkpoint",
        json!({"task_revision":checkpoint["task_revision"],"repository_mutation":false,"completion":false}),
    )?;
    Ok(
        json!({"execution":execution(tx, &input.execution_id)?,"task":checkpoint,
        "limits":{"repository_mutation":false,"completion":false,"test_execution":false}}),
    )
}

pub(super) fn release(
    tx: &Transaction<'_>,
    request: &Request,
    input: &WorkerReleaseInput,
) -> Result<Value> {
    if let Some(summary) = &input.summary {
        validate_summary(summary, "summary")?;
    }
    let value = execution(tx, &input.execution_id)?;
    if !matches!(value["status"].as_str(), Some("leased" | "running")) {
        return Err(Error::Conflict(
            "only an active worker execution can be released".into(),
        ));
    }
    if value["expired"] == true {
        return Err(Error::Conflict(
            "worker lease expired; recover it instead of releasing".into(),
        ));
    }
    if value["process"]["status"] == "running" {
        return Err(Error::Conflict(
            "supervised worker process is still running".into(),
        ));
    }
    let current = value["revision"].as_u64().unwrap();
    if input.expected_revision != current {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {current}"
        )));
    }
    if input.outcome == WorkerReleaseOutcome::Completed && value["task_state"] != "completed" {
        return Err(Error::Conflict(
            "completed execution requires a completed task".into(),
        ));
    }
    tx.execute(
        "UPDATE worker_executions SET status=?1,revision=revision+1,result=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            input.outcome.status().as_str(),
            serde_json::to_string(&json!({"outcome":input.outcome.as_str(),"summary":input.summary}))?,
            input.execution_id
        ],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.released",
        json!({"outcome":input.outcome.as_str()}),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn recover(tx: &Transaction<'_>, request: &Request) -> Result<Value> {
    let ids = tx
        .prepare("SELECT id FROM worker_executions WHERE status IN ('leased','running') AND lease_expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now') ORDER BY sequence")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut expired = Vec::with_capacity(ids.len());
    for id in &ids {
        expired.push(expire_internal(tx, request, id)?);
    }
    Ok(json!({
        "count":expired.len(),
        "expired":expired,
        "recovery":"task_requeued_when_assigned_or_running",
        "repository_mutation":false
    }))
}

pub(super) fn workspace_start(
    tx: &Transaction<'_>,
    dir: &Path,
    request: &Request,
    input: &WorkerWorkspaceInput,
) -> Result<Value> {
    if !valid_commit(&input.base_commit) {
        return Err(Error::Invalid(
            "base_commit must be a lowercase 40- or 64-character Git object ID".into(),
        ));
    }
    let value = execution(tx, &input.execution_id)?;
    if value["revision"].as_u64() != Some(input.expected_revision) {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {}",
            value["revision"]
        )));
    }
    if !value["active"].as_bool().unwrap_or(false) {
        return Err(Error::Conflict(
            "worker execution is not active or its lease expired".into(),
        ));
    }
    if !value["workspace"].is_null() {
        return Err(Error::Conflict(
            "worker execution already has a workspace".into(),
        ));
    }
    if !value["process"].is_null() {
        return Err(Error::Conflict(
            "worker execution already has a process".into(),
        ));
    }
    require_process_owner(&value)?;
    let path = worktree_path(dir, &input.execution_id)?;
    let path = path
        .to_str()
        .ok_or_else(|| Error::Invalid("worker worktree path must be UTF-8".into()))?;
    tx.execute(
        "INSERT INTO worker_workspaces(id,execution_id,path,base_commit,status) VALUES(?1,?2,?3,?4,'ready')",
        params![
            Uuid::new_v4().to_string(),
            input.execution_id,
            path,
            input.base_commit
        ],
    )?;
    let seconds = value["lease_seconds"].as_u64().unwrap() as u32;
    extend_lease(tx, &input.execution_id, seconds)?;
    tx.execute(
        "UPDATE worker_executions SET revision=revision+1 WHERE id=?1",
        [&input.execution_id],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.workspace_prepared",
        json!({"base_commit":input.base_commit,"path":path,"repository_mutation":false}),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn process_attach(
    tx: &Transaction<'_>,
    dir: &Path,
    request: &Request,
    input: &WorkerProcessAttachInput,
) -> Result<Value> {
    validate_seconds(input.lease_seconds)?;
    if input.pid <= 1 || input.pid > i32::MAX as u32 {
        return Err(Error::Invalid(
            "pid must be between 2 and the platform pid maximum".into(),
        ));
    }
    let value = execution(tx, &input.execution_id)?;
    if value["revision"].as_u64() != Some(input.expected_revision) {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {}",
            value["revision"]
        )));
    }
    if !value["active"].as_bool().unwrap_or(false) {
        return Err(Error::Conflict(
            "worker execution is not active or its lease expired".into(),
        ));
    }
    if value["workspace"]["status"] != "ready" {
        return Err(Error::Conflict(
            "worker process requires a ready workspace".into(),
        ));
    }
    if !value["process"].is_null() {
        return Err(Error::Conflict(
            "worker execution already has a process".into(),
        ));
    }
    if value["task_state"] != "assigned" {
        return Err(Error::Conflict(
            "worker process requires an assigned task".into(),
        ));
    }
    require_process_owner(&value)?;
    let adapter = value["adapter"].as_str().unwrap_or("mock").to_owned();
    let task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    let (source, reason) = if adapter != WorkerAdapter::Mock.as_str() {
        (
            "provider-process",
            "owner-run supervisor attached a provider process",
        )
    } else {
        (
            "mock-process",
            "owner-run supervisor attached a deterministic mock process",
        )
    };
    tasks::transition(
        tx,
        request,
        &TaskTransition {
            task_id: task["id"].as_str().unwrap().into(),
            state: TaskState::Running,
            source: source.into(),
            reason: reason.into(),
            owner_label: None,
            persona: None,
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        },
    )?;
    let root = worker_root(dir, &input.execution_id)?;
    let log_path = root.join("worker.log");
    let log_path = log_path
        .to_str()
        .ok_or_else(|| Error::Invalid("worker log path must be UTF-8".into()))?;
    tx.execute(
        "INSERT INTO worker_processes(id,execution_id,pid,status,log_path) VALUES(?1,?2,?3,'running',?4)",
        params![
            Uuid::new_v4().to_string(),
            input.execution_id,
            input.pid,
            log_path
        ],
    )?;
    tx.execute(
        "UPDATE worker_executions SET status='running',revision=revision+1 WHERE id=?1",
        [&input.execution_id],
    )?;
    extend_lease(tx, &input.execution_id, input.lease_seconds)?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.process_attached",
        json!({"pid":input.pid,"log_path":log_path,"process_supervised":"owner_run_cli","adapter":adapter}),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn process_finish(
    tx: &Transaction<'_>,
    dir: &Path,
    request: &Request,
    input: &WorkerProcessFinishInput,
) -> Result<Value> {
    let value = execution(tx, &input.execution_id)?;
    if value["revision"].as_u64() != Some(input.expected_revision) {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {}",
            value["revision"]
        )));
    }
    if value["process"]["status"] != "running" {
        return Err(Error::Conflict(
            "worker process is not running or was already finalized".into(),
        ));
    }
    if value["workspace"]["status"] != "ready" {
        return Err(Error::Conflict(
            "worker process requires a ready workspace".into(),
        ));
    }
    let stored = Path::new(value["workspace"]["path"].as_str().unwrap());
    let current = worktree_path(dir, &input.execution_id)?;
    if stored != current {
        return Err(Error::Conflict(
            "worker workspace no longer matches the recorded path".into(),
        ));
    }
    let adapter = value["adapter"].as_str().unwrap_or("mock").to_owned();
    let provider = input.provider.clone().unwrap_or(Value::Null);
    if let Some(provider) = &input.provider {
        if !provider.is_object() {
            return Err(Error::Invalid(
                "provider metadata must be a JSON object".into(),
            ));
        }
        if serde_json::to_vec(provider)?.len() > 32 * 1024 {
            return Err(Error::Invalid(
                "provider metadata must be at most 32768 bytes".into(),
            ));
        }
    }
    if adapter == WorkerAdapter::Mock.as_str() && input.provider.is_some() {
        return Err(Error::Invalid(
            "mock process completion must not include provider metadata".into(),
        ));
    }
    if adapter != WorkerAdapter::Mock.as_str() && input.provider.is_none() {
        return Err(Error::Invalid(
            "provider process completion requires provider metadata".into(),
        ));
    }
    let task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    let criteria = crate::tasks::file_constraints(&task)?;
    let evidence = crate::evidence::observe(&current, &criteria);
    // Paths are discovered by the supervisor's bounded Git observation, then files
    // are independently read by the runtime with the usual anchored verifier.
    let mut discovered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(changes) = provider["workspace_changes"].as_array() {
        for change in changes.iter().take(64) {
            if change["path_truncated"] == true {
                continue;
            }
            if let Some(path) = change["path"].as_str()
                && crate::evidence::valid_path(path)
                && seen.insert(path.to_owned())
            {
                discovered.push(crate::tasks::ArtifactCriterion {
                    path: path.into(),
                    expected_sha256: None,
                });
            }
        }
    }
    let discovered_artifacts = crate::evidence::observe(&current, &discovered);

    let succeeded =
        input.outcome == WorkerProcessOutcome::Exited && input.exit_code.unwrap_or(0) == 0;
    let (source, success_reason, failure_reason) = if adapter != WorkerAdapter::Mock.as_str() {
        (
            "provider-process",
            "Provider process exited successfully; host review is required before validation",
            "Provider process did not exit successfully; triage before retry",
        )
    } else {
        (
            "mock-process",
            "mock process exited successfully; host review is required before validation",
            "mock process did not exit successfully; triage before retry",
        )
    };
    if value["task_state"] == "running" {
        let (state, reason) = if succeeded {
            (TaskState::AwaitingInput, success_reason)
        } else {
            (TaskState::Failed, failure_reason)
        };
        tasks::transition(
            tx,
            request,
            &TaskTransition {
                task_id: task["id"].as_str().unwrap().into(),
                state,
                source: source.into(),
                reason: reason.into(),
                owner_label: None,
                persona: None,
                expected_task_revision: task["task_revision"].as_u64().unwrap(),
                expected_project_revision: None,
            },
        )?;
    }
    tx.execute(
        "UPDATE worker_processes SET status=?1,exit_code=?2,evidence=?3,ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE execution_id=?4",
        params![
            input.outcome.as_str(),
            input.exit_code.map(i64::from),
            serde_json::to_string(&evidence)?,
            input.execution_id
        ],
    )?;
    let repository_mutation = if adapter == WorkerAdapter::Mock.as_str() {
        json!(false)
    } else {
        Value::Null
    };
    let worktree_mutation = if adapter == WorkerAdapter::Mock.as_str() {
        json!(true)
    } else {
        provider
            .get("worktree_mutation")
            .cloned()
            .unwrap_or(Value::Null)
    };
    let result = json!({
        "discovered_artifacts":discovered_artifacts,
        "artifact_discovery_scope":"bounded_supervisor_git_paths_runtime_file_observations",
        "outcomes_verified":false,
        "adapter":adapter,
        "outcome":input.outcome.as_str(),
        "exit_code":input.exit_code,
        "evidence":evidence,
        "provider":provider,
        "repository_mutation":repository_mutation,
        "worktree_mutation":worktree_mutation,
        "completion":false
    });
    let next_status = if value["expired"] == true {
        "expired"
    } else if succeeded {
        "released"
    } else {
        "failed"
    };
    tx.execute(
        "UPDATE worker_executions SET status=?1,revision=revision+1,result=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            next_status,
            serde_json::to_string(&result)?,
            input.execution_id
        ],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.process_finished",
        json!({"adapter":adapter,"outcome":input.outcome.as_str(),"exit_code":input.exit_code,"evidence":evidence,"provider_session_id":provider.get("session_id"),"budget_exceeded":provider.get("budget_exceeded"),"repository_mutation":repository_mutation,"completion":false}),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn abandon(
    tx: &Transaction<'_>,
    request: &Request,
    input: &WorkerAbandonInput,
) -> Result<Value> {
    let value = execution(tx, &input.execution_id)?;
    if value["revision"].as_u64() != Some(input.expected_revision) {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {}",
            value["revision"]
        )));
    }
    let process = value["process"]["status"].as_str().map(str::to_owned);
    let process_running = process.as_deref() == Some("running");
    if !value["active"].as_bool().unwrap_or(false) && !process_running {
        return Err(Error::Conflict(
            "worker execution is not active and has no running process to abandon".into(),
        ));
    }
    if process_running {
        tx.execute(
            "UPDATE worker_processes SET status='lost',ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE execution_id=?1 AND status='running'",
            [&input.execution_id],
        )?;
    }
    let task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    if matches!(task["state"].as_str(), Some("assigned" | "running")) {
        tasks::transition(
            tx,
            request,
            &TaskTransition {
                task_id: task["id"].as_str().unwrap().into(),
                state: TaskState::Blocked,
                source: "worker-abandon".into(),
                reason: "owner abandoned worker supervision; reconcile the process and worktree before retry"
                    .into(),
                owner_label: None,
                persona: None,
                expected_task_revision: task["task_revision"].as_u64().unwrap(),
                expected_project_revision: None,
            },
        )?;
    }
    let result = json!({
        "reason":"owner_abandoned_supervision",
        "process_lost":process_running,
        "process_signal_sent":false,
        "repository_mutation":false,
        "completion":false
    });
    let status = if value["expired"] == true {
        "expired"
    } else {
        "failed"
    };
    tx.execute(
        "UPDATE worker_executions SET status=?1,revision=revision+1,result=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![
            status,
            serde_json::to_string(&result)?,
            input.execution_id
        ],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.abandoned",
        json!({
            "process_status":process,
            "process_lost":process_running,
            "process_signal_sent":false,
            "repository_mutation":false,
            "completion":false
        }),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn workspace_cleanup(
    tx: &Transaction<'_>,
    dir: &Path,
    request: &Request,
    input: &WorkerWorkspaceCleanupInput,
) -> Result<Value> {
    let value = execution(tx, &input.execution_id)?;
    if value["revision"].as_u64() != Some(input.expected_revision) {
        return Err(Error::Conflict(format!(
            "stale worker revision; current revision is {}",
            value["revision"]
        )));
    }
    if value["active"].as_bool() == Some(true) || value["process"]["status"] == "running" {
        return Err(Error::Conflict(
            "cannot clean up an active worker execution or running process".into(),
        ));
    }
    if value["workspace"]["status"] != "ready" {
        return Err(Error::Conflict(
            "worker workspace is not ready or was already cleaned up".into(),
        ));
    }
    let root = worker_root(dir, &input.execution_id)?;
    match std::fs::symlink_metadata(root.join("worktree")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Error::Conflict(
                "remove the Git worktree before marking the workspace cleaned up".into(),
            ));
        }
        Err(e) => return Err(e.into()),
    }
    tx.execute(
        "UPDATE worker_workspaces SET status='removed',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE execution_id=?1",
        [&input.execution_id],
    )?;
    tx.execute(
        "UPDATE worker_executions SET revision=revision+1 WHERE id=?1",
        [&input.execution_id],
    )?;
    let value = execution(tx, &input.execution_id)?;
    record_event(
        tx,
        request,
        &value,
        "worker.workspace_removed",
        json!({"repository_mutation":false}),
    )?;
    execution(tx, &input.execution_id)
}

pub(super) fn eligible(tx: &Transaction<'_>, project: Option<&str>, limit: u32) -> Result<Value> {
    if !(1..=100).contains(&limit) {
        return Err(Error::Invalid("limit must be in 1..=100".into()));
    }
    if let Some(project) = project {
        super::revision(tx, project)?;
    }
    let mut stmt = tx.prepare(
        "SELECT t.sequence,t.id,t.project_id,t.title,t.state,t.revision,t.project_revision,t.review_required,
          (SELECT count(*) FROM task_dependencies d WHERE d.task_id=t.id)
         FROM tasks t
         WHERE t.state='ready'
           AND (?1 IS NULL OR t.project_id=?1)
           AND NOT EXISTS (
             SELECT 1 FROM task_dependencies d JOIN tasks dep ON dep.id=d.dependency_id
             WHERE d.task_id=t.id AND dep.state<>'completed')
           AND NOT EXISTS (
             SELECT 1 FROM worker_executions e
             WHERE e.task_id=t.id AND e.status IN ('leased','running')
               AND e.lease_expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ORDER BY t.project_revision,t.sequence LIMIT ?2",
    )?;
    let items = stmt
        .query_map(params![project, limit as i64], |r| {
            Ok(json!({
                "sequence":r.get::<_,i64>(0)?,
                "id":r.get::<_,String>(1)?,
                "project_id":r.get::<_,String>(2)?,
                "title":r.get::<_,String>(3)?,
                "state":r.get::<_,String>(4)?,
                "task_revision":r.get::<_,i64>(5)?,
                "project_revision":r.get::<_,i64>(6)?,
                "review_required":r.get::<_,bool>(7)?,
                "dependency_count":r.get::<_,i64>(8)?
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(json!({
        "items":items,
        "count":items.len(),
        "project_id":project,
        "selection":"ready_with_completed_dependencies_and_no_unexpired_lease",
        "worker_adapter":"mock_or_opencode_lease",
        "bounded":true
    }))
}

pub(super) fn list(
    tx: &Transaction<'_>,
    task: Option<&str>,
    project: Option<&str>,
    status: Option<WorkerExecutionStatus>,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let mut stmt = tx.prepare(&format!(
        "SELECT {EXECUTION_COLUMNS} {EXECUTION_FROM}
         WHERE (?1 IS NULL OR e.task_id=?1) AND (?2 IS NULL OR e.project_id=?2)
           AND (?3 IS NULL OR e.status=?3) AND e.sequence>?4
         ORDER BY e.sequence LIMIT ?5"
    ))?;
    let items = stmt
        .query_map(
            params![
                task,
                project,
                status.map(WorkerExecutionStatus::as_str),
                after as i64,
                limit + 1
            ],
            execution_row,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["task_id"] = json!(task);
    result["project_id"] = json!(project);
    result["status"] = json!(status.map(WorkerExecutionStatus::as_str));
    Ok(result)
}

pub(super) fn get(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    execution(tx, id)
}

pub(super) fn history(tx: &Transaction<'_>, id: &str, after: u64, limit: u32) -> Result<Value> {
    let value = execution(tx, id)?;
    pagination(after, limit)?;
    let mut stmt = tx.prepare(
        "SELECT sequence,id,action,revision,details,created_at FROM worker_execution_events WHERE execution_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
    )?;
    let items = stmt
        .query_map(params![id, after as i64, limit + 1], |r| {
            Ok(json!({
                "sequence":r.get::<_,i64>(0)?,
                "id":r.get::<_,String>(1)?,
                "action":r.get::<_,String>(2)?,
                "revision":r.get::<_,i64>(3)?,
                "details":parsed_json(r.get(4)?)?,
                "created_at":r.get::<_,String>(5)?
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["execution_id"] = json!(id);
    result["revision"] = value["revision"].clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config,
        protocol::{Operation, PROTOCOL_VERSION},
        store::Store,
        tasks::ArtifactCriterion,
    };
    use std::{os::unix::fs::PermissionsExt, thread, time::Duration};

    fn fixture() -> (tempfile::TempDir, std::fs::File, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        config::initialize(dir.path()).unwrap();
        let lock = config::lock(dir.path()).unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let project = store
            .execute(&request(Operation::ProjectAttach {
                name: "workers".into(),
                repository: dir.path().to_str().unwrap().into(),
            }))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .into();
        (dir, lock, store, project)
    }

    fn request(operation: Operation) -> Request {
        Request {
            version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation,
        }
    }

    fn task_input(project: &str, title: &str) -> crate::tasks::TaskInput {
        crate::tasks::TaskInput {
            project_id: project.into(),
            title: title.into(),
            objective: "Exercise coordination leases".into(),
            source: "test".into(),
            acceptance_criteria: vec![],
            artifact_constraints: vec![ArtifactCriterion {
                path: "artifact".into(),
                expected_sha256: Some(crate::evidence::sha256(b"finished")),
            }],
            dependencies: vec![],
            review_required: false,
            persona: None,
            expected_project_revision: None,
        }
    }

    fn transition(task: &Value, state: TaskState) -> TaskTransition {
        TaskTransition {
            task_id: task["id"].as_str().unwrap().into(),
            state,
            source: "test".into(),
            reason: "prepare task".into(),
            owner_label: (state == TaskState::Assigned).then(|| "host".into()),
            persona: None,
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        }
    }

    fn ready(store: &mut Store, project: &str, title: &str) -> Value {
        let task = store
            .execute(&request(Operation::TaskCreate(task_input(project, title))))
            .unwrap();
        let task = store
            .execute(&request(Operation::TaskTransition(transition(
                &task,
                TaskState::Triaged,
            ))))
            .unwrap();
        store
            .execute(&request(Operation::TaskTransition(transition(
                &task,
                TaskState::Ready,
            ))))
            .unwrap()
    }

    fn lease(task: &Value, label: &str, seconds: u32, max_active: u32) -> WorkerLeaseInput {
        WorkerLeaseInput {
            task_id: task["id"].as_str().unwrap().into(),
            worker_label: label.into(),
            adapter: WorkerAdapter::Mock,
            lease_seconds: seconds,
            max_active,
            expected_task_revision: None,
            expected_project_revision: None,
        }
    }

    #[test]
    fn dependency_aware_selection_bounded_leases_and_heartbeats() {
        let (_dir, _lock, mut store, project) = fixture();
        let parent = ready(&mut store, &project, "Parent");
        let mut dependent_input = task_input(&project, "Dependent");
        dependent_input.dependencies = vec![parent["id"].as_str().unwrap().into()];
        let dependent = store
            .execute(&request(Operation::TaskCreate(dependent_input)))
            .unwrap();
        let dependent = store
            .execute(&request(Operation::TaskTransition(transition(
                &dependent,
                TaskState::Triaged,
            ))))
            .unwrap();
        let independent = ready(&mut store, &project, "Independent");

        let candidates = store
            .execute(&request(Operation::WorkerEligible {
                project_id: Some(project.clone()),
                limit: 20,
            }))
            .unwrap();
        assert_eq!(candidates["count"], 2);
        assert!(
            candidates["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["id"] != dependent["id"])
        );

        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &parent, "worker-a", 120, 1,
            ))))
            .unwrap();
        assert_eq!(leased["status"], "leased");
        assert_eq!(leased["attempt"], 1);
        let execution_id = leased["id"].as_str().unwrap();
        let current = store
            .execute(&request(Operation::TaskGet {
                task_id: parent["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(current["state"], "assigned");
        assert_eq!(current["owner_label"], "worker-a");
        assert_eq!(current["execution_mode"], "worker_leased");
        assert_eq!(current["worker_execution"]["id"], execution_id);

        assert!(matches!(
            store.execute(&request(Operation::WorkerLease(lease(
                &independent,
                "worker-b",
                120,
                1,
            )))),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            store.execute(&request(Operation::WorkerLease(lease(
                &parent, "worker-b", 120, 4,
            )))),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            store.execute(&request(Operation::WorkerHeartbeat(WorkerHeartbeatInput {
                execution_id: execution_id.into(),
                expected_revision: 99,
                lease_seconds: 120,
            }))),
            Err(Error::Conflict(_))
        ));
        let heartbeat = store
            .execute(&request(Operation::WorkerHeartbeat(WorkerHeartbeatInput {
                execution_id: execution_id.into(),
                expected_revision: 1,
                lease_seconds: 60,
            })))
            .unwrap();
        assert_eq!(heartbeat["revision"], 2);
        assert_eq!(heartbeat["lease_seconds"], 60);

        let released = store
            .execute(&request(Operation::WorkerRelease(WorkerReleaseInput {
                execution_id: execution_id.into(),
                expected_revision: 2,
                outcome: WorkerReleaseOutcome::Released,
                summary: Some("coordination finished".into()),
            })))
            .unwrap();
        assert_eq!(released["status"], "released");
        assert_eq!(released["active"], false);
        assert!(matches!(
            store.execute(&request(Operation::WorkerRelease(WorkerReleaseInput {
                execution_id: execution_id.into(),
                expected_revision: 3,
                outcome: WorkerReleaseOutcome::Released,
                summary: None,
            }))),
            Err(Error::Conflict(_))
        ));
        let history = store
            .execute(&request(Operation::WorkerExecutionHistory {
                execution_id: execution_id.into(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(history["items"].as_array().unwrap().len(), 3);
        assert_eq!(history["items"][2]["action"], "worker.released");

        let second = store
            .execute(&request(Operation::WorkerLease(lease(
                &independent,
                "worker-b",
                120,
                1,
            ))))
            .unwrap();
        assert_eq!(second["status"], "leased");
        assert_eq!(second["attempt"], 1);
    }

    #[test]
    fn mock_runner_checkpoints_but_cannot_complete_or_mutate_repository() {
        let (dir, _lock, mut store, project) = fixture();
        let artifact = dir.path().join("artifact");
        std::fs::write(&artifact, b"finished").unwrap();
        let task = ready(&mut store, &project, "Mock work");
        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &task, "mock-a", 120, 4,
            ))))
            .unwrap();
        let execution_id = leased["id"].as_str().unwrap();
        let result = store
            .execute(&request(Operation::WorkerMockRun(WorkerMockInput {
                execution_id: execution_id.into(),
                expected_revision: 1,
                lease_seconds: 120,
                summary: "Deterministic mock checkpoint".into(),
                work_remaining: vec!["Host must validate and complete".into()],
                blockers: vec![],
            })))
            .unwrap();
        assert_eq!(result["execution"]["status"], "running");
        assert_eq!(result["execution"]["revision"], 2);
        assert_eq!(result["task"]["state"], "running");
        assert_eq!(
            result["task"]["latest_checkpoint"]["submission"]["source"],
            "mock-worker"
        );
        assert_eq!(result["limits"]["completion"], false);
        assert_eq!(std::fs::read(&artifact).unwrap(), b"finished");
        assert_eq!(
            store
                .execute(&request(Operation::WorkerExecutionGet {
                    execution_id: execution_id.into(),
                }))
                .unwrap()["task_state"],
            "running"
        );
        assert!(matches!(
            store.execute(&request(Operation::WorkerMockRun(WorkerMockInput {
                execution_id: execution_id.into(),
                expected_revision: 1,
                lease_seconds: 120,
                summary: "stale".into(),
                work_remaining: vec![],
                blockers: vec![],
            }))),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            store.execute(&request(Operation::WorkerRelease(WorkerReleaseInput {
                execution_id: execution_id.into(),
                expected_revision: 2,
                outcome: WorkerReleaseOutcome::Completed,
                summary: None,
            }))),
            Err(Error::Conflict(_))
        ));
        let heartbeat = store
            .execute(&request(Operation::WorkerHeartbeat(WorkerHeartbeatInput {
                execution_id: execution_id.into(),
                expected_revision: 2,
                lease_seconds: 30,
            })))
            .unwrap();
        assert_eq!(heartbeat["revision"], 3);
        let released = store
            .execute(&request(Operation::WorkerRelease(WorkerReleaseInput {
                execution_id: execution_id.into(),
                expected_revision: 3,
                outcome: WorkerReleaseOutcome::Failed,
                summary: Some("mock stopped before verification".into()),
            })))
            .unwrap();
        assert_eq!(released["status"], "failed");
        assert_eq!(released["result"]["outcome"], "failed");
    }

    #[test]
    fn supervised_process_uses_private_worktree_and_never_completes_the_task() {
        let (dir, _lock, mut store, project) = fixture();
        let mut input = task_input(&project, "Process work");
        input.artifact_constraints = vec![ArtifactCriterion {
            path: "worker-output.txt".into(),
            expected_sha256: None,
        }];
        let task = store
            .execute(&request(Operation::TaskCreate(input)))
            .unwrap();
        let task = store
            .execute(&request(Operation::TaskTransition(transition(
                &task,
                TaskState::Triaged,
            ))))
            .unwrap();
        let task = store
            .execute(&request(Operation::TaskTransition(transition(
                &task,
                TaskState::Ready,
            ))))
            .unwrap();
        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &task,
                "process-a",
                30,
                2,
            ))))
            .unwrap();
        let execution_id = leased["id"].as_str().unwrap().to_owned();
        let root = crate::config::worker_execution_dir(dir.path(), &execution_id).unwrap();
        let worktree = root.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::write(worktree.join("worker-output.txt"), b"mock output").unwrap();
        let workspace = store
            .execute(&request(Operation::WorkerWorkspaceStart(
                WorkerWorkspaceInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 1,
                    base_commit: "a".repeat(40),
                },
            )))
            .unwrap();
        assert_eq!(workspace["workspace"]["status"], "ready");
        assert_eq!(workspace["workspace"]["base_commit"], "a".repeat(40));
        let attached = store
            .execute(&request(Operation::WorkerProcessAttach(
                WorkerProcessAttachInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 2,
                    lease_seconds: 30,
                    pid: std::process::id(),
                },
            )))
            .unwrap();
        assert_eq!(attached["status"], "running");
        assert_eq!(attached["process"]["status"], "running");
        assert_eq!(attached["task_state"], "running");
        assert_eq!(attached["repository_mutation"], false);
        let finished = store
            .execute(&request(Operation::WorkerProcessFinish(
                WorkerProcessFinishInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 3,
                    outcome: WorkerProcessOutcome::Exited,
                    exit_code: Some(0),
                    provider: None,
                },
            )))
            .unwrap();
        assert_eq!(finished["status"], "released");
        assert_eq!(finished["process"]["status"], "exited");
        assert_eq!(finished["process"]["evidence"][0]["status"], "present");
        assert_eq!(finished["task_state"], "awaiting_input");
        assert_eq!(finished["result"]["completion"], false);
        assert!(finished["result"].get("completion_evidence").is_none());
        let task = store
            .execute(&request(Operation::TaskGet {
                task_id: task["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(task["state"], "awaiting_input");
        assert_eq!(task["completion_evidence"], Value::Null);
        std::fs::remove_dir_all(&worktree).unwrap();
        let cleaned = store
            .execute(&request(Operation::WorkerWorkspaceCleanup(
                WorkerWorkspaceCleanupInput {
                    execution_id,
                    expected_revision: 4,
                },
            )))
            .unwrap();
        assert_eq!(cleaned["workspace"]["status"], "removed");
    }

    #[test]
    fn expiry_blocks_tasks_when_the_supervised_process_may_still_run() {
        let (dir, _lock, mut store, project) = fixture();
        let task = ready(&mut store, &project, "Running process expiry");
        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &task,
                "process-a",
                1,
                2,
            ))))
            .unwrap();
        let execution_id = leased["id"].as_str().unwrap().to_owned();
        let root = crate::config::worker_execution_dir(dir.path(), &execution_id).unwrap();
        std::fs::create_dir(root.join("worktree")).unwrap();
        store
            .execute(&request(Operation::WorkerWorkspaceStart(
                WorkerWorkspaceInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 1,
                    base_commit: "b".repeat(40),
                },
            )))
            .unwrap();
        store
            .execute(&request(Operation::WorkerProcessAttach(
                WorkerProcessAttachInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 2,
                    lease_seconds: 1,
                    pid: std::process::id(),
                },
            )))
            .unwrap();
        thread::sleep(Duration::from_millis(1200));
        let recovered = store.execute(&request(Operation::WorkerRecover)).unwrap();
        assert_eq!(recovered["count"], 1);
        assert_eq!(recovered["expired"][0]["process"]["status"], "running");
        let task = store
            .execute(&request(Operation::TaskGet {
                task_id: task["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(task["state"], "blocked");
        assert!(
            task["latest_transition"]["reason"]
                .as_str()
                .unwrap()
                .contains("supervised process")
        );
        let finished = store
            .execute(&request(Operation::WorkerProcessFinish(
                WorkerProcessFinishInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 4,
                    outcome: WorkerProcessOutcome::Killed,
                    exit_code: None,
                    provider: None,
                },
            )))
            .unwrap();
        assert_eq!(finished["status"], "expired");
        assert_eq!(finished["process"]["status"], "killed");
        let task = store
            .execute(&request(Operation::TaskGet {
                task_id: task["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(task["state"], "blocked");
    }

    #[test]
    fn abandoned_supervision_marks_process_lost_and_blocks_task() {
        let (dir, _lock, mut store, project) = fixture();
        let task = ready(&mut store, &project, "Abandoned process");
        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &task,
                "process-a",
                120,
                2,
            ))))
            .unwrap();
        let execution_id = leased["id"].as_str().unwrap().to_owned();
        let root = crate::config::worker_execution_dir(dir.path(), &execution_id).unwrap();
        std::fs::create_dir(root.join("worktree")).unwrap();
        store
            .execute(&request(Operation::WorkerWorkspaceStart(
                WorkerWorkspaceInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 1,
                    base_commit: "c".repeat(40),
                },
            )))
            .unwrap();
        store
            .execute(&request(Operation::WorkerProcessAttach(
                WorkerProcessAttachInput {
                    execution_id: execution_id.clone(),
                    expected_revision: 2,
                    lease_seconds: 120,
                    pid: std::process::id(),
                },
            )))
            .unwrap();
        let abandoned = store
            .execute(&request(Operation::WorkerAbandon(WorkerAbandonInput {
                execution_id: execution_id.clone(),
                expected_revision: 3,
            })))
            .unwrap();
        assert_eq!(abandoned["status"], "failed");
        assert_eq!(abandoned["process"]["status"], "lost");
        assert!(abandoned["process"]["ended_at"].is_string());
        assert_eq!(abandoned["task_state"], "blocked");
        assert_eq!(abandoned["result"]["process_lost"], true);
        assert_eq!(abandoned["result"]["process_signal_sent"], false);
        assert_eq!(abandoned["result"]["repository_mutation"], false);
        let history = store
            .execute(&request(Operation::WorkerExecutionHistory {
                execution_id: execution_id.clone(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(
            history["items"].as_array().unwrap().last().unwrap()["action"],
            "worker.abandoned"
        );
        std::fs::remove_dir_all(root.join("worktree")).unwrap();
        let cleaned = store
            .execute(&request(Operation::WorkerWorkspaceCleanup(
                WorkerWorkspaceCleanupInput {
                    execution_id,
                    expected_revision: 4,
                },
            )))
            .unwrap();
        assert_eq!(cleaned["workspace"]["status"], "removed");
    }

    #[test]
    fn abandoning_before_process_attachment_blocks_the_assigned_task() {
        let (_dir, _lock, mut store, project) = fixture();
        let task = ready(&mut store, &project, "Pre-process failure");
        let leased = store
            .execute(&request(Operation::WorkerLease(lease(
                &task,
                "process-a",
                120,
                2,
            ))))
            .unwrap();
        let execution_id = leased["id"].as_str().unwrap().to_owned();
        let abandoned = store
            .execute(&request(Operation::WorkerAbandon(WorkerAbandonInput {
                execution_id,
                expected_revision: 1,
            })))
            .unwrap();
        assert_eq!(abandoned["status"], "failed");
        assert_eq!(abandoned["process"], Value::Null);
        assert_eq!(abandoned["task_state"], "blocked");
        assert_eq!(abandoned["result"]["process_lost"], false);
    }

    #[test]
    fn expired_leases_are_recovered_and_reassigned() {
        let (_dir, _lock, mut store, project) = fixture();
        let task = ready(&mut store, &project, "Crash recovery");
        let first = store
            .execute(&request(Operation::WorkerLease(lease(
                &task, "worker-a", 1, 4,
            ))))
            .unwrap();
        let first_id = first["id"].as_str().unwrap();
        thread::sleep(Duration::from_millis(1200));
        let expired = store
            .execute(&request(Operation::WorkerExecutionGet {
                execution_id: first_id.into(),
            }))
            .unwrap();
        assert_eq!(expired["expired"], true);
        assert_eq!(expired["active"], false);
        let recovered = store.execute(&request(Operation::WorkerRecover)).unwrap();
        assert_eq!(recovered["count"], 1);
        assert_eq!(recovered["expired"][0]["status"], "expired");
        let requeued = store
            .execute(&request(Operation::TaskGet {
                task_id: task["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(requeued["state"], "ready");
        assert!(requeued["worker_execution"].is_null());
        assert_eq!(requeued["execution_mode"], "host_tracked");

        let second = store
            .execute(&request(Operation::WorkerLease(lease(
                &task, "worker-b", 1, 4,
            ))))
            .unwrap();
        assert_eq!(second["attempt"], 2);
        assert_eq!(second["status"], "leased");
        let second_id = second["id"].as_str().unwrap().to_owned();
        thread::sleep(Duration::from_millis(1200));
        let third = store
            .execute(&request(Operation::WorkerLease(lease(
                &task, "worker-c", 120, 4,
            ))))
            .unwrap();
        assert_eq!(third["attempt"], 3);
        assert_eq!(third["worker_label"], "worker-c");
        let expired = store
            .execute(&request(Operation::WorkerExecutionGet {
                execution_id: second_id,
            }))
            .unwrap();
        assert_eq!(expired["status"], "expired");
        assert_eq!(expired["result"]["reason"], "lease_expired");
    }
}
