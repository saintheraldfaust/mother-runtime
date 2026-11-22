# ADR 0003: Host-tracked tasks with bounded file evidence

Status: adopted for the task/checkpoint continuity slice.

Persist tasks independently from worker execution. Explicit transitions, immutable
checkpoints/history, same-project dependency edges, and separate task/project
revisions provide resumable coordination without claiming a provider process exists.
Require task revision checks on every update. Keep dependency edges fixed at creation,
referencing only existing tasks, so normal API operations cannot construct a cycle.

The first verifier handles declared repository-file contracts only. The runtime
reads bounded regular files, captures SHA-256 observations, and rechecks them before
local completion. Missing files and mismatched expectations remain durable checkpoint
evidence. A fresh validating checkpoint and unchanged passing observations are
required for completed; review-required tasks remain blocked pending a real verifier.
No test result text or free-form objective becomes proof of behavior.

Use descriptor-relative openat traversal with O_NOFOLLOW, bounded reads, metadata
checks, and a second path resolution. Reject symlinks, hard links, and non-regular
files. This reduces path confusion without claiming same-UID sandboxing or atomic
filesystem snapshots. Do not expose generic commands as a convenient verifier.

Handoff snapshot version 2 adds task state and checkpoints, while preserving old
packages exactly. A new optional task ID is omitted from serialized inputs when
absent, preserving existing retry-key payload comparisons. Snapshot byte budgets
prefer the explicitly focused task; old context, decisions, then other tasks may
be dropped with omission counts. All list pages also cap serialized item bytes.

Alternatives: accepting a model's completed flag would lose evidence; executing
arbitrary test strings would create an unrestricted shell boundary; pretending an
owner label is a persona would introduce authority confusion. General tests,
independent review, supervised worker processes, and mandatory persona policies remain separate work.

Schema migration 3 adds tasks, dependency edges, immutable checkpoints, and immutable
transition history. Task state, evidence, revision changes, events, and retry results
commit together. Existing handoff JSON and retry responses remain unchanged.
