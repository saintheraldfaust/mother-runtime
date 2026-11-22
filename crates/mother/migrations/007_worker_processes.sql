CREATE TABLE worker_workspaces (
    id TEXT NOT NULL PRIMARY KEY,
    execution_id TEXT NOT NULL UNIQUE REFERENCES worker_executions(id),
    path TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('ready','removed','failed')),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX worker_workspaces_by_status ON worker_workspaces(status);
CREATE TABLE worker_processes (
    id TEXT NOT NULL PRIMARY KEY,
    execution_id TEXT NOT NULL UNIQUE REFERENCES worker_executions(id),
    pid INTEGER NOT NULL CHECK(pid > 1),
    status TEXT NOT NULL CHECK(status IN ('running','exited','failed','killed')),
    exit_code INTEGER,
    log_path TEXT NOT NULL,
    evidence TEXT CHECK(evidence IS NULL OR json_valid(evidence)),
    started_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    ended_at TEXT,
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX worker_processes_by_status ON worker_processes(status);
PRAGMA user_version=7;
