# Terminal dashboard

From the repository root:

```sh
cargo build --locked
./target/debug/motherctl ui
```

The command initializes private state and starts the canonical service in the
background when needed. The default data directory is `~/.mother`; a global
`--data-dir PATH` selects another private directory. Quitting the dashboard leaves
the service running. `motherctl serve` remains available for foreground operation;
there is no login-time service installation.

The Ratatui dashboard polls the service through IPC in a background task. It never
opens or writes SQLite. If reads fail, the last successful snapshot remains visible
with an OFFLINE / STALE indicator. Each successful refresh comprises individual
live reads, not an atomic multi-view snapshot. Lists follow the runtime's pagination
cursor up to 1,000 records per collection; the displayed count is bounded by that cap.

Views show the selected project's overview, tasks, workers, personas, handoff
metadata, GitHub account metadata and activity. Accounts are global metadata.
Worker details show retained worktree paths, process outcome, reported usage, changed
paths and runtime file observations. Empty views show no records rather than mock
participants or invented activity. No authentication values or raw model transcripts
are displayed. Control characters in submitted text are removed before rendering.

| Key | Action |
| --- | --- |
| Tab / Right, Shift-Tab / Left | Next / previous view |
| 1–7 | Select a view |
| Up / Down | Select a record |
| j / k | Scroll details |
| p | Cycle registered projects |
| q / Esc / Ctrl-C | Quit and restore the terminal |

This first dashboard is read-only. Create tasks and change state through Codex MCP
or the [CLI](CLI.md). The worker supervisor remains an explicit owner-run CLI action.
Success means an observed process exit plus required provider events; it does not
mean the task passed tests or was approved.

`motherctl ui --once` renders one live overview frame as plain text without a TTY.
This supports diagnostics and process integration tests. Unit tests render worker
state and review boundaries using Ratatui's TestBackend, including narrow terminals.
