# Implementation roadmap

This repository began with only `MOTHER.md`. The first implementation is a complete
local project/context workflow, with verified restart and multiple-process access.

| Directive phase | Current state | Remaining work |
| --- | --- | --- |
| 1: foundation | Implemented | Continue extending diagnostics with new subsystems. |
| 2: persistent runtime | Initial slice verified | Background lifecycle integration; domain entities grow with later slices. |
| 3: MCP | Core bridge verified; persona and read-only identity tools added | Interactive host certification. |
| 4: continuity | Tasks, checkpoints, decisions and handoffs verified | Persona/policy/Git scopes in execution snapshots; verified tests/reviews/approvals. |
| 5: personas | Registry, revisions, status and asserted capability gates verified | Mandatory-persona client model, notification/refresh certification, worker handoff. |
| 6: GitHub accounts | Read-only profile verification, identity records and revocation verified with a fake `gh` | Real-account certification, repository/action scopes, credential broker, write authentication, Git push isolation. |
| 7–9: execution/workflow | Dependency-aware leases, bounded scheduling and mock checkpoints verified | Supervised provider adapter, isolated worktrees, authorized GitHub workflow. |
| 10–12: TUI/hardening/release | Pending | Ratatui, cross-provider validation, packaging, release readiness. |

MCP evidence: two independent rmcp clients launch separate bridge processes, use
2025-11-25 initialize and 2026-07-28 discover lifecycles, see CLI changes, and recover
after the runtime is killed and restarted. Setup examples are documentation-verified;
no Codex, Claude Code, or Copilot interactive session has been executed.

Continuity evidence: decision replacement excludes historical choices from live
context; a separate MCP client recovers an immutable handoff after restart, including
a failed test and blocker; competing clients cannot both claim continuation.
Schema-1 data and retry responses survive migration, and injected migration/audit
failures roll back. These are project continuation snapshots, not full execution
snapshots with persona, policy, or repository-base versions.

Task evidence: separate CLI/MCP processes recover a running task and failed
checkpoint after service restart; competing transitions cannot both commit; a
fresh runtime file check gates local completion; a dependent task becomes eligible
only after its prerequisite completes. An old handoff retains its earlier task
snapshot. The runtime verifies file contents only, not tests or software correctness.

Persona evidence: two independent MCP clients read a CLI-created persona; a CLI edit
increments the revision and a fresh read sees the new capabilities; a task transition
using the old revision conflicts, and using the new revision rejects the revoked
capability. Transition records retain the authorizing persona revision. The persona
assertion is optional, so this is runtime policy enforcement for asserted operations,
not mandatory authenticated authorization.

GitHub identity evidence: an isolated profile is created under private state; a fake
`gh` verifies the configured account while token environment variables are removed;
CLI and MCP reads expose the active record; revocation takes effect immediately and
immutable authorization history remains readable. This uses a fake executable, not
a real account, and covers no broker, repository/action scope, Git push or GitHub
write.

Worker scheduling evidence: ready tasks with completed dependencies are listed;
one lease is enforced per task and a global active limit rejects excess grants;
heartbeats extend expiry and reject stale revisions; the mock adapter writes a
checkpoint while leaving files unchanged and refusing completion; expired leases
recover after an actual runtime kill/restart; CLI and MCP reads agree. This is not
a supervised worker, worktree sandbox, test runner or evidence reconciliation.

Next vertical slice: a supervised worker-process adapter that runs in an isolated
Git worktree, reports bounded heartbeats, and reconciles repository evidence before
any task transition. Keep GitHub writes, credential brokering and independent
review gates separate until their authorization and reconciliation boundaries are
verified. Real-host cross-assistant certification remains required before claiming
cross-provider continuity.

Before optional features, complete competitive analysis as required by the directive.
Performance targets remain targets until representative benchmarks are recorded.
Backups/restores, retention, export, encryption/redaction, installers, launchd,
remote transports, supervised workers, worktrees, and credential handling are not
part of this slice.
