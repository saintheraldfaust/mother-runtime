use super::{apply, event, page, pagination, revision, text};
use crate::{
    Error, Result,
    continuity::{DecisionInput, HandoffInput},
    protocol::{Operation, Request},
};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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

fn advance(tx: &Transaction<'_>, project: &str, expected: Option<u64>) -> Result<i64> {
    let current = revision(tx, project)?;
    if expected.is_some_and(|n| n != current as u64) {
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

const DECISION_COLUMNS: &str = "sequence,id,project_id,title,decision,rationale,alternatives,source,revision,status,supersedes,superseded_by,created_at";
fn decision_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    // JSON is constrained by SQLite and produced from the typed input.
    let alternatives: String = r.get(6)?;
    Ok(
        json!({"sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"project_id":r.get::<_,String>(2)?,
        "title":r.get::<_,String>(3)?,"decision":r.get::<_,String>(4)?,"rationale":r.get::<_,String>(5)?,
        "alternatives":serde_json::from_str::<Value>(&alternatives).map_err(|e|rusqlite::Error::FromSqlConversionFailure(6,rusqlite::types::Type::Text,Box::new(e)))?,
        "source":r.get::<_,String>(7)?,"revision":r.get::<_,i64>(8)?,"status":r.get::<_,String>(9)?,
        "supersedes":r.get::<_,Option<String>>(10)?,"superseded_by":r.get::<_,Option<String>>(11)?,
        "created_at":r.get::<_,String>(12)?,"trust":"unverified_submission"}),
    )
}

pub(super) fn decision(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let mut value = tx
        .query_row(
            &format!("SELECT {DECISION_COLUMNS} FROM decisions WHERE id=?1"),
            [id],
            decision_row,
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("decision {id} does not exist")))?;
    value["current_project_revision"] = json!(revision(tx, value["project_id"].as_str().unwrap())?);
    Ok(value)
}

pub(super) fn decisions(
    tx: &Transaction<'_>,
    project: &str,
    history: bool,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let current = revision(tx, project)?;
    let mut stmt=tx.prepare(&format!("SELECT {DECISION_COLUMNS} FROM decisions WHERE project_id=?1 AND (?2 OR status='active') AND sequence>?3 ORDER BY sequence LIMIT ?4"))?;
    let items = stmt
        .query_map(
            params![project, history, after as i64, limit + 1],
            decision_row,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["project_id"] = json!(project);
    result["revision"] = json!(current);
    Ok(result)
}

pub(super) fn record_decision(
    tx: &Transaction<'_>,
    request: &Request,
    input: &DecisionInput,
) -> Result<Value> {
    text(&input.title, "title", 256)?;
    text(&input.decision, "decision", 4096)?;
    text(&input.rationale, "rationale", 4096)?;
    text(&input.source, "source", 256)?;
    strings(&input.alternatives, "alternatives", 8, 512)?;
    if let Some(old) = &input.supersedes {
        let previous = decision(tx, old)?;
        if previous["project_id"] != input.project_id || previous["status"] != "active" {
            return Err(Error::Conflict(
                "supersession requires an active decision in the same project".into(),
            ));
        }
    }
    let next = advance(tx, &input.project_id, input.expected_revision)?;
    let id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO decisions(id,project_id,title,decision,rationale,alternatives,source,revision,supersedes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![id,input.project_id,input.title,input.decision,input.rationale,serde_json::to_string(&input.alternatives)?,input.source,next,input.supersedes])?;
    if let Some(old) = &input.supersedes {
        tx.execute(
            "UPDATE decisions SET status='superseded',superseded_by=?1 WHERE id=?2",
            params![id, old],
        )?;
        event(
            tx,
            request,
            &input.project_id,
            old,
            "decision.superseded",
            &input.source,
            next,
        )?;
    }
    event(
        tx,
        request,
        &input.project_id,
        &id,
        "decision.recorded",
        &input.source,
        next,
    )?;
    decision(tx, &id)
}

fn validate_handoff(input: &HandoffInput) -> Result<()> {
    text(&input.objective, "objective", 4096)?;
    text(&input.next_action, "next_action", 4096)?;
    text(&input.source, "source", 256)?;
    for (name, value) in [
        ("originating_host", &input.originating_host),
        ("originating_session", &input.originating_session),
        ("expected_recipient", &input.expected_recipient),
    ] {
        if let Some(value) = value {
            text(value, name, 256)?;
        }
    }
    for (name, values) in [
        ("work_completed", &input.work_completed),
        ("work_remaining", &input.work_remaining),
        ("changed_files", &input.changed_files),
        ("git_references", &input.git_references),
        ("test_results", &input.test_results),
        ("blockers", &input.blockers),
        ("unresolved_questions", &input.unresolved_questions),
        ("hypotheses", &input.hypotheses),
        ("provenance_references", &input.provenance_references),
    ] {
        strings(values, name, 16, 1024)?;
    }
    if serde_json::to_vec(input)?.len() > 32_768 {
        return Err(Error::Invalid(
            "handoff submission exceeds 32768 serialized bytes".into(),
        ));
    }
    Ok(())
}

fn now(tx: &Transaction<'_>) -> Result<String> {
    Ok(
        tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
            r.get(0)
        })?,
    )
}

fn snapshot(
    tx: &Transaction<'_>,
    request: &Request,
    project: &str,
    focused_task: Option<&str>,
) -> Result<Value> {
    let context_after:i64=tx.query_row("SELECT sequence FROM context_records WHERE project_id=?1 ORDER BY sequence DESC LIMIT 1 OFFSET 20",[project],|r|r.get(0)).optional()?.unwrap_or(0);
    let scoped = |operation| Request {
        operation,
        ..request.clone()
    };
    let project_data = apply(
        tx,
        &scoped(Operation::ProjectGet {
            project_id: project.into(),
        }),
    )?;
    let context = apply(
        tx,
        &scoped(Operation::ContextGet {
            project_id: project.into(),
            after: context_after as u64,
            limit: 20,
        }),
    )?;
    // Snapshot selection must not use a byte-paginated prefix that could omit newest decisions.
    let ids=tx.prepare("SELECT id FROM decisions WHERE project_id=?1 AND status='active' ORDER BY sequence DESC LIMIT 20")?.query_map([project],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut active = ids
        .iter()
        .map(|id| decision(tx, id))
        .collect::<Result<Vec<_>>>()?;
    active.reverse();
    let (task_records, current_task, task_total) =
        super::tasks::snapshot_tasks(tx, project, focused_task)?;
    let context_total: i64 = tx.query_row(
        "SELECT count(*) FROM context_records WHERE project_id=?1",
        [project],
        |r| r.get(0),
    )?;
    let decision_total: i64 = tx.query_row(
        "SELECT count(*) FROM decisions WHERE project_id=?1 AND status='active'",
        [project],
        |r| r.get(0),
    )?;
    let mut result = json!({"format_version":2,"project":project_data,"captured_at":now(tx)?,
        "context_records":context["items"],"active_decisions":active,
        "tasks":task_records,"current_task":current_task,"omitted_tasks":0,
        "omitted_context_records":0,"omitted_active_decisions":0,
        "unavailable_scopes":["persona","authorization_policy","verified_repository_base_commit"],
        "trust":"unverified_submissions_with_runtime_record_provenance"});
    // Bound serialized size even when submitted strings contain many escaped characters.
    while serde_json::to_vec(&result)?.len() + 96 > 192 * 1024 {
        let key = if !result["context_records"].as_array().unwrap().is_empty() {
            "context_records"
        } else if !result["active_decisions"].as_array().unwrap().is_empty() {
            "active_decisions"
        } else {
            "tasks"
        };
        let items = result[key].as_array_mut().unwrap();
        if items.is_empty() {
            return Err(Error::Invalid(
                "snapshot metadata exceeds size limit".into(),
            ));
        }
        items.remove(0);
    }
    result["omitted_context_records"] =
        json!(context_total - result["context_records"].as_array().unwrap().len() as i64);
    result["omitted_active_decisions"] =
        json!(decision_total - result["active_decisions"].as_array().unwrap().len() as i64);
    result["omitted_tasks"] = json!(task_total - result["tasks"].as_array().unwrap().len() as i64);
    Ok(result)
}

fn hash(bytes: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes.as_bytes()) {
        write!(&mut result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}

pub(super) fn create_handoff(
    tx: &Transaction<'_>,
    request: &Request,
    input: &HandoffInput,
) -> Result<Value> {
    validate_handoff(input)?;
    let captured = revision(tx, &input.project_id)?;
    let snapshot = snapshot(tx, request, &input.project_id, input.task_id.as_deref())?;
    let next = advance(tx, &input.project_id, input.expected_revision)?;
    let id = Uuid::new_v4().to_string();
    let created = now(tx)?;
    let package = json!({"format_version":1,"id":id,"project_id":input.project_id,"snapshot_revision":captured,"revision":next,
        "created_at":created,"submission":input,"snapshot":snapshot,"submission_trust":"unverified_submission",
        "verification":{"work_verified_by_runtime":false,"references_verified_by_runtime":false,"human_approval_recorded":false}});
    let encoded = serde_json::to_string(&package)?;
    if encoded.len() > 512 * 1024 {
        return Err(Error::Invalid("handoff package exceeds 512 KiB".into()));
    }
    tx.execute("INSERT INTO handoffs(id,project_id,snapshot_revision,revision,package_json,sha256,source,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![id,input.project_id,captured,next,encoded,hash(&encoded),input.source,created])?;
    event(
        tx,
        request,
        &input.project_id,
        &id,
        "handoff.created",
        &input.source,
        next,
    )?;
    handoff(tx, &id)
}

pub(super) fn handoff(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let (project,captured,created,encoded,digest):(String,i64,i64,String,String)=tx.query_row(
        "SELECT project_id,snapshot_revision,revision,package_json,sha256 FROM handoffs WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))
        .optional()?.ok_or_else(||Error::NotFound(format!("handoff {id} does not exist")))?;
    if hash(&encoded) != digest {
        return Err(Error::Integrity(
            "handoff package content hash mismatch".into(),
        ));
    }
    let package: Value = serde_json::from_str(&encoded)?;
    let current = revision(tx, &project)?;
    let claim:Option<Value>=tx.query_row("SELECT recipient,revision,claimed_at FROM handoff_claims WHERE handoff_id=?1",[id],|r|Ok(json!({"recipient":r.get::<_,String>(0)?,"revision":r.get::<_,i64>(1)?,"claimed_at":r.get::<_,String>(2)?}))).optional()?;
    Ok(
        json!({"id":id,"project_id":project,"snapshot_revision":captured,"revision":created,"current_project_revision":current,
        "project_changed_since_creation":current!=created,"claim":claim,"package":package,"package_json":encoded,"sha256":digest,"hash_encoding":"sha256_of_package_json_utf8"}),
    )
}

pub(super) fn handoffs(
    tx: &Transaction<'_>,
    project: &str,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let current = revision(tx, project)?;
    let mut stmt=tx.prepare("SELECT h.sequence,h.id,h.snapshot_revision,h.revision,h.sha256,h.source,h.created_at,c.recipient FROM handoffs h LEFT JOIN handoff_claims c ON c.handoff_id=h.id WHERE h.project_id=?1 AND h.sequence>?2 ORDER BY h.sequence LIMIT ?3")?;
    let items=stmt.query_map(params![project,after as i64,limit+1],|r|Ok(json!({"sequence":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,
        "snapshot_revision":r.get::<_,i64>(2)?,"revision":r.get::<_,i64>(3)?,"sha256":r.get::<_,String>(4)?,"source":r.get::<_,String>(5)?,
        "created_at":r.get::<_,String>(6)?,"claimed_by":r.get::<_,Option<String>>(7)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut result = page(items, limit);
    result["project_id"] = json!(project);
    result["revision"] = json!(current);
    Ok(result)
}

pub(super) fn resume(
    tx: &Transaction<'_>,
    request: &Request,
    id: &str,
    recipient: &str,
    expected: u64,
) -> Result<Value> {
    text(recipient, "recipient", 256)?;
    let existing = handoff(tx, id)?;
    if !existing["claim"].is_null() {
        return Err(Error::Conflict(
            "handoff is already claimed; only an identical request retry may replay the claim"
                .into(),
        ));
    }
    let project = existing["project_id"].as_str().unwrap();
    if existing["project_changed_since_creation"] == true {
        return Err(Error::Conflict("project changed after this handoff was created; retrieve live context and create a fresh handoff".into()));
    }
    if let Some(target) = existing["package"]["submission"]["expected_recipient"].as_str()
        && target != recipient
    {
        return Err(Error::Conflict(
            "recipient does not match the handoff routing label".into(),
        ));
    }
    let next = advance(tx, project, Some(expected))?;
    tx.execute(
        "INSERT INTO handoff_claims(handoff_id,recipient,revision) VALUES(?1,?2,?3)",
        params![id, recipient, next],
    )?;
    event(tx, request, project, id, "handoff.claimed", recipient, next)?;
    handoff(tx, id)
}
