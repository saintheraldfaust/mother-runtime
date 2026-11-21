# Security model and current limits

This implementation trusts programs running under the same OS user. A private
0700 state directory, private files, and a 0600 Unix socket restrict access from
other ordinary users. They do not isolate a hostile coding assistant running as
the owner, and they do not protect against root or local database manipulation.
Runtime lock files, configuration, and databases reject symlinks, insecure file
modes, other owners, and hard links. Parent directories must also be trusted.

The IPC surface contains project/context/decision/handoff/task operations, health,
and event retrieval. File observations can read only task contract paths under a
registered repository, subject to the verifier limits. There is no shell execution, credential broker, account connection,
GitHub mutation, worker startup, or authorization-policy editing endpoint.

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
persona revisions.
Checkpoints and local completion perform bounded file observations. None grants
independent execution authority. Tool annotations describe
behavior and are not an authorization boundary. Persona-attributed task operations
receive current-policy checks in the runtime; other surfaces still need them.

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

The runtime and automated tests do not authenticate GitHub accounts, write remotes,
deploy software, or handle provider credentials. Repository maintenance uses the
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
review/approval claims. State changes do not launch workers or terminate processes.

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
