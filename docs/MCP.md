# MCP stdio bridge

Run `motherctl --data-dir /absolute/path/to/state mcp` as the host's stdio server
command. Start `motherctl serve` separately against the same directory. The MCP
process opens no SQLite database, acquires no writer lock, creates no state, and
never auto-starts a runtime. Multiple bridge processes share the canonical service.

The bridge uses the official Rust SDK, `rmcp` 3.11.0. Process integration tests
exercise the 2025-11-25 initialize lifecycle and the 2026-07-28 discover/per-request
lifecycle. The SDK handles version negotiation, response-shape compatibility, and
discovery caching metadata. Other SDK-supported protocol versions have not been
individually tested here. Codex, Claude Code and Copilot configuration examples
are documented below; no interactive host session has yet been certified.

## Tool schemas

Each tool publishes a JSON Schema for its typed inputs and the Mother response
envelope. `structuredContent` and a JSON text content block carry the same response:

```json
{"version":1,"request_id":"correlation-or-retry-key","result":{}}
```

Hash mismatches return the application code `integrity_error`; this is separate
from SQLite structural integrity checks in `doctor`. Application errors replace `result` with `error: {code, message}` and set MCP
`isError: true`. Unknown tool names and malformed argument shapes use JSON-RPC
errors. Domain validation remains enforced by the runtime, even when clients
ignore schema constraints. `result` is operation-specific JSON; its individual
fields are documented here but not yet constrained by separate output schemas.

| Tool | Input | Result |
| --- | --- | --- |
| `mother_health` | `{}` | Runtime status, schema/IPC/SQLite versions, project count, support flags. |
| `mother_projects_list` | Optional `after`, `limit` | `items` containing project identities/revisions, `next_cursor`. |
| `mother_project_get` | `project_id` | `id`, `name`, `repository`, current `revision`, UTC `created_at`. |
| `mother_context_get` | `project_id`; optional `after`, `limit` | `project_id`, current `revision`, `items`, `next_cursor`. |
| `mother_context_record` | `idempotency_key`, `project_id`, `kind`, `content`, `source`; optional `expected_revision` | Record `id`, `project_id`, new `revision`, submitted fields and `trust`. |
| `mother_decision_record` | `idempotency_key`, nested `input` matching DecisionInput | Decision ID, creation/current revisions, status and supersession links. |
| `mother_decisions_list` | `project_id`; optional `include_superseded`, `after`, `limit` | Active decisions by default, project revision, cursor. |
| `mother_decision_get` | `decision_id` | One decision including historical status and current project revision. |
| `mother_handoff_create` | `idempotency_key`, nested `input` matching HandoffInput | Hashed immutable package plus live metadata. |
| `mother_handoffs_list` | `project_id`; optional `after`, `limit` | Handoff metadata, current project revision, cursor. |
| `mother_handoff_get` | `handoff_id` | Package, exact JSON bytes, hash, current revision and claim. |
| `mother_handoff_resume` | `idempotency_key`, nested `input: {handoff_id, recipient, expected_revision}` | One durable claim; no worker is launched. |
| `mother_task_create` | `idempotency_key`, nested `TaskInput` | Host-tracked task contract, IDs and revisions. |
| `mother_tasks_list` | `project_id`; optional `state`, `after`, `limit` | Task summaries and current project revision. |
| `mother_task_get` | `task_id` | Task, latest checkpoint/transition, dependencies, limited evidence and active worker lease. |
| `mother_task_transition` | `idempotency_key`, nested `TaskTransition`; optional `persona` claim | Allowed state update with task/project revision, evidence gates and current-persona capability checks. |
| `mother_task_checkpoint` | `idempotency_key`, nested `CheckpointInput`; optional `persona` claim | Immutable progress submission plus runtime file observations and optional persona checks. |
| `mother_task_checkpoints_list` | `task_id`; optional `after`, `limit` | Immutable checkpoint pages. |
| `mother_task_transitions_list` | `task_id`; optional `after`, `limit` | Immutable transition reasons, sources, states, revisions and persona attribution. |
| `mother_personas_list` | Optional `status`, `project_id`, `after`, `limit` | Persona summaries and cursor. |
| `mother_persona_get` | `persona_id` | Current persona revision, capabilities and scope. |
| `mother_persona_create` | `idempotency_key`, nested `PersonaInput` | Created persona revision and audit record. |
| `mother_persona_update` | `idempotency_key`, nested `PersonaUpdate` | Replacement revision; stale expected revisions conflict. |
| `mother_persona_set_status` | `idempotency_key`, nested `PersonaStatusChange` | Disabled/active/archived revision. |
| `mother_persona_history` | `persona_id`; optional `after`, `limit` | Immutable persona revision pages. |
| `mother_github_identities_list` | Optional `status`, `persona_id`, `after`, `limit` | Verified read-only GitHub identity records; no credentials. |
| `mother_github_identity_get` | `identity_id` | One identity, persona binding, verification limits and revocation state. |
| `mother_github_identity_history` | `identity_id`; optional `after`, `limit` | Immutable identity authorization revisions. |
| `mother_worker_eligible_tasks` | Optional `project_id`, `limit` | Bounded ready tasks with completed dependencies and no unexpired lease. |
| `mother_worker_executions_list` | Optional `task_id`, `project_id`, `status`, `after`, `limit` | Worker leases with workspace, process, expiry, revision and explicit limits. |
| `mother_worker_execution_get` | `execution_id` | One lease, workspace, process, bounded evidence, task state, revision and result. |
| `mother_worker_execution_history` | `execution_id`; optional `after`, `limit` | Immutable lease, heartbeat, mock-checkpoint, workspace, process, release and expiry events. |
| `mother_activity_get` | Optional `project_id`, `after`, `limit` | `items`, `next_cursor`, project `revision` when scoped. |

Pagination defaults to `after=0`, `limit=20`, with `1..=100` records per page. Pass
`next_cursor` as the next call's `after`. Serialized item data is also limited to
roughly 1 MiB per collection, so follow cursors even after a short page. Lists are ascending and pages are not frozen
snapshots. Context kinds are `fact`, `observation`, `failure`, `blocker`, `objective`.
All remain unverified submissions, even when labeled `fact`. Context reads also
include a separate `active_decisions` page with its own `next_cursor`; use
`mother_decisions_list` to page through those decisions.

Context writes accept at most 4096 UTF-8 bytes of content, 256 source bytes, and
128 retry-key bytes. The runtime rejects empty/whitespace-only values and NULs.
The MCP string-length schema is a preliminary character bound; authoritative byte
limits remain in the runtime. Unknown input fields are rejected. Project registration
remains CLI-only; there is no MCP tool for arbitrary filesystem or shell access.

Persist `idempotency_key` before a write. Reuse it with identical arguments after
any ambiguous transport failure. Keys use the same runtime-wide namespace as CLI
`--request-id`. A replay returns the original response, including its original
revision; it does not imply that the project still has that revision. Fresh reads
always query the service. No bridge cache can hide CLI changes.

## Resources

- `mother://health`: runtime diagnostics.
- `mother://tasks/{task_id}`: live host-tracked task and latest checkpoint.
- `mother://handoffs/{handoff_id}`: immutable package plus live revision and claim.
- `mother://personas/{persona_id}`: current persona revision and capabilities.
- `mother://github-identities/{identity_id}`: verified identity metadata and limits.
- `mother://worker-executions/{execution_id}`: worker lease and task coordination.
- `mother://projects/{project_id}`: project identity with current revision.
- `mother://projects/{project_id}/context`: first 20 context records with current
  revision and `next_cursor`; use `mother_context_get` for remaining pages.

Resource templates require canonical lowercase, hyphenated UUID identity values.
Resources contain JSON with the same response envelope. Reads go to the service
each time; URI stability does not imply immutable contents. Subscriptions and
change notifications are not advertised. Refresh explicitly before further work. A persona assertion on a protected task
operation is checked against the current runtime revision, so a client remembering
an older persona gets a conflict rather than silently applying old policy.

GitHub identity tools are read-only. Identity verification and revocation are
CLI-only owner actions. MCP never receives tokens, credential values, profile paths
or arbitrary GitHub operation access. A verified record is not a sandbox and does
not make task operations require a persona.

Worker tools are read-only. Lease acquisition, heartbeat, mock execution, process
supervision, release and recovery are CLI-only. `workers run` is an owner-run
supervisor: it creates an isolated Git worktree, starts a deterministic helper,
heartbeats the lease, records bounded artifact evidence, and retains the worktree
for inspection. The runtime itself never launches a process. A lease does not grant
identity, authorize project-repository writes or complete a task; a worktree prevents
checkout collisions but is not a sandbox.

## Transport and failure behavior

Stdout contains MCP messages only; diagnostics go to stderr, including fatal bridge
errors. Input lines are bounded at 64 KiB. SDK transport drops oversized lines;
clients must time out such requests. At most 16 operations per bridge are forwarded
concurrently; excess calls return a `busy` tool error. Local IPC enforces its own
client limits and five-second read/write timeouts. Responses remain bounded by
the runtime's page/content limits. JSON text compatibility duplicates structured
content, so MCP response sizes can exceed the corresponding IPC response size.

An absent/stopped service returns `runtime_unavailable`, never a successful empty
result. An already connected bridge recovers on its next call after service restart.
An interrupted write may already have committed: always retry its original key.
Closing one MCP connection neither shuts down the service nor loses records.

## Host configuration examples

Build/install `motherctl`, initialize state, register a project, and start `serve`.
Replace `/absolute/path/to/motherctl` and `/absolute/path/to/state` in the examples
with actual absolute paths. Desktop hosts may have a different PATH or HOME.
These are opt-in templates; Mother does not edit host settings.

For Codex, merge [examples/mcp/codex.toml](../examples/mcp/codex.toml) into the
appropriate configuration after review. The installed CLI's `codex mcp add --help`
also confirms the equivalent command syntax:

```sh
codex mcp add mother -- /absolute/path/to/motherctl --data-dir /absolute/path/to/state mcp
```

The TOML `mcp_servers` command/args format was checked against the
[official Codex MCP documentation](https://developers.openai.com/codex/mcp/).

For Claude Code, merge [examples/mcp/portable.json](../examples/mcp/portable.json)
into project `.mcp.json`. The `mcpServers` stdio entry is documented in the
[official Claude Code MCP guide](https://code.claude.com/docs/en/mcp). Review the
project server in the host before using it.

For the current Copilot harness in VS Code, the same portable JSON can go in
workspace-root `.mcp.json`, or be merged into native user MCP configuration.
These locations and the `mcpServers` format were verified in the
[official VS Code reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration).
Older editor-managed `servers` configurations use a different format; this sample
targets native Copilot configuration. No installation or host settings were changed.

After connecting, list tools, call `mother_projects_list`, then call
`mother_context_get` with a returned project ID. Record context through a separate
CLI process and refresh it in the host. This confirms the host's actual tool path;
successful configuration parsing alone is insufficient.

An optional [project instruction example](../examples/mcp/PROJECT_INSTRUCTIONS.md)
teaches a host to refresh Mother's state without copying persona definitions.
Worker dispatch mutations, GitHub credentials, HTTP transport and host
subscription support are outside this slice.

## Decision and handoff input models

`DecisionInput` contains `project_id`, `title`, `decision`, `rationale`, `source`,
optional `alternatives` (default empty), `supersedes`, and `expected_revision`.
`HandoffInput` follows [the example submission](../examples/handoff.json). Omitted
list fields default to empty; omitted nullable fields default to null. New
mutations use a required stable retry key outside the nested `input` object:

```json
{"idempotency_key":"decision-storage-1","input":{"project_id":"PROJECT_ID","title":"Storage","decision":"Use SQLite","rationale":"Local-first operation","source":"engineering-session"}}
```

Only explicit supersession changes which decisions are current. Handoff snapshots
capture the latest bounded context and active decisions before their own creation
revision. Resume requires that the project still matches that creation revision;
a changed project requires fresh context and a new package. Retrying the identical
successful key still replays the original response. All progress/artifact/test
claims remain unverified; recipient labels route work without authenticating it.
Detailed field limits, hash verification, truncation, and revision semantics are
in [continuity documentation](CONTINUITY.md).

## Task input and verification models

The seven task tools use the same required `idempotency_key`/nested `input` mutation
wrapper as decisions and handoffs. Read tools have direct argument fields.
TaskInput is shown in [task.json](../examples/task.json); CheckpointInput in
[task-checkpoint.json](../examples/task-checkpoint.json). TaskTransition requires
`task_id`, `state`, `source`, `reason`, `expected_task_revision`; `owner_label` and
`expected_project_revision` are optional. JSON state values use underscores.

The runtime, not the submitting model, reads and hashes declared files. Checkpoint
input contains no trusted artifact evidence field. Local completion requires fresh
passing file evidence and does not verify tests, independent review, human approval,
or objective text. `review_required: true` blocks completion pending a verifier.
Checkpoints and state transitions do not launch or cancel external processes.
See [state graph, limits and evidence scope](ORCHESTRATION.md).

HandoffInput now accepts optional `task_id`. New snapshots include task revisions
and checkpoints; old immutable packages retain their original format. The task ID
must belong to the handoff project. Task snapshots do not confer permissions.
