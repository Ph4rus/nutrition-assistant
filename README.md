# Nutrition Assistant

Nutrition Assistant is a self-hosted weekly meal planner written in Rust. A language model may
propose recipes and meal assignments, while application code remains authoritative for units,
leftovers, nutrition, shopping totals, validation, and idempotency.

The project integrates with [Mealie](https://mealie.io/) as the system of record for recipes,
meal plans, and shopping lists. The included Home Assistant app currently runs as a manual,
one-shot dry-run and does not publish or modify Home Assistant.

> [!IMPORTANT]
> Nutrition Assistant is not a medical device and does not provide medical advice. Nutrition
> totals are ingredient-based estimates from sourced data. Dietary targets are user inputs and
> are never tightened automatically.

## Design principles

- Model output is an untrusted proposal validated against strict Structured Outputs.
- Nutrition is calculated from source records, never invented by the model.
- Quantities use decimal arithmetic and explicit, food-specific conversions.
- Dinner leftovers are reserved servings of the original recipe and are counted once.
- Shopping lines aggregate only compatible canonical units.
- Stable run identities and a SQLite ledger prevent duplicate weekly runs.
- Publishing fails closed on unresolved ingredients or occupied Mealie slots.
- Approval is bound to the SHA-256 hash of an exact validated preview.
- Profiles, credentials, generated plans, databases, and source datasets stay outside Git.

## Current status

Implemented:

- strict Rust domain models and business validation;
- OpenAI Responses API adapter with `store=false` and strict JSON Schema;
- local import of the German Bundeslebensmittelschlüssel (BLS) 4.0 dataset;
- deterministic nutrition, unit, leftover, and shopping calculations;
- read-only Mealie preflight and immutable dry-run artifacts;
- private permanent or cycle-scoped feedback;
- immutable single-recipe refinement with full recalculation;
- hash-bound preview approval and revocation;
- a tested Mealie publishing library with ownership markers and readback.

Intentionally not exposed yet:

- a production publishing CLI;
- automatic Home Assistant scheduling;
- destructive remote reconciliation.

## Requirements

- Rust 1.100, installed through [rustup](https://rustup.rs/);
- a local BLS 4.0 XLSX dataset;
- an OpenAI API key for generation;
- Mealie v3 for destination checks and future publishing.

The BLS dataset is published by the Max Rubner-Institut under CC BY 4.0. It is downloaded at
runtime and is not redistributed by this repository.

## Quick start

```bash
cp .env.example .env
cp config/profile.example.yaml config/profile.local.yaml
cp config/food_catalog.example.yaml config/food_catalog.local.yaml
bash scripts/download-bls.sh
```

Fill the local profile and curate real BLS mappings before running the preflight. The example
catalog contains only a synthetic placeholder and is not usable for a real plan.

```bash
cargo run --locked -- preflight
cargo run --locked -- dry-run --year 2026 --week 49
cargo run --locked -- review --input dry-run.local.json --output review.local.md
```

`dry-run` checks Mealie by default and performs zero remote writes. The flag
`--skip-destination-check` exists for local adapter development only.

## Feedback and refinement

```bash
cargo run --locked -- feedback add \
  --kind ingredient --signal avoid --scope permanent \
  --target "example ingredient" --reason "example reason"

cargo run --locked -- refine \
  --input dry-run.local.json \
  --recipe-id example-recipe-id \
  --instruction "Use a different allow-listed vegetable" \
  --output revision.local.json

cargo run --locked -- review-diff \
  --parent dry-run.local.json \
  --revision revision.local.json \
  --output revision-review.local.md
```

Refinement locks the selected recipe ID and serving count as well as every other recipe and meal
slot. Nutrition and shopping totals are recalculated from scratch.

## Explicit approval

Approval changes only the local SQLite ledger. These commands do not publish anything:

```bash
cargo run --locked -- approval status --input revision.local.json
cargo run --locked -- approval approve --input revision.local.json
cargo run --locked -- approval revoke --input revision.local.json
```

Any content change creates a different hash and therefore requires a new approval.

## Home Assistant app

The repository contains an experimental Home Assistant app definition. It has no privileged,
host-network, Home Assistant API, or Supervisor API access. It starts manually, creates the next
configured planning cycle, stores its private ledger and preview under `/data`, and exits.

[![Open Home Assistant and add the Nutrition Assistant app repository.](https://my.home-assistant.io/badges/supervisor_add_addon_repository.svg)](https://my.home-assistant.io/redirect/supervisor_add_addon_repository/?repository_url=https%3A%2F%2Fgithub.com%2FPh4rus%2Fnutrition-assistant)

See [DOCS.md](DOCS.md) for configuration and deployment details. Publishing remains disabled in
the app.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

An opt-in integration test is available for a disposable, loopback-only Mealie instance. It is
ignored during the normal test suite and must never target production.

For a useful reading order, start with `src/models.rs`, continue with the pure calculation modules
(`units`, `nutrition`, `leftovers`, and `shopping`), then read `validation`, `storage`, `dry_run`,
and the provider adapters.

## Privacy

The ignore rules exclude `.env`, local profiles and catalogs, generated plans and reviews, SQLite
databases, BLS files, logs, and build output. Treat ignored files as sensitive even though Git does
not track them. Use synthetic data in tests, issues, and pull requests.

## Contributing and license

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and
[SECURITY.md](SECURITY.md) before opening a pull request or reporting a vulnerability.

Licensed under the [MIT License](LICENSE).
