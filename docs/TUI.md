# Terminal interface

Project registration, task forms and Codex start/stop controls pass the current
regression suite and a scripted terminal workflow using a fake Codex executable.
The real landing-page workflow in VS Code remains to be exercised.

From the repository root:

```sh
cargo build --locked
./target/debug/motherctl ui
```

The command initializes private state and starts the canonical service when needed.
The default data directory is `~/.mother`; `--data-dir PATH` selects another private
directory. Quitting leaves the service running. `motherctl serve` remains available
for foreground operation. There is no login-time service installation.

## Register a project

Press **a**, enter a name and the absolute path to an existing project directory,
then press **Ctrl-S**. Registration preserves the directory's files and Git history.
The new project becomes selected after the service refreshes.

Use **p** to cycle registered projects. Registration does not initialize Git or
configure the project's Codex MCP connection. Use `motherctl setup-codex` from that
project directory when you also want to access Mother through Codex in VS Code.

## Create and prepare a task

Press **n** in the selected project. Supply a title and implementation objective.
Optional acceptance criteria describe desired behavior, one per line: for example,
"Works on mobile" or "Includes a contact section." The agent inspects the repository
and chooses the implementation files. No filenames are required.

Press **Ctrl-O** to reveal optional advanced fields. File constraints specify known
repository-relative paths only when needed. Dependencies are existing task IDs, one
per line. Both fields can remain empty. Hidden advanced settings retain any values
you have explicitly entered; toggling their visibility does not clear them.

Use Tab / Shift-Tab to move between fields. Enter inserts a newline in multiline
fields and advances other fields. Arrow keys, Home, End, Backspace and Delete edit
text; Ctrl-U clears the current field. Bracketed paste accepts multiline content.
Ctrl-S submits; Esc closes an unsubmitted form.

Tasks start in `created`. Select the task in Tasks and press **t** to review and
confirm its readiness transitions. The runtime enforces dependencies and current
task revisions. Preparing a task neither starts execution nor approves its result.
Failed or blocked tasks can be triaged through this control before retrying.

New task submissions retain their retry key until edited. Editing creates a new
key. A failed or uncertain submission leaves the form available for retry. Readiness
transitions also retain per-step keys, so a retry does not repeat a successful first
transition. State conflicts require reviewing the current task before proceeding.

## Start and stop Codex

Select a `ready` task in Tasks and press **r**. Review the timeout, step limit and
optional model, then press Ctrl-S. This explicitly starts a Codex supervisor using
the existing host login and subscription. No new login is performed.

The repository must already contain an initial Git commit. Codex starts from that
committed HEAD in a separate worktree; uncommitted files are not copied. The task
objective, behavioral criteria and any optional file constraints form the worker brief. The worker does not automatically
receive every stored project context item or persona policy.

Workers shows process status, usage metadata, the retained worktree and observed
files, including artifacts discovered after the agent chooses its paths. A successful run leaves its task `awaiting_input`. Inspect and integrate
changes separately; the TUI does not merge output or certify tests, reviews or
software correctness.

To stop a running worker launched by this dashboard session, select it in Workers
and press **s**, then confirm. The dashboard signals its owned supervisor child;
it never signals a PID taken from a stored execution record. The supervisor stops
its worker process group and records the outcome. A stopped process is `killed`,
its task becomes `failed`, and the worktree is retained. A stop request is not
presented as completed termination before that result is observed.

Workers started elsewhere or detached by an earlier dashboard session remain
readable but cannot be stopped from a new session through this control. Existing
CLI abandonment and cleanup procedures still apply to crashed supervisors.

Press q and confirm to leave the dashboard. Running supervisors continue in separate
process groups with private file output. Stop them first if you intend to end their
execution. The runtime also remains running.

## Controls and state

| Key | Action |
| --- | --- |
| a | Register an existing project directory |
| n | Create a task in the selected project |
| t | Confirm readiness transitions for the selected task |
| r | Configure and start Codex for a ready task |
| s | Request a stop for a worker owned by this dashboard session |
| Ctrl-O (task form) | Show / hide advanced file constraints and dependency fields |
| Tab / Right, Shift-Tab / Left | Next / previous view |
| 1–7 | Select a view |
| Up / Down | Select a record |
| j / k | Scroll details |
| p | Cycle projects |
| q / Esc / Ctrl-C | Open the quit confirmation |

Views show project overview, tasks, workers, personas, handoff metadata, global
GitHub account metadata and activity. Task details include the selected task's
objective. Lists follow service cursors up to 1,000 records per collection.

Reads and mutations run outside the terminal event loop. The TUI uses canonical
IPC operations and the existing CLI supervisor, never a separate database writer.
If reads fail, the last successful snapshot is marked OFFLINE / STALE and project,
task and launch actions are disabled. Individual reads do not form an atomic snapshot.
Submitted control characters are removed before display; raw model transcripts and
authentication values are not shown.

`motherctl ui --once` prints one live overview without a TTY or mutation controls.

Behavioral criteria remain unverified until appropriate verification is implemented.
Worker output and discovered file hashes do not satisfy them automatically. A task
with no file constraints cannot complete through an empty file-evidence check.
