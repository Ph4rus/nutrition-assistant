## Summary

Describe the problem and the smallest change that solves it.

## Safety and privacy

- [ ] Tests use only synthetic or redacted data.
- [ ] No profiles, credentials, generated plans, datasets, or live databases are included.
- [ ] Calculation, publishing, and idempotency invariants are preserved or documented.

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --locked --all-targets --all-features -- -D warnings`
- [ ] `cargo test --locked --all-targets`
- [ ] `cargo build --locked --release`
