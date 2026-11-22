# ADR 0006: Worker leases are coordination metadata

## Status

Accepted for the deterministic mock scheduling slice.

## Context

Tasks need durable execution ownership before Mother can dispatch a worker or
recover from a crashed process. A claimed owner label in task history is not
enough: two clients could act concurrently, a crashed client could leave a task
`running` indefinitely, and task state alone cannot express bounded lease time.

At the same time, this slice must not imply that Mother launches, supervises or
authorizes a process. No provider adapter, worktree isolation, credential broker,
test runner or GitHub write path exists. Completion must still depend on runtime
file evidence and explicit host transitions.

## Decision

- Persist worker executions separately from task state. A lease records an
  execution ID, task/project ID, worker label, attempt, lease duration, expiry,
  heartbeat, revision, status and bounded result.
- Permit at most one active (`leased` or `running`) execution per task through a
  partial unique index. Enforce a caller-supplied global `max_active` bound in the
  same SQLite transaction.
- Acquire from a `ready` task only. The transaction validates completed
  dependencies and revision expectations, transitions the task
  `ready → assigned`, records the worker label as owner, and inserts the lease.
- Require an explicit heartbeat with the current execution revision. Heartbeats
  extend expiry from the current time; stale or expired leases are rejected.
- Expire overdue leases explicitly or during reacquisition. Expiry marks the
  execution `expired`; `assigned` tasks return to `ready`, and `running` tasks
  return through `awaiting_input` to `ready`. Other task states are left for
  operator reconciliation.
- Provide one deterministic `mock` adapter. Invocation transitions an assigned
  task to `running` and writes an ordinary immutable checkpoint. It never edits a
  repository, runs tests, completes a task or attests correctness.
- Release only records an execution outcome. `completed` execution requires a
  task that is already completed; release does not change task state.
- Expose worker reads through MCP. Lease acquisition, heartbeat, mock execution,
  release and recovery are owner-run CLI operations. All same-UID processes remain
  inside the existing trust boundary; MCP exposure would not create a new one, but
  keeping dispatch mutations CLI-only avoids presenting workers as host authority.

## Consequences

Leases survive runtime restarts and prevent duplicate coordination inside Mother's
canonical database. Expiry restores eligible work without pretending that the
lost process succeeded. Every lease transition has an immutable event and a
revision for optimistic retries.

A lease is not an OS lock, sandbox, process supervisor, identity proof or write
authorization. It does not stop a same-UID process from editing files or sending
task transitions, and it cannot detect work after a lease expires. The mock
runner only proves scheduling, checkpointing and recovery mechanics. A future
adapter must add process lifecycle, isolated worktrees, budget enforcement,
evidence reconciliation and independent review as separate verified boundaries.

ADR 0007 later added an owner-run supervisor and checkout-isolated worktree for
one deterministic mock helper. That does not turn a lease into a sandbox or add
provider, budget, test or review capabilities.
