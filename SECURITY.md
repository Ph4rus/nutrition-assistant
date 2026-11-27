# Security policy

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use the repository's private
vulnerability reporting feature when available. Include a minimal reproduction and expected
impact, but never include real API keys, profiles, generated meal plans, or health information.

If a credential may have been exposed, revoke it immediately before preparing the report.

## Supported version

Until the first stable release, security fixes target the latest commit on the default branch.

## Operational boundary

The Home Assistant app is experimental and dry-run only. Production publishing is not exposed by
the CLI. Operators are responsible for securing OpenAI and Mealie credentials, restricting Mealie
network exposure, and maintaining tested backups of the app configuration and `/data` ledger.
