# Testing and current evidence

Run from the workspace root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The suite contains 34 unit tests and twelve process integration tests.
Tests use temporary local directories and never touch real account credentials,
remotes, or the default `~/.mother` installation.

Unit coverage:

- Configuration field/version validation and client-limit bounds.
- Symlink rejection for configuration and database paths.
- Rejection of future schemas without downgrading their version.
- Invalid cursors, page sizes, request IDs, protocol versions, missing projects.
- Injected audit-insert failure rolls back the context record, project revision,
  and retry key; the same request can then succeed after removing the fault.
  Event UPDATE/DELETE triggers also reject rewriting existing audit records.
- Persona create/update/status/history revisions, duplicate names, archived
  reactivation rejection, and stale expected revisions.
- Persona-attributed task operations enforce live capabilities and record persona
  attribution; stale and revoked policies are rejected. Schema 3→4 upgrade
  preserves a task and adds the persona tables.
- GitHub verification command construction uses the isolated profile and explicitly
  removes token environment variables. Public account-response parsing rejects
  malformed, organization and invalid-ID responses.
- GitHub identity records require an active `github.read` persona, reject duplicate
  active bindings, and preserve immutable authorization/revocation revisions.
  Schema 4→5 upgrade preserves personas and adds identity tables. Schema 5→6
  preserves identities and adds worker lease tables. Schema 6→7 preserves leases
  and adds workspace/process tables.
- Worker eligibility excludes tasks with incomplete dependencies or an unexpired
  lease. One active lease per task and the global `max_active` bound are enforced.
- Heartbeats extend leases and reject stale revisions. Expired leases are recovered
  explicitly and during reacquisition, with attempt increments and history retained.
- The mock adapter writes an ordinary checkpoint, leaves repository files unchanged,
  cannot complete a task, and is rejected after expiry or a stale revision.
- A process execution requires a ready workspace and owner-matched revision; its
  finish records bounded acceptance-criterion evidence under the worktree and never
  writes project completion evidence.
- Lease recovery blocks a running supervised task instead of requeueing it, while
  allowing the process finish to preserve expired status and evidence.

Process integration coverage:

- Real foreground service, SQLite file, Unix socket, and separate `motherctl`
  processes. Eight parallel CLI writers preserve all records and revisions.
- Exact retries return the same result; changed payload/key collisions and stale
  revisions fail without adding events. Retry responses survive a forced kill.
- Kill the actual runtime process, restart it against the same state, and recover
  identical context and revision. Verify cursor pagination and event counts.
- Reject a second service and offline init while the writer lock is held.
- Reject malformed, oversized, and unsupported-version requests; subsequent
  health requests still work. Verify socket permissions and graceful SIGTERM cleanup.
- A supervised deterministic mock process leases a task, creates a detached Git
  worktree, writes only its fixed artifact there, records log/process evidence, and
  transitions the task to `awaiting_input` without touching the project checkout or
  completing the task.
- Refuse to replace a regular file at the socket path and refuse insecure
  directory permissions. `doctor` reports missing state without initializing it.

MCP process coverage (official rmcp 3.11.0 clients over child-process stdio):

- Two independent bridges share one real runtime; the 2025-11-25 initialize and
  2026-07-28 discover lifecycles both execute tools successfully.
- Tool/schema discovery, project lookup, live CLI updates, context and activity
  pagination, structured errors, exact mutation retries, and stale-write rejection.
- Resource/template discovery and revision-bearing context resources; unknown
  resources/tools and missing required mutation arguments fail.
- Existing MCP connections resume after runtime process kill/restart. A fresh
  connection retrieves the same project revision. Retry responses remain durable.
- A bridge against missing state reports failure without creating a directory or
  database. EOF during initialization exits unsuccessfully with empty stdout.

Continuity coverage:

- Schema-1 migration preserves context, project revisions, and original retry
  responses; an injected schema collision rolls back all pending migration DDL.
- Supersession is scoped to one project, rejects a second replacement of an
  already replaced record, and removes historical decisions from live context.
  Injected event failure rolls back both records and project revision.
- Snapshot bounds are exercised with 25 maximum-size escaped-text context records.
  Exact package JSON parses back to its public value, and an independent SHA-256
  computation matches the returned digest. Tampered body hashes are rejected.
- Package/claim SQL immutability, handoff-creation rollback, and claim rollback.
- A CLI-created handoff recovers through another real MCP client after restart,
  including the replacement decision, failed test, blocker, changed-file claim,
  hypotheses, source, next action, and explicitly false verification flags.
- Wrong recipient labels, stale expected revisions, and changed project state
  reject resume without claiming. Competing MCP clients commit one claim, with
  exact retry surviving a second runtime restart. Original package bytes remain
  unchanged after claims and later project changes.

Task and evidence coverage:

- A separate MCP client recovers a running host-tracked task, failed checkpoint,
  reported test failure, blocker, and task-focused handoff after process restart.
- Competing transitions against the same task revision admit exactly one update;
  successful completion replays identically across CLI/MCP after another restart.
- A prerequisite blocks readiness until it completes. Cross-project dependencies
  and a handoff focused on another project's task are rejected.
- Missing/stale checkpoints, missing/mismatched/changed artifacts, illegal state
  jumps, terminal checkpoints, and review-required completion are rejected.
- Anchored file checks reject traversal, absolute paths, intermediate and leaf
  symlinks, root symlinks, hard links, and FIFOs. A shell-shaped filename is read
  literally. A known SHA-256 vector is checked; file/aggregate read caps are tested.
- Injected event failures roll back task creation, transitions, checkpoints,
  project/task revisions, and retry-key effects. Checkpoints/history are immutable.
- Schema-2 packages and legacy retry payloads survive upgrade; a schema-3 DDL
  collision rolls back newly created tables and preserves the old schema version.
- Byte-limited context pagination returns a cursor and recovers all 60 large records
  across pages without gaps. The suite still exercises prior migrations and handoffs.

GitHub identity process coverage:

- A CLI-created `github.read` persona and isolated profile are verified through a
  fake `gh` executable that fails if token environment variables leak. Two MCP
  clients read the active identity, and revocation is visible immediately while
  immutable authorization history remains readable.

Persona process coverage:

- A CLI-created persona is read by two independent MCP clients. Editing it through
  the CLI increments the revision and a fresh MCP read sees the new capability set.
- A task created and transitioned under persona revision 1 is rejected after the
  capability is revoked, both with the stale revision and with the current revision.
  Immutable persona history retains both revisions.

Locally exercised on macOS with Rust 1.97.1. The checked-in CI workflow requests
macOS and Linux validation; no hosted CI run is claimed. No interactive
coding-assistant host, coding provider, mandatory-persona authorization, real GitHub
account login, GitHub write, credential broker, remote workflow, provider worker,
sandbox isolation, worker-executed tests, performance target, power-loss, or hostile
same-UID isolation test has been performed. Supervisor-crash orphan reaping is not
implemented. The supervised process test uses a deterministic local helper, not a
model provider. GitHub identity coverage uses a
fake `gh`; it is not certification with a real account.
