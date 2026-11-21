ALTER TABLE task_transitions ADD COLUMN persona_id TEXT;
ALTER TABLE task_transitions ADD COLUMN persona_revision INTEGER;
ALTER TABLE task_checkpoints ADD COLUMN persona_id TEXT;
ALTER TABLE task_checkpoints ADD COLUMN persona_revision INTEGER;
CREATE TABLE personas (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL,
    description TEXT NOT NULL,
    responsibilities TEXT NOT NULL CHECK(json_valid(responsibilities)),
    prohibitions TEXT NOT NULL CHECK(json_valid(prohibitions)),
    capabilities TEXT NOT NULL CHECK(json_valid(capabilities)),
    project_id TEXT REFERENCES projects(id),
    preferred_runtime TEXT,
    github_identity TEXT,
    context_retrieval TEXT,
    review_required INTEGER NOT NULL CHECK(review_required IN (0,1)),
    max_concurrency INTEGER NOT NULL CHECK(max_concurrency BETWEEN 1 AND 32),
    budget_units INTEGER CHECK(budget_units IS NULL OR budget_units >= 0),
    status TEXT NOT NULL CHECK(status IN ('active','disabled','archived')),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX personas_by_scope ON personas(project_id,status,sequence);
CREATE TABLE persona_revisions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    persona_id TEXT NOT NULL REFERENCES personas(id),
    revision INTEGER NOT NULL,
    config TEXT NOT NULL CHECK(json_valid(config)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(persona_id,revision)
);
CREATE INDEX persona_revisions_by_persona ON persona_revisions(persona_id,sequence);
CREATE TABLE persona_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    persona_id TEXT NOT NULL REFERENCES personas(id),
    action TEXT NOT NULL,
    revision INTEGER NOT NULL,
    details TEXT NOT NULL CHECK(json_valid(details)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TRIGGER persona_revisions_no_update BEFORE UPDATE ON persona_revisions BEGIN
    SELECT RAISE(ABORT,'persona revisions are immutable');
END;
CREATE TRIGGER persona_revisions_no_delete BEFORE DELETE ON persona_revisions BEGIN
    SELECT RAISE(ABORT,'persona revisions are immutable');
END;
CREATE TRIGGER persona_events_no_update BEFORE UPDATE ON persona_events BEGIN
    SELECT RAISE(ABORT,'persona events are immutable');
END;
CREATE TRIGGER persona_events_no_delete BEFORE DELETE ON persona_events BEGIN
    SELECT RAISE(ABORT,'persona events are immutable');
END;
PRAGMA user_version=4;
