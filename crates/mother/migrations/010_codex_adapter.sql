CREATE TEMP TABLE worker_executions_v10 AS SELECT * FROM worker_executions;
CREATE TEMP TABLE worker_execution_events_v10 AS SELECT * FROM worker_execution_events;
CREATE TEMP TABLE worker_workspaces_v10 AS SELECT * FROM worker_workspaces;
CREATE TEMP TABLE worker_processes_v10 AS SELECT * FROM worker_processes;

DROP TABLE worker_execution_events;
DROP TABLE worker_workspaces;
DROP TABLE worker_processes;
DROP TABLE worker_executions;

CREATE TABLE worker_executions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    worker_label TEXT NOT NULL,
    adapter TEXT NOT NULL CHECK(adapter IN ('mock','opencode','codex')),
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
INSERT INTO worker_executions(sequence,id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at,result,created_at,updated_at)
SELECT sequence,id,task_id,project_id,worker_label,adapter,status,revision,attempt,lease_seconds,lease_expires_at,heartbeat_at,result,created_at,updated_at
FROM worker_executions_v10;
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
INSERT INTO worker_execution_events(sequence,id,execution_id,action,revision,details,created_at)
SELECT sequence,id,execution_id,action,revision,details,created_at FROM worker_execution_events_v10;
CREATE INDEX worker_execution_events_by_execution ON worker_execution_events(execution_id,sequence);
CREATE TRIGGER worker_execution_events_no_update BEFORE UPDATE ON worker_execution_events BEGIN
    SELECT RAISE(ABORT,'worker execution events are immutable');
END;
CREATE TRIGGER worker_execution_events_no_delete BEFORE DELETE ON worker_execution_events BEGIN
    SELECT RAISE(ABORT,'worker execution events are immutable');
END;

CREATE TABLE worker_workspaces (
    id TEXT NOT NULL PRIMARY KEY,
    execution_id TEXT NOT NULL UNIQUE REFERENCES worker_executions(id),
    path TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('ready','removed','failed')),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT INTO worker_workspaces(id,execution_id,path,base_commit,status,created_at,updated_at)
SELECT id,execution_id,path,base_commit,status,created_at,updated_at FROM worker_workspaces_v10;
CREATE INDEX worker_workspaces_by_status ON worker_workspaces(status);

CREATE TABLE worker_processes (
    id TEXT NOT NULL PRIMARY KEY,
    execution_id TEXT NOT NULL UNIQUE REFERENCES worker_executions(id),
    pid INTEGER NOT NULL CHECK(pid > 1),
    status TEXT NOT NULL CHECK(status IN ('running','exited','failed','killed','lost')),
    exit_code INTEGER,
    log_path TEXT NOT NULL,
    evidence TEXT CHECK(evidence IS NULL OR json_valid(evidence)),
    started_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    ended_at TEXT,
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT INTO worker_processes(id,execution_id,pid,status,exit_code,log_path,evidence,started_at,ended_at,updated_at)
SELECT id,execution_id,pid,status,exit_code,log_path,evidence,started_at,ended_at,updated_at FROM worker_processes_v10;
CREATE INDEX worker_processes_by_status ON worker_processes(status);

DROP TABLE worker_processes_v10;
DROP TABLE worker_workspaces_v10;
DROP TABLE worker_execution_events_v10;
DROP TABLE worker_executions_v10;
PRAGMA user_version=10;
