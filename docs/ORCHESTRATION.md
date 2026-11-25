# Host-tracked tasks and checkpoints

Mother persists task contracts, dependency IDs, state transitions, checkpoints,
narrowly scoped file evidence, durable worker execution leases, and owner-run
supervised mock, Codex and optional OpenCode CLI processes. A task without a lease reports
`execution_mode: host_tracked` and null `worker_execution`; a leased task reports
`worker_leased` plus its active execution. A `running` record still describes
reported work, not proof of correctness or independent review. The mock helper runs
in an isolated worktree and never edits the project repository or completes a task.
The OpenCode adapter can edit its worktree, but project-repository mutation is not
verified and no process path completes a task automatically.

## Contracts and revisions

Create a task from [examples/task.json](../examples/task.json), replacing its
project ID. Required fields are project, title, objective, source, and one to 16
artifact criteria. A criterion is a normalized repository-relative `path` and an
optional lowercase SHA-256 expectation. When the digest is absent, acceptance
checks file presence and unchanged content since the validating checkpoint.
These contracts do not encode arbitrary behavioral or test requirements.

Titles/sources allow 256 UTF-8 bytes, objectives 4096, and paths 512. Up to 16
unique dependencies must already exist in the same project. Since dependencies
are fixed at creation and only reference existing tasks, API-created graphs are
acyclic. Task input is bounded to 24 KiB of serialized JSON. `review_required`
defaults to false; true blocks completion until a review verifier is implemented.

A task starts at task revision 1 and project revision N+1. Each accepted transition
or checkpoint increments both revisions. Mutations require `expected_task_revision`
and optionally `expected_project_revision`. Unrelated project activity does not
invalidate a task mutation unless that optional project check was supplied.
Task updates, immutable history/checkpoints, audit events, and retry responses
commit atomically. Exact retries return their original result without rereading files.

Task create, transition and checkpoint inputs may include an optional `persona`
claim with `persona_id` and `expected_revision`. The runtime then loads the current
persona and requires active status, matching project scope and the relevant
capability (`task.create`, `task.transition` or `task.checkpoint`). A stale claim
is rejected before any task mutation. Accepted transitions/checkpoints record the
persona ID and revision; the fields are null on the local-operator path. Omitting
the claim does not bypass other task revision, state or evidence checks.

## State transitions

The local completion path is:

```text
created → triaged → ready → assigned → running → validating
                                                  ↓
                                      fresh file checkpoint → completed
```

All allowed edges are listed below. Other edges, including self-transitions, fail.

| From | Allowed next states |
| --- | --- |
| created | triaged, cancelled |
| triaged | ready, awaiting_dependency, awaiting_input, blocked, cancelled |
| ready | assigned, awaiting_dependency, blocked, cancelled |
| assigned | running, ready, blocked, cancelled |
| running | awaiting_dependency, awaiting_input, validating, failed, blocked, cancelled |
| awaiting_dependency | ready, blocked, cancelled |
| awaiting_input | ready, running, blocked, cancelled |
| validating | awaiting_review, changes_requested, completed, failed, blocked, cancelled |
| awaiting_review | changes_requested, ready_for_human_approval, blocked, cancelled |
| changes_requested | assigned, blocked, cancelled |
| ready_for_human_approval | changes_requested, blocked, cancelled |
| blocked | triaged, cancelled |
| failed | triaged, cancelled |
| completed / cancelled | none |

Entering ready, assigned, running, validating, or completed requires all dependencies
to be completed. Dependency changes do not auto-dispatch tasks. Assignment requires
an owner label, which can only be set while entering assigned. Running also requires
an existing owner label. Labels are self-reported coordination metadata, not
persona definitions, verified identities, or permissions.

Review/approval states track pending coordination only. There is no transition
from ready_for_human_approval to completed; no review or human approval is fabricated.
Failed tasks can explicitly return to triage; completed/cancelled tasks are final.
Cancellation changes a host-tracked record and does not terminate external processes.

## Checkpoints and file observations

Use [examples/task-checkpoint.json](../examples/task-checkpoint.json). A checkpoint
contains a summary, source, remaining work, blockers, and reported test results.
The runtime observes the contract's files itself; callers cannot submit fabricated
artifact hashes in checkpoint input. Checkpoints do not change task state.

Checkpoint summaries allow 4096 UTF-8 bytes and source 256; each list permits 16
entries of up to 1024 bytes, subject to an overall 24 KiB serialized-input cap.
Missing or rejected files produce explicit failed observations while the checkpoint
still persists. This preserves partial progress and failures. Checkpoints on
completed, cancelled, or failed tasks are rejected; retry failed work through triage.

File observations anchor at the registered repository and open every relative
component with `openat` and `O_NOFOLLOW`. Absolute paths, traversal, repeated slashes,
and dot components are rejected at contract creation. The verifier rejects symlinks,
hard links, and non-regular files, including FIFOs. Reads are limited to 4 MiB per
file and 16 MiB aggregate per observation pass, with one-byte over-limit probes.
No commands or tests execute, and raw file contents are neither stored nor returned.
Metadata is checked before/after reading and the final path is reopened to detect
replacement. SHA-256 and byte counts are exposed only for successfully read files.

Observation statuses include `present`, `matched`, `mismatched`, `missing`,
`unavailable`, `rejected_file_type_or_link`, `too_large`, `budget_exceeded`,
`read_limit_exceeded`, `changed_during_read`, and `repository_unavailable`.
`matched` compares the actual digest with an expected digest. `present` means no
expected digest was supplied. All statuses are runtime observations, not agent claims.

These are point-in-time observations of individual files, not an atomic filesystem
snapshot or protection against a hostile same-UID process. The filesystem can
change after a check. Use a local filesystem with trusted repository ancestry.

## Completion boundary

Local completion is allowed only from validating. Its latest checkpoint must be
fresh: the checkpoint's task revision equals the current task revision, so another
transition requires a new checkpoint. The runtime rereads every contracted file,
requires present/matched status, and compares the observations with the checkpoint.
Changed, missing, mismatched, or unreadable files reject completion without mutation.

Accepted completion stores the checkpoint ID, runtime observation time, hashes,
file sizes, and explicit `tests_verified: false`,
`independent_review_verified: false`, `human_approval_recorded: false` flags.
`completed` means this declared local file contract passed at that check. It does
not establish software correctness, test success, independent review, or merge
approval. Free-form objective text is not interpreted as an executable verifier.

## CLI and MCP

```sh
motherctl --request-id task-create-1 tasks create --file task.json
motherctl tasks inspect --task TASK_ID
motherctl tasks list --project PROJECT_ID --state running
motherctl --request-id triage-1 tasks transition --task TASK_ID \
  --state triaged --source cli-session --reason 'Scope reviewed' \
  --expected-task-revision 1
motherctl --request-id checkpoint-1 tasks checkpoint --file task-checkpoint.json
motherctl tasks checkpoints --task TASK_ID
motherctl tasks history --task TASK_ID
motherctl workers eligible --project PROJECT_ID
motherctl workers list --task TASK_ID
motherctl workers history --execution EXECUTION_ID
```

The checkpoint example assumes a newly created task; set its task ID and expected
revision to the current values before using it. Continue through the explicit state
path, adding `--owner-label LABEL` for assignment. CLI multiword state values use
hyphens; JSON/MCP state values use underscores. See [MCP schemas](MCP.md) for the
seven task tools and the `mother://tasks/{task_id}` resource. Worker
reads use four dedicated tools and `mother://worker-executions/{execution_id}`.

Task lists return summaries; inspect returns the latest checkpoint and transition,
criteria, dependency IDs, and completion evidence. Checkpoint and transition history
use sequence pagination. All pages obey both item limits and an approximately 1 MiB
serialized-item budget; a short page with `next_cursor` can still have more records.

Handoff input can specify `task_id` from the same project. Its immutable snapshot
then includes that full task, revision, and latest checkpoint as `current_task`.
Snapshots also select up to 20 other unfinished tasks by latest project mutation,
with omission counts and the existing snapshot byte budget. Old packages remain
unchanged even after a task completes; query live task state when continuing.

## Worker execution leases and supervised runs

A lease is durable coordination metadata, not a process or permission. It records
an execution ID, task/project, worker label, attempt, lease duration, expiry,
heartbeat, revision, status, and a bounded result. At most one execution can be
active per task, and callers supply a global `max_active` bound (1–64). The store
checks dependencies, task state and bounds transactionally.

Only a `ready` task can be leased. Acquisition transitions it to `assigned`, sets
the worker label as owner, and creates a `leased` execution. The worker label is
self-reported coordination data, not a persona or verified identity. Start the
mock checkpoint adapter, extend the lease, or release it explicitly:

```sh
motherctl workers eligible --project PROJECT_ID
motherctl --request-id lease-1 workers lease --task TASK_ID \
  --worker-label mock-a --lease-seconds 120 --max-active 4
motherctl workers inspect --execution EXECUTION_ID
motherctl workers heartbeat --execution EXECUTION_ID \
  --expected-revision 1 --lease-seconds 120
motherctl workers mock --execution EXECUTION_ID \
  --expected-revision 2 --summary 'Mock checkpoint only'
motherctl workers release --execution EXECUTION_ID \
  --expected-revision 3 --outcome failed --summary 'Stopped before verification'
```

Lease durations are 1–3600 seconds. Heartbeats require the current execution
revision and extend expiry from the current time. A stale, released or expired
execution cannot heartbeat. Execution statuses are `leased`, `running`, `released`,
`failed`, `expired`, and `completed`; only the first two are active.

The checkpoint-only `mock` operation requires an assigned/running task, transitions
an assigned task to `running`, writes a normal checkpoint with source `mock-worker`,
and extends the lease. It performs no repository reads beyond the ordinary checkpoint
contract, no writes, no test execution, and no completion. Its response explicitly
reports `repository_mutation: false` and `completion: false`.

### Owner-run supervised process

`workers run` is a separate CLI supervisor for one selected execution adapter. It
leases a ready task, resolves the registered repository's current Git commit, creates
a detached worktree under `data/workers/<execution_id>/worktree`, records the base
commit, launches the adapter in its own process group, attaches its PID, heartbeats
while it is alive, reaps remaining process-group members after exit, writes a
bounded `worker.log`, and finalizes process evidence.

The durable execution records the adapter selected at lease time. The default is
`mock`; `--adapter codex` uses the existing Codex login, and `--adapter opencode` selects the optional OpenCode CLI adapter.

```sh
motherctl workers run --task TASK_ID --worker-label mock-a \
  --lease-seconds 120 --max-active 2 --timeout-seconds 60
motherctl workers run --task TASK_ID --worker-label opencode-a \
  --adapter opencode --model provider/model \
  --max-cost-usd 1.00 --max-steps 20 \
  --lease-seconds 120 --max-active 2 --timeout-seconds 300
motherctl workers inspect --execution EXECUTION_ID
motherctl workers cleanup --execution EXECUTION_ID
```

The mock helper starts with a cleared environment, a worktree `HOME`, Git terminal
prompts disabled, and Git hooks/fsmonitor disabled for worktree operations. It checks
that the supervisor is alive, optionally sleeps, and on success writes only
`worker-output.txt` (0600) with `mock worker execution <execution_id>`.

Standard output and error are captured up to 64 KiB each in `worker.log` (0600).
Process records expose PID, status, exit code, log path, timestamps, and bounded
acceptance-criterion observations from the worktree. They do not prove correctness,
tests, independent review, or completion.

#### OpenCode CLI adapter

The OpenCode adapter invokes only:

```text
opencode run --format json --pure --auto \
  --dir WORKTREE --model MODEL --title TITLE [--agent AGENT] PROMPT
```

`--model` and a positive `--max-cost-usd` are required. `--max-steps` is 1–200 and
defaults to 20. The supervisor supplies a bounded prompt containing the task title,
objective, source and acceptance criteria. The prompt is not authorization, and
provider text is not evidence.

The child receives an environment allowlist plus relevant `XDG_*` variables. Its
OpenCode configuration lives in the private execution directory and project
configuration is disabled. The host `HOME` and OpenCode data store are retained
because 1.19.6 keeps login credentials there; provider sessions may therefore
remain in host OpenCode storage. Mother does not copy, store or return provider
tokens.
`--pure` disables external plugins. `--auto` explicitly authorizes tool execution
without interactive prompts. The installed 1.19.6 non-interactive path uses an
in-process server, so no external OpenCode server is required.

The supervisor parses newline-delimited JSON for the session ID, provider-reported
cost, token counts, step and tool counts, retries, bounded errors and parse failures.
Provider records store bounded accounting metadata, not model text or hidden
reasoning; the bounded raw process log may still contain streamed model output.
Cost is accumulated from `step_finish` events and checked while the process runs and
after output completes. Budget exhaustion terminates the process group when needed;
timeout and cost/step exhaustion mark the execution killed and a running task failed.
`budget_reason` records `cost`, `steps`, `timeout` or `heartbeat_failed` when known.
Cost is provider-reported, not independently metered; a provider that underreports or
delays accounting can exceed the intended ceiling.

Provider completion requires bounded JSON provider metadata. `repository_mutation`
is `null` because Mother does not verify project-repository changes.
`worktree_mutation` is derived from a bounded `git status` observation inside the
worktree; up to 64 changed paths are recorded. Acceptance-criterion evidence is
observed from the worktree before any transition.

The adapter uses the owner's OS-user authority and may use network access, host
OpenCode authentication, files outside the worktree, and arbitrary provider tools
authorized by `--auto`. The worktree and isolated OpenCode configuration prevent
checkout/config collisions, not hostile access. This is not a sandbox, credential
broker, independent metering service or billing guarantee.

A successful mock or provider exit transitions a `running` task to `awaiting_input`;
a failed, killed or budget-exhausted process transitions it to `failed`. No path
completes the task. If the lease expires while a process may still be running,
recovery changes the task to `blocked` instead of requeueing it. Explicit release is
rejected while a supervised process is running. The worktree is retained for
inspection; `workers cleanup` only removes an inactive execution's worktree, prunes
Git metadata, and marks the workspace removed. Cleanup does not delete the execution
record, log, or evidence.

Expiry recovery is explicit (`workers recover`) or occurs when reacquiring the same
task. An expired execution is marked `expired`. An `assigned` task returns to
`ready`; a `running` task without a running process returns through `awaiting_input`
to `ready`; a running process blocks automatic requeue; other task states are left
for operator reconciliation. Recovery preserves history and sets a new attempt on
the next lease. Leases persist across runtime restarts.

A lease is not an OS lock, sandbox, process supervisor, identity assertion, or write
authorization. The owner-run CLI supervises the selected adapter; the runtime itself
does not launch, signal or inspect arbitrary processes. A Git worktree prevents
checkout collisions, not filesystem access: a same-UID process can still edit files
outside it or submit task transitions, and Mother cannot detect work performed after
expiry. Read worker state through MCP; lease, heartbeat, mock-run, process supervision,
abandonment, cleanup, release and recovery mutations are CLI-only.

If the supervisor itself is killed, the adapter may exit, but its durable process row
still says `running`. After confirming the recorded PID is gone, the owner can
reconcile it:

```sh
motherctl workers abandon --execution EXECUTION_ID
motherctl workers cleanup --execution EXECUTION_ID
```

`abandon` marks the process `lost`, leaves the execution `failed` (or `expired`),
and blocks an assigned/running task. It never signals or kills the recorded PID.
Without `--force`, it refuses while that PID still exists. Worktree cleanup remains
a separate explicit step. Pre-process failures such as worktree preparation,
workspace registration, provider spawn, or process attachment invoke the same
best-effort abandonment so the task does not remain assigned. There is no automatic
orphan reaper or cross-host process reconciliation. See
[ADR 0006](DECISIONS/0006-worker-lease-boundary.md),
[ADR 0007](DECISIONS/0007-worker-process-boundary.md),
[ADR 0008](DECISIONS/0008-abandoned-process-reconciliation.md) and
[ADR 0009](DECISIONS/0009-opencode-provider-adapter.md).

## Remaining execution work

The mock, Codex and optional OpenCode adapters use the owner-run supervisor.
A small real Codex execution created a declared file with a matching SHA-256 on
2026-11-25. OpenCode remains fake-tested. Codex requests its workspace-write sandbox;
Mother does not independently certify hostile-code isolation or broker credentials.
Independent spend metering, automatic retry, test execution, review verification,
human approval and GitHub writes remain unimplemented. Successful workers leave
tasks awaiting input rather than completed. See [Codex execution](CODEX.md) for
subscription accounting, privacy and limit semantics, and [TUI](TUI.md) for live reads.
