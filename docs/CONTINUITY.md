# Continuity semantics

Mother persists projects, context, structured decisions, handoff packages, claims,
and audit events. CLI and MCP use the same service and transactional SQLite state.
All submitted content and source labels remain unverified, including decision
rationales, completion claims, test results, Git references, and originating hosts.
Persisting a reference does not prove the artifact exists or that a test passed.

## Revisions and retry keys

Registration begins at project revision 1. Context writes, decision writes, handoff
creation, and handoff claims increment the project revision exactly once per
accepted mutation. Decision replacement also creates a supersession event within
that same revision. State, events, and the retry response commit atomically.

Supply an expected revision to reject stale writes. Resume requires one. Failed
transactions leave no new records, revision, event, or retry key. Replaying a
successful request ID with identical arguments returns its original response,
including original revisions; reusing the key with changed arguments conflicts.
Keys share one runtime-wide namespace and currently have no retention limit.
A lost response does not imply failure: persist keys before sending mutations.

## Decisions

A decision stores its UUID, project, title, decision, rationale, alternatives,
source, UTC timestamp, creation revision, and explicit supersession links. Titles
are limited to 256 UTF-8 bytes, decision/rationale to 4096 bytes each, source to
256 bytes, and alternatives to eight entries of up to 512 bytes each. Empty,
whitespace-only, and NUL-containing strings are rejected.

A new record can supersede exactly one active decision in the same project. The
replacement and predecessor update, project revision, two audit events, and retry
response share a transaction. A replaced decision cannot be superseded again.
Default decision lists and the `active_decisions` section of project context exclude
superseded records. Use `--include-superseded` or explicit ID lookup for history.

`revision` on a decision denotes its creation revision. Single-decision reads also
return `current_project_revision`, because its status may have changed. List
responses carry the current project revision. `active` means current recorded
selection, not human approval or enforced architecture policy. Unrelated active
decisions can contradict each other; conflict detection, deprecation, and explicit
human approval workflows are not implemented.

## Handoff packages

`motherctl handoff create --file FILE` accepts a typed JSON submission. See
[example input](../examples/handoff.json). CLI files and serialized submissions are
limited to 32768 bytes. Objective and next action allow 4096 bytes each; source,
originating host/session, and expected recipient allow 256 bytes. Each list allows
16 entries of up to 1024 bytes. Unknown fields are rejected.

The submission separates work completed, work remaining, changed files, Git
references, test results, blockers, unresolved questions, hypotheses, provenance,
and recommended next action. Its verification section explicitly reports that
runtime work verification, reference verification, and human approval are absent.
No caller can promote a claim into verified or human-approved state using these
input fields. Task checkpoints can separately contain runtime file observations;
these verify file bytes only and do not promote handoff summary/test claims. Originating host and recipient labels are self-reported.

Creation atomically captures a snapshot before incrementing the revision. Optional
`task_id` focuses a task from the same project; another project's task is rejected. Thus a
handoff with `snapshot_revision=N` is created at `revision=N+1`. The snapshot
contains project identity and revision, UTC capture time, and at most the latest
20 context records and latest 20 active decisions, ordered by ascending sequence
within each collection. It also reports omission counts. Superseded decisions
remain in database history and cannot become current again through a snapshot.

New snapshots use format version 2 and also include up to 20 other unfinished
tasks by recent project mutation. Each includes its task revision and latest
checkpoint; a focused task is preserved separately as `current_task`.
Snapshots are bounded to 192 KiB of compact JSON. When necessary, oldest selected
context records, then decisions, then other tasks are dropped until the budget
fits. The focused task is retained; `omitted_tasks` counts excluded other tasks. Omission counts include both earlier records and byte-budget exclusions.
Use normal paginated reads for history. Persona, authorization-policy, verified
GitHub identity and repository-base scopes remain explicitly unavailable inside
snapshots; identity metadata can be read separately. These are continuation
snapshots, not complete worker execution snapshots. Historical package bytes and
snapshot versions remain unchanged; omitting a task ID also preserves legacy
mutation serialization for retry-key comparisons.

The immutable package contains the submission, snapshot, ID, timestamps, and both
revisions. `package_json` contains the exact stored UTF-8 JSON text; `sha256` is the
SHA-256 digest of those exact bytes. `package` is its parsed representation for
convenient access. Hash verification runs on every normal handoff lookup/resume.
Hash the exact `package_json` string, not a reserialized JSON object. Packages are
limited to 512 KiB; SQL triggers prevent normal UPDATE/DELETE of packages and claims.
The hash detects body changes, not an owner who changes both body and hash.

Later project changes never rewrite a package. A read returns live
`current_project_revision`, `project_changed_since_creation`, and optional `claim`
alongside the immutable package. The changed flag compares revisions, so creating
a claim itself makes it true. Claimed packages remain historical records.

## Claiming continuation

`handoff resume` claims an unchanged, unclaimed handoff. It does not start a worker,
restore a model session, verify external artifacts, or grant permissions.

The runtime checks package integrity, absence of a prior claim, the current project
revision against both the creation revision and caller's expected revision, and any
recipient routing label. A stale package requires refreshing live context and
creating a new package. The claim, project revision increment, event, and retry
response commit together. Competing callers cannot both claim the package. Only
an exact retry with the original key replays a successful claim; another key fails,
even with the same recipient label.

Recipient labels are routing metadata. They do not authenticate a human, provider,
or persona: all clients still operate under the same local OS-user trust boundary.
There is no claim release/reassignment or execution lease yet; create a fresh
handoff for a new continuation. A snapshot cannot override future permissions.

## Retrieval and remaining work

Context, decisions, handoffs, and activity use ascending sequence cursors, default
20 items and maximum 100. `next_cursor` is null at the end. Pass a returned cursor
as `--after`. Each request is coherent within one database transaction; separate
pages are not one frozen snapshot. Lists also cap serialized item data to roughly
1 MiB per collection; follow cursors even when a page is shorter than requested. Project context includes a separate first page
of active decisions; page further with decision-list tools. Handoff lists return
metadata only, keeping bulk package reads explicit.

Process integration tests recover this state across distinct MCP clients and real
runtime restarts. Interactive Codex/Claude/Copilot sessions are not yet certified.
Task records/checkpoints and bounded file evidence are implemented. Persona,
policy and read-only GitHub identity revisions are stored, but independent
execution, verified test/review/approval evidence, full execution snapshots,
exports, retention, and backup remain unfinished. Mother does not transfer proprietary conversations or hidden
model reasoning.
