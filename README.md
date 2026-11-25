# Mother Runtime

Mother Runtime is a local Rust service for coordinating AI coding agents and storing
project state across sessions. It maintains project context, engineering decisions,
persona definitions, tasks, dependencies, checkpoints and worker execution records
in SQLite. Coding assistants access that state through Model Context Protocol (MCP).
A CLI and terminal dashboard connect to the same service.

## What it does

- Records project context and decisions, including the rationale and decisions they replace.
- Tracks tasks through explicit states, with dependencies, revision checks and checkpoints.
- Creates hashed handoff packages containing project state and outstanding work for another session.
- Stores versioned personas with responsibilities and declared capabilities.
- Runs Codex workers in separate Git worktrees, with execution leases, heartbeats,
  timeout limits and file observations.
- Displays tasks, workers, personas, handoffs, GitHub identity metadata and activity
  in a read-only Ratatui dashboard.

Codex workers use the existing host login. Mock and OpenCode adapters are also
available. GitHub integration currently supports read-only identity verification
and persona bindings.

## How it works

One service owns the database and serializes state changes. CLI and TUI clients
connect through a private Unix socket. Each MCP client starts a lightweight stdio
bridge that forwards requests to that same service.

Mutations use revision checks and idempotency keys. State changes, audit events and
retry responses commit in a single transaction. Project records persist independently
of coding-assistant sessions; continuity uses explicitly recorded context and
checkpoints rather than importing conversation history.

Worker execution is an explicit CLI operation. The supervisor leases a ready task,
creates a detached worktree from the repository's current commit, starts the selected
coding agent and records its outcome. For Codex, it requests the `workspace-write`
sandbox and retains usage metadata and file observations while discarding raw model
output. A successful run leaves the task `awaiting_input` and preserves the worktree
for inspection. It does not automatically complete the task.

## Build and run

Requires Rust 1.97 or newer on macOS or Linux. Worker execution requires Git and the
selected coding-agent CLI. To use Codex, sign in through Codex first.

From the repository root:

```sh
cargo build --locked
./target/debug/motherctl setup-codex
./target/debug/motherctl ui
```

`setup-codex` starts Mother, registers the current project and adds its MCP server
to the project's `.codex/config.toml`. Reload the Codex VS Code session in the trusted
project to load the tools. The default runtime directory is `~/.mother`.

In the dashboard, use Tab to switch views, Up and Down to select records, and q to
quit. Task creation and state changes use the CLI or MCP; worker startup uses the CLI.
See [Codex setup](docs/CODEX.md), [CLI commands](docs/CLI.md) and
[TUI controls](docs/TUI.md).

## Current scope

Mother is under active development. A real Codex worker has produced a file matching
its declared SHA-256 contract, and a real Codex CLI client has queried Mother through
MCP. Interactive VS Code integration and cross-assistant continuation have not yet
been verified end to end.

File observations verify content against task contracts. Automated test execution,
independent review, human approval records and GitHub write workflows are not yet
implemented. Persona capability checks apply when clients supply a persona claim;
authenticated persona enforcement is pending. Local clients share the authority of
the operating-system user.

See [architecture](docs/ARCHITECTURE.md), [security](docs/SECURITY.md),
[roadmap](docs/ROADMAP.md) and [development status](development-status.toml) for
implementation details and remaining work.

## License

[Apache License 2.0](LICENSE).
