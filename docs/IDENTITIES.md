# GitHub identity status

Not implemented. Mother neither inspects existing GitHub credentials nor invokes
`gh`. A registered directory is not a verified GitHub repository or account binding.

Account work requires explicit owner authorization, documented `gh` token and
credential-helper precedence, verified identity, per-operation scope checks,
revocation, and an isolation model stronger than directory naming. See the full
requirements in `MOTHER.md` before implementing this boundary.
