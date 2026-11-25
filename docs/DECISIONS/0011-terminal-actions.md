# ADR 0011: Terminal project, task and execution actions

Status: implemented; regression and scripted terminal fixture checks passed, 2026-11-25.

The terminal client needs to register projects, create tasks and launch work without
requiring users to assemble JSON contracts manually. Canonical writes must remain
in the service and provider execution must retain the existing supervisor boundary.

Add bounded forms with explicit submission, stable retry keys, visible errors and
fixed project/task identities. Project registration and task creation use typed IPC.
Preparing a task follows the existing state graph with per-step retry keys and
reviewed task revisions. New task creation is additive and does not require an
unchanged project revision while the user edits its objective.

The terminal action task starts the current `motherctl workers run` executable in
a separate process group with private output files. It passes the task revision
confirmed in the launch form; the lease operation rejects a stale launch. A Git HEAD
preflight explains empty repositories before acquiring a lease. The TUI continues
polling and handling input while work runs.

Start/stop controls use the existing Codex adapter and host authentication. Stop is
restricted to Child handles owned by the current dashboard session. Stored process
IDs and owner labels do not establish signal authority. SIGTERM/SIGINT handling in
the supervisor terminates the worker group and records its result. A stop produces
a killed process and a failed task; it grants no approval or completion.

Quit confirmation explains detachment. Supervisors have no terminal pipes to keep
drained and continue after the dashboard closes. A later dashboard can inspect
these executions but cannot adopt signal authority from their stored PIDs. Durable
cross-session cancellation needs a separate authenticated control mechanism.

Regression cases are added for form identity/retries, multiline Unicode input,
launch revision rejection and graceful operator stop. The regression suite and scripted form/launch/owned-stop workflow pass.
Active-worker detachment, cross-session stop rejection and the real landing-page
workflow remain pending. No broad provider or browser verification is claimed.
