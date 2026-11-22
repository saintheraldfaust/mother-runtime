CREATE TABLE github_identities (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    host TEXT NOT NULL,
    account_id INTEGER NOT NULL CHECK(account_id > 0),
    login TEXT NOT NULL,
    display_name TEXT,
    profile_id TEXT NOT NULL UNIQUE,
    persona_id TEXT NOT NULL REFERENCES personas(id),
    persona_revision INTEGER NOT NULL CHECK(persona_revision >= 1),
    operations TEXT NOT NULL CHECK(json_valid(operations)),
    verification_method TEXT NOT NULL CHECK(verification_method = 'gh_api_user'),
    status TEXT NOT NULL CHECK(status IN ('active','revoked')),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    verified_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX github_identities_account ON github_identities(host,account_id);
CREATE UNIQUE INDEX github_identities_login ON github_identities(host,login COLLATE NOCASE);
CREATE UNIQUE INDEX github_identities_active_persona ON github_identities(persona_id) WHERE status='active';
CREATE INDEX github_identities_by_persona ON github_identities(persona_id,status,sequence);
CREATE TABLE github_identity_revisions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    identity_id TEXT NOT NULL REFERENCES github_identities(id),
    revision INTEGER NOT NULL,
    config TEXT NOT NULL CHECK(json_valid(config)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(identity_id,revision)
);
CREATE INDEX github_identity_revisions_by_identity ON github_identity_revisions(identity_id,sequence);
CREATE TABLE github_identity_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    identity_id TEXT NOT NULL REFERENCES github_identities(id),
    action TEXT NOT NULL,
    revision INTEGER NOT NULL,
    details TEXT NOT NULL CHECK(json_valid(details)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TRIGGER github_identity_revisions_no_update BEFORE UPDATE ON github_identity_revisions BEGIN
    SELECT RAISE(ABORT,'GitHub identity revisions are immutable');
END;
CREATE TRIGGER github_identity_revisions_no_delete BEFORE DELETE ON github_identity_revisions BEGIN
    SELECT RAISE(ABORT,'GitHub identity revisions are immutable');
END;
CREATE TRIGGER github_identity_events_no_update BEFORE UPDATE ON github_identity_events BEGIN
    SELECT RAISE(ABORT,'GitHub identity events are immutable');
END;
CREATE TRIGGER github_identity_events_no_delete BEFORE DELETE ON github_identity_events BEGIN
    SELECT RAISE(ABORT,'GitHub identity events are immutable');
END;
PRAGMA user_version=5;
