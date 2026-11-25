# Security model and current limits

This implementation trusts programs running under the same OS user. A private
0700 state directory, private files, and a 0600 Unix socket restrict access from
other ordinary users. They do not isolate a hostile coding assistant running as
the owner, and they do not protect against root or local database manipulation.
Runtime lock files, configuration, and databases reject symlinks, insecure file
modes, other owners, and hard links. Parent directories must also be trusted.

The IPC surface contains project/context/decision/handoff/task operations, worker
execution leases/workspaces/processes, persona and read-only GitHub identity records,
health, and event retrieval. File observations can read only task contract paths
under a registered repository, subject to the verifier limits. There is no general
shell execution, credential broker, GitHub mutation, or arbitrary worker startup.
Local IPC can manage persona and identity authorization records under the same
OS-user trust boundary. The owner-run CLI alone may invoke one fixed `gh api user`
identity-verification command and supervise the mock helper, Codex, or optional OpenCode CLI adapter.

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
acquire, heartbeat, mock-run, superintend, clean up, release or recover worker
executions. Checkpoints and local completion perform bounded file observations.
None grants independent execution authority. Tool annotations describe behavior and
are not an authorization boundary. Persona-attributed task operations receive
current-policy checks in the runtime; other surfaces still need them.

### Worker lease and process boundary

Worker executions are coordination records. A lease prevents a second lease on the
same task inside Mother's single-writer database and bounds concurrent unexpired
leases. It does not lock the repository, stop task transitions, prove that work
occurred or detect edits after expiry. Any same-UID process can edit files or send
IPC with local-operator authority.

The `mock` checkpoint adapter only transitions an assigned task to `running` and
writes the same checkpoint the checkpoint operation would write. It executes no
command, reads only the task contract files, writes no repository content, runs no
tests, and cannot complete a task. `release` updates only execution metadata.

`workers run` is an owner-run supervisor, not a runtime service or sandbox. It
verifies that the task can be leased, resolves the registered repository's current
commit, and creates a detached Git worktree inside the private worker directory.
Git commands disable hooks and fsmonitor and set `GIT_TERMINAL_PROMPT=0` with a
failing askpass. The selected adapter runs in its own process group with inherited
OS-user authority. The mock helper receives a cleared environment and a worktree
`HOME`; it can write only the deterministic `worker-output.txt` by design, but this
is not enforced filesystem isolation. The worktree prevents concurrent-checkout
collisions, not hostile access.

For mock/OpenCode workers the supervisor captures at most 64 KiB of each output stream into a 0600 log. Codex output is discarded; only accounting/completion metadata is retained. It attaches
the PID, heartbeats the lease, sends SIGTERM/SIGKILL to the process group on timeout
or budget exhaustion, and records bounded observations of declared acceptance
criteria. Successful exit moves a running task to `awaiting_input`; failure, timeout
or budget exhaustion moves it to `failed`. No path completes a task. Worktrees are
retained; `workers cleanup` removes only an inactive execution's worktree and prunes
Git metadata. If a lease expires while a process may still run, recovery marks the
task `blocked` instead of requeueing it.

The OpenCode adapter runs `opencode run --format json --pure --auto`
with a required model, positive provider-reported cost ceiling, step ceiling and
timeout. It uses the owner's existing OpenCode login, an isolated OpenCode config
under the private execution directory, project config disabled, and an environment
allowlist rather than a full environment copy. The host OpenCode data store remains
available because 1.19.6 keeps credentials there, so provider sessions can persist
outside Mother. Mother does not copy, store, return or broker provider credentials.

`--auto` authorizes provider tool execution without interactive approval. The
provider process still has the owner's OS-user authority: it may access the network,
files outside the worktree, SSH/GitHub credentials in the host home, and any tool the
provider exposes. The task prompt is untrusted input, not an authorization grant.
The worktree is not a sandbox. Provider-reported cost/token telemetry is parsed into
bounded metadata and raw output into a bounded log; hidden reasoning is not stored.
Cost is not independently metered, so the ceiling is not a billing guarantee and a
provider can underreport or delay events. Provider text and token counts are not
completion evidence.

If the supervisor is killed, the owner must reconcile the stale process explicitly.
`workers abandon` checks PID existence and refuses without `--force`, then records
the process as `lost` without sending a signal, blocks the task, and permits cleanup.
This prevents an unrecoverable running row but does not prove the OS process is dead
or prevent PID reuse. It is not an automatic reaper.

Expiry recovery may still miscategorize late work, and process PIDs are not
authenticated worker identity. Mutating worker operations are CLI-only, while MCP
exposes reads. Provider credentials are host-managed and not brokered; sandbox
isolation, independent spend metering, test execution, automatic retry, and
independent evidence reconciliation remain unimplemented.

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
performed it. Completing a local task verifies its declared file contract only;
free-form objectives and reported tests remain unverified. Review-required
completion is blocked rather than accepting invented review/approval claims. Task
state changes do not launch or terminate processes; only the separate owner-run
supervisor does, and the checkpoint mock runner only records a checkpoint.

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

### Codex and dashboard boundary

Codex workers request `workspace-write` sandboxing with approval policy `never`,
ignore user configuration and receive no configured MCP servers. Host HOME and
optional CODEX_HOME remain available for existing authentication. API-key and
GitHub-token environment variables are excluded, but this is not account isolation
or credential brokering. The installed Codex sandbox enforces its own policy;
Mother's same-UID trust boundary and worktree limits remain. Event-based step limits
and reported usage cannot establish independent billing or preauthorization of each
operation. Subscription dollar cost is null. Raw Codex messages, reasoning, command
output and stderr are not retained by Mother. See [Codex details](CODEX.md).

The first TUI is read-only, uses IPC and removes terminal control characters from
submitted text. Setup edits only project-local MCP configuration and registers the
project through IPC. Generated configuration, runtime state, logs and retained
worktrees remain private. Quitting the dashboard leaves the service running.
