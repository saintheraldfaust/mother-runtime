# Foundational technology verification

Checked official documentation and the Cargo registry on 2026-11-20 before writing
the foundational runtime. Exact resolved dependencies are in `Cargo.lock`.

- [Clap](https://docs.rs/clap/latest/clap/): 4.6.8; derive-based CLI grammar.
- [Tokio UnixListener](https://docs.rs/tokio/latest/tokio/net/struct.UnixListener.html):
  1.54.1; Unix socket transport, no externally exposed network listener.
- [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/): 0.43.0; bundled SQLite,
  transactions and parameterized queries. SQLite integers are signed 64-bit;
  externally unsigned pagination cursors are checked before conversion.
- [SQLite WAL](https://www.sqlite.org/wal.html): local persistence with a serialized
  writer; network filesystems are outside this implementation's support boundary.
- [SQLite pragmas](https://www.sqlite.org/pragma.html): foreign keys,
  `user_version`, synchronous mode, integrity and foreign-key checks.
- [fs2 file locking](https://docs.rs/fs2/latest/fs2/trait.FileExt.html): 0.4.3;
  exclusive lock held on one persistent lockfile, never unlinked on shutdown.
- [MCP official versioning source](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/versioning.mdx):
  2026-07-28 uses a different lifecycle from older initialize-based clients.
- [Official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk): registry
  release rmcp 3.11.0, inspected downloaded source and official server/client
  examples before implementation. The bridge uses only server/stdio features in
  production; child-process client support is a dev dependency. Both 2025-11-25
  and 2026-07-28 lifecycles pass real process integration tests.
- Host setup schemas were checked against official Codex, Claude Code, and VS Code
  references, plus the installed Codex CLI help. Links and configuration scope
  details are maintained in [MCP documentation](MCP.md). No settings were changed
  and no interactive host session is claimed.

For the worker provider slice, inspected the installed OpenCode 1.19.6
`opencode run --help` and source tag `v1.19.6` from
[anomalyco/opencode](https://github.com/anomalyco/opencode). The installed CLI does
not accept `--standalone`; its non-interactive `run` path uses an in-process server.
`run --format json` emits newline-delimited events with a top-level `sessionID`, and
step-finish parts carry provider-reported cost and tokens. Version 1.19.6 keeps
credentials in its data database, so this adapter retains host `HOME` and the host
data store for an existing login while isolating OpenCode configuration. Provider
tests use a fake executable; no authenticated or paid provider call is claimed.

No other authentication or provider API is called by the implemented feature set.
Research those interfaces immediately before the relevant implementation rather
than making unverified integration claims now.


For the continuity slice, verified [SQLite transaction behavior](https://www.sqlite.org/lang_transaction.html)
and [composite foreign keys](https://www.sqlite.org/foreignkeys.html) before implementing
same-project supersession and transactional migration/claim state. Added
[RustCrypto sha2](https://docs.rs/sha2/latest/sha2/) 0.11.1 as a direct dependency for
SHA-256 of exact stored handoff bytes. Digest bytes are encoded explicitly as
lowercase hexadecimal. No new authentication or provider API is introduced.


For task evidence, verified Rust's [Unix OpenOptionsExt](https://doc.rust-lang.org/std/os/unix/fs/trait.OpenOptionsExt.html)
and the [open/openat descriptor and flag semantics](https://man7.org/linux/man-pages/man2/openat.2.html).
Implementation uses existing libc and SHA-256 dependencies, descriptor-relative
O_NOFOLLOW traversal, regular-file checks, bounded reads, before/after metadata,
and a final path recheck. macOS behavior is exercised by actual filesystem tests;
Linux is requested in CI but has not been run locally. Observations are not an
atomic filesystem snapshot and no shell/test/provider API is introduced.
