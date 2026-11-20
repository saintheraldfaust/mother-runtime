# Mother Runtime

**By TFRI**

*A persistent engineering control plane for AI coding agents.*

Mother Runtime is an open-source, Rust-native orchestration system designed to coordinate engineering work across AI coding assistants, agent personas, and independently authorized GitHub identities.

Its goal is to make AI-assisted software development continuous, governable, and accountable—even when developers switch between Codex, Claude Code, GitHub Copilot, or other supported coding tools.

**The project remembers. The agents execute.**

## Why Mother exists

Coding assistants are typically organized around individual sessions. Projects, however, continue across sessions, tools, contributors, and engineering decisions.

Mother is designed to provide a durable coordination layer that preserves:

- Project state, technical context, and architectural decisions.
- Engineering responsibilities and versioned agent configurations.
- Task ownership, dependencies, checkpoints, and execution history.
- Verified deliverables, GitHub artifacts, and review outcomes.
- Context needed to continue work through a different coding assistant.

Mother is not intended to replace existing coding agents or become another general-purpose conversational memory tool.

Its long-term purpose is to coordinate what agents are responsible for, what they are authorized to do, what work has been verified, and what should happen next.

## Current development status

**Early development — persistent runtime foundation implemented; governed agent orchestration remains in development.**

The current implementation includes:

- A canonical local SQLite database owned by a foreground Rust service.
- Shared service access from independent CLI processes through a Unix socket.
- Project registration and persistent context records.
- Structured engineering decisions with supersession.
- Hashed handoff packages and one-time continuation claims.
- Durable host-tracked tasks and checkpoints.
- Runtime file evidence.
- Revision checks and idempotent writes.
- Runtime diagnostics.
- An MCP stdio bridge implemented using the official Rust SDK.

Two MCP protocol lifecycles have been tested using SDK clients.

Interactive coding-assistant sessions and cross-provider continuation have not yet been certified end to end.

## Planned orchestration capabilities

Mother's next engineering milestones include:

**Persistent personas:** Configurable engineering roles with versioned responsibilities, permissions, instructions, and execution policies.

**Cross-assistant continuity:** Verified retrieval and continuation of engineering state across supported coding clients.

**GitHub identities:** Separate GitHub user accounts, explicitly authorized by their legitimate owners and associated with configured engineering personas.

**Independent workers:** Specialized coding agents operating through supported provider runtimes and isolated execution environments.

**Governed engineering workflows:** Assignment, implementation, validation, independent review, and human-controlled merging through GitHub.

**Terminal interface:** A Rust-native CLI and interactive TUI for managing personas, projects, tasks, accounts, and execution history.

## Architectural principles

- **Provider-independent:** Coding assistants are interchangeable clients, not owners of canonical project state.
- **Local-first:** Project coordination state is controlled by the local runtime.
- **Persistent:** Tasks, decisions, and handoffs survive ordinary session termination.
- **Policy-governed:** Operational permissions must be enforced by the runtime, not merely described in prompts.
- **Identity-aware:** Automated GitHub actions must be attributable to the correctly authorized account.
- **Evidence-based:** Completion claims should be supported by verifiable artifacts.
- **Rust-native:** Core execution, persistence, orchestration, and terminal interfaces are implemented in Rust.

## Project maturity

Mother Runtime is under active development and is not yet a production-ready autonomous engineering orchestrator.

The current foundation should not be interpreted as proof that independent workers, account-isolated GitHub automation, or end-to-end multi-agent review workflows are already available.

See the development status, architecture, security documentation, and roadmap for verified capabilities and ongoing work.

## Open source

Mother Runtime is developed by **TFRI**.

License: Apache-2.0, subject to the repository containing the corresponding license and package metadata.
