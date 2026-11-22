# Architecture

The Cargo workspace currently has one package, `mother-runtime`, with a library
and the `motherctl` executable. Modules exist only for implemented responsibilities:

- `config`: bounded TOML loading, private state directory, runtime lock, initialization.
- `protocol`: versioned, typed IPC requests and structured response envelopes.
- `store`: SQLite migrations and transactional application operations.
- `continuity`: typed decision and handoff submission models.
- `tasks`: typed host-tracked task contracts and explicit state graph.
- `personas`: typed persona definitions, statuses, capabilities and authorization claims.
- `github`: typed read-only GitHub identity records and the fixed `gh api user` verifier.
- `workers`: typed execution-lease, workspace and process inputs, statuses, outcomes and limits.
- `evidence`: bounded descriptor-relative repository file observations.
- `store/tasks`: task/dependency persistence, immutable checkpoints/transitions,
  optimistic revisions, and local file-contract completion gates.
- `store/continuity`: decision supersession, bounded snapshots, immutable packages,
  SHA-256 integrity checks, and one-time continuation claims.
- `store/personas`: revisioned persona persistence, immutable history, status changes,
  project scope and capability enforcement for asserted task operations.
- `store/github`: verified GitHub identity records, persona bindings, immutable
  revision history, revocation and paginated read-only metadata.
- `store/workers`: transactional dependency-aware lease acquisition, bounded
  concurrency, heartbeats, expiry recovery, release, mock checkpoints, workspace
  registration, process attachment/finish and bounded artifact evidence.
- `runtime`: Unix socket server/client, connection limits, lifecycle, diagnostics.
- `mcp`: official rmcp SDK stdio bridge, typed tool schemas, resources, lazy IPC.
- `main`: Clap command grammar and JSON presentation.

`motherctl` clients → private Unix socket → runtime → serialized SQLite connection.

MCP hosts → separate `motherctl mcp` stdio bridges → the same Unix socket. Bridges
never open the database or start a service. Each call obtains live state; bridge
connections survive a runtime restart and reconnect on their next call. See
[MCP integration](MCP.md) for schema and protocol compatibility details.

Only `init` (offline) and `serve` open the database for writes. Both hold the same
exclusive advisory file lock. CLI operations never start another database writer.
`doctor` opens SQLite read-only for integrity checks. It does not initialize or
migrate state; SQLite can create WAL shared-memory sidecars during read-only access.

The service runs Tokio's current-thread executor. Requests perform SQLite work
on blocking threads, under a mutex around one connection. Active client tasks
are bounded by `max_clients` (default 32, maximum 128). SQL uses parameters and
indexed project/sequence queries. No network listener or general shell executor exists.
The owner-run CLI may invoke one fixed read-only `gh api user` verification command;
the runtime itself does not execute GitHub commands. Worker lease operations are
SQLite coordination: the runtime does not launch, supervise or signal processes.
The separate owner-run `workers run` command creates a detached Git worktree and
supervises one fixed deterministic mock helper in its own process group.

Each socket connection carries one newline-terminated request and response.
Requests are limited to 64 KiB and responses to 4 MiB, with five-second read/write
deadlines. Excess clients are disconnected; callers may retry. Retry a mutation
with its original request ID and identical payload because a lost response does
not imply that the transaction failed. All current callers run with the local
operator's authority. A caller may assert a persona on task operations; the runtime
then enforces the current persona revision, status, project scope and declared
capability. Omitting the assertion retains the local-operator path.

SQLite uses foreign keys, WAL mode, FULL synchronous writes, `user_version`
migrations, and normalized current-state tables plus an event ledger. State,
revision increments, events, and idempotency responses commit atomically. Failed
operations roll back. A future schema version is rejected before setting pragmas
or running migrations. Pending migrations execute in one transaction; migration 2
adds continuity tables without rewriting existing context or retry responses. See [ADR 0001](DECISIONS/0001-local-runtime.md).

The runtime lock is acquired before touching the database or removing a stale
socket. It remains held until clients drain on graceful shutdown. Only an existing
socket can be removed at startup; ordinary files and symlinks are refused. A killed
process releases the OS lock, permitting restart against the same database.

TOML is authoritative only for startup transport settings. SQLite is authoritative
for project, context, persona, GitHub identity and worker execution records. Configuration is applied by restarting the
service, with strict field and version validation. Persona updates are runtime
operations with their own revisions; editing TOML does not update them.


Handoffs separate immutable package bytes from live project revision and claim
status. Context reads expose active decisions only; explicit history retains
superseded decisions. [ADR 0002](DECISIONS/0002-continuity-packages.md) defines the
continuity boundary and conservative claim semantics.

Schema 3 adds host-tracked tasks. Schema 4 adds versioned personas, immutable
persona revision snapshots/audit rows and optional persona attribution on task
transitions/checkpoints. Schema 5 adds read-only verified GitHub identity records,
persona bindings, revocation and immutable identity revisions/events. Schema 6 adds
worker execution leases and immutable lease events. Schema 7 adds worker workspaces
and supervised process records; schema 8 adds an explicit `lost` process state for
abandoned supervision. File observation runs on the runtime blocking threads within
a state transaction, without invoking any shell or provider. Filesystem observations
are not transactional with SQLite; they are point-in-time evidence with size/path
restrictions. Completion rechecks the latest validating checkpoint. See
[ADR 0003](DECISIONS/0003-host-tracked-tasks.md),
[ADR 0004](DECISIONS/0004-persona-policy.md),
[ADR 0005](DECISIONS/0005-github-identity-boundary.md),
[ADR 0006](DECISIONS/0006-worker-lease-boundary.md),
[ADR 0007](DECISIONS/0007-worker-process-boundary.md) and
[ADR 0008](DECISIONS/0008-abandoned-process-reconciliation.md).

Lists cap both item count and approximately 1 MiB of serialized item data. Cursors
advance only to the final returned item, so byte-limited pages do not lose records.
Snapshot selection reads the latest records directly before applying its own
smaller byte budget, rather than inheriting a truncated list prefix.
