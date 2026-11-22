# GitHub identity status

Mother now records verified, owner-authorized GitHub account identities for a
**read-only** boundary. It does not store OAuth tokens, expose credentials to
agents, invoke arbitrary GitHub commands, or perform GitHub writes.

## Connect a profile

Create a dedicated GitHub CLI profile directory:

```sh
motherctl --data-dir /path/to/state github profile --host github.com
```

The result includes a canonical `profile_id`, a private `config_dir`, and the next
action. The directory is created under the Mother state directory with mode 0700.
It is deliberately separate from the operator's global `~/.config/gh` profile.

The legitimate account owner completes GitHub's own login in that directory:

```sh
GH_CONFIG_DIR=/path/to/state/github/profiles/PROFILE_ID \
  gh auth login --hostname github.com
```

Then the owner verifies the actual account and records its authorization:

```sh
motherctl --data-dir /path/to/state github connect \
  --profile PROFILE_ID \
  --host github.com \
  --persona PERSONA_ID \
  --expected-persona-revision N
```

`github connect` runs only `gh api --hostname HOST user` with that profile. It
removes `GH_TOKEN`, `GITHUB_TOKEN`, enterprise token variables and `GH_HOST` from
the child environment so the isolated profile is authoritative. It reads the
public login, numeric account ID and display name from GitHub. Stderr is not
surfaced because it can contain local authentication details.

The runtime then requires an active persona at the expected revision with the
`github.read` capability. A persona can have at most one active verified identity.
The account must not already be actively authorized.

## Stored records

Schema 5 stores:

- Host, verified numeric account ID, login and optional display name.
- The isolated profile ID, not its path and not any token.
- The bound persona ID and authorization-time persona revision.
- Supported operations, currently only `read`.
- Active/revoked status, record revision, verification timestamp and audit rows.
- Immutable authorization/revocation revision snapshots and events.

Reads expose verification metadata and current persona status/revision. They never
return credential values or the profile path.

## Revocation and history

```sh
motherctl --data-dir /path/to/state github revoke \
  --identity IDENTITY_ID --expected-revision N
```

Revocation blocks future authorized use and records an immutable revision. It does
not delete history, delete the isolated `gh` profile, or rewrite any GitHub data.
To remove local credentials as well, the owner must run `gh auth logout` with that
profile's `GH_CONFIG_DIR`. No worker execution exists yet to pause or cancel. Re-running `github connect` with the
same verified account after owner reauthorization creates a later active revision;
past revisions remain readable.

## MCP exposure

MCP exposes only read-only identity metadata:

- `mother_github_identities_list`
- `mother_github_identity_get`
- `mother_github_identity_history`
- `mother://github-identities/{identity_id}`

There is no MCP tool to create, verify, reauthorize or revoke identities. Those are
owner-run CLI actions.

## Limits

- **Same OS user:** profiles are separate directories, not separate security
  principals. Same-UID processes can potentially access each other's profile
  files. This is not a sandbox for hostile local software.
- **Local trust:** the canonical runtime records evidence produced by the trusted
  CLI. It does not independently re-run `gh` during every read. A same-UID process
  can send IPC requests directly; protecting against that requires OS-level
  isolation or a stronger authenticated client model.
- **Read-only:** no credential broker, push/PR/issue/review operation, repository
  scope check or write authorization exists yet. Revocation does not erase local
  `gh` credential files; use `gh auth logout` for that separate action.
- **Persona policy drift:** an identity records the persona revision used when it
  was authorized. Current persona status and revision are exposed, but the identity
  is not automatically revoked if the persona later changes. Future brokers must
  check the current revision.
- **Human approval:** the local CLI operator is trusted, not biometrically or
  remotely authenticated. The record does not prove which human clicked confirm.
