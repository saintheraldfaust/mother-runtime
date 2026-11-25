# Mother Runtime

**By TFRI**

*A persistent engineering control plane for AI coding agents.*

Mother Runtime is an open-source, Rust-native orchestration system designed to coordinate engineering work across AI coding assistants, agent personas, and independently authorized GitHub identities.

Its goal is to make AI-assisted software development continuous, governable, and accountable—even when developers switch between Codex, Claude Code, GitHub Copilot, or other supported coding tools.

**The project remembers. The agents execute.**

## Try Mother with Codex

```sh
cargo build --locked
./target/debug/motherctl setup-codex
./target/debug/motherctl ui
```

Use your existing Codex login. Reload the Codex VS Code session in this trusted
project after setup. The live TUI shows real tasks, workers, file observations and
activity; Tab changes views and q exits. See [Codex setup](docs/CODEX.md) and
[TUI controls](docs/TUI.md). OpenCode is optional.

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

- A canonical local SQLite database owned by a Rust service, with foreground and explicit background startup.
- Shared service access from independent CLI processes through a Unix socket.
- Project registration and persistent context records.
- Structured engineering decisions with supersession.
- Hashed handoff packages and one-time continuation claims.
- Durable host-tracked tasks and checkpoints.
- Dependency-aware worker execution leases with heartbeat, bounded concurrency and crash recovery.
- A deterministic mock worker adapter that records checkpoints without editing files or auto-completing work.
- Owner-run supervised mock processes with isolated Git worktrees, bounded logs, artifact evidence, and explicit abandoned-supervisor reconciliation.
- A Codex CLI worker adapter using the existing host login, workspace-write sandbox, bounded usage metadata, timeout/step limits and retained file evidence.
- A read-only Ratatui dashboard and project-local Codex MCP setup.
- An optional OpenCode CLI provider adapter under the same owner-run supervisor, using the owner's host login, with explicit model, cost, step and timeout limits, bounded telemetry, and worktree observations. It is exercised with a fake `opencode` executable; no real paid model call is claimed.
- Runtime file evidence.
- Revision checks and idempotent writes.
- Runtime diagnostics.
- Versioned personas with immutable history and runtime capability checks for asserted task operations.
- Read-only verified GitHub identity records with isolated CLI profiles, persona binding, revocation, and no stored tokens.
- An MCP stdio bridge implemented using the official Rust SDK.

Two MCP protocol lifecycles have been tested using SDK clients. Persona policy updates have been exercised across independent CLI/MCP clients. GitHub identity verification uses a fake `gh` in tests; the OpenCode provider adapter uses a fake `opencode` executable. A real Codex worker using a ChatGPT login produced a matching declared artifact, and a separate real Codex CLI client successfully read Mother through MCP. No real GitHub account or interactive VS Code session is certified.

Interactive coding-assistant sessions and cross-provider continuation have not yet been certified end to end.

## Planned orchestration capabilities

Mother's next engineering milestones include:

**Persistent personas:** Expand the verified registry into mandatory authenticated adoption, execution policies, and live client notifications.

**Cross-assistant continuity:** Verified retrieval and continuation of engineering state across supported coding clients.

**GitHub identities:** Expand read-only verified account records into repository/action scopes, a credential broker, write authorization, and reconciliation without exposing tokens to agents.

**Independent workers:** Expand the Codex and optional OpenCode adapters and verified worktree boundary into certified provider workflows, stronger isolation, test execution and independent evidence reconciliation.

**Governed engineering workflows:** Assignment, implementation, validation, independent review, and human-controlled merging through GitHub.

**Terminal interface:** Extend the implemented read-only dashboard with governed task and workflow actions.

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

The current foundation should not be interpreted as proof that certified provider-backed independent workers, account-isolated GitHub automation, or end-to-end multi-agent review workflows are already available. The OpenCode adapter is implemented but has not been certified with a real model call.

See the development status, architecture, security documentation, and roadmap for verified capabilities and ongoing work.

## Open source

Mother Runtime is developed by **TFRI**.

License: Apache-2.0, subject to the repository containing the corresponding license and package metadata.
