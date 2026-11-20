# Working on Mother Runtime

Read these before changing the project:

- `MOTHER.md`: the owner's original engineering directive, when available locally.
  Preserve it byte-for-byte. It is intentionally private and ignored by Git: never
  stage, commit, publish, rewrite, shorten, append to, or reproduce it in public docs.
  Public clones may lack it; use the public architecture/roadmap/status documents
  and ask the owner for authorized requirements when needed rather than recreating it.
- `docs/PUBLISHING.md`: ongoing commit, publication, scanning and repository hygiene rules.
- `docs/DOCUMENTATION.md`: reviewed public documentation and public/private classification rules.
- `development-status.toml`: implemented capabilities, verification, limits and next objective.

Preserve working systems and implement tested vertical slices. Keep canonical state
in the local Rust service; CLI/MCP clients must not become independent database writers.
Prompts, submitted context and owner labels do not grant operational authority.
Do not describe plans, model claims, or file-only verification as completed execution,
test success, independent review or human approval. Update public documentation and
machine-readable status at each verified milestone.

Before a stable checkpoint run formatting, workspace tests and strict Clippy, plus
the publication checks. Stage reviewed paths explicitly. Exclude runtime databases,
credentials, private conversations, local logs and build output. Do not force-push,
discard unrelated work, create another remote repository, switch global accounts,
or fabricate authorship. Follow current owner instructions for external writes.

The owner selected Apache-2.0; preserve third-party licenses and notices. Do not
invent copyright attribution.

Inspect documentation contents before staging. Keep internal research, confidential
plans, unsanitized security investigations, transcripts and operational data under
the ignored `.mother-private/` directory. Preserve mixed-content originals privately
and create reviewed public versions; do not move documents based only on filenames.
Audit the index and unpublished history as well as ignore patterns before a push.
