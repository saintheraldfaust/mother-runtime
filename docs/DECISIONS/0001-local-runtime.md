# ADR 0001: One local writer, transactional SQLite records

Status: adopted for the foundational implementation.

Mother must preserve state independently of a coding assistant. Use a long-lived
foreground Rust service with a private Unix domain socket. Every CLI mutation is
a typed request to that service. A shared file lock also protects offline
initialization. Future MCP and TUI clients must use the same service.

Use one SQLite connection serialized under a mutex, with SQL running on Tokio
blocking threads. This makes transaction/revision ordering deterministic without
introducing a connection pool or a separate database service. WAL, FULL synchronous
writes, and foreign keys provide the local persistence foundation. The bundled
SQLite release is selected by the committed Cargo lockfile.

Keep normalized project/context tables and append-only events. Mutation results
and payloads are stored transactionally for exact idempotent replay. Reject key
reuse with a different payload. Replays return the original response before
checking a now-stale expected revision.

Alternatives considered: per-client writable SQLite connections would undermine
the single runtime boundary; a network database or HTTP listener adds deployment
and authorization work before it is needed; event-only storage complicates simple
context reads. A dedicated database actor may replace the blocking-thread/mutex
strategy if measured contention warrants it.

Consequences: mutations are serialized; retry history currently grows without
retention; all same-UID clients have operator authority; process-crash persistence
is tested, but power-loss resilience is not certified. `doctor` has narrowly scoped
read-only database access, while all normal application reads use the runtime.
