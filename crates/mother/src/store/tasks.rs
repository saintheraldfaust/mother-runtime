use super::{event, page, pagination, revision, text};
use crate::{
    Error, Result, evidence,
    personas::Capability,
    protocol::Request,
    tasks::{
        ArtifactCriterion, ArtifactObservation, CheckpointInput, TaskInput, TaskState,
        TaskTransition,
    },
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};
use uuid::Uuid;

fn parsed_json(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn checkpoint_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"task_id":r.get::<_,String>(2)?,
        "task_revision":r.get::<_,i64>(3)?,"project_revision":r.get::<_,i64>(4)?,"submission":parsed_json(r.get(5)?)?,
        "artifacts":parsed_json(r.get(6)?)?,"created_at":r.get::<_,String>(7)?,
        "persona_id":r.get::<_,Option<String>>(8)?,"persona_revision":r.get::<_,Option<i64>>(9)?,
        "submission_trust":"unverified_submission","artifact_verification_scope":"bounded_regular_file_contents_only","reported_tests_verified":false}),
    )
}
const CHECKPOINT_COLUMNS: &str = "sequence,id,task_id,task_revision,project_revision,submission,artifacts,created_at,persona_id,persona_revision";

pub(super) fn get(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let mut value=tx.query_row("SELECT sequence,id,project_id,title,objective,source,acceptance_criteria,review_required,state,owner_label,revision,project_revision,latest_checkpoint_id,completion_evidence,created_at,updated_at FROM tasks WHERE id=?1",[id],|r|Ok(json!({
        "sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"project_id":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,
        "objective":r.get::<_,String>(4)?,"source":r.get::<_,String>(5)?,"acceptance_criteria":parsed_json(r.get(6)?)?,
        "review_required":r.get::<_,bool>(7)?,"state":r.get::<_,String>(8)?,"owner_label":r.get::<_,Option<String>>(9)?,
        "task_revision":r.get::<_,i64>(10)?,"project_revision":r.get::<_,i64>(11)?,"latest_checkpoint_id":r.get::<_,Option<String>>(12)?,
        "completion_evidence":r.get::<_,Option<String>>(13)?.map(parsed_json).transpose()?,"created_at":r.get::<_,String>(14)?,"updated_at":r.get::<_,String>(15)?,
        "execution_mode":"host_tracked","worker_execution":null,"submission_trust":"unverified_submission"})))
        .optional()?.ok_or_else(||Error::NotFound(format!("task {id} does not exist")))?;
    let dependencies = tx
        .prepare(
            "SELECT dependency_id FROM task_dependencies WHERE task_id=?1 ORDER BY dependency_id",
        )?
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    value["dependencies"] = json!(dependencies);
    value["current_project_revision"] = json!(revision(tx, value["project_id"].as_str().unwrap())?);
    value["latest_checkpoint"] = if let Some(checkpoint) = value["latest_checkpoint_id"].as_str() {
        tx.query_row(
            &format!("SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints WHERE id=?1"),
            [checkpoint],
            checkpoint_row,
        )?
    } else {
        Value::Null
    };
    value["latest_transition"]=tx.query_row("SELECT id,from_state,to_state,reason,source,task_revision,project_revision,created_at,persona_id,persona_revision FROM task_transitions WHERE task_id=?1 ORDER BY sequence DESC LIMIT 1",[id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"from_state":r.get::<_,String>(1)?,"to_state":r.get::<_,String>(2)?,"reason":r.get::<_,String>(3)?,"source":r.get::<_,String>(4)?,"task_revision":r.get::<_,i64>(5)?,"project_revision":r.get::<_,i64>(6)?,"created_at":r.get::<_,String>(7)?,"persona_id":r.get::<_,Option<String>>(8)?,"persona_revision":r.get::<_,Option<i64>>(9)?}))).optional()?.unwrap_or(Value::Null);
    Ok(value)
}

pub(super) fn list(
    tx: &Transaction<'_>,
    project: &str,
    state: Option<TaskState>,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let current = revision(tx, project)?;
    let mut stmt=tx.prepare("SELECT sequence,id,title,state,owner_label,revision,project_revision,updated_at FROM tasks WHERE project_id=?1 AND (?2 IS NULL OR state=?2) AND sequence>?3 ORDER BY sequence LIMIT ?4")?;
    let rows=stmt.query_map(params![project,state.map(TaskState::as_str),after as i64,limit+1],|r|Ok(json!({
        "sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,
        "owner_label":r.get::<_,Option<String>>(4)?,"task_revision":r.get::<_,i64>(5)?,"project_revision":r.get::<_,i64>(6)?,"updated_at":r.get::<_,String>(7)?,"execution_mode":"host_tracked"})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut result = page(rows, limit);
    result["project_id"] = json!(project);
    result["revision"] = json!(current);
    Ok(result)
}

fn advance_project(tx: &Transaction<'_>, project: &str, expected: Option<u64>) -> Result<i64> {
    let current = revision(tx, project)?;
    if expected.is_some_and(|r| r != current as u64) {
        return Err(Error::Conflict(format!(
            "stale project revision; current revision is {current}"
        )));
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| Error::Conflict("project revision exhausted".into()))?;
    tx.execute(
        "UPDATE projects SET revision=?1 WHERE id=?2",
        params![next, project],
    )?;
    Ok(next)
}

fn check_task(task: &Value, expected: u64) -> Result<i64> {
    let current = task["task_revision"].as_i64().unwrap();
    if expected != current as u64 {
        return Err(Error::Conflict(format!(
            "stale task revision; current revision is {current}"
        )));
    }
    current
        .checked_add(1)
        .ok_or_else(|| Error::Conflict("task revision exhausted".into()))
}

fn validate_criteria(criteria: &[ArtifactCriterion]) -> Result<()> {
    if criteria.is_empty() || criteria.len() > 16 {
        return Err(Error::Invalid(
            "acceptance_criteria requires 1..=16 repository files".into(),
        ));
    }
    let mut paths = HashSet::new();
    for c in criteria {
        if !evidence::valid_path(&c.path) || !paths.insert(&c.path) {
            return Err(Error::Invalid(
                "artifact paths must be unique normalized relative paths of at most 512 bytes"
                    .into(),
            ));
        }
        if c.expected_sha256.as_ref().is_some_and(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err(Error::Invalid(
                "expected_sha256 must be 64 lowercase hexadecimal characters".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn create(tx: &Transaction<'_>, request: &Request, input: &TaskInput) -> Result<Value> {
    text(&input.title, "title", 256)?;
    text(&input.objective, "objective", 4096)?;
    text(&input.source, "source", 256)?;
    validate_criteria(&input.acceptance_criteria)?;
    if let Some(claim) = &input.persona {
        super::personas::authorize(tx, claim, Capability::TaskCreate, &input.project_id)?;
    }
    if input.dependencies.len() > 16 || serde_json::to_vec(input)?.len() > 24 * 1024 {
        return Err(Error::Invalid(
            "task supports at most 16 dependencies and 24 KiB of serialized input".into(),
        ));
    }
    let mut seen = HashSet::new();
    for id in &input.dependencies {
        if !seen.insert(id) {
            return Err(Error::Invalid("duplicate task dependency".into()));
        }
        if get(tx, id)?["project_id"] != input.project_id {
            return Err(Error::Conflict(
                "dependencies must belong to the same project".into(),
            ));
        }
    }
    let next = advance_project(tx, &input.project_id, input.expected_project_revision)?;
    let id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO tasks(id,project_id,title,objective,source,acceptance_criteria,review_required,state,revision,project_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,'created',1,?8)",
        params![id,input.project_id,input.title,input.objective,input.source,serde_json::to_string(&input.acceptance_criteria)?,input.review_required,next])?;
    for dependency in &input.dependencies {
        tx.execute(
            "INSERT INTO task_dependencies(task_id,dependency_id,project_id) VALUES(?1,?2,?3)",
            params![id, dependency, input.project_id],
        )?;
    }
    event(
        tx,
        request,
        &input.project_id,
        &id,
        "task.created",
        &input.source,
        next,
    )?;
    get(tx, &id)
}

fn repository(tx: &Transaction<'_>, task: &Value) -> Result<String> {
    Ok(tx.query_row(
        "SELECT repository FROM projects WHERE id=?1",
        [task["project_id"].as_str().unwrap()],
        |r| r.get(0),
    )?)
}

fn observations(tx: &Transaction<'_>, task: &Value) -> Result<Vec<ArtifactObservation>> {
    let criteria: Vec<ArtifactCriterion> =
        serde_json::from_value(task["acceptance_criteria"].clone())?;
    Ok(evidence::observe(
        Path::new(&repository(tx, task)?),
        &criteria,
    ))
}

pub(super) fn checkpoint(
    tx: &Transaction<'_>,
    request: &Request,
    input: &CheckpointInput,
) -> Result<Value> {
    text(&input.summary, "summary", 4096)?;
    text(&input.source, "source", 256)?;
    for (name, values) in [
        ("work_remaining", &input.work_remaining),
        ("blockers", &input.blockers),
        ("reported_test_results", &input.reported_test_results),
    ] {
        if values.len() > 16 {
            return Err(Error::Invalid(format!("{name} allows at most 16 entries")));
        }
        for value in values {
            text(value, name, 1024)?;
        }
    }
    if serde_json::to_vec(input)?.len() > 24 * 1024 {
        return Err(Error::Invalid(
            "checkpoint submission exceeds 24 KiB".into(),
        ));
    }
    let task = get(tx, &input.task_id)?;
    let project = task["project_id"].as_str().unwrap();
    if let Some(claim) = &input.persona {
        super::personas::authorize(tx, claim, Capability::TaskCheckpoint, project)?;
    }
    let task_next = check_task(&task, input.expected_task_revision)?;
    let state: TaskState = serde_json::from_value(task["state"].clone())?;
    if state.terminal() {
        return Err(Error::Conflict(
            "terminal task cannot receive a checkpoint; retry failed work through triage first"
                .into(),
        ));
    }
    let artifacts = observations(tx, &task)?;
    let next = advance_project(tx, project, input.expected_project_revision)?;
    let id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO task_checkpoints(id,task_id,task_revision,project_revision,submission,artifacts,persona_id,persona_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,input.task_id,task_next,next,serde_json::to_string(input)?,serde_json::to_string(&artifacts)?,
        input.persona.as_ref().map(|p|p.persona_id.as_str()),input.persona.as_ref().map(|p|p.expected_revision as i64)])?;
    tx.execute("UPDATE tasks SET revision=?1,project_revision=?2,latest_checkpoint_id=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?4",params![task_next,next,id,input.task_id])?;
    event(
        tx,
        request,
        project,
        &id,
        "task.checkpointed",
        &input.source,
        next,
    )?;
    get(tx, &input.task_id)
}

fn completion(tx: &Transaction<'_>, task: &Value) -> Result<Value> {
    if task["review_required"] == true {
        return Err(Error::Conflict(
            "review-gated completion requires a review verifier, which is not implemented".into(),
        ));
    }
    let checkpoint = &task["latest_checkpoint"];
    if checkpoint.is_null() || checkpoint["task_revision"] != task["task_revision"] {
        return Err(Error::Conflict(
            "completion requires a fresh checkpoint in validating state".into(),
        ));
    }
    let artifacts = observations(tx, task)?;
    if artifacts
        .iter()
        .any(|a| !matches!(a.status.as_str(), "present" | "matched"))
        || serde_json::to_value(&artifacts)? != checkpoint["artifacts"]
    {
        return Err(Error::Conflict("artifact acceptance failed or files changed after the checkpoint; refresh the checkpoint before completion".into()));
    }
    Ok(
        json!({"checkpoint_id":checkpoint["id"],"checked_at":tx.query_row::<String,_,_>("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')",[],|r|r.get(0))?,
        "artifacts":artifacts,"verification_scope":"bounded_regular_file_contents_only","tests_verified":false,"independent_review_verified":false,"human_approval_recorded":false}),
    )
}

pub(super) fn transition(
    tx: &Transaction<'_>,
    request: &Request,
    input: &TaskTransition,
) -> Result<Value> {
    text(&input.reason, "reason", 4096)?;
    text(&input.source, "source", 256)?;
    let task = get(tx, &input.task_id)?;
    let project = task["project_id"].as_str().unwrap();
    if let Some(claim) = &input.persona {
        super::personas::authorize(tx, claim, Capability::TaskTransition, project)?;
    }
    let task_next = check_task(&task, input.expected_task_revision)?;
    let from: TaskState = serde_json::from_value(task["state"].clone())?;
    if !from.permits(input.state) {
        return Err(Error::Conflict(format!(
            "transition from {} to {} is not allowed",
            from.as_str(),
            input.state.as_str()
        )));
    }
    let owner = if input.state == TaskState::Assigned {
        let owner = input
            .owner_label
            .as_deref()
            .or_else(|| task["owner_label"].as_str())
            .ok_or_else(|| Error::Invalid("assignment requires an owner_label".into()))?;
        text(owner, "owner_label", 256)?;
        Some(owner.to_owned())
    } else {
        if input.owner_label.is_some() {
            return Err(Error::Invalid(
                "owner_label can only be set when assigning".into(),
            ));
        }
        task["owner_label"].as_str().map(str::to_owned)
    };
    if input.state == TaskState::Running && owner.is_none() {
        return Err(Error::Conflict(
            "running host-tracked task requires an owner label".into(),
        ));
    }
    if matches!(
        input.state,
        TaskState::Ready
            | TaskState::Assigned
            | TaskState::Running
            | TaskState::Validating
            | TaskState::Completed
    ) {
        let pending:i64=tx.query_row("SELECT count(*) FROM task_dependencies d JOIN tasks t ON t.id=d.dependency_id WHERE d.task_id=?1 AND t.state<>'completed'",[&input.task_id],|r|r.get(0))?;
        if pending != 0 {
            return Err(Error::Conflict(
                "task dependencies are not completed".into(),
            ));
        }
    }
    let evidence = if input.state == TaskState::Completed {
        Some(completion(tx, &task)?)
    } else {
        None
    };
    let next = advance_project(tx, project, input.expected_project_revision)?;
    tx.execute("UPDATE tasks SET state=?1,owner_label=?2,revision=?3,project_revision=?4,completion_evidence=?5,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?6",
        params![input.state.as_str(),owner,task_next,next,evidence.map(|v|serde_json::to_string(&v)).transpose()?,input.task_id])?;
    // Keep the transition reason as durable provenance, alongside the state change.
    let event_id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO task_transitions(id,task_id,from_state,to_state,reason,source,task_revision,project_revision,persona_id,persona_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![event_id,input.task_id,from.as_str(),input.state.as_str(),input.reason,input.source,task_next,next,
        input.persona.as_ref().map(|p|p.persona_id.as_str()),input.persona.as_ref().map(|p|p.expected_revision as i64)])?;
    event(
        tx,
        request,
        project,
        &input.task_id,
        "task.transitioned",
        &input.source,
        next,
    )?;
    get(tx, &input.task_id)
}

pub(super) fn checkpoints(tx: &Transaction<'_>, id: &str, after: u64, limit: u32) -> Result<Value> {
    pagination(after, limit)?;
    let task = get(tx, id)?;
    let mut stmt=tx.prepare(&format!("SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints WHERE task_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3"))?;
    let items = stmt
        .query_map(params![id, after as i64, limit + 1], checkpoint_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["task_id"] = json!(id);
    result["task_revision"] = task["task_revision"].clone();
    result["revision"] = task["current_project_revision"].clone();
    Ok(result)
}

pub(super) fn snapshot_tasks(
    tx: &Transaction<'_>,
    project: &str,
    focused: Option<&str>,
) -> Result<(Vec<Value>, Option<Value>, i64)> {
    let current = focused.map(|id| get(tx, id)).transpose()?;
    if current.as_ref().is_some_and(|t| t["project_id"] != project) {
        return Err(Error::Conflict(
            "handoff task must belong to its project".into(),
        ));
    }
    let ids=tx.prepare("SELECT id FROM tasks WHERE project_id=?1 AND state NOT IN ('completed','cancelled') AND (?2 IS NULL OR id<>?2) ORDER BY project_revision DESC LIMIT 20")?.query_map(params![project,focused],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut tasks = ids
        .iter()
        .map(|id| get(tx, id))
        .collect::<Result<Vec<_>>>()?;
    tasks.reverse();
    let total:i64=tx.query_row("SELECT count(*) FROM tasks WHERE project_id=?1 AND state NOT IN ('completed','cancelled') AND (?2 IS NULL OR id<>?2)",params![project,focused],|r|r.get(0))?;
    Ok((tasks, current, total))
}

pub(super) fn transitions(tx: &Transaction<'_>, id: &str, after: u64, limit: u32) -> Result<Value> {
    pagination(after, limit)?;
    let task = get(tx, id)?;
    let mut stmt=tx.prepare("SELECT sequence,id,from_state,to_state,reason,source,task_revision,project_revision,created_at,persona_id,persona_revision FROM task_transitions WHERE task_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
    let items=stmt.query_map(params![id,after as i64,limit+1],|r|Ok(json!({"sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"from_state":r.get::<_,String>(2)?,"to_state":r.get::<_,String>(3)?,"reason":r.get::<_,String>(4)?,"source":r.get::<_,String>(5)?,"task_revision":r.get::<_,i64>(6)?,"project_revision":r.get::<_,i64>(7)?,"created_at":r.get::<_,String>(8)?,"persona_id":r.get::<_,Option<String>>(9)?,"persona_revision":r.get::<_,Option<i64>>(10)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut result = page(items, limit);
    result["task_id"] = json!(id);
    result["task_revision"] = task["task_revision"].clone();
    result["revision"] = task["current_project_revision"].clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config,
        protocol::{Operation, PROTOCOL_VERSION},
        store::Store,
    };
    fn fixture() -> (tempfile::TempDir, std::fs::File, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        config::initialize(dir.path()).unwrap();
        let lock = config::lock(dir.path()).unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let project = store
            .execute(&request(Operation::ProjectAttach {
                name: "tasks".into(),
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
    fn input(project: &str) -> TaskInput {
        TaskInput {
            project_id: project.into(),
            title: "Create artifact".into(),
            objective: "Produce the declared file".into(),
            source: "test".into(),
            acceptance_criteria: vec![ArtifactCriterion {
                path: "artifact".into(),
                expected_sha256: Some(evidence::sha256(b"finished")),
            }],
            dependencies: vec![],
            review_required: false,
            persona: None,
            expected_project_revision: None,
        }
    }
    fn move_to(task: &Value, state: TaskState) -> TaskTransition {
        TaskTransition {
            task_id: task["id"].as_str().unwrap().into(),
            state,
            source: "test".into(),
            reason: "explicit step".into(),
            owner_label: if state == TaskState::Assigned {
                Some("engineer".into())
            } else {
                None
            },
            persona: None,
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        }
    }
    fn validate(store: &mut Store, mut task: Value) -> Value {
        for state in [
            TaskState::Triaged,
            TaskState::Ready,
            TaskState::Assigned,
            TaskState::Running,
            TaskState::Validating,
        ] {
            task = store
                .execute(&request(Operation::TaskTransition(move_to(&task, state))))
                .unwrap();
        }
        task
    }
    fn checkpoint_input(task: &Value) -> CheckpointInput {
        CheckpointInput {
            task_id: task["id"].as_str().unwrap().into(),
            summary: "Progress checkpoint".into(),
            source: "test".into(),
            work_remaining: vec![],
            blockers: vec![],
            reported_test_results: vec!["No test execution was verified".into()],
            persona: None,
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        }
    }

    #[test]
    fn local_completion_requires_fresh_unchanged_matching_artifacts() {
        let (dir, _lock, mut store, project) = fixture();
        let task = store
            .execute(&request(Operation::TaskCreate(input(&project))))
            .unwrap();
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &task,
                TaskState::Completed
            )))),
            Err(Error::Conflict(_))
        ));
        let mut task = validate(&mut store, task);
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &task,
                TaskState::Completed
            )))),
            Err(Error::Conflict(_))
        ));
        task = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&task))))
            .unwrap();
        assert_eq!(
            task["latest_checkpoint"]["artifacts"][0]["status"],
            "missing"
        );
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &task,
                TaskState::Completed
            )))),
            Err(Error::Conflict(_))
        ));
        std::fs::write(dir.path().join("artifact"), b"finished").unwrap();
        task = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&task))))
            .unwrap();
        assert_eq!(
            task["latest_checkpoint"]["artifacts"][0]["status"],
            "matched"
        );
        std::fs::write(dir.path().join("artifact"), b"modified").unwrap();
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &task,
                TaskState::Completed
            )))),
            Err(Error::Conflict(_))
        ));
        task = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&task))))
            .unwrap();
        assert_eq!(
            task["latest_checkpoint"]["artifacts"][0]["status"],
            "mismatched"
        );
        std::fs::write(dir.path().join("artifact"), b"finished").unwrap();
        task = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&task))))
            .unwrap();
        let complete = request(Operation::TaskTransition(move_to(
            &task,
            TaskState::Completed,
        )));
        let done = store.execute(&complete).unwrap();
        assert_eq!(done["state"], "completed");
        assert_eq!(done["completion_evidence"]["tests_verified"], false);
        assert_eq!(store.execute(&complete).unwrap(), done);
        assert!(
            store
                .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&done))))
                .is_err()
        );
        assert!(
            store
                .execute(&request(Operation::TaskTransition(move_to(
                    &done,
                    TaskState::Running
                ))))
                .is_err()
        );
    }

    #[test]
    fn task_dependency_scope_review_gate_and_contract_validation() {
        let (dir, _lock, mut store, project) = fixture();
        let parent = store
            .execute(&request(Operation::TaskCreate(input(&project))))
            .unwrap();
        let mut dependent = input(&project);
        dependent.dependencies = vec![parent["id"].as_str().unwrap().into()];
        let mut child = store
            .execute(&request(Operation::TaskCreate(dependent.clone())))
            .unwrap();
        child = store
            .execute(&request(Operation::TaskTransition(move_to(
                &child,
                TaskState::Triaged,
            ))))
            .unwrap();
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &child,
                TaskState::Ready
            )))),
            Err(Error::Conflict(_))
        ));
        dependent
            .dependencies
            .push(dependent.dependencies[0].clone());
        assert!(
            store
                .execute(&request(Operation::TaskCreate(dependent)))
                .is_err()
        );
        let second_dir = dir.path().join("second");
        std::fs::create_dir(&second_dir).unwrap();
        let second = store
            .execute(&request(Operation::ProjectAttach {
                name: "second".into(),
                repository: second_dir.to_str().unwrap().into(),
            }))
            .unwrap();
        let mut wrong = input(second["id"].as_str().unwrap());
        wrong.dependencies = vec![parent["id"].as_str().unwrap().into()];
        assert!(
            store
                .execute(&request(Operation::TaskCreate(wrong)))
                .is_err()
        );
        let scoped_handoff:crate::continuity::HandoffInput=serde_json::from_value(json!({"project_id":second["id"],"task_id":parent["id"],"objective":"Wrong scope","source":"test","next_action":"Must fail"})).unwrap();
        assert!(matches!(
            store.execute(&request(Operation::HandoffCreate(Box::new(scoped_handoff)))),
            Err(Error::Conflict(_))
        ));
        for path in ["../outside", "/absolute", "a//b", "a/./b"] {
            let mut bad = input(&project);
            bad.acceptance_criteria[0].path = path.into();
            assert!(store.execute(&request(Operation::TaskCreate(bad))).is_err());
        }
        std::fs::write(dir.path().join("artifact"), b"finished").unwrap();
        let mut reviewed = input(&project);
        reviewed.review_required = true;
        let reviewed = store
            .execute(&request(Operation::TaskCreate(reviewed)))
            .unwrap();
        let reviewed = validate(&mut store, reviewed);
        let reviewed = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(
                &reviewed,
            ))))
            .unwrap();
        assert!(matches!(
            store.execute(&request(Operation::TaskTransition(move_to(
                &reviewed,
                TaskState::Completed
            )))),
            Err(Error::Conflict(_))
        ));
        let done = validate(&mut store, parent);
        let done = store
            .execute(&request(Operation::TaskCheckpoint(checkpoint_input(&done))))
            .unwrap();
        store
            .execute(&request(Operation::TaskTransition(move_to(
                &done,
                TaskState::Completed,
            ))))
            .unwrap();
        assert_eq!(
            store
                .execute(&request(Operation::TaskTransition(move_to(
                    &child,
                    TaskState::Ready
                ))))
                .unwrap()["state"],
            "ready"
        );
    }

    #[test]
    fn task_checkpoint_and_transition_audit_failures_rollback_every_revision() {
        let (_dir, _lock, mut store, project) = fixture();
        let create = request(Operation::TaskCreate(input(&project)));
        let inject = |store: &Store| {
            store.connection.execute_batch("CREATE TRIGGER fail_task_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap()
        };
        let clear = |store: &Store| {
            store
                .connection
                .execute_batch("DROP TRIGGER fail_task_event")
                .unwrap()
        };
        inject(&store);
        assert!(store.execute(&create).is_err());
        clear(&store);
        let task = store.execute(&create).unwrap();
        assert_eq!(task["project_revision"], 2);
        let change = request(Operation::TaskTransition(move_to(
            &task,
            TaskState::Triaged,
        )));
        inject(&store);
        assert!(store.execute(&change).is_err());
        clear(&store);
        let task = store.execute(&change).unwrap();
        assert_eq!(task["task_revision"], 2);
        assert_eq!(task["project_revision"], 3);
        let checkpoint = request(Operation::TaskCheckpoint(checkpoint_input(&task)));
        inject(&store);
        assert!(store.execute(&checkpoint).is_err());
        clear(&store);
        let task = store.execute(&checkpoint).unwrap();
        assert_eq!(task["task_revision"], 3);
        assert_eq!(task["project_revision"], 4);
        assert!(
            store
                .connection
                .execute("DELETE FROM task_checkpoints", [])
                .is_err()
        );
        assert!(
            store
                .connection
                .execute("UPDATE task_transitions SET reason='forged'", [])
                .is_err()
        );
        assert_eq!(store.execute(&checkpoint).unwrap(), task);
    }
}
