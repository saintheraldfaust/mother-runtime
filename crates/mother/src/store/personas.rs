use super::{page, pagination, revision, text};
use crate::{
    Error, Result,
    personas::{
        Capability, PersonaClaim, PersonaInput, PersonaStatus, PersonaStatusChange, PersonaUpdate,
    },
};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

const PERSONA_COLUMNS: &str = "sequence,id,name,role,description,responsibilities,prohibitions,capabilities,project_id,preferred_runtime,github_identity,context_retrieval,review_required,max_concurrency,budget_units,status,revision,created_at,updated_at";

fn parsed(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn persona_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "sequence":r.get::<_,i64>(0)?,
        "id":r.get::<_,String>(1)?,
        "name":r.get::<_,String>(2)?,
        "role":r.get::<_,String>(3)?,
        "description":r.get::<_,String>(4)?,
        "responsibilities":parsed(r.get(5)?)?,
        "prohibitions":parsed(r.get(6)?)?,
        "capabilities":parsed(r.get(7)?)?,
        "project_id":r.get::<_,Option<String>>(8)?,
        "preferred_runtime":r.get::<_,Option<String>>(9)?,
        "github_identity":r.get::<_,Option<String>>(10)?,
        "github_identity_verified":false,
        "context_retrieval":r.get::<_,Option<String>>(11)?,
        "review_required":r.get::<_,bool>(12)?,
        "max_concurrency":r.get::<_,i64>(13)?,
        "budget_units":r.get::<_,Option<i64>>(14)?,
        "status":r.get::<_,String>(15)?,
        "revision":r.get::<_,i64>(16)?,
        "created_at":r.get::<_,String>(17)?,
        "updated_at":r.get::<_,String>(18)?,
        "kind":"persona",
        "worker_execution":null
    }))
}

fn persona_snapshot(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    get(tx, id)
}

fn record_revision(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let snapshot = persona_snapshot(tx, id)?;
    tx.execute(
        "INSERT INTO persona_revisions(id,persona_id,revision,config) VALUES(?1,?2,?3,?4)",
        params![
            Uuid::new_v4().to_string(),
            id,
            snapshot["revision"].as_i64().unwrap(),
            serde_json::to_string(&snapshot)?
        ],
    )?;
    Ok(snapshot)
}

fn record_event(
    tx: &Transaction<'_>,
    request_id: &str,
    id: &str,
    action: &str,
    revision: i64,
    details: &Value,
) -> Result<()> {
    tx.execute(
        "INSERT INTO persona_events(id,persona_id,action,revision,details) VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            id,
            action,
            revision,
            serde_json::to_string(&json!({"request_id":request_id,"persona":details}))?
        ],
    )?;
    Ok(())
}

fn strings(values: &[String], name: &str, max_items: usize, max_bytes: usize) -> Result<()> {
    if values.len() > max_items {
        return Err(Error::Invalid(format!(
            "{name} allows at most {max_items} entries"
        )));
    }
    for value in values {
        text(value, name, max_bytes)?;
    }
    Ok(())
}

fn validate(input: &PersonaInput) -> Result<()> {
    text(&input.name, "name", 128)?;
    text(&input.role, "role", 64)?;
    text(&input.description, "description", 2048)?;
    strings(&input.responsibilities, "responsibilities", 16, 512)?;
    strings(&input.prohibitions, "prohibitions", 16, 512)?;
    for (name, value) in [
        ("preferred_runtime", &input.preferred_runtime),
        ("github_identity", &input.github_identity),
    ] {
        if let Some(value) = value {
            text(value, name, 256)?;
        }
    }
    if let Some(value) = &input.context_retrieval {
        text(value, "context_retrieval", 2048)?;
    }
    if !(1..=32).contains(&input.max_concurrency) {
        return Err(Error::Invalid("max_concurrency must be in 1..=32".into()));
    }
    if input.budget_units.is_some_and(|v| v > 1_000_000) {
        return Err(Error::Invalid(
            "budget_units must be at most 1000000".into(),
        ));
    }
    if input.capabilities.len() > 32 {
        return Err(Error::Invalid(
            "capabilities allows at most 32 entries".into(),
        ));
    }
    let mut seen = HashSet::new();
    for capability in &input.capabilities {
        if !seen.insert(capability.as_str()) {
            return Err(Error::Invalid("duplicate capability".into()));
        }
    }
    if serde_json::to_vec(input)?.len() > 16 * 1024 {
        return Err(Error::Invalid(
            "persona definition exceeds 16 KiB of serialized input".into(),
        ));
    }
    Ok(())
}

fn name_available(tx: &Transaction<'_>, name: &str, id: Option<&str>) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM personas WHERE name=?1 AND (?2 IS NULL OR id<>?2))",
        params![name, id],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict("persona name is already in use".into()));
    }
    Ok(())
}

fn scope(tx: &Transaction<'_>, input: &PersonaInput) -> Result<()> {
    if let Some(project) = &input.project_id {
        revision(tx, project)?;
    }
    Ok(())
}

pub(super) fn get(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    tx.query_row(
        &format!("SELECT {PERSONA_COLUMNS} FROM personas WHERE id=?1"),
        [id],
        persona_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("persona {id} does not exist")))
}

pub(super) fn create(
    tx: &Transaction<'_>,
    request_id: &str,
    input: &PersonaInput,
) -> Result<Value> {
    validate(input)?;
    scope(tx, input)?;
    name_available(tx, &input.name, None)?;
    let id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO personas(id,name,role,description,responsibilities,prohibitions,capabilities,project_id,preferred_runtime,github_identity,context_retrieval,review_required,max_concurrency,budget_units,status,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,'active',1)",
        params![id,input.name,input.role,input.description,serde_json::to_string(&input.responsibilities)?,serde_json::to_string(&input.prohibitions)?,
        serde_json::to_string(&input.capabilities.iter().map(|c|c.as_str()).collect::<Vec<_>>())?,input.project_id,input.preferred_runtime,input.github_identity,input.context_retrieval,
        input.review_required,input.max_concurrency,input.budget_units.map(|v|v as i64)])?;
    let snapshot = record_revision(tx, &id)?;
    record_event(tx, request_id, &id, "persona.created", 1, &snapshot)?;
    Ok(snapshot)
}

pub(super) fn update(
    tx: &Transaction<'_>,
    request_id: &str,
    update: &PersonaUpdate,
) -> Result<Value> {
    validate(&update.input)?;
    scope(tx, &update.input)?;
    let existing = get(tx, &update.persona_id)?;
    let current = existing["revision"].as_i64().unwrap();
    if update.expected_revision != current as u64 {
        return Err(Error::Conflict(format!(
            "stale persona revision; current revision is {current}"
        )));
    }
    if existing["status"] == "archived" {
        return Err(Error::Conflict("archived persona cannot be edited".into()));
    }
    name_available(tx, &update.input.name, Some(&update.persona_id))?;
    let next = current
        .checked_add(1)
        .ok_or_else(|| Error::Conflict("persona revision exhausted".into()))?;
    tx.execute("UPDATE personas SET name=?1,role=?2,description=?3,responsibilities=?4,prohibitions=?5,capabilities=?6,project_id=?7,preferred_runtime=?8,github_identity=?9,context_retrieval=?10,review_required=?11,max_concurrency=?12,budget_units=?13,revision=?14,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?15",
        params![update.input.name,update.input.role,update.input.description,serde_json::to_string(&update.input.responsibilities)?,serde_json::to_string(&update.input.prohibitions)?,
        serde_json::to_string(&update.input.capabilities.iter().map(|c|c.as_str()).collect::<Vec<_>>())?,update.input.project_id,update.input.preferred_runtime,update.input.github_identity,update.input.context_retrieval,
        update.input.review_required,update.input.max_concurrency,update.input.budget_units.map(|v|v as i64),next,update.persona_id])?;
    let snapshot = record_revision(tx, &update.persona_id)?;
    record_event(
        tx,
        request_id,
        &update.persona_id,
        "persona.updated",
        next,
        &snapshot,
    )?;
    Ok(snapshot)
}

pub(super) fn set_status(
    tx: &Transaction<'_>,
    request_id: &str,
    change: &PersonaStatusChange,
) -> Result<Value> {
    let existing = get(tx, &change.persona_id)?;
    let current = existing["revision"].as_i64().unwrap();
    if change.expected_revision != current as u64 {
        return Err(Error::Conflict(format!(
            "stale persona revision; current revision is {current}"
        )));
    }
    if existing["status"] == "archived" && change.status != PersonaStatus::Archived {
        return Err(Error::Conflict(
            "archived persona cannot be reactivated; create a new revision of the definition"
                .into(),
        ));
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| Error::Conflict("persona revision exhausted".into()))?;
    tx.execute(
        "UPDATE personas SET status=?1,revision=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
        params![change.status.as_str(), next, change.persona_id],
    )?;
    let snapshot = record_revision(tx, &change.persona_id)?;
    record_event(
        tx,
        request_id,
        &change.persona_id,
        "persona.status_changed",
        next,
        &snapshot,
    )?;
    Ok(snapshot)
}

pub(super) fn list(
    tx: &Transaction<'_>,
    status: Option<PersonaStatus>,
    project: Option<&str>,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    if let Some(project) = project {
        revision(tx, project)?;
    }
    let mut stmt = tx.prepare(&format!(
        "SELECT {PERSONA_COLUMNS} FROM personas WHERE (?1 IS NULL OR status=?1) AND (?2 IS NULL OR project_id=?2) AND sequence>?3 ORDER BY sequence LIMIT ?4"
    ))?;
    let items = stmt
        .query_map(
            params![
                status.map(PersonaStatus::as_str),
                project,
                after as i64,
                limit + 1
            ],
            persona_row,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(page(items, limit))
}

pub(super) fn history(
    tx: &Transaction<'_>,
    persona_id: &str,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let persona = get(tx, persona_id)?;
    let mut stmt = tx.prepare("SELECT sequence,revision,config,created_at FROM persona_revisions WHERE persona_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
    let items = stmt
        .query_map(params![persona_id, after as i64, limit + 1], |r| {
            Ok(json!({
                "sequence":r.get::<_,i64>(0)?,
                "revision":r.get::<_,i64>(1)?,
                "config":parsed(r.get(2)?)?,
                "created_at":r.get::<_,String>(3)?
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["persona_id"] = json!(persona_id);
    result["revision"] = persona["revision"].clone();
    Ok(result)
}

/// Enforce the current persona policy at a protected runtime operation.
pub(super) fn authorize(
    tx: &Transaction<'_>,
    claim: &PersonaClaim,
    capability: Capability,
    project: &str,
) -> Result<Value> {
    let persona = get(tx, &claim.persona_id)?;
    let current = persona["revision"].as_i64().unwrap();
    if claim.expected_revision != current as u64 {
        return Err(Error::Conflict(format!(
            "stale persona revision; current revision is {current}"
        )));
    }
    if persona["status"] != "active" {
        return Err(Error::Conflict(format!(
            "persona is {} and cannot authorize operations",
            persona["status"].as_str().unwrap_or("inactive")
        )));
    }
    if let Some(scope) = persona["project_id"].as_str()
        && scope != project
    {
        return Err(Error::Conflict(
            "persona is scoped to a different project".into(),
        ));
    }
    let allowed = persona["capabilities"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item == capability.as_str()));
    if !allowed {
        return Err(Error::Conflict(format!(
            "persona does not allow {}",
            capability.as_str()
        )));
    }
    Ok(persona)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config,
        protocol::{Operation, PROTOCOL_VERSION, Request},
        store::Store,
        tasks::{ArtifactCriterion, TaskInput, TaskState, TaskTransition},
    };
    use uuid::Uuid;

    fn fixture() -> (tempfile::TempDir, std::fs::File, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        config::initialize(dir.path()).unwrap();
        let lock = config::lock(dir.path()).unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let project = store
            .execute(&request(Operation::ProjectAttach {
                name: "personas".into(),
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

    fn input(name: &str, project: Option<&str>, capabilities: Vec<Capability>) -> PersonaInput {
        PersonaInput {
            name: name.into(),
            role: "implementation_engineer".into(),
            description: "Implements bounded engineering tasks".into(),
            responsibilities: vec!["Implement reviewed changes".into()],
            prohibitions: vec!["Merge protected branches".into()],
            capabilities,
            project_id: project.map(str::to_owned),
            preferred_runtime: Some("codex".into()),
            github_identity: Some("unverified-label".into()),
            context_retrieval: Some("project-and-decisions".into()),
            review_required: false,
            max_concurrency: 1,
            budget_units: None,
        }
    }

    fn task_input(project: &str, persona: Option<PersonaClaim>) -> TaskInput {
        TaskInput {
            project_id: project.into(),
            title: "Persona-gated task".into(),
            objective: "Exercise runtime capability checks".into(),
            source: "test".into(),
            acceptance_criteria: vec![ArtifactCriterion {
                path: "artifact".into(),
                expected_sha256: None,
            }],
            dependencies: vec![],
            review_required: false,
            persona,
            expected_project_revision: None,
        }
    }

    fn transition(task: &Value, state: TaskState, persona: PersonaClaim) -> TaskTransition {
        TaskTransition {
            task_id: task["id"].as_str().unwrap().into(),
            state,
            source: "test".into(),
            reason: "explicit step".into(),
            owner_label: None,
            persona: Some(persona),
            expected_task_revision: task["task_revision"].as_u64().unwrap(),
            expected_project_revision: None,
        }
    }

    #[test]
    fn persona_revisions_status_and_history_are_durable() {
        let (_dir, _lock, mut store, project) = fixture();
        let created = store
            .execute(&request(Operation::PersonaCreate(input(
                "implementation-engineer",
                Some(&project),
                vec![Capability::TaskCreate],
            ))))
            .unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        assert_eq!(created["revision"], 1);
        assert_eq!(created["status"], "active");
        assert_eq!(created["github_identity_verified"], false);

        let mut changed = input(
            "implementation-engineer",
            Some(&project),
            vec![Capability::TaskCreate, Capability::TaskTransition],
        );
        changed.description = "Updated responsibilities".into();
        let updated = store
            .execute(&request(Operation::PersonaUpdate(PersonaUpdate {
                persona_id: id.clone(),
                expected_revision: 1,
                input: changed,
            })))
            .unwrap();
        assert_eq!(updated["revision"], 2);
        assert_eq!(updated["description"], "Updated responsibilities");

        let stale = store.execute(&request(Operation::PersonaUpdate(PersonaUpdate {
            persona_id: id.clone(),
            expected_revision: 1,
            input: input("implementation-engineer", Some(&project), vec![]),
        })));
        assert!(matches!(stale, Err(Error::Conflict(_))));

        let history = store
            .execute(&request(Operation::PersonaHistory {
                persona_id: id.clone(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(history["items"].as_array().unwrap().len(), 2);
        assert_eq!(history["items"][1]["revision"], 2);

        let disabled = store
            .execute(&request(Operation::PersonaSetStatus(PersonaStatusChange {
                persona_id: id.clone(),
                status: PersonaStatus::Disabled,
                expected_revision: 2,
            })))
            .unwrap();
        assert_eq!(disabled["status"], "disabled");
        assert_eq!(disabled["revision"], 3);
        let archived = store
            .execute(&request(Operation::PersonaSetStatus(PersonaStatusChange {
                persona_id: id.clone(),
                status: PersonaStatus::Archived,
                expected_revision: 3,
            })))
            .unwrap();
        assert_eq!(archived["status"], "archived");
        let reactivate =
            store.execute(&request(Operation::PersonaSetStatus(PersonaStatusChange {
                persona_id: id,
                status: PersonaStatus::Active,
                expected_revision: 4,
            })));
        assert!(matches!(reactivate, Err(Error::Conflict(_))));
    }

    #[test]
    fn persona_capabilities_use_live_revision_and_reject_stale_claims() {
        let (_dir, _lock, mut store, project) = fixture();
        let persona = store
            .execute(&request(Operation::PersonaCreate(input(
                "implementation-engineer",
                Some(&project),
                vec![Capability::TaskCreate, Capability::TaskTransition],
            ))))
            .unwrap();
        let persona_id = persona["id"].as_str().unwrap().to_owned();
        let claim = PersonaClaim {
            persona_id: persona_id.clone(),
            expected_revision: 1,
        };
        let task = store
            .execute(&request(Operation::TaskCreate(task_input(
                &project,
                Some(claim.clone()),
            ))))
            .unwrap();
        let task = store
            .execute(&request(Operation::TaskTransition(transition(
                &task,
                TaskState::Triaged,
                claim.clone(),
            ))))
            .unwrap();
        assert_eq!(task["latest_transition"]["persona_id"], persona_id);

        let revoked = store
            .execute(&request(Operation::PersonaUpdate(PersonaUpdate {
                persona_id: persona_id.clone(),
                expected_revision: 1,
                input: input(
                    "implementation-engineer",
                    Some(&project),
                    vec![Capability::TaskCreate],
                ),
            })))
            .unwrap();
        assert_eq!(revoked["revision"], 2);

        let stale = store.execute(&request(Operation::TaskTransition(transition(
            &task,
            TaskState::Ready,
            claim,
        ))));
        assert!(matches!(stale, Err(Error::Conflict(_))));

        let denied = store.execute(&request(Operation::TaskTransition(transition(
            &task,
            TaskState::Ready,
            PersonaClaim {
                persona_id: persona_id.clone(),
                expected_revision: 2,
            },
        ))));
        assert!(matches!(denied, Err(Error::Conflict(_))));

        let inactive = store
            .execute(&request(Operation::PersonaSetStatus(PersonaStatusChange {
                persona_id,
                status: PersonaStatus::Disabled,
                expected_revision: 2,
            })))
            .unwrap();
        assert_eq!(inactive["status"], "disabled");
    }
}
