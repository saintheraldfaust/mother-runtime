CREATE TABLE worker_executions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    worker_label TEXT NOT NULL,
    adapter TEXT NOT NULL CHECK(adapter = 'mock'),
    status TEXT NOT NULL CHECK(status IN ('leased','running','released','failed','expired','completed')),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    attempt INTEGER NOT NULL CHECK(attempt >= 1),
    lease_seconds INTEGER NOT NULL CHECK(lease_seconds BETWEEN 1 AND 3600),
    lease_expires_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    result TEXT CHECK(result IS NULL OR json_valid(result)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX worker_executions_active_task ON worker_executions(task_id) WHERE status IN ('leased','running');
CREATE INDEX worker_executions_by_task ON worker_executions(task_id,sequence);
CREATE INDEX worker_executions_by_status ON worker_executions(status,lease_expires_at,sequence);
CREATE TABLE worker_execution_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    execution_id TEXT NOT NULL REFERENCES worker_executions(id),
    action TEXT NOT NULL,
    revision INTEGER NOT NULL,
    details TEXT NOT NULL CHECK(json_valid(details)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX worker_execution_events_by_execution ON worker_execution_events(execution_id,sequence);
CREATE TRIGGER worker_execution_events_no_update BEFORE UPDATE ON worker_execution_events BEGIN
    SELECT RAISE(ABORT,'worker execution events are immutable');
END;
CREATE TRIGGER worker_execution_events_no_delete BEFORE DELETE ON worker_execution_events BEGIN
    SELECT RAISE(ABORT,'worker execution events are immutable');
END;
PRAGMA user_version=6;
