# Public and private documentation

Every document below was inspected by content for this initial publication. They
contain public interfaces, architecture, supported behavior, source bibliography,
policy or sanitized development status. None contains private master-directive
content, authentication data, raw transcripts, real participant mappings, private
customer context or an unresolved sensitive investigation. No existing document
required a private move at this review; public research is classified by its
external-source content, not automatically by its filename.

| Reviewed document | Classification and reason |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Public — Public component boundaries, storage and IPC behavior; no operational records. |
| [CLI.md](CLI.md) | Public — Public command grammar and placeholder identifiers. |
| [CONTINUITY.md](CONTINUITY.md) | Public — Public revision, snapshot and handoff semantics; no real project memory. |
| [DECISIONS/0001-local-runtime.md](DECISIONS/0001-local-runtime.md) | Public — Public architecture decision and tradeoffs. |
| [DECISIONS/0002-continuity-packages.md](DECISIONS/0002-continuity-packages.md) | Public — Public data-model decision; no private session contents. |
| [DECISIONS/0003-host-tracked-tasks.md](DECISIONS/0003-host-tracked-tasks.md) | Public — Public task/verifier architecture and supported limitations. |
| [DECISIONS/0004-persona-policy.md](DECISIONS/0004-persona-policy.md) | Public — Public persona/revision/capability architecture and remaining limits. |
| [DECISIONS/0005-github-identity-boundary.md](DECISIONS/0005-github-identity-boundary.md) | Public — Public read-only identity verification boundary and limits. |
| [DECISIONS/0006-worker-lease-boundary.md](DECISIONS/0006-worker-lease-boundary.md) | Public — Public worker lease, mock checkpoint and recovery boundary; no operational data. |
| [DECISIONS/0007-worker-process-boundary.md](DECISIONS/0007-worker-process-boundary.md) | Public — Public owner-run process supervision, worktree retention and evidence boundary; no operational data. |
| [DECISIONS/0008-abandoned-process-reconciliation.md](DECISIONS/0008-abandoned-process-reconciliation.md) | Public — Public abandoned-supervisor reconciliation, PID safety and lost-process cleanup boundary; no operational data. |
| [DECISIONS/0009-opencode-provider-adapter.md](DECISIONS/0009-opencode-provider-adapter.md) | Public — Public OpenCode CLI adapter, budget, telemetry, credential and sandbox limits; no operational data. |
| [DOCUMENTATION.md](DOCUMENTATION.md) | Public — Public classification inventory and contributor guidance. |
| [IDENTITIES.md](IDENTITIES.md) | Public — Public read-only identity setup, stored metadata, revocation and limits. |
| [LICENSING.md](LICENSING.md) | Public — Public license provenance and third-party notice obligations. |
| [MCP.md](MCP.md) | Public — Public schemas and opt-in placeholder host configurations. |
| [ORCHESTRATION.md](ORCHESTRATION.md) | Public — Public state machine, evidence scope and sanitized examples. |
| [PERSONAS.md](PERSONAS.md) | Public — Public persona registry, capabilities and enforcement limits; no real account mappings. |
| [PUBLISHING.md](PUBLISHING.md) | Public — Public repository maintenance policy; no credentials or private master content. |
| [RESEARCH.md](RESEARCH.md) | Public — Public official-source bibliography and technology verification; no internal-only research or transcripts. |
| [ROADMAP.md](ROADMAP.md) | Public — Public implementation milestones and known gaps. |
| [SECURITY.md](SECURITY.md) | Public — Public trust model, limitations and disclosure guidance; no unresolved private exploit report. |
| [TESTING.md](TESTING.md) | Public — Public test methodology and aggregate outcomes; no raw logs or participant records. |

`MOTHER.md` remains private, unmodified and local at the repository root.
`.mother-private/` holds private development materials and detailed publication
audit records. Both are ignored and must never enter public Git history. Public
contributors can use README, the documents above, AGENTS.md and the sanitized
development-status file without access to the original private directive.

Reinspect changed and new documents before each publication. Preserve confidential
originals privately; create a separate public-facing version when content is mixed.
Do not automatically publish Markdown, move files by name, or treat ignore patterns
as proof that history is clean. Classifications can change as contents change.
Use the publication guard, redacted secret scanning and manual review together.
