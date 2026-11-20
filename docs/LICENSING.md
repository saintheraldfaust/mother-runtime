# Licensing

The project owner selected **Apache License, Version 2.0**. Root [LICENSE](../LICENSE)
contains the complete official text, copied unchanged from
[apache.org](https://www.apache.org/licenses/LICENSE-2.0.txt). Cargo workspace metadata
declares `Apache-2.0`, and the runtime package inherits it. No copyright owner,
organization or additional ownership notice has been invented.

Third-party code retains its original licenses and notices. Dependencies are
resolved through the committed Cargo lockfile; this repository does not vendor
their source or rewrite their notices. The license check in [deny.toml](../deny.toml)
permits Apache-2.0, MIT, Unicode-3.0 and Zlib, selecting compatible alternatives
where a dependency offers multiple licenses. An offered LGPL alternative is not
selected when an allowed permissive alternative satisfies the same expression.

Run `cargo deny --locked check` before publication and after dependency changes.
The policy does not ignore workspace packages merely because `publish = false`;
source publication is still licensed. Undeclared or unapproved license expressions
must be investigated rather than broadly waived. Dependency advisory/source checks
are separate from license checks.

For future binary/source-bundle distributions, collect the original license texts
and required notices for the resolved components. This includes dependency notices
such as `cfg_aliases`' original `NOTICES.md`, build tooling where applicable, and
SQLite's original public-domain provenance. This source checkpoint does not claim
that a distributable binary notice bundle or release package is complete.

Do not remove or alter third-party notices or apply the project's ownership claims
to third-party code. A future dependency with additional license/notice obligations
requires explicit review. See [publishing policy](PUBLISHING.md).
