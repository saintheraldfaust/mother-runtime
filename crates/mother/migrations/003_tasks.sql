CREATE TABLE tasks (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    project_id TEXT NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL,
    objective TEXT NOT NULL,
    source TEXT NOT NULL,
    acceptance_criteria TEXT NOT NULL CHECK(json_valid(acceptance_criteria)),
    review_required INTEGER NOT NULL CHECK(review_required IN (0,1)),
    state TEXT NOT NULL CHECK(state IN ('created','triaged','ready','assigned','running','awaiting_dependency','awaiting_input','validating','awaiting_review','changes_requested','ready_for_human_approval','completed','failed','cancelled','blocked')),
    owner_label TEXT,
    revision INTEGER NOT NULL CHECK(revision>=1),
    project_revision INTEGER NOT NULL,
    latest_checkpoint_id TEXT,
    completion_evidence TEXT CHECK(completion_evidence IS NULL OR json_valid(completion_evidence)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(id,project_id),
    FOREIGN KEY(latest_checkpoint_id,id) REFERENCES task_checkpoints(id,task_id)
);
CREATE INDEX tasks_by_project ON tasks(project_id,sequence);
CREATE INDEX tasks_by_activity ON tasks(project_id,project_revision);
CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL,
    dependency_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    PRIMARY KEY(task_id,dependency_id),
    FOREIGN KEY(task_id,project_id) REFERENCES tasks(id,project_id),
    FOREIGN KEY(dependency_id,project_id) REFERENCES tasks(id,project_id),
    CHECK(task_id<>dependency_id)
);
CREATE TABLE task_checkpoints (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    task_revision INTEGER NOT NULL,
    project_revision INTEGER NOT NULL,
    submission TEXT NOT NULL CHECK(json_valid(submission)),
    artifacts TEXT NOT NULL CHECK(json_valid(artifacts)),
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(id,task_id),
    UNIQUE(task_id,task_revision)
);
CREATE INDEX checkpoints_by_task ON task_checkpoints(task_id,sequence);
CREATE TABLE task_transitions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    reason TEXT NOT NULL,
    source TEXT NOT NULL,
    task_revision INTEGER NOT NULL,
    project_revision INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TRIGGER transitions_no_update BEFORE UPDATE ON task_transitions BEGIN
    SELECT RAISE(ABORT,'task transitions are immutable');
END;
CREATE TRIGGER transitions_no_delete BEFORE DELETE ON task_transitions BEGIN
    SELECT RAISE(ABORT,'task transitions are immutable');
END;
CREATE TRIGGER checkpoints_no_update BEFORE UPDATE ON task_checkpoints BEGIN
    SELECT RAISE(ABORT,'task checkpoints are immutable');
END;
CREATE TRIGGER checkpoints_no_delete BEFORE DELETE ON task_checkpoints BEGIN
    SELECT RAISE(ABORT,'task checkpoints are immutable');
END;
PRAGMA user_version=3;
