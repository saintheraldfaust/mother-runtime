# Using Mother with Codex

Mother works with the Codex CLI and the Codex VS Code extension. OpenCode is an
optional adapter and is not required. A Codex worker uses the existing host login;
Mother does not copy credentials or require a separate API key.

Build from the repository root, with Rust 1.97 or newer:

```sh
cargo build --locked
./target/debug/motherctl setup-codex
./target/debug/motherctl ui
```

`setup-codex` starts the canonical service if needed, registers the current project
through IPC, and appends a Mother server table to the project's `.codex/config.toml`.
It uses absolute executable/data paths. Keep this generated configuration local;
this repository ignores `.codex/`. Existing settings and comments are preserved.
An incompatible existing Mother table produces an error instead of being replaced.
The directory/file must be owned by the current user with modes 0700/0600.
No global Codex settings, accounts or project-trust settings are changed.

Reload the Codex VS Code session in the trusted project so it discovers the server.
The IDE and CLI share the [Codex MCP configuration format](https://developers.openai.com/codex/mcp/).
Ask Codex to call `mother_health`, list Mother's projects, and retrieve the selected
project's tasks/context. The bridge connects to the existing service; it never
opens SQLite or independently starts a service. Setup does not prove that a running
VS Code session has reloaded its tools.

## Run a Codex worker

Create a task using `motherctl tasks create --file FILE` (see
[the task contract example](../examples/task.json)), then explicitly transition it
through `triaged` to `ready` using the current task revision at each step.

```sh
./target/debug/motherctl workers run \
  --adapter codex --task TASK_ID --worker-label codex \
  --timeout-seconds 180 --max-steps 20
```

The repository must contain a Git commit. The supervisor creates a detached
worktree at the registered repository's current HEAD. Uncommitted source files
and ignored private documents are not copied into that checkout. `--model NAME`
is optional; without it Codex chooses its default and persisted model metadata is
null rather than an invented model name. `MOTHER_CODEX_BIN` can select an installed
executable if `codex` is not on PATH. Run `codex login status` to check the existing
host login; Mother does not log in on the owner's behalf.

The adapter invokes documented [noninteractive Codex execution](https://developers.openai.com/codex/noninteractive/)
with JSON events, ephemeral sessions, ignored user configuration, the OpenAI provider,
`workspace-write` sandbox and `approval_policy="never"`. It supplies no MCP servers
to workers and passes only a small environment allowlist, including host HOME and
optional CODEX_HOME. API-key and GitHub-token environment variables are excluded.
Authentication remains in the owner's Codex storage; this is not a credential broker.
Host configuration, repository instructions and the installed Codex sandbox still
need appropriate trust. Mother does not independently certify hostile-code isolation.

`--timeout-seconds` bounds elapsed process supervision. `--max-steps` bounds observed
completed turns and started command/MCP calls; telemetry can be delayed, so this is
not a preauthorization of each operation. The process group is terminated when a
reported limit is exceeded. Codex subscription usage is recorded as reported token
counts, with dollar cost and dollar budget null. `--max-cost-usd` and `--agent` are
rejected for Codex. A zero exit alone is insufficient: a completed turn is required
and a failed turn/error prevents successful classification.

Mother discards raw Codex stdout and stderr, retaining accounting/completion metadata
and bounded Git/file observations. It does not persist model messages, reasoning,
command bodies or command output as a transcript. Codex may maintain its own host
operational data; ephemeral mode does not establish secure erasure.

Successful execution leaves the task `awaiting_input` and the worktree retained for
review. Inspect it with `workers inspect --execution ID` or the Workers TUI view.
File hashes establish observed content, not tests, correctness or independent review.
No worker auto-completes a task, merges a branch or pushes to GitHub.

## Verification scope

On 2026-11-25, an owner-authorized real Codex run using an existing ChatGPT login
created a declared text artifact whose SHA-256 matched the task contract. A separate
real Codex client successfully called `mother_health` and `mother_projects_list`
over MCP. This establishes a small authenticated CLI execution/MCP path on macOS.
It does not certify an interactive VS Code reload, cross-provider continuation,
large engineering jobs, hostile-code isolation or a general autonomous workflow.
Raw operational records and local configuration remain private.
