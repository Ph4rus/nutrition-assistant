use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, Duration};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::{MealType, NutritionProfile, RecipeProposal, StringOrList, WeeklyPlan};
use crate::nutrition::RecipeNutrition;
use crate::planning_cycle::cycle_start_for;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Blocker,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub code: String,
    pub path: String,
    pub message: String,
}

fn unknown(value: Option<&StringOrList>) -> bool {
    value.is_none_or(StringOrList::is_unknown)
}
fn time_limit(
    recipe: &RecipeProposal,
    meal_type: MealType,
    weekday: u32,
    profile: &NutritionProfile,
) -> u32 {
    let _ = recipe;
    if meal_type == MealType::Breakfast {
        profile.planning.breakfast_max_morning_minutes
    } else if weekday >= 5 {
        profile.planning.weekend_max_active_minutes
    } else {
        profile.planning.weekday_max_active_minutes
    }
}
fn issue(severity: IssueSeverity, code: &str, path: String, message: String) -> ValidationIssue {
    ValidationIssue {
        severity,
        code: code.into(),
        path,
        message,
    }
}
fn sort_issues(issues: &mut [ValidationIssue]) {
    issues.sort_by(|a, b| (&a.severity, &a.code, &a.path).cmp(&(&b.severity, &b.code, &b.path)));
}

pub(crate) fn violates_dairy_preference(profile: &NutritionProfile, ingredient_name: &str) -> bool {
    let Some(dairy) = profile.preferences.dairy.as_ref() else {
        return false;
    };
    if !dairy
        .most_other_dairy
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("disliked"))
    {
        return false;
    }
    let name = ingredient_name.to_lowercase();
    if name == "butter"
        && dairy
            .butter
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("accepted"))
    {
        return false;
    }
    if [
        "soja", "soy", "kokos", "coconut", "hafer", "oat", "erdnuss", "peanut", "mandel", "almond",
        "nuss",
    ]
    .iter()
    .any(|term| name.contains(term))
    {
        return false;
    }
    [
        "skyr", "quark", "joghurt", "yogurt", "käse", "cheese", "kefir", "butter", "molke", "whey",
    ]
    .iter()
    .any(|term| name.contains(term))
}

pub fn validate_profile_readiness(profile: &NutritionProfile) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if profile.planning.household_size.is_none() {
        issues.push(issue(
            IssueSeverity::Blocker,
            "household_size_missing",
            "profile.planning.household_size".into(),
            "Household size must be explicit before planning portions.".into(),
        ));
    }
    if unknown(profile.preferences.allergies.as_ref()) {
        issues.push(issue(
            IssueSeverity::Blocker,
            "allergies_unknown",
            "profile.preferences.allergies".into(),
            "Allergy status must be confirmed before publication.".into(),
        ));
    }
    if !profile.planning.schedule.confirmed {
        issues.push(issue(
            IssueSeverity::Review,
            "schedule_unconfirmed",
            "profile.planning.schedule".into(),
            "The proposed weekly schedule has not been confirmed.".into(),
        ));
    }
    sort_issues(&mut issues);
    issues
}

pub fn validate_weekly_plan(plan: &WeeklyPlan, profile: &NutritionProfile) -> Vec<ValidationIssue> {
    let mut issues = validate_profile_readiness(profile);
    if let Some(day) = profile.planning.shopping_day
        && cycle_start_for(plan.iso_year, plan.iso_week, day).ok() != Some(plan.start_date)
    {
        issues.push(issue(
            IssueSeverity::Blocker,
            "cycle_start_mismatch",
            "plan.start_date".into(),
            "Plan start date does not match the configured shopping day.".into(),
        ));
    }
    let recipes: HashMap<_, _> = plan
        .recipes
        .iter()
        .map(|r| (r.recipe_id.as_str(), r))
        .collect();
    let slots_by_id: HashMap<_, _> = plan
        .meals
        .iter()
        .map(|meal| (meal.slot_id.as_str(), meal))
        .collect();
    let mut slots_by_date_type = HashMap::new();
    for meal in &plan.meals {
        let key = (meal.date, meal.meal_type);
        if slots_by_date_type.insert(key, meal).is_some() {
            issues.push(issue(
                IssueSeverity::Blocker,
                "duplicate_meal_slot",
                format!("meals.{}.{}", meal.date, meal.meal_type.as_str()),
                "A date may contain at most one slot of each meal type.".into(),
            ));
        }
    }

    for offset in 0..7 {
        let date = plan.start_date + Duration::days(offset);
        for meal_type in [MealType::Breakfast, MealType::Lunch, MealType::Dinner] {
            if !slots_by_date_type.contains_key(&(date, meal_type)) {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "required_meal_missing",
                    format!("meals.{date}.{}", meal_type.as_str()),
                    "Each cycle day requires breakfast, lunch, and dinner.".into(),
                ));
            }
        }
    }

    let rollover_date = plan.start_date + Duration::days(7);
    let rollover_slots: Vec<_> = plan
        .meals
        .iter()
        .filter(|meal| meal.date == rollover_date)
        .collect();
    if rollover_slots.len() != 1
        || rollover_slots[0].meal_type != MealType::Lunch
        || rollover_slots[0].leftover_of.is_none()
    {
        issues.push(issue(
            IssueSeverity::Blocker,
            "rollover_lunch_invalid",
            format!("meals.{rollover_date}"),
            "The only day-eight slot must be lunch left over from day-seven dinner.".into(),
        ));
    }

    for meal in &plan.meals {
        if let Some(origin_id) = &meal.leftover_of {
            let valid = slots_by_id.get(origin_id.as_str()).is_some_and(|origin| {
                origin.meal_type == MealType::Dinner
                    && meal.meal_type == MealType::Lunch
                    && meal.date == origin.date + Duration::days(1)
                    && meal.recipe_id == origin.recipe_id
            });
            if !valid {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "leftover_link_invalid",
                    format!("meals.{}.leftover_of", meal.slot_id),
                    "A leftover must be the next-day lunch from the referenced dinner recipe."
                        .into(),
                ));
            }
        }
    }

    for offset in 0..7 {
        let dinner_date = plan.start_date + Duration::days(offset);
        let lunch_date = dinner_date + Duration::days(1);
        if let Some(dinner) = slots_by_date_type.get(&(dinner_date, MealType::Dinner)) {
            let linked = slots_by_date_type
                .get(&(lunch_date, MealType::Lunch))
                .is_some_and(|lunch| lunch.leftover_of.as_deref() == Some(&dinner.slot_id));
            if !linked {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "dinner_leftover_missing",
                    format!("meals.{}.dinner", dinner_date),
                    "Every dinner must reserve leftovers for the following lunch.".into(),
                ));
            }
        }
    }

    let mut allocated_servings: HashMap<&str, Decimal> = HashMap::new();
    for meal in &plan.meals {
        *allocated_servings
            .entry(meal.recipe_id.as_str())
            .or_insert(Decimal::ZERO) += meal.servings;
    }
    for recipe in &plan.recipes {
        if allocated_servings
            .get(recipe.recipe_id.as_str())
            .copied()
            .unwrap_or(Decimal::ZERO)
            != recipe.servings
        {
            issues.push(issue(
                IssueSeverity::Blocker,
                "recipe_servings_mismatch",
                format!("recipes.{}.servings", recipe.recipe_id),
                "Recipe servings must equal all allocated meal-slot servings exactly.".into(),
            ));
        }
    }
    let disliked: Vec<_> = profile
        .preferences
        .dislikes
        .iter()
        .chain(profile.learned.avoid_ingredients.iter())
        .map(|v| v.to_lowercase())
        .collect();
    let egg_unknown = unknown(profile.preferences.eggs.as_ref());
    let token_re = Regex::new(r"\w+").expect("constant regex");
    for recipe in &plan.recipes {
        let title = recipe.title.to_lowercase();
        let tags: Vec<_> = recipe
            .cuisine_tags
            .iter()
            .map(|tag| tag.to_lowercase())
            .collect();
        if profile.learned.avoid_dishes.iter().any(|term| {
            let term = term.to_lowercase();
            title.contains(&term) || tags.iter().any(|tag| tag.contains(&term))
        }) {
            issues.push(issue(
                IssueSeverity::Blocker,
                "learned_dish_avoidance",
                format!("recipes.{}.title", recipe.recipe_id),
                "Recipe conflicts with an active learned dish or cuisine avoidance rule.".into(),
            ));
        }
        for (index, ingredient) in recipe.ingredients.iter().enumerate() {
            let name = ingredient.name.to_lowercase();
            if disliked.iter().any(|term| name.contains(term)) {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "disliked_ingredient",
                    format!("recipes.{}.ingredients.{index}", recipe.recipe_id),
                    "Recipe contains an explicitly disliked ingredient.".into(),
                ));
            }
            if violates_dairy_preference(profile, &name) {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "dairy_preference_violation",
                    format!("recipes.{}.ingredients.{index}", recipe.recipe_id),
                    "Recipe contains a dairy product excluded by the profile.".into(),
                ));
            }
            let tokens: HashSet<_> = token_re.find_iter(&name).map(|m| m.as_str()).collect();
            if egg_unknown
                && ["egg", "eggs", "ei", "eier"]
                    .iter()
                    .any(|v| tokens.contains(v))
            {
                issues.push(issue(
                    IssueSeverity::Blocker,
                    "egg_preference_unknown",
                    format!("recipes.{}.ingredients.{index}", recipe.recipe_id),
                    "Recipe uses egg while the egg preference is unresolved.".into(),
                ));
            }
        }
    }
    let mut checked = HashSet::new();
    for meal in &plan.meals {
        if meal.leftover_of.is_some() {
            continue;
        }
        let Some(recipe) = recipes.get(meal.recipe_id.as_str()) else {
            continue;
        };
        let weekday = meal.date.weekday().num_days_from_monday();
        if !checked.insert((recipe.recipe_id.as_str(), meal.meal_type, weekday)) {
            continue;
        }
        let limit = time_limit(recipe, meal.meal_type, weekday, profile);
        if recipe.active_minutes > limit {
            issues.push(issue(
                IssueSeverity::Blocker,
                "active_time_exceeded",
                format!("recipes.{}.active_minutes", recipe.recipe_id),
                format!("Active time exceeds the configured {limit}-minute limit."),
            ));
        }
    }
    sort_issues(&mut issues);
    issues
}

/// Validates calculated energy totals against the explicit profile reference without changing it.
/// The day-eight rollover lunch belongs to the next cycle and is intentionally excluded.
pub fn validate_energy_budget(
    plan: &WeeklyPlan,
    profile: &NutritionProfile,
    nutrition: &[RecipeNutrition],
) -> Vec<ValidationIssue> {
    let Some(reference) = profile.nutrition.provisional_daily_kcal_reference else {
        return Vec::new();
    };
    if nutrition.len() != plan.recipes.len() {
        return Vec::new();
    }

    let energy_by_recipe: HashMap<_, _> = nutrition
        .iter()
        .map(|item| (item.recipe_id.as_str(), item.per_serving.energy_kcal))
        .collect();
    let last_cycle_day = plan.start_date + Duration::days(6);
    let mut daily = BTreeMap::new();
    for meal in &plan.meals {
        if meal.date < plan.start_date || meal.date > last_cycle_day {
            continue;
        }
        if let Some(per_serving) = energy_by_recipe.get(meal.recipe_id.as_str()) {
            *daily.entry(meal.date).or_insert(Decimal::ZERO) += *per_serving * meal.servings;
        }
    }

    let daily_limit = reference + Decimal::from(150);
    let mut issues = Vec::new();
    let mut cycle_total = Decimal::ZERO;
    for offset in 0..7 {
        let date = plan.start_date + Duration::days(offset);
        let total = daily.get(&date).copied().unwrap_or(Decimal::ZERO);
        cycle_total += total;
        if total > daily_limit {
            issues.push(issue(
                IssueSeverity::Blocker,
                "daily_kcal_above_limit",
                format!("nutrition.daily.{date}.energy_kcal"),
                format!(
                    "Calculated daily energy {} kcal exceeds the configured limit {} kcal.",
                    total.round_dp(1),
                    daily_limit.round_dp(1)
                ),
            ));
        }
    }
    let average = cycle_total / Decimal::from(7);
    if average > reference {
        issues.push(issue(
            IssueSeverity::Blocker,
            "weekly_kcal_average_above_reference",
            "nutrition.weekly_average.energy_kcal".into(),
            format!(
                "Calculated seven-day average {} kcal exceeds the explicit reference {} kcal.",
                average.round_dp(1),
                reference.round_dp(1)
            ),
        ));
    }
    sort_issues(&mut issues);
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        IngredientProposal, MealSlot, NutrientProfile, Quantity, RecipeProposal, Unit,
    };
    use crate::nutrition::RecipeNutrition;
    use crate::profile::load_profile;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::path::Path;
    #[test]
    fn incomplete_example_fails_closed() {
        let issues = validate_profile_readiness(
            &load_profile(Path::new("config/profile.example.yaml")).unwrap(),
        );
        assert_eq!(
            issues
                .iter()
                .filter(|v| v.severity == IssueSeverity::Blocker)
                .count(),
            2
        );
    }

    #[test]
    fn disliked_dairy_blocks_skyr_but_allows_soy_yogurt() {
        let mut profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        profile.preferences.dairy = Some(crate::models::DairyPreferences {
            milk: Some("accepted".into()),
            cream: Some("accepted".into()),
            butter: Some("accepted".into()),
            most_other_dairy: Some("disliked".into()),
            soy_alternatives: Some("preferred".into()),
        });
        assert!(violates_dairy_preference(&profile, "Skyr"));
        assert!(violates_dairy_preference(&profile, "Naturjoghurt"));
        assert!(!violates_dairy_preference(&profile, "Sojajoghurt"));
        assert!(!violates_dairy_preference(&profile, "Erdnussbutter"));
        assert!(!violates_dairy_preference(&profile, "Butter"));
        assert!(!violates_dairy_preference(&profile, "Milch"));
        assert!(!violates_dairy_preference(&profile, "Sahne"));
    }

    #[test]
    fn incomplete_week_and_unallocated_servings_fail_closed() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let plan = WeeklyPlan {
            iso_year: 2026,
            iso_week: 48,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![RecipeProposal {
                recipe_id: "r1".into(),
                title: "Synthetic".into(),
                servings: Decimal::from(2),
                active_minutes: 5,
                cuisine_tags: vec![],
                ingredients: vec![IngredientProposal {
                    name: "synthetic".into(),
                    quantity: Quantity {
                        amount: Decimal::from(100),
                        unit: Unit::Gram,
                    },
                    preparation_note: None,
                }],
                instructions: vec!["Prepare".into()],
            }],
            meals: vec![MealSlot {
                slot_id: "dinner".into(),
                date: start,
                meal_type: MealType::Dinner,
                recipe_id: "r1".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        let profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        let codes: HashSet<_> = validate_weekly_plan(&plan, &profile)
            .into_iter()
            .map(|item| item.code)
            .collect();
        assert!(codes.contains("required_meal_missing"));
        assert!(codes.contains("dinner_leftover_missing"));
        assert!(codes.contains("rollover_lunch_invalid"));
        assert!(codes.contains("recipe_servings_mismatch"));
    }

    #[test]
    fn explicit_energy_reference_blocks_average_and_daily_excess() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let meals = (0..7)
            .map(|offset| MealSlot {
                slot_id: format!("meal-{offset}"),
                date: start + Duration::days(offset),
                meal_type: MealType::Dinner,
                recipe_id: "regular".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            })
            .chain(std::iter::once(MealSlot {
                slot_id: "rollover".into(),
                date: start + Duration::days(7),
                meal_type: MealType::Lunch,
                recipe_id: "rollover".into(),
                servings: Decimal::ONE,
                leftover_of: Some("meal-6".into()),
            }))
            .collect();
        let plan = WeeklyPlan {
            iso_year: 2026,
            iso_week: 49,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![
                RecipeProposal {
                    recipe_id: "regular".into(),
                    title: "Regular".into(),
                    servings: Decimal::from(7),
                    active_minutes: 5,
                    cuisine_tags: vec![],
                    ingredients: vec![],
                    instructions: vec!["Prepare".into()],
                },
                RecipeProposal {
                    recipe_id: "rollover".into(),
                    title: "Rollover".into(),
                    servings: Decimal::ONE,
                    active_minutes: 5,
                    cuisine_tags: vec![],
                    ingredients: vec![],
                    instructions: vec!["Prepare".into()],
                },
            ],
            meals,
        };
        let mut profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        profile.nutrition.provisional_daily_kcal_reference = Some(Decimal::from(2800));
        let item = |recipe_id: &str, energy_kcal: i64| RecipeNutrition {
            recipe_id: recipe_id.into(),
            servings: Decimal::ONE,
            recipe_total: NutrientProfile::default(),
            per_serving: NutrientProfile {
                energy_kcal: Decimal::from(energy_kcal),
                ..Default::default()
            },
            trace: vec![],
            method: "synthetic".into(),
        };

        let issues = validate_energy_budget(
            &plan,
            &profile,
            &[item("regular", 3000), item("rollover", 10000)],
        );
        assert_eq!(
            issues
                .iter()
                .filter(|item| item.code == "daily_kcal_above_limit")
                .count(),
            7
        );
        assert!(
            issues
                .iter()
                .any(|item| item.code == "weekly_kcal_average_above_reference")
        );

        assert!(
            validate_energy_budget(
                &plan,
                &profile,
                &[item("regular", 2800), item("rollover", 10000)]
            )
            .is_empty(),
            "the rollover lunch must not affect the seven-day budget"
        );
    }
}
