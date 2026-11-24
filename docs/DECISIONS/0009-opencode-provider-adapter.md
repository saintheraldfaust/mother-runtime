# ADR 0009: The first provider adapter is owner-run OpenCode CLI

## Status

Accepted for schema 9.

## Context

ADR 0007 established one owner-run supervisor and an isolated Git worktree for a
fixed deterministic helper. ADR 0008 added explicit reconciliation for a killed
supervisor. The next execution slice needed one real coding-agent adapter without
turning the runtime into a process launcher, credential broker, network service or
sandbox.

OpenCode provides a documented non-interactive interface:
`opencode run --format json`. It emits newline-delimited JSON with a top-level
session identifier, provider-reported step cost and token accounting, tool events,
retries and errors. The owner already has an OpenCode CLI installation and may have
provider authentication in the host account.

## Decision

Add an `opencode` adapter alongside the existing `mock` adapter.

- Schema 9 rebuilds `worker_executions` to permit `adapter IN ('mock','opencode')`
  while preserving executions, events, workspaces and processes. The adapter is
  selected at lease acquisition and remains durable.
- `workers lease` and `workers run` accept `--adapter mock|opencode`. The default
  remains `mock`.
- An OpenCode run requires an explicit `--model provider/model`, a positive
  `--max-cost-usd` ceiling and `--max-steps` in 1..=200. `--agent` is optional.
  These options are rejected for the mock adapter.
- The owner-run supervisor invokes only
  `opencode run --format json --pure --auto --dir WORKTREE
  --model MODEL --title ... [--agent AGENT] PROMPT`. The installed 1.19.6 CLI
  does not accept `--standalone`, so the adapter does not use it.
- The child receives an environment allowlist plus relevant `XDG_*` variables.
  `OPENCODE_CONFIG_DIR` points into the private execution directory,
  `OPENCODE_DISABLE_PROJECT_CONFIG=1`, and `NO_COLOR=1`. The host `HOME` and
  OpenCode data store are retained because version 1.19.6 stores login credentials
  in its data database. The adapter therefore can use an existing owner login but
  does not isolate provider session records from that host data store. Mother does
  not copy, store, return or broker provider tokens.
- `--pure` disables external plugins and `--auto` explicitly authorizes tool
  execution without interactive approval. The worktree path is supplied both as
  the process working directory and through the prompt, but neither is a sandbox.
- The prompt contains bounded task title, objective, source and acceptance
  criteria. Provider text and reasoning are not completion evidence.
- The supervisor parses provider telemetry into bounded metadata: session ID,
  accumulated cost, input/output/reasoning/cache-read/cache-write tokens, steps,
  tool calls, retries, up to five bounded errors, parse-error count and line count.
  Raw stdout is captured only in the existing 64 KiB process log. No hidden
  reasoning is stored.
- The supervisor checks cost and step budgets while the process runs and again
  after stdout completes. An exceeded budget terminates the process group when
  still running and marks the execution killed and the task failed. Timeout is
  recorded separately. The provider metadata records `budget_reason`. After any
  exit, remaining process-group members are terminated before output handles are
  joined, so a background child cannot stall finalization indefinitely.
- Provider process completion requires bounded JSON provider metadata. The result
  reports `repository_mutation: null` because Mother does not verify project
  repository changes, and derives `worktree_mutation` from a bounded `git status`
  observation inside the worktree. Mock completion continues to report
  `repository_mutation: false`.
- Acceptance-criterion evidence is still observed from the worktree before any
  transition. A successful provider exit moves a running task to `awaiting_input`;
  failure, timeout or budget exhaustion moves it to `failed`. No path completes the
  task automatically.
- ADR 0008 reconciliation remains the recovery boundary. Provider runs do not add
  an automatic reaper, cross-host ownership, signal delivery to an unverified PID,
  or credential brokering.

## Consequences

The repository now contains a real provider-facing CLI adapter and durable adapter
selection, but tests use a fake OpenCode executable. No paid or authenticated model
call, real provider certification, interactive host certification or billing
reconciliation is claimed.

Budgets are based on provider-reported events. A provider that underreports cost,
delays emitting events, or exits before accounting is observed can exceed the
intended spend; the ceiling is not an independent metering or billing guarantee.
Token counts are recorded but there is no independent token budget.

`--auto` allows the model to execute tools with the owner's OS-user authority.
OpenCode session records and credentials remain in the host data store, not Mother's
private execution directory. The worktree and isolated OpenCode configuration do
not prevent network access, access
to the host home directory, SSH keys, existing GitHub CLI configuration or other
files available to that user. Task text can influence those tools and must not be
treated as trusted authorization. This adapter is owner-run execution, not a
sandbox or a secure multi-tenant boundary.

A successful provider exit proves only the observed process outcome. It does not
prove that requested work was correct, tests passed, the project repository was
untouched or independent review occurred.
