# ADR 0008: Abandoned supervision requires explicit reconciliation

## Status

Accepted for schema 8.

## Context

ADR 0007 made one owner-run supervisor responsible for a fixed mock helper. If that
supervisor is killed, the helper detects parent loss and exits, but the durable
process row still says `running`. Lease expiry can block the task, yet the apparent
running process prevents cleanup, so the retained worktree becomes unrecoverable
through public commands. Automatically deciding that the process is gone inside the
runtime would require unsafe OS-process assumptions and could race a live helper.

## Decision

Add an explicit `lost` process state and an owner-run `workers abandon` command.

- Schema 8 rebuilds `worker_processes` to permit `lost` while preserving every
  existing process row.
- `abandon` requires the current execution revision and either an active execution
  or a running process.
- If the process is running, it becomes `lost` with an end timestamp. The execution
  becomes `failed`, or remains `expired` if its lease already expired.
- An assigned or running task becomes `blocked` with a durable abandonment reason.
  A task already blocked or in another state is not silently advanced.
- The operation never sends a signal. Without `--force`, the CLI checks whether the
  recorded PID still exists and refuses while it does. With `--force`, the owner
  asserts that the PID is gone or acceptable to abandon without signaling it.
- The worktree remains. `workers cleanup` can now remove it because the process is
  no longer `running` and the execution is inactive.
- Failures after lease acquisition but before process attachment use a best-effort
  abandonment, preventing a permanently assigned task after worktree, workspace,
  spawn, or attach errors.
- MCP remains read-only and exposes the resulting `lost` process state.

## Consequences

A killed supervisor is recoverable through supported commands without editing the
database or removing a worktree under an apparently live process. The guard reduces
routine errors, but PID liveness is not proof of identity: PID reuse can produce a
false positive, and `--force` deliberately bypasses the guard. Abandonment also does
not prove that the helper stopped or that its work is valid.

There is no automatic orphan reaper, cross-host ownership, signal delivery, evidence
recalculation, or provider/budget integration. Those require stronger process
identity, credential, and host-coordination controls. A future provider adapter must
reuse this explicit reconciliation boundary rather than silently treating a stale
process as successful or failed work.
