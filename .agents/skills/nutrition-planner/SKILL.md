---
name: nutrition-planner
description: Design, implement, review, or operate a self-hosted AI-assisted meal planner with deterministic quantities, sourced nutrition, Mealie/Home Assistant integration, private profiles, and idempotent weekly publishing. Use for nutrition-planner architecture, schemas, generation pipelines, tests, dry-runs, or production-readiness reviews; not for medical diagnosis or unsourced nutrition advice.
---

# Nutrition Planner

Build or assess a meal-planning workflow in which AI proposes and typed application code verifies. Preserve the user's deployment choices and authorization boundaries.

## Start with the operating context

- Read repository guidance and the current project decision record before proposing changes.
- Inspect a live Home Assistant or Mealie system read-only unless the user has explicitly approved the specific mutation phase.
- Identify the systems of record, current integrations/entities, deployment constraints, household scope, scheduling rules, privacy needs, and unresolved dietary inputs.
- Separate facts observed from a live system, documented product capabilities, assumptions, and proposed decisions.

## Keep responsibilities narrow

- Let Mealie own recipes, meal plans, and shopping lists when it is in scope.
- Let Home Assistant schedule, display, notify, and provide operator controls.
- Use one small backend for profile rules, schema-constrained model calls, unit conversion, nutrition lookup/calculation, leftovers, aggregation, validation, idempotency, and reconciliation.
- Do not introduce multiple services, a vector database, agents, or a custom Home Assistant integration unless a demonstrated requirement justifies them.

## Treat model output as a proposal

Use strict Structured Outputs for recipe and plan candidates. The output may contain recipe text, explicit ingredient quantities/units, servings, preparation time, cuisine tags, and meal allocation. It must not be the authority for calories/nutrients, unit conversions, shopping totals, pantry inventory, target changes, allergy compliance, or overwrite permission.

Validate the schema, then validate domain rules. Handle refusal, incomplete output, timeout, and provider errors explicitly with bounded retries.

## Preserve traceability and safety

- Resolve ingredients to stable nutrition-source IDs and store source/dataset versions.
- Calculate nutrition from source values and edible mass in deterministic code. Fail closed when a required mapping or conversion is missing.
- Keep calorie and macro targets immutable during planning. Only a user-authorized profile update may change them.
- Model leftovers as reserved servings of an existing recipe so quantities are counted once.
- Consolidate only compatible canonical units and round only for display or a documented purchasable-package rule.
- Before publishing, read the destination and stop on unowned occupied slots. Never silently overwrite.
- Use a stable run key and durable ledger. A repeated successful run returns the existing result; a partial run reconciles only objects it created.
- Default new workflows to dry-run and preview.

## Privacy

Keep profiles, targets, health information, tokens, generated personal plans, live database files, and sensitive logs out of Git. Use redacted examples and synthetic fixtures. Do not place personal dietary data in issues, test snapshots, telemetry, or tool output unnecessarily.

## Verification

Test pure domain behavior independently of APIs. Cover serving scale, leftovers across week boundaries, units, aggregation, nutrition arithmetic, immutable targets, conflict detection, stable run keys, duplicate prevention, retry boundaries, and partial failure. Use a disposable Mealie instance for integration tests and production only after dry-run acceptance.

## Detailed contracts

Read [references/contracts.md](references/contracts.md) when defining or changing schemas, run states, publishing behavior, nutrition calculations, or acceptance tests.
