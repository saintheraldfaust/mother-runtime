# CLI reference

Global flags: `--data-dir PATH`, `--json`, `--request-id KEY`.
Default output is indented JSON; `--json` is compact. The request ID flag is
intended for mutations. Only exact successful mutation replays are cached.

| Command | Behavior |
| --- | --- |
| `init` | Create configuration, initialize/migrate the database offline. Repeatable. |
| `mcp` | Serve MCP stdio; forward operations to the existing runtime, without opening SQLite. |
| `start` | Initialize private state if needed and start the canonical service in the background; reuse a reachable service. |
| `setup-codex` | Start Mother, register the current project, and preserve/extend local Codex MCP configuration. |
| `ui [--once]` | Open the read-only live dashboard, or print one overview frame; start Mother if needed. |
| `serve` | Hold the writer lock and run the foreground local service. |
| `doctor` | Report private-directory, configuration, schema/integrity, and runtime health. |
| `status` | Retrieve health from the running service. |
| `projects attach --name NAME --repository PATH` | Register a canonical directory, revision 1. |
| `projects inspect --project ID` | Retrieve canonical project metadata and current revision. |
| `projects list [--after N] [--limit N]` | Page through registered projects. |
| `context record --project ID --kind KIND --content TEXT --source SOURCE [--expected-revision N]` | Atomically append context, revision, event, retry response. |
| `context get --project ID [--after N] [--limit N]` | Retrieve a page of context and current revision. |
| `decisions record --project ID --title TITLE --decision TEXT --rationale TEXT --source SOURCE [--alternative TEXT] [--supersedes ID] [--expected-revision N]` | Record a decision and optionally replace one active decision in the same project. |
| `decisions list --project ID [--include-superseded] [--after N] [--limit N]` | List active decisions by default. |
| `decisions inspect --decision ID` | Read a decision, supersession links and current project revision. |
| `handoff create --file FILE` | Capture a submitted JSON summary and immutable bounded runtime snapshot. |
| `handoff list --project ID [--after N] [--limit N]` | Discover handoff metadata. |
| `handoff show --handoff ID` | Read exact package bytes, SHA-256, live revision and claim status. |
| `handoff resume --handoff ID --recipient LABEL --expected-revision N` | Claim an unchanged, unclaimed package once; does not start execution. |
| `tasks create --file FILE` | Create a host-tracked task from a bounded JSON file contract. |
| `tasks inspect --task ID` | Retrieve task revisions, dependencies, latest checkpoint/transition, and evidence. |
| `tasks list --project ID [--state STATE] [--after N] [--limit N]` | Page through task summaries. |
| `tasks transition --task ID --state STATE --source SOURCE --reason TEXT --expected-task-revision N [--owner-label LABEL] [--persona ID --expected-persona-revision N] [--expected-project-revision N]` | Apply an allowed state transition with evidence/dependency gates and optional current-persona capability enforcement. |
| `tasks checkpoint --file FILE` | Persist progress and runtime observations of the task contract files. |
| `tasks checkpoints --task ID [--after N] [--limit N]` | Retrieve immutable checkpoint history. |
| `tasks history --task ID [--after N] [--limit N]` | Retrieve transition reasons, sources and revisions. |
| `workers eligible [--project ID] [--limit N]` | List bounded ready tasks with completed dependencies and no unexpired lease. |
| `workers list [--task ID] [--project ID] [--status STATUS] [--after N] [--limit N]` | Page through worker execution leases. |
| `workers inspect --execution ID` | Read one lease, task state, expiry and coordination-only limits. |
| `workers history --execution ID [--after N] [--limit N]` | Retrieve immutable lease, heartbeat, mock-checkpoint, release and expiry events. |
| `workers lease --task ID --worker-label LABEL [--adapter mock\|codex\|opencode] [--lease-seconds N] [--max-active N] [--expected-task-revision N] [--expected-project-revision N]` | Acquire a durable coordination lease with a durable adapter selection; no process is launched. |
| `workers heartbeat --execution ID --expected-revision N [--lease-seconds N]` | Extend an active lease from the current time. |
| `workers mock --execution ID --expected-revision N --summary TEXT [--work-remaining TEXT] [--blocker TEXT]` | Write a deterministic mock checkpoint without editing files or completing the task. |
| `workers release --execution ID --expected-revision N --outcome released\|failed\|completed [--summary TEXT]` | Record an execution outcome without changing task state; a running supervised process blocks release. |
| `workers run --task ID --worker-label LABEL [--adapter mock\|codex\|opencode] [--model PROVIDER/MODEL] [--agent NAME] [--max-cost-usd N] [--max-steps N] [--lease-seconds N] [--max-active N] [--timeout-seconds N] [--helper-delay-ms N] [--helper-exit-code N]` | Lease a ready task, create an isolated worktree, supervise the selected mock, Codex or OpenCode CLI process, and finalize evidence. Codex uses host login and optional `--model`, rejects dollar budgets and agent selection; OpenCode requires `--model` and positive `--max-cost-usd`. |
| `workers abandon --execution ID [--force]` | Reconcile a crashed supervisor: mark a running process `lost`, block its task, and leave the worktree for explicit cleanup. Refuses while the recorded PID exists unless forced. |
| `workers cleanup --execution ID` | Remove a retained non-active worktree, prune Git metadata, and mark the workspace removed. |
| `workers recover` | Expire overdue leases; a task with a running supervised process becomes blocked instead of requeued. |
| `history [--project ID] [--after N] [--limit N]` | Retrieve committed events, optionally project-scoped. |
| `personas create --file FILE` | Create a revisioned persona definition from bounded JSON. |
| `personas list [--status STATUS] [--project ID] [--after N] [--limit N]` | Page through personas, optionally filtered by status or project scope. |
| `personas inspect --persona ID` | Read the current persona revision and declared capabilities. |
| `personas edit --persona ID --expected-revision N --file FILE` | Replace a persona definition, rejecting a stale revision. |
| `personas disable/enable/archive --persona ID --expected-revision N` | Change status with an immutable audit revision; archived is the safe deletion path. |
| `personas history --persona ID [--after N] [--limit N]` | Retrieve immutable persona revision snapshots. |
| `github profile [--host HOST]` | Create an isolated, private GitHub CLI profile directory; no login is performed. |
| `github connect --profile ID [--host HOST] --persona ID --expected-persona-revision N` | Run one read-only `gh api user` check in that profile and record the verified identity. |
| `github list [--status STATUS] [--persona ID] [--after N] [--limit N]` | Page through verified identity metadata; no credentials are returned. |
| `github inspect --identity ID` | Read one identity, its persona binding, verification metadata and limits. |
| `github revoke --identity ID --expected-revision N` | Revoke future authorized use while preserving immutable history. |
| `github history --identity ID [--after N] [--limit N]` | Retrieve immutable identity authorization revisions. |

Kinds: `fact`, `observation`, `failure`, `blocker`, `objective`. All are unverified
submissions. The kind does not imply authorization or evidence.

IPC-backed commands return `{version, request_id, result}` or
`{version, request_id, error: {code, message}}`. Expected domain errors have codes
`invalid_request`, `not_found`, `conflict`, or `integrity_error`; unexpected storage failures return
`internal_error` without request contents. Pre-IPC failures return `command_failed`.
`init` and `doctor` return command-specific diagnostic objects. `workers run`,
`workers abandon` and `workers cleanup` are owner-run composites and emit only
`{result}`. Clap syntax errors use its standard stderr output and exit status 2. All
operational failures exit 1.

`doctor` exits 1 if the service is stopped, even if offline checks pass. It does
not fix permissions or create a missing database. `init` refuses to run against
an active runtime. Use SIGINT/Ctrl-C or SIGTERM for graceful shutdown. There is
no login-time installation or `launchd` integration. `start`, `ui`, and `setup-codex` explicitly start the background service; quitting the TUI leaves it running.


`mcp` reserves stdout exclusively for MCP JSON-RPC messages, including when
initialization fails. Its diagnostics go to stderr. Global `--json` and
`--request-id` do not change MCP transport behavior; mutations supply their retry
key in tool arguments. See [MCP usage and schemas](MCP.md).

Use explicit `--request-id` keys for decision/handoff mutations when reliable retry
is needed. `handoff create` accepts [this JSON shape](../examples/handoff.json);
replace its project ID before use. Use the handoff creation revision as the resume
expected revision while the project is unchanged. See [continuity semantics](CONTINUITY.md).

Task input and checkpoint examples are in [task.json](../examples/task.json) and
[task-checkpoint.json](../examples/task-checkpoint.json). Task state values use
hyphens in CLI flags and underscores in JSON. All task mutations use the same
retry-key namespace as other writes. Lists may return fewer than the requested
items due to the serialized-byte budget; follow `next_cursor` until null. See
[task semantics](ORCHESTRATION.md), especially the limited file-completion scope.

Persona examples are in [persona-implementation.json](../examples/persona-implementation.json)
and [persona-review.json](../examples/persona-review.json). Task JSON inputs may
include `persona: {"persona_id":"...","expected_revision":N}`; the CLI transition
form uses `--persona` and `--expected-persona-revision`. The runtime checks the
latest persona revision, active status, project scope and capability. Omitting the
assertion uses the existing local-operator path. See [personas](PERSONAS.md).

GitHub account setup is owner-run and read-only. `github profile` creates a
private profile directory; the account owner completes `gh auth login` with
`GH_CONFIG_DIR` set to it; `github connect` verifies the account and records the
identity. No token is stored or returned. See [GitHub identities](IDENTITIES.md).

Worker leases default to 120 seconds and four active leases. Leases coordinate
owner-run runners inside Mother's database; they are not OS locks, identity proof or
write authorization. `workers run` is a separate owner-run supervisor for the mock
helper or an OpenCode CLI process in an isolated Git worktree. It is not a sandbox.

For `--adapter opencode`, `--model` and a positive `--max-cost-usd` are required;
`--max-steps` defaults to 20 and `--timeout-seconds` to 300. `--helper-delay-ms` and
`--helper-exit-code` are mock-only. The child uses the
owner's existing OpenCode login, an isolated OpenCode configuration inside the
execution directory, `--pure --auto`, and an environment allowlist. Mother does not
store or return provider credentials. Cost accounting is provider-reported and is
not an independent billing guarantee. A successful run still requires host review;
budget exhaustion, timeout or failure marks the task failed. See
[provider boundary](ORCHESTRATION.md#opencode-cli-adapter) and
[ADR 0009](DECISIONS/0009-opencode-provider-adapter.md).

`workers mock` records a checkpoint without launching a process. If a supervisor is
killed, `workers abandon` marks the process `lost` without signaling an unverified
PID; use `--force` only after confirming the recorded PID is gone or belongs to the
abandoned helper. Retained worktrees require explicit `workers cleanup`. See
[worker operations](ORCHESTRATION.md#worker-execution-leases).

See [Codex setup and execution](CODEX.md) and [dashboard controls](TUI.md).
