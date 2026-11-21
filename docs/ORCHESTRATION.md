# Host-tracked tasks and checkpoints

Mother now persists task contracts, dependency IDs, state transitions, checkpoints,
and narrowly scoped file evidence. `execution_mode` is always `host_tracked`, and
`worker_execution` is null. A `running` record describes reported host work; Mother
has not launched a process. `workers_supported` remains false in health.

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
```

The checkpoint example assumes a newly created task; set its task ID and expected
revision to the current values before using it. Continue through the explicit state
path, adding `--owner-label LABEL` for assignment. CLI multiword state values use
hyphens; JSON/MCP state values use underscores. See [MCP schemas](MCP.md) for the
seven task tools and the `mother://tasks/{task_id}` resource.

Task lists return summaries; inspect returns the latest checkpoint and transition,
criteria, dependency IDs, and completion evidence. Checkpoint and transition history
use sequence pagination. All pages obey both item limits and an approximately 1 MiB
serialized-item budget; a short page with `next_cursor` can still have more records.

Handoff input can specify `task_id` from the same project. Its immutable snapshot
then includes that full task, revision, and latest checkpoint as `current_task`.
Snapshots also select up to 20 other unfinished tasks by latest project mutation,
with omission counts and the existing snapshot byte budget. Old packages remain
unchanged even after a task completes; query live task state when continuing.

## Remaining execution work

No worker scheduling, process leases, worktrees, provider adapters, automatic retries,
credential broker, test runner, review verifier, or human-approval workflow exists.
The next milestone is a versioned persona registry and runtime-enforced live
capabilities before independent execution or external writes are added.
