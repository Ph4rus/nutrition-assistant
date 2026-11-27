use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::Decimal;

use crate::dry_run::DryRunPreview;
use crate::models::{MealType, NutrientProfile, RecipeProposal, Unit};

fn decimal(value: Decimal) -> String {
    value.round_dp(1).normalize().to_string()
}

fn unit(value: Unit) -> &'static str {
    match value {
        Unit::Gram => "g",
        Unit::Kilogram => "kg",
        Unit::Milliliter => "ml",
        Unit::Liter => "l",
        Unit::Piece => "Stück",
    }
}

fn meal_type(value: MealType) -> &'static str {
    match value {
        MealType::Breakfast => "Frühstück",
        MealType::Lunch => "Mittagessen",
        MealType::Dinner => "Abendessen",
        MealType::Snack => "Snack",
    }
}

fn meal_order(value: MealType) -> u8 {
    match value {
        MealType::Breakfast => 0,
        MealType::Lunch => 1,
        MealType::Dinner => 2,
        MealType::Snack => 3,
    }
}

fn weekday(value: Weekday) -> &'static str {
    match value {
        Weekday::Mon => "Montag",
        Weekday::Tue => "Dienstag",
        Weekday::Wed => "Mittwoch",
        Weekday::Thu => "Donnerstag",
        Weekday::Fri => "Freitag",
        Weekday::Sat => "Samstag",
        Weekday::Sun => "Sonntag",
    }
}

fn safe_text(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace('|', "\\|")
}

fn add_scaled(total: &mut NutrientProfile, value: &NutrientProfile, factor: Decimal) {
    total.energy_kcal += value.energy_kcal * factor;
    total.protein_g += value.protein_g * factor;
    total.carbohydrate_g += value.carbohydrate_g * factor;
    total.fat_g += value.fat_g * factor;
    total.fiber_g += value.fiber_g * factor;
}

fn nutrient_row(label: &str, value: &NutrientProfile) -> String {
    format!(
        "| {label} | {} | {} | {} | {} | {} |\n",
        decimal(value.energy_kcal),
        decimal(value.protein_g),
        decimal(value.carbohydrate_g),
        decimal(value.fat_g),
        decimal(value.fiber_g)
    )
}

/// Renders the immutable dry-run artifact without recalculating source nutrition.
pub fn render_markdown(preview: &DryRunPreview) -> String {
    let mut output = format!(
        "# Nutrition Assistant – Wochenprüfung\n\n- Status: `{}`\n- Run: `{}`\n- Schreibvorgänge: {}\n",
        preview.state.as_str(),
        safe_text(&preview.run_key),
        preview.writes_performed
    );

    if !preview.issues.is_empty() {
        output.push_str("\n## Prüfhinweise\n\n");
        for issue in &preview.issues {
            output.push_str(&format!(
                "- `{:?}` `{}` bei `{}`: {}\n",
                issue.severity,
                safe_text(&issue.code),
                safe_text(&issue.path),
                safe_text(&issue.message)
            ));
        }
    }

    let Some(plan) = &preview.plan else {
        output.push_str("\nKein Wochenplan vorhanden.\n");
        return output;
    };

    let recipes: BTreeMap<_, _> = plan
        .recipes
        .iter()
        .map(|recipe| (recipe.recipe_id.as_str(), recipe))
        .collect();
    let nutrition: BTreeMap<_, _> = preview
        .nutrition
        .iter()
        .map(|item| (item.recipe_id.as_str(), item))
        .collect();

    output.push_str(&format!(
        "\nZeitraum: **{} bis {}** (ISO-Woche {}-{:02}, {})\n\n## Wochenplan\n\n| Tag | Mahlzeit | Gericht | Portionen | Rest von |\n|---|---|---|---:|---|\n",
        plan.start_date,
        plan.start_date + chrono::Duration::days(6),
        plan.iso_year,
        plan.iso_week,
        safe_text(&plan.timezone)
    ));
    let mut meals: Vec<_> = plan.meals.iter().collect();
    meals.sort_by_key(|meal| (meal.date, meal_order(meal.meal_type), meal.slot_id.as_str()));
    for meal in &meals {
        let title = recipes
            .get(meal.recipe_id.as_str())
            .map_or(meal.recipe_id.as_str(), |recipe| recipe.title.as_str());
        output.push_str(&format!(
            "| {}, {} | {} | {} | {} | {} |\n",
            weekday(meal.date.weekday()),
            if meal.date > plan.start_date + chrono::Duration::days(6) {
                format!("{} (Restübertrag)", meal.date)
            } else {
                meal.date.to_string()
            },
            meal_type(meal.meal_type),
            safe_text(title),
            decimal(meal.servings),
            meal.leftover_of
                .as_deref()
                .map(safe_text)
                .unwrap_or_else(|| "–".into())
        ));
    }

    output.push_str("\n## Tagesnährwerte\n\nDie Werte sind Summen der eingeplanten Portionen und stammen aus der zutatenbasierten BLS-Berechnung. Der zusätzliche Folgetag enthält ausschließlich den Restübertrag aus dem letzten Abendessen.\n\n| Tag | kcal | Protein (g) | Kohlenhydrate (g) | Fett (g) | Ballaststoffe (g) |\n|---|---:|---:|---:|---:|---:|\n");
    let mut daily: BTreeMap<NaiveDate, NutrientProfile> = BTreeMap::new();
    for meal in &meals {
        if let Some(item) = nutrition.get(meal.recipe_id.as_str()) {
            add_scaled(
                daily.entry(meal.date).or_default(),
                &item.per_serving,
                meal.servings,
            );
        }
    }
    for (date, nutrients) in daily {
        output.push_str(&nutrient_row(
            &format!(
                "{}, {}{}",
                weekday(date.weekday()),
                date,
                if date > plan.start_date + chrono::Duration::days(6) {
                    " (Restübertrag)"
                } else {
                    ""
                }
            ),
            &nutrients,
        ));
    }

    output.push_str("\n## Rezepte\n");
    for recipe in &plan.recipes {
        output.push_str(&format!(
            "\n### {}\n\n{} Portionen · {} Minuten aktiv",
            safe_text(&recipe.title),
            decimal(recipe.servings),
            recipe.active_minutes
        ));
        if !recipe.cuisine_tags.is_empty() {
            output.push_str(&format!(
                " · {}",
                recipe
                    .cuisine_tags
                    .iter()
                    .map(|tag| safe_text(tag))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        output.push_str("\n\nZutaten:\n\n");
        for ingredient in &recipe.ingredients {
            output.push_str(&format!(
                "- {} {} {}{}\n",
                decimal(ingredient.quantity.amount),
                unit(ingredient.quantity.unit),
                safe_text(&ingredient.name),
                ingredient
                    .preparation_note
                    .as_deref()
                    .map(|note| format!(" ({})", safe_text(note)))
                    .unwrap_or_default()
            ));
        }
        output.push_str("\nZubereitung:\n\n");
        for (index, instruction) in recipe.instructions.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", index + 1, safe_text(instruction)));
        }
        if let Some(item) = nutrition.get(recipe.recipe_id.as_str()) {
            output.push_str("\nNährwerte pro Portion:\n\n| kcal | Protein (g) | Kohlenhydrate (g) | Fett (g) | Ballaststoffe (g) |\n|---:|---:|---:|---:|---:|\n");
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                decimal(item.per_serving.energy_kcal),
                decimal(item.per_serving.protein_g),
                decimal(item.per_serving.carbohydrate_g),
                decimal(item.per_serving.fat_g),
                decimal(item.per_serving.fiber_g)
            ));
            output.push_str(&format!(
                "\nBerechnung: `{}`; {} rückverfolgbare Zutatenquellen.\n",
                safe_text(&item.method),
                item.trace.len()
            ));
        }
    }

    output.push_str("\n## Konsolidierte Einkaufsliste\n\n| Zutat | Menge |\n|---|---:|\n");
    for line in &preview.shopping {
        output.push_str(&format!(
            "| {} | {} {} |\n",
            safe_text(&line.display_name),
            decimal(line.amount),
            unit(line.unit)
        ));
    }
    output
}

fn recipe_summary(recipe: &RecipeProposal) -> String {
    let ingredients = recipe
        .ingredients
        .iter()
        .map(|ingredient| {
            format!(
                "{} {} {}",
                decimal(ingredient.quantity.amount),
                unit(ingredient.quantity.unit),
                safe_text(&ingredient.name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "**{}** — {} Portionen, {} Minuten aktiv\n\n{}",
        safe_text(&recipe.title),
        decimal(recipe.servings),
        recipe.active_minutes,
        ingredients
    )
}

/// Renders a concise audit of one immutable preview revision.
pub fn render_revision_diff(parent: &DryRunPreview, revision: &DryRunPreview) -> String {
    let mut output = format!(
        "# Nutrition Assistant – Revisionsvergleich\n\n- Ausgangs-Run: `{}`\n- Revisions-Run: `{}`\n- Revisionsstatus: `{}`\n- Schreibvorgänge: {}\n",
        safe_text(&parent.run_key),
        safe_text(&revision.run_key),
        revision.state.as_str(),
        revision.writes_performed
    );
    let (Some(before), Some(after)) = (&parent.plan, &revision.plan) else {
        output.push_str("\nEin Plan fehlt; ein Vergleich ist nicht möglich.\n");
        return output;
    };
    output.push_str(&format!(
        "- Meal-Slots unverändert: **{}**\n\n",
        if before.meals == after.meals {
            "ja"
        } else {
            "nein"
        }
    ));
    let before_recipes: BTreeMap<_, _> = before
        .recipes
        .iter()
        .map(|recipe| (recipe.recipe_id.as_str(), recipe))
        .collect();
    let after_recipes: BTreeMap<_, _> = after
        .recipes
        .iter()
        .map(|recipe| (recipe.recipe_id.as_str(), recipe))
        .collect();
    let changed: Vec<_> = before_recipes
        .iter()
        .filter_map(|(id, old)| {
            after_recipes
                .get(id)
                .filter(|new| *new != old)
                .map(|new| (*id, *old, *new))
        })
        .collect();
    output.push_str(&format!("Geänderte Rezepte: **{}**\n", changed.len()));
    for (id, old, new) in changed {
        output.push_str(&format!(
            "\n## Rezept `{}`\n\n### Vorher\n\n{}\n\n### Nachher\n\n{}\n",
            safe_text(id),
            recipe_summary(old),
            recipe_summary(new)
        ));
        let old_nutrition = parent
            .nutrition
            .iter()
            .find(|item| item.recipe_id == id)
            .map(|item| &item.per_serving);
        let new_nutrition = revision
            .nutrition
            .iter()
            .find(|item| item.recipe_id == id)
            .map(|item| &item.per_serving);
        if let (Some(old_values), Some(new_values)) = (old_nutrition, new_nutrition) {
            output.push_str("\n### Nährwerte pro Portion\n\n| Version | kcal | Protein (g) | Kohlenhydrate (g) | Fett (g) | Ballaststoffe (g) |\n|---|---:|---:|---:|---:|---:|\n");
            output.push_str(&nutrient_row("Vorher", old_values));
            output.push_str(&nutrient_row("Nachher", new_values));
        }
    }
    let before_shopping: BTreeMap<_, _> = parent
        .shopping
        .iter()
        .map(|line| ((line.food_id.as_str(), line.dimension), line))
        .collect();
    let after_shopping: BTreeMap<_, _> = revision
        .shopping
        .iter()
        .map(|line| ((line.food_id.as_str(), line.dimension), line))
        .collect();
    output.push_str(
        "\n## Änderung der Einkaufsliste\n\n| Zutat | Vorher | Nachher |\n|---|---:|---:|\n",
    );
    let mut keys: Vec<_> = before_shopping
        .keys()
        .chain(after_shopping.keys())
        .copied()
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let old = before_shopping.get(&key);
        let new = after_shopping.get(&key);
        if old.map(|line| (line.amount, line.unit)) == new.map(|line| (line.amount, line.unit)) {
            continue;
        }
        let name = new.or(old).map_or(key.0, |line| line.display_name.as_str());
        let amount = |line: Option<&&crate::models::CanonicalShoppingLine>| {
            line.map(|line| format!("{} {}", decimal(line.amount), unit(line.unit)))
                .unwrap_or_else(|| "–".into())
        };
        output.push_str(&format!(
            "| {} | {} | {} |\n",
            safe_text(name),
            amount(old),
            amount(new)
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dry_run::DryRunPreview;
    use crate::models::RunState;

    #[test]
    fn renders_review_without_a_plan() {
        let preview = DryRunPreview {
            run_key: "synthetic".into(),
            state: RunState::NeedsReview,
            plan: None,
            issues: vec![],
            nutrition: vec![],
            shopping: vec![],
            destination_conflicts: vec![],
            writes_performed: 0,
        };
        let rendered = render_markdown(&preview);
        assert!(rendered.contains("Status: `needs_review`"));
        assert!(rendered.contains("Kein Wochenplan vorhanden"));
    }

    #[test]
    fn revision_diff_reports_locked_slots() {
        let preview = DryRunPreview {
            run_key: "synthetic".into(),
            state: RunState::Validated,
            plan: None,
            issues: vec![],
            nutrition: vec![],
            shopping: vec![],
            destination_conflicts: vec![],
            writes_performed: 0,
        };
        assert!(render_revision_diff(&preview, &preview).contains("Vergleich ist nicht möglich"));
    }
}
