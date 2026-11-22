# ADR 0002: Explicit decision supersession and immutable continuation packages

Status: adopted for the continuity slice.

Represent decisions separately from free-form context. An explicit replacement
references one active decision in the same project; the predecessor retains its
original contents and receives a superseded-by link. Defaults expose active
records, while history remains queryable. All writes use existing project
revisions, transactional events, and runtime-wide idempotency keys.

A handoff combines an explicitly submitted summary with a bounded snapshot of
actual persisted project/context/decision records. The runtime distinguishes
record provenance from unverified content and never treats supplied test results,
completion text, or Git references as verified evidence. Missing task, persona,
policy, and repository-base scopes are explicit.

Keep exact serialized package bytes immutable and hash them with SHA-256. Return
both those bytes and parsed JSON so clients can verify the digest without assuming
cross-language canonicalization. Limit the snapshot and package sizes. Keep live
project revision and continuation claims outside the hashed package so later
coordination cannot rewrite history.

Resume is a one-time durable coordination claim, not execution. Require that the
project has not changed since package creation, plus the caller's expected
revision. Serialize the claim with project revision and audit state. An identical
request retry replays its result; another claimant fails. Recipient strings route
work but are not identities or permissions. New packages represent reassignment;
handoff execution leases remain future task-engine work.

Alternatives: a mutable summary would lose the original continuation boundary;
full-history snapshots would grow without bounds; automatic natural-language
supersession could silently discard knowledge; optimistic claims without a unique
handoff key could admit duplicate continuations. These choices favor inspectable,
conservative behavior while the task and authorization engines are absent.

Schema migration 2 adds decisions, immutable handoffs, and immutable claim records.
All pending migrations run in one transaction; a failure preserves the old schema
and data. No external authentication or remote writes are involved.
