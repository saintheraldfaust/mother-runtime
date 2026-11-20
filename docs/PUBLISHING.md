# Publishing and continuous development policy

This is the ongoing repository-maintenance policy supplied by the owner, including
the subsequent preservation/licensing amendment. It supplements the original
engineering requirements and does not replace them. Current explicit owner
instructions take precedence.

## Private engineering directive

`MOTHER.md` is the original local master directive. Do not modify, restructure,
shorten, append to, replace, or reproduce it in public files. Preserve it locally
and keep it excluded from every public commit/push. `.gitignore` alone does not
protect tracked files: if tracked, remove only its index entry, retaining the local
file. Inspect all unpublished commits before pushing to ensure it never appears.
Block publication if it appears in outgoing history; obtain authorization before
any destructive history rewrite. Never force-push.

Keep private development materials in the ignored `.mother-private/` directory.
Audit every document by content before staging, including all files under `docs/`.
Move internal-only research, confidential plans, raw transcripts, sensitive operations
and unsanitized security investigations there while preserving their exact contents.
For mixed public/private documents, preserve the original privately and create a
separate sanitized public document. A filename or Markdown extension does not decide
classification. See [documentation classification](DOCUMENTATION.md).

Check staged/tracked documents and every unpublished commit for private materials.
If discovered in outgoing history, stop publication and report before changing
history. If already published, report the exposure and request authorization before
history remediation. Removing an index entry must retain the local copy.

`AGENTS.md` points assistants to the local directive, this policy and the factual
public `development-status.toml`. Public clones intentionally lack the private
directive; public architecture, roadmap and status documents explain supported
behavior without duplicating the private master specification.

## Destination and development identity

The owner confirmed the existing repository as
`https://github.com/saintheraldfaust/mother-runtime`. Verify the current remote,
repository name, visibility, default branch, push permission and Git author before
publication. Do not infer an owner from a local folder or create another GitHub
repository. Never overwrite an existing remote without authorization. If the
destination is absent, ambiguous or unexpected, stop the remote push and continue
safe local work while requesting the missing information.

Use the legitimate developer's configured Git identity and existing authorized
account, not a test persona or teammate. Do not switch global accounts, invent
contributors or rewrite published authorship. Inspect only the authentication
metadata needed for verification; never print credential values or credential-bearing
URLs. Do not initiate a new login without the required owner authorization.

## Public source versus private operations

Publish Rust code, Cargo manifests/lockfile, SQL migrations, public APIs, MCP/CLI/TUI
implementations, architecture/security/user/developer documentation, sanitized
examples/tests/fixtures, decisions, roadmap, public progress and benchmark methodology.
The core orchestration design is intended to be open source.

Never publish the local master directive, tokens, API/OAuth keys, cookies, SSH/signing
keys, Git/gh credential stores, real environment files, authentication profiles,
private repository context, real participant/account mappings without consent,
private runtime SQLite databases or sidecars, execution transcripts, agent sessions,
sensitive diagnostics, private notes/customer information, unresolved sensitive
security findings, crash/profiling/coverage dumps or build outputs.

Inspect content regardless of extension. JSON/TOML/SQL/Markdown may be legitimate
source or sensitive operational data. Preserve public examples and migration scripts;
use placeholders and fictional identities. Do not include personal absolute paths
or private repository URLs. Retain existing legitimate ignore entries, add specific
missing exclusions, and inspect tracked files separately from ignore rules.

## Before committing or pushing

1. Inspect `git status --short`, branch, remotes, `git log --oneline -10`, tracked
   files, unstaged/staged diff statistics, and all outgoing commits. Fetch the
   verified remote before deciding how to integrate existing work. Never overwrite
   remote history or discard unrelated local changes.
2. Manually review the intended files and unpublished history for credentials,
   private data, personal paths, account mappings, copied confidential code and
   operational artifacts. Verify the private directive is absent from both the
   index and every outgoing commit, including previous names/content.
3. Run Gitleaks (redacted output) on the exact intended/staged tree and outgoing
   history. Keep raw reports outside the repository. Investigate every finding;
   do not automatically dismiss or broadly suppress results. A clean scanner is
   not a replacement for content review. If credentials appear in history, block
   publication, arrange revocation/rotation and obtain approval for any destructive
   remediation. A deletion commit alone does not remove historical exposure.
4. Run dependency advisory and license checks using `cargo deny check`; investigate
   findings. Keep compatible-license choices explicit and preserve original
   third-party notices. Do not blindly update dependencies to make a scanner green.
5. Run `cargo fmt --all -- --check`, `cargo test --workspace --locked`, and
   `cargo clippy --workspace --all-targets --locked -- -D warnings`. Add relevant
   integration checks for changed behavior. Do not disable tests or claim absent
   external-provider checks passed. Fix failures before calling a checkpoint stable.
6. Stage only reviewed source paths, then inspect `git diff --cached`,
   `git diff --cached --check` and the staged file inventory. Confirm excluded
   content is not tracked; never rely on a blind whole-workspace add.
7. Commit a coherent, verified milestone with a conventional-style message, such
   as `feat(runtime): ...`, `feat(personas): ...`, `fix(tasks): ...` or
   `chore(repo): ...`. Avoid one commit per edit, vague messages and fabricated
   historical milestones. Accumulated coherent work can be one initial checkpoint.
8. Push only to the confirmed existing repository using the established branch
   workflow. Respect protection rules and use an appropriate development branch
   if needed. No force pushes, unsolicited history rewrites or final merges.
   The owner authorized this initial safe publication and subsequent stable,
   checked development milestones. Unresolved destination/security concerns block
   the push; never claim success before Git confirms it.

If no history exists, history checks should explicitly report an empty initial
history rather than claiming an existing history was scanned. After creating a
commit, scan/check its reachable tree/history again before publication.

## Continuous checkpoints and public documentation

Commit and push coherent tested milestones, including a safe checkpoint before
major refactors. Avoid excessive pushes/API activity and misleading activity counts.
An explicitly labeled local WIP checkpoint is acceptable when useful; incomplete
or unsafe work must not be presented or published as stable. Preserve completed
implementation and resume the existing roadmap after publication.

Keep README, architecture, continuity, identities, MCP, orchestration, personas,
security, testing, roadmap and development-status documentation factual and sanitized.
Clearly distinguish implemented, experimental, planned and unsupported behavior.
The private directive itself remains untouched. Public security guidance should
offer a private disclosure route without exposing active exploit details.

Update `development-status.toml` with implemented features, verified tests, limits,
outstanding tasks, decisions, blockers and next objective. Store no transcripts,
credentials or transient execution/debug logs there. Publication facts should be
verified; do not embed personal author email, local absolute paths or raw scan reports.

## Licensing and reporting

The owner selected **Apache License 2.0**. Keep the complete official unmodified
text in root `LICENSE`, declare `Apache-2.0` in Cargo metadata and public docs, and
check new code/dependency licensing. Do not invent ownership attribution or alter
third-party notices. Report any conflicting existing license before changing it.

At each publication milestone report the repository/branch, confirmed commit hash,
commit messages, included/excluded categories, actual scan/test results, documentation
changes, unresolved warnings and next objective. Avoid credentials, unnecessary
personal information and sensitive paths. Publication does not establish production
readiness, credential isolation, review approval or worker execution support.
