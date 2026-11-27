# Nutrition planner contracts

Use these contracts as invariants, adapting field names to the repository's established models.

## Core records

### Profile

- `profile_id`
- version or content hash
- household members and portion multipliers
- immutable user-set calorie/macro targets, if supplied
- allergies, exclusions, dislikes, cuisines, time and leftover rules
- explicit pantry inventory only

Store the real profile outside Git. Repository examples must be synthetic.

### Ingredient proposal

The model supplies display name, numeric quantity, allow-listed unit, optional preparation note, and recipe servings. Resolution adds stable food ID, canonical edible grams, conversion source, nutrition source ID/version, and review status. Do not ask the model to populate authoritative resolution fields.

### Weekly plan

Store ISO week/time zone, dated meal slots, recipe reference, servings eaten, servings reserved for a later leftover slot, preparation-time class, and validation messages. A leftover slot references the originating recipe and reserved servings.

### Run ledger

Use a unique key equivalent to:

```text
(profile_id, iso_year, iso_week, profile_version, planner_schema_version)
```

Useful states are `drafting`, `needs_review`, `validated`, `publishing`, `published`, `failed`, and `partial`. Persist timestamps, input hashes, provider/model identifiers, remote IDs created by the run, and a redacted failure code. Do not store secrets.

## Calculation rules

Convert each resolved ingredient to edible grams through an explicit mapping. For nutrient `n`:

```text
ingredient_n = edible_grams / 100 * source_value_per_100g_n
recipe_n     = sum(ingredient_n)
serving_n    = recipe_n / declared_recipe_servings
```

Retain full precision through calculation and round only presentation values. If cooking yield/retention is not modeled from a cited source, label totals as ingredient-based estimates rather than fabricating adjustments.

Shopping aggregation groups by stable food ID plus compatible canonical unit. Never combine mass, volume, and count without an explicit food-specific conversion. Do not subtract assumed pantry staples.

## Publish protocol

1. Return an already published matching run without writes.
2. Read the target date range and list immediately before mutation.
3. Reject unowned occupied slots according to the configured conflict policy.
4. Publish only a fully validated draft.
5. Record every created remote object immediately after its API call succeeds.
6. Mark success only after all required objects are verified by readback.
7. On failure, reconcile only recorded run-owned objects; otherwise leave them visible as a partial run for manual repair.

Never implement destructive rollback that can touch pre-existing recipes, plan entries, or list items.

## Minimum acceptance scenarios

- Same successful run invoked twice creates no duplicates.
- Occupied target slot causes a conflict and zero writes.
- Model-provided nutrition fields are rejected or ignored.
- Unresolved ingredient prevents publish and identifies the review item.
- Dinner scaled for lunch leftovers contributes ingredients exactly once.
- Compatible units consolidate independently of input order.
- Partial Mealie failure records created IDs and never reports success.
- Changing a target requires a new explicit profile version; planning cannot mutate it.
- Logs and test artifacts contain no real profile values or tokens.
