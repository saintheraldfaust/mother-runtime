CREATE TABLE worker_processes_v8 (
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
INSERT INTO worker_processes_v8(id,execution_id,pid,status,exit_code,log_path,evidence,started_at,ended_at,updated_at)
SELECT id,execution_id,pid,status,exit_code,log_path,evidence,started_at,ended_at,updated_at
FROM worker_processes;
DROP TABLE worker_processes;
ALTER TABLE worker_processes_v8 RENAME TO worker_processes;
CREATE INDEX worker_processes_by_status ON worker_processes(status);
PRAGMA user_version=8;
