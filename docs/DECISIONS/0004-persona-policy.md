# ADR 0004: Revisioned personas with runtime capability gates

Status: adopted for the persona-registry slice.

Personas are durable configuration entities, not prompts and not workers. Store each
persona in normalized SQLite state with a unique ID, current revision, explicit
status and bounded JSON arrays for responsibilities, prohibitions and capabilities.
Write an immutable revision snapshot and a persona audit row on every material
change.

Protected task operations may include a persona assertion: persona ID plus expected
revision. The runtime, not the caller, reads the current persona and verifies active
status, project scope and the required capability before applying the existing task
revision and transition checks. A stale assertion conflicts rather than silently
using new policy. Transitions and checkpoints record the authorizing persona ID and
revision when supplied.

The assertion remains optional. All local clients still share the OS user's operator
authority and can omit it; making personas mandatory for every mutation requires a
separate authenticated client model. This ADR therefore claims enforced policy on
asserted operations, not strong same-UID isolation or authenticated persona identity.

Alternatives: embedding persona rules in prompts would not be enforceable; a flat
unversioned table would lose historical policy; a separate service would add
deployment complexity before measured need. Capability names are a closed set for
now, with reserved names for future context, decision, handoff and GitHub operations.
Worker execution, verified GitHub bindings, review independence and live
notifications remain future work.

Schema 4 adds personas, immutable persona revisions and persona audit rows, and
nullable persona attribution columns on existing task transitions/checkpoints.
Existing task rows and retry payloads remain valid.
