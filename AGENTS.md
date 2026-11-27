# Contributor instructions

Read `README.md` and `.agents/skills/nutrition-planner/SKILL.md` before changing planner behavior.
Read `.agents/skills/nutrition-planner/references/contracts.md` when changing schemas,
calculations, publishing, run states, or idempotency.

## Safety invariants

- Model output is a proposal; deterministic Rust code owns calculations and validation.
- Nutrition values require a stable source ID and dataset version.
- User-provided calorie and macro targets are immutable during planning.
- Publishing must fail closed on unresolved ingredients and destination conflicts.
- Never overwrite or delete pre-existing Mealie objects.
- Reconciliation may touch only remote IDs recorded as owned by the current run.
- Default all new workflows to dry-run and explicit preview.

## Privacy

- Never commit profiles, targets, health information, credentials, BLS files, generated plans,
  live databases, or sensitive logs.
- Tests, documentation, issues, and pull requests must use synthetic or redacted data.
- Secrets are injected at runtime and must not appear in command output.

## Engineering

- Keep the architecture a single Rust modular monolith unless a measured requirement proves
  another service is necessary.
- Keep pure domain logic separate from provider adapters.
- Do not add a second runtime, agent framework, or vector database.
- Use a disposable Mealie instance for integration tests; never run them against production.
- Before handoff, run rustfmt, Clippy with warnings denied, all tests, and a locked release build.
