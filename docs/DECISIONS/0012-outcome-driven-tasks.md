# ADR 0012: Outcome-driven tasks and optional file constraints

Status: implemented, 2026-11-25.

A user creating an agent task knows the goal and desired behavior. Implementation
files are chosen during repository inspection and planning. Requiring their names
before dispatch incorrectly makes implementation decisions part of task intake.

Task creation requires a project, title, objective and submission source. Plain-text
acceptance criteria are optional. An objective-only task is valid. Known paths and
hashes are optional advanced artifact constraints; they retain the bounded existing
file verifier. The default TUI form exposes title, objective and behavioral criteria.
File constraints and dependency IDs are behind Ctrl-O.

The worker brief includes the objective, behavioral criteria and any explicit file
constraints. It instructs the agent to inspect the repository, plan briefly and
choose implementation files. Mother records bounded Git changes and independently
observes discovered file paths in the retained worktree, using the anchored file
reader. Discovery neither creates user requirements nor proves behavioral success.

Behavioral criteria are never considered satisfied by file presence or model claims.
An empty file contract cannot pass completion. Existing explicit file-only contracts
retain their narrow completion semantics. Tasks with behavioral requirements remain
blocked from completion until an outcome verifier is available. Worker success still
leaves the task awaiting input, with execution and artifact evidence inspectable.

Schema 11 marks the expanded task-contract JSON format. It changes no existing task,
checkpoint, handoff bytes or cached retry response. New acceptance criteria serialize
as strings; original path objects remain accepted for compatibility. Optional
artifact_constraints are combined into the one persisted contract. Task reads expose
outcome_criteria and artifact_constraints projections. Empty new input fields are
omitted from request serialization to preserve legacy retry keys.

Tests cover objective-only intake, textual criteria, optional and legacy file
contracts, empty-evidence and behavioral completion rejection, schema-10 upgrades,
legacy replay, worker-selected artifact discovery and restart persistence. A local
Codex fixture tests execution without a model call; no general behavioral or browser
verification is claimed.
