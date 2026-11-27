# Contributing

Thank you for helping improve Nutrition Assistant.

## Before opening an issue

- Search existing issues first.
- Remove profiles, health information, generated plans, credentials, database contents, and
  internal URLs from logs and screenshots.
- Use a minimal synthetic example whenever possible.
- Use the security process in `SECURITY.md` for vulnerabilities.

## Pull requests

Keep changes focused and explain which invariant they preserve or change. New behavior should have
tests at the lowest useful layer. Provider integrations need synthetic contract tests; Mealie
integration tests must target a disposable instance only.

Run the complete local check before requesting review:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

Do not commit generated artifacts, local datasets, private configuration, or secrets. If a test
needs representative data, create a deliberately synthetic fixture.

## Architecture

The project intentionally uses one Rust binary with pure domain modules and narrow provider
adapters. Proposals for additional services or runtimes should include a concrete requirement that
cannot be met within this architecture.
