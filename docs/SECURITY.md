# Security model and current limits

This implementation trusts programs running under the same OS user. A private
0700 state directory, private files, and a 0600 Unix socket restrict access from
other ordinary users. They do not isolate a hostile coding assistant running as
the owner, and they do not protect against root or local database manipulation.
Runtime lock files, configuration, and databases reject symlinks, insecure file
modes, other owners, and hard links. Parent directories must also be trusted.

The IPC surface contains project/context/decision/handoff/task operations, worker
execution leases, persona and read-only GitHub identity records, health, and event
retrieval. File observations can read only task contract paths under a registered repository, subject to the
verifier limits. There is no general shell execution, credential broker, GitHub
mutation, or worker startup. Local IPC can manage persona and identity authorization
records under the same OS-user trust boundary. The owner-run CLI alone may invoke one
fixed `gh api user` identity-verification command.

All local clients currently have operator-level access to all registered projects.
Do not expose the socket through network forwarding or give untrusted processes
access to the state directory. MCP tools have this same local operator authority;
there is no per-host or per-human authentication.

Personas add a runtime policy gate, not a new OS trust boundary. A task create,
transition or checkpoint may assert a persona and expected revision. The runtime
checks the current revision, active status, project scope and declared capability.
Stale or revoked policy is rejected even when a host remembers older instructions.
The assertion is optional: any same-UID client can omit it and use the existing
local-operator path. Persona claims are not authenticated identity and do not
isolate a hostile process. Making them mandatory requires a separate authenticated
client model.

MCP mutations append context/decisions, explicitly supersede decisions, create
continuation packages, claim a handoff, mutate host-tracked task records, or manage
persona revisions. MCP cannot create, verify or revoke GitHub identities and cannot
acquire, heartbeat, run, release or recover worker leases.
Checkpoints and local completion perform bounded file observations. None grants
independent execution authority. Tool annotations describe
behavior and are not an authorization boundary. Persona-attributed task operations
receive current-policy checks in the runtime; other surfaces still need them.

### Worker lease boundary

Worker executions are coordination records, not a sandbox or process supervisor.
A lease prevents a second lease on the same task inside Mother's single-writer
database and bounds concurrent unexpired leases. It does not lock the repository,
stop task transitions, prove that work occurred or detect edits after expiry. Any
same-UID process can edit files or send IPC with local-operator authority.

The `mock` adapter only transitions an assigned task to `running` and writes the
same checkpoint the checkpoint operation would write. It executes no command,
reads only the task contract files, writes no repository content, runs no tests,
and cannot complete a task. `release` updates only execution metadata. Expiry
recovery returns assigned/running tasks to `ready`; other states require operator
reconciliation. Mutating worker operations are CLI-only, while MCP exposes reads.

A future worker process needs separate worktree, process, credential, budget and
reconciliation controls before it can be described as isolated execution.

### GitHub identity boundary

GitHub identity records are read-only metadata, not a credential store. The owner
creates an isolated `gh` profile under the private state directory, completes
GitHub's login, and runs `github connect`. That CLI command removes token
environment variables, invokes only `gh api --hostname HOST user`, reads the public
login/numeric ID/display name, and sends the evidence to the runtime. The profile
directory receives mode 0700; no token is copied into SQLite or returned by CLI or
MCP output.

The runtime requires an active persona with `github.read` at the expected revision
and permits one active identity per persona. Revocation is revisioned and
non-destructive. The runtime records the authorization-time persona revision and
exposes the current persona revision/status; it does not automatically revoke an
identity when a persona later changes.

This is not a security boundary against another process running as the same OS
user. Separate `GH_CONFIG_DIR` directories are not separate security principals,
and a same-UID process can send IPC requests directly. The trusted-CLI claim is
intended to prevent accidental account confusion and remote/model-supplied
identity injection, not to isolate hostile local software. No credential broker,
repository/action scope, Git push authentication, GitHub write or remote
reconciliation exists yet.

Context contents and submitted source labels are unverified data. A record named
`fact` is still a submitted claim. No submitted text becomes runtime policy. Event
actor `local_operator` denotes the OS trust boundary, not a verified human or a
particular provider. Source labels do not authenticate their author.

Do not submit tokens, passwords, or private conversation transcripts. Automatic
secret detection/redaction, encryption at rest, retention controls, exports, and
scoped deletion are not implemented. CLI arguments may appear in shell history
and process listings. Mother application errors omit request bodies and detailed
storage errors; SDK tracing is also controlled by RUST_LOG. Avoid verbose SDK
tracing for sensitive workloads.
Returned context may itself be sensitive; sending it to an external model is a
separate disclosure controlled by the caller.

Event UPDATE/DELETE triggers prevent accidental rewriting through SQL, but the
ledger is not cryptographically tamper-evident against its owner. SQLite durability
depends on the OS/filesystem honoring synchronization; no power-loss guarantee
has been tested. Use a local filesystem; network filesystem locking is unsupported.

The runtime verifies GitHub account identity only through the owner-run read-only
CLI path described above; it does not write remotes, deploy software, act as a
credential broker or handle provider credentials. Automated tests use a fake `gh`
and have not exercised a real GitHub account. Repository maintenance uses the
owner-authorized developer account separately under [publishing policy](PUBLISHING.md).

## Reporting vulnerabilities

Avoid posting sensitive exploit details, real tokens or private runtime records in
public issues. Use the repository's GitHub **Security → Report a vulnerability**
private-reporting route when available. If that option is unavailable, request a
private contact route from a maintainer without including sensitive details in the
public request. No private email address or guaranteed response time is asserted.
Keep investigation originals under the ignored `.mother-private/` directory until
reviewed for coordinated disclosure; publish sanitized guidance after resolution.


Handoff hashes cover exact package JSON bytes and detect changed bodies on normal
lookup/resume. They are not signatures and cannot prevent an OS owner from editing
both package and hash. Package and claim triggers prevent ordinary application
rewrites. Recipient routing labels and originating-host names are self-reported,
not authenticated principals. A claim coordinates continuation and does not launch
a worker. Snapshot permissions are explicitly absent and cannot grant authority.

Task owner labels are not persona/identity bindings. Persona IDs in transition
records describe which runtime policy authorized the operation, not which human
performed it. Completing a local task verifies
its declared file contract only; free-form objectives and reported tests remain
unverified. Review-required completion is blocked rather than accepting invented
review/approval claims. Task state changes do not launch or terminate processes;
the mock runner only records a checkpoint.

The file verifier anchors at the registered repository, rejects symlink traversal,
hard links, and non-regular files, and limits read sizes. It exports hashes/sizes,
not raw contents. Repository ancestry and same-UID processes remain trusted.
Observations are not atomic across files and do not stop edits after verification.
No generic shell/test-command tool was added. See [verifier details](ORCHESTRATION.md).

## Dependency maintenance exception

The owner approved a temporary exception for
[RUSTSEC-2026-0543](https://rustsec.org/advisories/RUSTSEC-2026-0543.html) through
2026-12-20. It is an INFO notice that `chrono` is unmaintained, with no patched
version. The current official `rmcp` 3.11.0 SDK and its schema support require it.
This acknowledges maintenance risk, not a waiver for a known vulnerability.
`deny.toml` scopes the exception to that advisory and expiration; other advisory
checks remain active. Reevaluate the SDK/dependency replacement before expiry.

The dependency check also reports both `syn` 2 and 3 through upstream procedural
macros. This is a dependency-duplication warning, not a vulnerability advisory;
their major-version APIs cannot be forcibly unified by this application.
