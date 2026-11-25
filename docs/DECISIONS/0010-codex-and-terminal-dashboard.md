# ADR 0010: Codex host integration and a live terminal dashboard

Status: accepted and implemented, 2026-11-25.

The owner's existing coding client is Codex with a ChatGPT login. Requiring a
separate provider would obstruct the first useful execution workflow. Mother also
needs a visible interface backed by real canonical state.

Add Codex as an owner-run CLI worker adapter under the existing lease, detached
worktree, process-group, heartbeat, timeout and artifact-observation boundary.
Preserve the optional OpenCode and mock adapters. Schema 10 transactionally retains
the execution graph and permits the `codex` adapter. User configuration is ignored
for worker launches; credentials remain host-managed. Request the Codex
workspace-write sandbox and no interactive approvals, disable worker MCP servers,
and retain only completion/accounting metadata rather than raw provider output.
Subscription usage has no dollar estimate; reject dollar budgets for this adapter.

Successful exit requires a completed turn and no reported failure, then leaves the
task awaiting input. Timeout/observed step limits use the existing group termination
mechanism. These event-based limits cannot preauthorize or independently meter
provider operations. No new test execution, task completion or approval claim exists.

Add explicit background startup and project-local Codex MCP setup, plus a read-only
Ratatui client. Dashboard polling uses IPC outside the terminal event loop, retains
stale data on failures, follows bounded pagination and distinguishes actual worker
state from verified task completion. Existing project configuration is preserved;
setup refuses incompatible Mother entries and does not change global trust.

A real Codex execution produced a matching declared file and a separate Codex CLI
client read Mother through MCP. Automated tests cover invocation isolation, metadata,
privacy, missing/failed completion, limits, migration preservation, setup conflicts
and live dashboard rendering. Interactive VS Code discovery, cross-assistant
continuity, hostile-code isolation and task mutation inside the TUI remain future
work. See [Codex setup](../CODEX.md) and [TUI usage](../TUI.md).
