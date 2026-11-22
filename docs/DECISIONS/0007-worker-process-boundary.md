# ADR 0007: Worker processes remain owner-run and worktrees are not sandboxes

## Status

Accepted for the deterministic mock process slice.

## Context

Leases made task execution durable and bounded inside the runtime, but they still
described only host-reported work. A process can hang, outlive a lease, exit with a
failure, or collide with an existing checkout. Recording an arbitrary PID without a
lifecycle would let a crashed row look active forever. Giving the runtime a general
process-launching or shell surface would expand the trusted IPC boundary far beyond
coordination.

## Decision

Keep the runtime free of process launching and signaling. Add one owner-run CLI
supervisor, `workers run`, that composes existing durable operations with one fixed
deterministic helper.

- Acquire the lease only through the existing dependency-aware, bounded operation.
- Resolve the registered project repository's current commit and create a detached
  Git worktree under the private worker execution directory. Disable hooks and
  fsmonitor for these Git commands and disable terminal credential prompts.
- Record workspace path and base commit before process attachment. The helper runs
  with a cleared environment, a worktree `HOME`, a separate process group, and the
  hidden `__mock-worker` command only.
- The helper checks that its supervisor is alive, optionally delays, and on success
  writes one deterministic `worker-output.txt` (0600). It makes no network call,
  runs no tests, edits no project checkout, and cannot complete a task.
- Record the PID, log path, timestamps, exit status, and bounded observations of the
  task's declared acceptance criteria. Capture at most 64 KiB each of stdout and
  stderr into a private log.
- Heartbeat from the supervisor while the helper runs. On timeout or shutdown,
  terminate the helper's process group. A successful exit moves a running task to
  `awaiting_input`; failure or timeout moves it to `failed`.
- If a lease expires while the process may still be running, block the task instead
  of requeueing it. Allow the process finish to record evidence while preserving the
  expired execution status.
- Retain the worktree. Require explicit `workers cleanup` to remove an inactive
  worktree, prune Git metadata, and mark the workspace removed.
- Expose workspace/process metadata through existing worker reads. Keep all process
  mutations CLI-only.

## Consequences

The slice proves owner-run supervision, isolated checkout creation, bounded logs and
artifact evidence, lease enforcement during a live process, explicit failure mapping,
and cleanup. It also removes the stale claim that a `running` task necessarily has no
process.

The worktree is checkout isolation, not a security sandbox. The helper has the same
OS-user authority as the owner and could access files outside its worktree; the fixed
write behavior is application design, not kernel enforcement. The runtime cannot
prove that a launched process performed requested work, detect late edits after
expiry, or prevent a same-UID process from submitting transitions.

If the supervisor itself is killed, the helper observes parent loss and exits, but
no online reaper currently finalizes the process row or releases blocked cleanup.
Operator reconciliation is required. Provider adapters, credentials, budgets, test
execution, cancellation across hosts, automatic retry, independent review, and
evidence reconciliation remain separate work. Any real provider adapter must reuse this ownership boundary and add explicit
credential, network, resource, and reconciliation controls before being described as
isolated execution.
