use super::{page, pagination, text};
use crate::{
    Error, Result,
    github::{
        GithubIdentityAttach, GithubIdentityRevoke, GithubIdentityStatus, valid_host, valid_login,
        valid_profile_id,
    },
    personas::{Capability, PersonaStatus},
};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde_json::{Value, json};
use std::collections::HashSet;
use uuid::Uuid;

const IDENTITY_COLUMNS: &str = "i.sequence,i.id,i.host,i.account_id,i.login,i.display_name,i.persona_id,i.persona_revision,i.operations,i.status,i.revision,i.verified_at,i.created_at,i.updated_at,p.status,p.revision";

fn parsed(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn identity_row(r: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "sequence":r.get::<_,i64>(0)?,
        "id":r.get::<_,String>(1)?,
        "host":r.get::<_,String>(2)?,
        "account_id":r.get::<_,i64>(3)?,
        "login":r.get::<_,String>(4)?,
        "display_name":r.get::<_,Option<String>>(5)?,
        "persona_id":r.get::<_,String>(6)?,
        "authorized_persona_revision":r.get::<_,i64>(7)?,
        "operations":parsed(r.get(8)?)?,
        "status":r.get::<_,String>(9)?,
        "revision":r.get::<_,i64>(10)?,
        "verified_at":r.get::<_,String>(11)?,
        "created_at":r.get::<_,String>(12)?,
        "updated_at":r.get::<_,String>(13)?,
        "current_persona_status":r.get::<_,String>(14)?,
        "current_persona_revision":r.get::<_,i64>(15)?,
        "kind":"github_identity",
        "verification":{
            "method":"gh_api_user",
            "read_only":true,
            "credentials_stored":false,
            "account_verified":true,
            "human_authenticated":false,
            "authorization_source":"local_operator_cli"
        },
        "limits":{
            "single_os_user":true,
            "mandatory_persona_assertion":false,
            "arbitrary_github_operations":false
        }
    }))
}

pub(super) fn get(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    tx.query_row(
        &format!(
            "SELECT {IDENTITY_COLUMNS} FROM github_identities i JOIN personas p ON p.id=i.persona_id WHERE i.id=?1"
        ),
        [id],
        identity_row,
    )
    .optional()?
    .ok_or_else(|| Error::NotFound(format!("GitHub identity {id} does not exist")))
}

fn snapshot(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    get(tx, id)
}

fn record_revision(tx: &Transaction<'_>, id: &str) -> Result<Value> {
    let value = snapshot(tx, id)?;
    tx.execute(
        "INSERT INTO github_identity_revisions(id,identity_id,revision,config) VALUES(?1,?2,?3,?4)",
        params![
            Uuid::new_v4().to_string(),
            id,
            value["revision"].as_i64().unwrap(),
            serde_json::to_string(&value)?
        ],
    )?;
    Ok(value)
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
        "INSERT INTO github_identity_events(id,identity_id,action,revision,details) VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            id,
            action,
            revision,
            serde_json::to_string(&json!({"request_id":request_id,"identity":details}))?
        ],
    )?;
    Ok(())
}

fn validate(input: &GithubIdentityAttach) -> Result<()> {
    if !valid_profile_id(&input.profile_id) {
        return Err(Error::Invalid(
            "profile_id must be a canonical lowercase UUID".into(),
        ));
    }
    if !valid_host(&input.host) {
        return Err(Error::Invalid(
            "host must be a lowercase DNS hostname without a scheme or path".into(),
        ));
    }
    if !valid_login(&input.login) {
        return Err(Error::Invalid(
            "login must be a 1..=39 character GitHub username".into(),
        ));
    }
    if input.account_id == 0 || input.account_id > i64::MAX as u64 {
        return Err(Error::Invalid(
            "account_id must be a positive signed 64-bit integer".into(),
        ));
    }
    if let Some(name) = &input.display_name {
        text(name, "display_name", 256)?;
    }
    if input.operations.is_empty() || input.operations.len() > 8 {
        return Err(Error::Invalid(
            "operations requires 1..=8 supported read-only operations".into(),
        ));
    }
    let mut seen = HashSet::new();
    for operation in &input.operations {
        if !seen.insert(operation.as_str()) {
            return Err(Error::Invalid("duplicate GitHub operation".into()));
        }
    }
    if serde_json::to_vec(input)?.len() > 8 * 1024 {
        return Err(Error::Invalid(
            "GitHub identity evidence exceeds 8 KiB".into(),
        ));
    }
    Ok(())
}

fn validate_persona(tx: &Transaction<'_>, input: &GithubIdentityAttach) -> Result<Value> {
    let persona = super::personas::get(tx, &input.persona_id)?;
    let current = persona["revision"].as_i64().unwrap();
    if input.expected_persona_revision != current as u64 {
        return Err(Error::Conflict(format!(
            "stale persona revision; current revision is {current}"
        )));
    }
    if persona["status"] != PersonaStatus::Active.as_str() {
        return Err(Error::Conflict(
            "GitHub identities require an active persona".into(),
        ));
    }
    let allowed = persona["capabilities"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item == Capability::GithubRead.as_str())
    });
    if !allowed {
        return Err(Error::Conflict("persona does not allow github.read".into()));
    }
    Ok(persona)
}

fn persona_in_use(tx: &Transaction<'_>, persona_id: &str, except: Option<&str>) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM github_identities WHERE persona_id=?1 AND status='active' AND (?2 IS NULL OR id<>?2))",
        params![persona_id, except],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict(
            "persona already has an active GitHub identity".into(),
        ));
    }
    Ok(())
}

fn profile_in_use(tx: &Transaction<'_>, profile_id: &str, except: Option<&str>) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM github_identities WHERE profile_id=?1 AND (?2 IS NULL OR id<>?2))",
        params![profile_id, except],
        |r| r.get(0),
    )?;
    if exists {
        return Err(Error::Conflict(
            "GitHub profile is already linked to another identity".into(),
        ));
    }
    Ok(())
}

pub(super) fn list(
    tx: &Transaction<'_>,
    status: Option<GithubIdentityStatus>,
    persona: Option<&str>,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    if let Some(persona) = persona {
        super::personas::get(tx, persona)?;
    }
    let mut stmt = tx.prepare(&format!(
        "SELECT {IDENTITY_COLUMNS} FROM github_identities i JOIN personas p ON p.id=i.persona_id WHERE (?1 IS NULL OR i.status=?1) AND (?2 IS NULL OR i.persona_id=?2) AND i.sequence>?3 ORDER BY i.sequence LIMIT ?4"
    ))?;
    let items = stmt
        .query_map(
            params![
                status.map(GithubIdentityStatus::as_str),
                persona,
                after as i64,
                limit + 1
            ],
            identity_row,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(page(items, limit))
}

pub(super) fn history(
    tx: &Transaction<'_>,
    identity_id: &str,
    after: u64,
    limit: u32,
) -> Result<Value> {
    pagination(after, limit)?;
    let identity = get(tx, identity_id)?;
    let mut stmt = tx.prepare("SELECT sequence,revision,config,created_at FROM github_identity_revisions WHERE identity_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
    let items = stmt
        .query_map(params![identity_id, after as i64, limit + 1], |r| {
            Ok(json!({
                "sequence":r.get::<_,i64>(0)?,
                "revision":r.get::<_,i64>(1)?,
                "config":parsed(r.get(2)?)?,
                "created_at":r.get::<_,String>(3)?
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = page(items, limit);
    result["identity_id"] = json!(identity_id);
    result["revision"] = identity["revision"].clone();
    Ok(result)
}

pub(super) fn attach(
    tx: &Transaction<'_>,
    request_id: &str,
    input: &GithubIdentityAttach,
) -> Result<Value> {
    validate(input)?;
    validate_persona(tx, input)?;
    profile_in_use(tx, &input.profile_id, None)?;
    let existing = tx
        .query_row(
            "SELECT id,status FROM github_identities WHERE host=?1 AND (account_id=?2 OR login=?3 COLLATE NOCASE) ORDER BY sequence LIMIT 1",
            params![input.host, input.account_id as i64, input.login],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((id, status)) = existing {
        if status == GithubIdentityStatus::Active.as_str() {
            return Err(Error::Conflict(
                "GitHub identity is already authorized".into(),
            ));
        }
        persona_in_use(tx, &input.persona_id, Some(&id))?;
        profile_in_use(tx, &input.profile_id, Some(&id))?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM github_identities WHERE id=?1",
            [&id],
            |r| r.get(0),
        )?;
        let next = revision
            .checked_add(1)
            .ok_or_else(|| Error::Conflict("GitHub identity revision exhausted".into()))?;
        tx.execute(
            "UPDATE github_identities SET host=?1,account_id=?2,login=?3,display_name=?4,profile_id=?5,persona_id=?6,persona_revision=?7,operations=?8,status='active',revision=?9,verified_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?10",
            params![
                input.host,
                input.account_id as i64,
                input.login,
                input.display_name,
                input.profile_id,
                input.persona_id,
                input.expected_persona_revision as i64,
                serde_json::to_string(&input.operations.iter().map(|o| o.as_str()).collect::<Vec<_>>())?,
                next,
                id
            ],
        )?;
        let value = record_revision(tx, &id)?;
        record_event(
            tx,
            request_id,
            &id,
            "github_identity.reauthorized",
            next,
            &value,
        )?;
        return Ok(value);
    }
    persona_in_use(tx, &input.persona_id, None)?;
    let id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO github_identities(id,host,account_id,login,display_name,profile_id,persona_id,persona_revision,operations,verification_method,status,revision,verified_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'gh_api_user','active',1,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            id,
            input.host,
            input.account_id as i64,
            input.login,
            input.display_name,
            input.profile_id,
            input.persona_id,
            input.expected_persona_revision as i64,
            serde_json::to_string(&input.operations.iter().map(|o| o.as_str()).collect::<Vec<_>>())?
        ],
    )?;
    let value = record_revision(tx, &id)?;
    record_event(tx, request_id, &id, "github_identity.authorized", 1, &value)?;
    Ok(value)
}

pub(super) fn revoke(
    tx: &Transaction<'_>,
    request_id: &str,
    change: &GithubIdentityRevoke,
) -> Result<Value> {
    let existing = get(tx, &change.identity_id)?;
    let current = existing["revision"].as_i64().unwrap();
    if change.expected_revision != current as u64 {
        return Err(Error::Conflict(format!(
            "stale GitHub identity revision; current revision is {current}"
        )));
    }
    if existing["status"] == GithubIdentityStatus::Revoked.as_str() {
        return Err(Error::Conflict("GitHub identity is already revoked".into()));
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| Error::Conflict("GitHub identity revision exhausted".into()))?;
    tx.execute(
        "UPDATE github_identities SET status='revoked',revision=?1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?2",
        params![next, change.identity_id],
    )?;
    let value = record_revision(tx, &change.identity_id)?;
    record_event(
        tx,
        request_id,
        &change.identity_id,
        "github_identity.revoked",
        next,
        &value,
    )?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config,
        github::{GithubOperation, valid_profile_id},
        personas::{PersonaInput, PersonaStatusChange},
        protocol::{Operation, PROTOCOL_VERSION, Request},
        store::Store,
    };

    fn fixture() -> (tempfile::TempDir, std::fs::File, Store) {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
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

    fn persona(store: &mut Store, capabilities: Vec<Capability>) -> String {
        store
            .execute(&request(Operation::PersonaCreate(PersonaInput {
                name: format!("persona-{}", Uuid::new_v4()),
                role: "implementation_engineer".into(),
                description: "Implements reviewed work".into(),
                responsibilities: vec![],
                prohibitions: vec![],
                capabilities,
                project_id: None,
                preferred_runtime: None,
                github_identity: None,
                context_retrieval: None,
                review_required: false,
                max_concurrency: 1,
                budget_units: None,
            })))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }

    fn evidence(persona_id: &str) -> GithubIdentityAttach {
        GithubIdentityAttach {
            profile_id: Uuid::new_v4().to_string(),
            host: "github.com".into(),
            login: "alice".into(),
            account_id: 4242,
            display_name: Some("Alice Example".into()),
            persona_id: persona_id.into(),
            expected_persona_revision: 1,
            operations: vec![GithubOperation::Read],
        }
    }

    #[test]
    fn identities_require_a_capable_active_persona_and_support_revocation() {
        let (_dir, _lock, mut store) = fixture();
        let persona_id = persona(&mut store, vec![Capability::GithubRead]);
        let created = store
            .execute(&request(Operation::GithubIdentityAttach(evidence(
                &persona_id,
            ))))
            .unwrap();
        assert_eq!(created["status"], "active");
        assert_eq!(created["revision"], 1);
        assert_eq!(created["login"], "alice");
        assert_eq!(created["verification"]["credentials_stored"], false);
        assert_eq!(created["current_persona_status"], "active");

        let listed = store
            .execute(&request(Operation::GithubIdentitiesList {
                status: Some(GithubIdentityStatus::Active),
                persona_id: Some(persona_id.clone()),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 1);

        let duplicate = store.execute(&request(Operation::GithubIdentityAttach(evidence(
            &persona_id,
        ))));
        assert!(matches!(duplicate, Err(Error::Conflict(_))));

        let revoked = store
            .execute(&request(Operation::GithubIdentityRevoke(
                GithubIdentityRevoke {
                    identity_id: created["id"].as_str().unwrap().into(),
                    expected_revision: 1,
                },
            )))
            .unwrap();
        assert_eq!(revoked["status"], "revoked");
        assert_eq!(revoked["revision"], 2);
        assert!(matches!(
            store.execute(&request(Operation::GithubIdentityRevoke(
                GithubIdentityRevoke {
                    identity_id: created["id"].as_str().unwrap().into(),
                    expected_revision: 1,
                },
            ))),
            Err(Error::Conflict(_))
        ));

        let mut reauthored = evidence(&persona_id);
        reauthored.expected_persona_revision = 1;
        let reauthorized = store
            .execute(&request(Operation::GithubIdentityAttach(reauthored)))
            .unwrap();
        assert_eq!(reauthorized["status"], "active");
        assert_eq!(reauthorized["revision"], 3);

        let history = store
            .execute(&request(Operation::GithubIdentityHistory {
                identity_id: created["id"].as_str().unwrap().into(),
                after: 0,
                limit: 20,
            }))
            .unwrap();
        assert_eq!(history["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn validation_and_stale_policy_are_rejected() {
        let (_dir, _lock, mut store) = fixture();
        let without_read = persona(&mut store, vec![Capability::TaskCreate]);
        assert!(matches!(
            store.execute(&request(Operation::GithubIdentityAttach(evidence(
                &without_read,
            )))),
            Err(Error::Conflict(_))
        ));

        let persona_id = persona(&mut store, vec![Capability::GithubRead]);
        let mut invalid = evidence(&persona_id);
        invalid.host = "GitHub.com".into();
        assert!(matches!(
            store.execute(&request(Operation::GithubIdentityAttach(invalid))),
            Err(Error::Invalid(_))
        ));

        let mut stale = evidence(&persona_id);
        stale.expected_persona_revision = 2;
        assert!(matches!(
            store.execute(&request(Operation::GithubIdentityAttach(stale))),
            Err(Error::Conflict(_))
        ));

        let profile = Uuid::new_v4().to_string();
        assert!(valid_profile_id(&profile));
        assert!(!valid_profile_id("not-a-uuid"));

        let created = store
            .execute(&request(Operation::GithubIdentityAttach(evidence(
                &persona_id,
            ))))
            .unwrap();
        let updated = store
            .execute(&request(Operation::PersonaSetStatus(PersonaStatusChange {
                persona_id: persona_id.clone(),
                status: crate::personas::PersonaStatus::Archived,
                expected_revision: 1,
            })))
            .unwrap();
        assert_eq!(updated["status"], "archived");
        let archived = store
            .execute(&request(Operation::GithubIdentityGet {
                identity_id: created["id"].as_str().unwrap().into(),
            }))
            .unwrap();
        assert_eq!(archived["current_persona_status"], "archived");
        let mut other = evidence(&persona_id);
        other.login = "bob".into();
        other.account_id = 7;
        assert!(matches!(
            store.execute(&request(Operation::GithubIdentityAttach(other))),
            Err(Error::Conflict(_))
        ));
    }
}
