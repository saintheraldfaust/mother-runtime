# 0005: Read-only GitHub identity records

Status: Accepted.

## Context

The roadmap requires real, distinct GitHub accounts rather than one shared bot
identity. It also requires a trusted operation boundary and forbids exposing raw
tokens to agents. Building write operations or a credential broker before account
verification and revocation exist would create unreviewable authority.

## Decision

Add schema 5 identity records with immutable revisions and events. An owner creates
an isolated `gh` profile under the Mother state directory, completes GitHub's own
login, and runs a CLI-only `github connect` command. That command performs one
read-only `gh api user` call with token environment variables removed, then sends
the resulting public account evidence to the runtime.

The runtime accepts an identity only when an active persona at the expected
revision declares `github.read`. It permits one active verified identity per
persona and records authorization-time persona revision. MCP may read identity
metadata but cannot create or revoke identities. Revocation is revisioned and
non-destructive.

No token, credential value or profile path is stored in output or exposed over MCP.

## Consequences

- Account verification and revocation are testable without GitHub writes.
- A later broker can require current persona revision, identity status, operation
  and repository/task checks before invoking GitHub.
- The boundary is limited by the single-OS-user trust domain. Separate directories
  are not separate security principals.
- Write operations, repository/action scopes, remote-state reconciliation and real
  account certification remain unimplemented and must not be claimed.
