CREATE TABLE decisions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL,
    decision TEXT NOT NULL,
    rationale TEXT NOT NULL,
    alternatives TEXT NOT NULL CHECK(json_valid(alternatives)),
    source TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 1),
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','superseded')),
    supersedes TEXT UNIQUE,
    superseded_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(id, project_id),
    FOREIGN KEY(supersedes, project_id) REFERENCES decisions(id, project_id),
    FOREIGN KEY(superseded_by, project_id) REFERENCES decisions(id, project_id),
    CHECK((status='active' AND superseded_by IS NULL) OR (status='superseded' AND superseded_by IS NOT NULL))
);
CREATE INDEX decisions_by_project ON decisions(project_id, status, sequence);
CREATE TABLE handoffs (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL REFERENCES projects(id),
    snapshot_revision INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK(revision = snapshot_revision + 1),
    package_json TEXT NOT NULL CHECK(json_valid(package_json)),
    sha256 TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(id, project_id)
);
CREATE INDEX handoffs_by_project ON handoffs(project_id, sequence);
CREATE TABLE handoff_claims (
    handoff_id TEXT PRIMARY KEY REFERENCES handoffs(id),
    recipient TEXT NOT NULL,
    revision INTEGER NOT NULL,
    claimed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TRIGGER handoffs_no_update BEFORE UPDATE ON handoffs BEGIN
    SELECT RAISE(ABORT, 'handoff packages are immutable');
END;
CREATE TRIGGER handoffs_no_delete BEFORE DELETE ON handoffs BEGIN
    SELECT RAISE(ABORT, 'handoff packages are immutable');
END;
CREATE TRIGGER claims_no_update BEFORE UPDATE ON handoff_claims BEGIN
    SELECT RAISE(ABORT, 'handoff claims are immutable');
END;
CREATE TRIGGER claims_no_delete BEFORE DELETE ON handoff_claims BEGIN
    SELECT RAISE(ABORT, 'handoff claims are immutable');
END;
PRAGMA user_version = 2;
