# Persona registry

Personas are durable engineering identities stored by the runtime. They are not
model prompts, host assistants or worker processes. A persona is separate from the
provider that might eventually execute its work.

The current implementation provides a versioned registry, runtime capability
checks for persona-attributed task operations, and read-only verified GitHub
identity records bound to personas. It does not dispatch workers, expose GitHub
credentials or prove that a host assistant followed persona instructions.

## Data model

Each persona stores:

- Stable UUID and unique human-readable name.
- Role identifier, description, responsibilities and explicit prohibitions.
- Operational capabilities from the supported set.
- Optional project scope.
- Preferred coding-agent runtime label.
- Self-reported GitHub identity label, explicitly marked unverified; an owner-run
  CLI flow can separately bind a verified read-only identity record.
- Context retrieval description.
- Review requirement, concurrency limit and optional budget.
- Status (`active`, `disabled`, `archived`), revision and timestamps.

Supported capabilities:

| Capability | Protected operation |
| --- | --- |
| `context.record` | Reserved; not enforced on context writes yet. |
| `decision.record` | Reserved; not enforced on decision writes yet. |
| `handoff.create` | Reserved; not enforced on handoff creation yet. |
| `handoff.claim` | Reserved; not enforced on handoff claims yet. |
| `task.create` | Persona-attributed task creation. |
| `task.transition` | Persona-attributed task state transitions. |
| `task.checkpoint` | Persona-attributed task checkpoints. |
| `persona.manage` | Reserved for future management authorization. |
| `github.read` | Required to bind a verified read-only GitHub identity; future read operations. |
| `github.write` | Reserved; GitHub writes are not implemented. |

A declared capability is necessary but not sufficient for a protected operation.
The operation's existing revision, state and evidence checks still apply.

## CLI

```sh
motherctl personas create --file examples/persona-implementation.json
motherctl personas list
motherctl personas inspect --persona PERSONA_ID
motherctl personas edit --persona PERSONA_ID --expected-revision N --file persona.json
motherctl personas disable --persona PERSONA_ID --expected-revision N
motherctl personas enable --persona PERSONA_ID --expected-revision N
motherctl personas archive --persona PERSONA_ID --expected-revision N
motherctl personas history --persona PERSONA_ID
```

Create and edit accept a bounded JSON definition. Replace `PROJECT_ID` in the
examples before use. `archive` is the safe deletion path: archived personas and their
revisions remain readable and cannot be reactivated.

## Live policy enforcement

A persona-attributed task mutation supplies both `persona_id` and
`expected_revision`. The runtime reads the current persona and rejects the operation
when:

- The expected revision differs from the current revision.
- The persona is disabled or archived.
- The persona is scoped to a different project.
- The required capability is not present.

Changing a persona increments its revision and writes an immutable revision snapshot
plus an audit row. A client that remembers an older revision receives a conflict; it
must refresh through `personas inspect` or the MCP resource. A client that supplies
the new revision is checked against the new capability set.

Task transitions and checkpoints record the authorizing persona ID and revision when
one was supplied. Those fields are `null` for the local-operator path.

## Security boundary

The assertion is not authenticated as a particular human or host. All local clients
still operate under the same OS user and may omit the persona assertion. The current
gate therefore enforces declared persona policy on asserted operations; it does not
yet make a persona mandatory for every mutation, and it does not isolate a hostile
same-UID process. See [security model](SECURITY.md).

## Planned work

Worker execution, GitHub write authorization and brokering, independent review
enforcement and cross-assistant persona continuation remain future milestones.
Verified read-only identity binding is documented in [GitHub identities](IDENTITIES.md).
