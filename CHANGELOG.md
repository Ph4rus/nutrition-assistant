# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.4] - 2026-11-27

### Added

- Home Assistant `refine` operation for locked single-recipe refinement with private revision and
  human-readable diff exports.

### Changed

- A needs-review preview may be refined only when every open issue points to the selected recipe;
  clean validated previews remain supported and all other incomplete previews fail closed.

## [0.3.3] - 2026-11-27

### Added

- Deterministic calorie-budget validation after ingredient-based nutrition calculation.
- Blocking issues when the seven-day average exceeds the explicit profile reference or a cycle day
  exceeds that reference by more than 150 kcal.

### Changed

- Planning policy version 7 keeps the day-eight rollover outside the seven complete cycle days and
  forces existing cached plans through the new policy without overwriting them.
- The model prompt now proposes conservative quantities for the explicit budget, while Rust remains
  authoritative and never changes the configured target.

## [0.3.2] - 2026-11-27

### Added

- Atomic private exports of `review.md` and `dry-run.json` to the Home Assistant app configuration
  directory after a completed dry-run.

## [0.3.1] - 2026-11-27

### Added

- Transparent Home Assistant app icon and horizontal project wordmark.
- Versioned, prebuilt `linux/amd64` container images published to GHCR.

### Changed

- Home Assistant now pulls the release image instead of compiling Rust during installation.

## [0.3.0] - 2026-11-27

### Added

- Deterministic Rust domain core for units, leftovers, nutrition, and shopping aggregation.
- OpenAI strict Structured Outputs adapter with disabled response storage.
- Local BLS 4.0 import and curated ingredient allow-list.
- Immutable, idempotent dry-run artifacts and private feedback ledger.
- Single-recipe refinement with locked meal slots and full recalculation.
- Hash-bound preview approval and revocation.
- Mealie v3 publishing library with conflict checks, ownership ledger, and readback.
- Experimental one-shot Home Assistant dry-run app.

[Unreleased]: https://github.com/Ph4rus/nutrition-assistant/compare/v0.3.4...HEAD
[0.3.4]: https://github.com/Ph4rus/nutrition-assistant/releases/tag/v0.3.4
[0.3.3]: https://github.com/Ph4rus/nutrition-assistant/releases/tag/v0.3.3
[0.3.2]: https://github.com/Ph4rus/nutrition-assistant/releases/tag/v0.3.2
[0.3.1]: https://github.com/Ph4rus/nutrition-assistant/releases/tag/v0.3.1
[0.3.0]: https://github.com/Ph4rus/nutrition-assistant/releases/tag/v0.3.0
