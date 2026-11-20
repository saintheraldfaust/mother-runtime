# Mother-runtime by TFRI

A Rust runtime for persistent engineering context across coding assistants.

Mother keeps canonical project state in a local SQLite database owned by one
foreground service. Independent CLI processes share that service over a Unix
socket. This initial implementation provides project registration, context
records, structured decisions with supersession, hashed handoff packages, one-time
continuation claims, durable host-tracked tasks/checkpoints, runtime file evidence,
revision checks, idempotent writes, diagnostics, and an
official Rust SDK MCP stdio bridge.

Personas, independent workers, GitHub operations, and the TUI are subsequent
milestones. Two MCP protocol lifecycles are tested with SDK clients; interactive
coding-assistant sessions are not yet certified. See
[development status](development-status.toml) and [roadmap](docs/ROADMAP.md).

## Build and run

Requires Rust 1.97 or newer and a C compiler for bundled SQLite. macOS is tested
locally; Linux is included in CI but has not yet been executed in this workspace.

```sh
cargo build --workspace --locked
cargo install --path crates/mother --locked
motherctl init
motherctl serve
```

In a second terminal:

```sh
motherctl doctor
motherctl projects attach --name example --repository /absolute/path/to/project
motherctl projects list
```

Use the returned project ID to record and retrieve context:

```sh
motherctl --request-id initial-objective context record \
  --project PROJECT_ID --kind objective \
  --content 'Implement the authentication API' --source human \
  --expected-revision 1
motherctl context get --project PROJECT_ID
motherctl history --project PROJECT_ID
```

Record decisions with `motherctl decisions record`. Create a continuation package
with `motherctl handoff create --file handoff.json`, using the
[example submission](examples/handoff.json). A later session can list, show, and
claim it once. See [continuity semantics](docs/CONTINUITY.md) for revisions, snapshot
limits, and the distinction between recorded state and unverified progress claims.

Track local work with `motherctl tasks create --file task.json`, using the
[example contract](examples/task.json). Tasks have explicit states, checkpoints,
dependency checks, and bounded runtime file observations. Local completion proves
only the declared file contract, not test success or software correctness. See
[task and evidence semantics](docs/ORCHESTRATION.md).

Stop the service with Ctrl-C or SIGTERM. Restart `motherctl serve` and query the
same project to recover its records. A directory can be attached before Git is
initialized; registration does not verify a remote repository or Git identity.

Schema 3 migrates existing schema-1/2 state transactionally on `init` or `serve`.
Stop the old service and all bridges before upgrading binaries, then restart them.

State defaults to `~/.mother`. Override it on **every command** with
`--data-dir /short/absolute/path`. The data directory must be owned by the current
user and private (0700). Runtime/configuration/database files are private (0600).
The socket path must fit within 100 bytes. Run on a local filesystem.

Default command output is indented JSON; `--json` emits compact JSON. Errors exit
with a nonzero status. `doctor` reports a stopped runtime as an unhealthy check,
while still reporting configuration and database checks separately.

## Verification

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

For assistant connections, launch `motherctl mcp` as a stdio server against the
same state directory. The canonical `serve` process must already be running. See
[MCP tools and opt-in host configuration examples](docs/MCP.md).

See [CLI usage](docs/CLI.md), [architecture](docs/ARCHITECTURE.md),
[security boundaries](docs/SECURITY.md), and [test evidence](docs/TESTING.md).

Licensed under the [Apache License, Version 2.0](LICENSE) (`Apache-2.0`).
Third-party dependencies retain their own licenses and notices; see
[licensing](docs/LICENSING.md) and [publication policy](docs/PUBLISHING.md).
