use super::{event, page, pagination, tasks};
use crate::{
    Error, Result,
    protocol::Request,
    tasks::{CheckpointInput, TaskState, TaskTransition},
    workers::{
        WorkerExecutionStatus, WorkerHeartbeatInput, WorkerLeaseInput, WorkerMockInput,
        WorkerReleaseInput, WorkerReleaseOutcome,
    },
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use uuid::Uuid;

const EXECUTION_COLUMNS: &str = "e.sequence,e.id,e.task_id,e.project_id,e.worker_label,e.adapter,e.status,e.revision,e.attempt,e.lease_seconds,e.lease_expires_at,e.heartbeat_at,e.result,e.created_at,e.updated_at,t.state,t.revision,t.project_revision,t.owner_label,(e.status='expired' OR (e.status IN ('leased','running') AND e.lease_expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')))";

fn parsed_json(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn execution_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let status = r.get::<_, String>(6)?;
    let active = matches!(status.as_str(), "leased" | "running");
    Ok(json!({
        "sequence":r.get::<_,i64>(0)?,
        "id":r.get::<_,String>(1)?,
        "task_id":r.get::<_,String>(2)?,
        "project_id":r.get::<_,String>(3)?,
        "worker_label":r.get::<_,String>(4)?,
        "adapter":r.get::<_,String>(5)?,
        "status":status,
        "revision":r.get::<_,i64>(7)?,
        "attempt":r.get::<_,i64>(8)?,
        "lease_seconds":r.get::<_,i64>(9)?,
        "lease_expires_at":r.get::<_,String>(10)?,
        "heartbeat_at":r.get::<_,String>(11)?,
        "result":r.get::<_,Option<String>>(12)?.map(parsed_json).transpose()?,
        "created_at":r.get::<_,String>(13)?,
        "updated_at":r.get::<_,String>(14)?,
        "task_state":r.get::<_,String>(15)?,
        "task_revision":r.get::<_,i64>(16)?,
        "project_revision":r.get::<_,i64>(17)?,
        "task_owner_label":r.get::<_,Option<String>>(18)?,
        "expired":r.get::<_,bool>(19)?,
        "active":active && !r.get::<_,bool>(19)?,
        "lease_trust":"coordination_only",
        "process_supervised":false,
        "repository_mutation":false
    }))
}

fn execution(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    tx.query_row(
        &format!("SELECT {EXECUTION_COLUMNS} FROM worker_executions e JOIN tasks t ON t.id=e.task_id WHERE e.id=?1"),
        [id],
        execution_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("worker execution {id} does not exist")))
}

pub(super) fn for_task(tx: &Transaction<'_>, id: &str) -> Result<Option<Value>> {
    Ok(tx
        .query_row(
            &format!("SELECT {EXECUTION_COLUMNS} FROM worker_executions e JOIN tasks t ON t.id=e.task_id WHERE e.task_id=?1 AND e.status IN ('leased','running') ORDER BY e.sequence DESC LIMIT 1"),
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
    let mut task = tasks::get(tx, value["task_id"].as_str().unwrap())?;
    let mut requeued = false;
    if task["state"] == "assigned" {
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
        json!({"reason":"lease_expired","task_requeued":requeued,"task_state":task["state"]}),
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
        "INSERT INTO worker_executions(id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at) VALUES(?1,?2,?3,?4,'mock','leased',1,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now',?7),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            id,
            input.task_id,
            project,
            input.worker_label,
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
        json!({"task_revision":assigned["task_revision"],"project_revision":assigned["project_revision"]}),
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
    if value["adapter"] != "mock" {
        return Err(Error::Conflict(
            "only the deterministic mock adapter is implemented".into(),
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
        "worker_adapter":"mock_lease_only",
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
        "SELECT {EXECUTION_COLUMNS} FROM worker_executions e JOIN tasks t ON t.id=e.task_id
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
            acceptance_criteria: vec![ArtifactCriterion {
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
