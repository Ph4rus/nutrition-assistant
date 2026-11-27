use std::collections::HashSet;

use chrono::{Datelike, Duration, NaiveDate, Weekday as ChronoWeekday};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const PLANNER_SCHEMA_VERSION: &str = "7";

fn default_true() -> bool {
    true
}
fn default_language() -> String {
    "de".into()
}
fn default_profile_id() -> String {
    "primary".into()
}
fn default_timezone() -> String {
    "Europe/Berlin".into()
}
fn default_weekday_minutes() -> u32 {
    20
}
fn default_weekend_minutes() -> u32 {
    120
}
fn default_breakfast_minutes() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodyProfile {
    pub age: Option<u32>,
    pub sex: Option<String>,
    pub height_cm: Option<Decimal>,
    pub weight_kg: Option<Decimal>,
    pub activity: Option<String>,
    pub target_weight_kg: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NutritionPreferences {
    pub provisional_daily_kcal_reference: Option<Decimal>,
    pub clinician_review_recommended: bool,
    pub automatic_target_adjustment: bool,
    pub prioritize_satiety: bool,
    pub prefer_large_portions: bool,
}

impl Default for NutritionPreferences {
    fn default() -> Self {
        Self {
            provisional_daily_kcal_reference: None,
            clinician_review_recommended: true,
            automatic_target_adjustment: false,
            prioritize_satiety: true,
            prefer_large_portions: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DairyPreferences {
    pub milk: Option<String>,
    pub cream: Option<String>,
    pub butter: Option<String>,
    pub most_other_dairy: Option<String>,
    pub soy_alternatives: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StringOrList {
    String(String),
    List(Vec<String>),
}

impl StringOrList {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::String(value) if value.eq_ignore_ascii_case("unknown"))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FoodPreferences {
    pub cuisines: Vec<String>,
    pub likes: Vec<String>,
    pub vegetables: Vec<String>,
    pub dislikes: Vec<String>,
    pub dairy: Option<DairyPreferences>,
    pub eggs: Option<StringOrList>,
    pub allergies: Option<StringOrList>,
    pub breakfast: Vec<String>,
    pub drinks: Vec<String>,
    pub current_pattern: Option<String>,
    pub equipment: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LearnedPreferences {
    pub avoid_ingredients: Vec<String>,
    pub prefer_ingredients: Vec<String>,
    pub avoid_dishes: Vec<String>,
    pub prefer_dishes: Vec<String>,
    pub less_often: Vec<String>,
    pub more_often: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub const fn iso_number(self) -> u32 {
        match self {
            Self::Monday => 1,
            Self::Tuesday => 2,
            Self::Wednesday => 3,
            Self::Thursday => 4,
            Self::Friday => 5,
            Self::Saturday => 6,
            Self::Sunday => 7,
        }
    }
    pub const fn chrono(self) -> ChronoWeekday {
        match self {
            Self::Monday => ChronoWeekday::Mon,
            Self::Tuesday => ChronoWeekday::Tue,
            Self::Wednesday => ChronoWeekday::Wed,
            Self::Thursday => ChronoWeekday::Thu,
            Self::Friday => ChronoWeekday::Fri,
            Self::Saturday => ChronoWeekday::Sat,
            Self::Sunday => ChronoWeekday::Sun,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulePreferences {
    pub proposed_day: Option<Weekday>,
    pub proposed_time: Option<String>,
    pub timezone: String,
    pub confirmed: bool,
}
impl Default for SchedulePreferences {
    fn default() -> Self {
        Self {
            proposed_day: None,
            proposed_time: None,
            timezone: default_timezone(),
            confirmed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PlanningPreferences {
    pub weekday_max_active_minutes: u32,
    pub weekend_max_active_minutes: u32,
    pub breakfast_max_morning_minutes: u32,
    pub lunch_from_previous_dinner: bool,
    pub emergency_meals_minimum: u32,
    pub household_size: Option<u32>,
    pub shopping_day: Option<Weekday>,
    pub schedule: SchedulePreferences,
}
impl Default for PlanningPreferences {
    fn default() -> Self {
        Self {
            weekday_max_active_minutes: default_weekday_minutes(),
            weekend_max_active_minutes: default_weekend_minutes(),
            breakfast_max_morning_minutes: default_breakfast_minutes(),
            lunch_from_previous_dinner: true,
            emergency_meals_minimum: 0,
            household_size: None,
            shopping_day: None,
            schedule: SchedulePreferences::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NutritionProfile {
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub private: bool,
    pub goal: String,
    pub body: Option<BodyProfile>,
    pub nutrition: NutritionPreferences,
    pub preferences: FoodPreferences,
    #[serde(default)]
    pub learned: LearnedPreferences,
    pub planning: PlanningPreferences,
}

impl NutritionProfile {
    pub fn validate(&self) -> Result<()> {
        if self.profile_id.is_empty() || self.goal.is_empty() {
            return Err(Error::Validation(
                "profile ID and goal must not be empty".into(),
            ));
        }
        if self.nutrition.automatic_target_adjustment {
            return Err(Error::Validation(
                "automatic target adjustment is not allowed".into(),
            ));
        }
        if self
            .nutrition
            .provisional_daily_kcal_reference
            .is_some_and(|value| value <= Decimal::ZERO)
        {
            return Err(Error::Validation(
                "daily kcal reference must be positive".into(),
            ));
        }
        if let Some(body) = &self.body
            && (body.age == Some(0)
                || [body.height_cm, body.weight_kg, body.target_weight_kg]
                    .into_iter()
                    .flatten()
                    .any(|value| value <= Decimal::ZERO))
        {
            return Err(Error::Validation(
                "body measurements must be positive".into(),
            ));
        }
        if self.planning.weekday_max_active_minutes == 0
            || self.planning.weekend_max_active_minutes == 0
            || self.planning.breakfast_max_morning_minutes == 0
        {
            return Err(Error::Validation("time limits must be positive".into()));
        }
        if self.planning.household_size == Some(0) {
            return Err(Error::Validation("household size must be positive".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
pub enum Unit {
    #[serde(rename = "g")]
    #[schemars(rename = "g")]
    Gram,
    #[serde(rename = "kg")]
    #[schemars(rename = "kg")]
    Kilogram,
    #[serde(rename = "ml")]
    #[schemars(rename = "ml")]
    Milliliter,
    #[serde(rename = "l")]
    #[schemars(rename = "l")]
    Liter,
    #[serde(rename = "piece")]
    #[schemars(rename = "piece")]
    Piece,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum UnitDimension {
    Mass,
    Volume,
    Count,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub amount: Decimal,
    pub unit: Unit,
}

impl Quantity {
    pub fn validate(&self) -> Result<()> {
        if self.amount <= Decimal::ZERO {
            Err(Error::Validation("quantity must be positive".into()))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FoodMassConversion {
    pub density_g_per_ml: Option<Decimal>,
    pub grams_per_piece: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IngredientProposal {
    pub name: String,
    pub quantity: Quantity,
    pub preparation_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecipeProposal {
    pub recipe_id: String,
    pub title: String,
    pub servings: Decimal,
    pub active_minutes: u32,
    #[serde(default)]
    pub cuisine_tags: Vec<String>,
    pub ingredients: Vec<IngredientProposal>,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NutrientProfile {
    pub energy_kcal: Decimal,
    pub protein_g: Decimal,
    pub carbohydrate_g: Decimal,
    pub fat_g: Decimal,
    pub fiber_g: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedIngredient {
    pub name: String,
    pub food_id: String,
    pub source_id: String,
    pub source_version: String,
    pub edible_grams: Decimal,
    pub nutrients_per_100g: NutrientProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedRecipe {
    pub recipe_id: String,
    pub servings: Decimal,
    pub ingredients: Vec<ResolvedIngredient>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
pub enum MealType {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

impl MealType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Breakfast => "breakfast",
            Self::Lunch => "lunch",
            Self::Dinner => "dinner",
            Self::Snack => "snack",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MealSlot {
    pub slot_id: String,
    pub date: NaiveDate,
    pub meal_type: MealType,
    pub recipe_id: String,
    pub servings: Decimal,
    pub leftover_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WeeklyPlan {
    pub iso_year: i32,
    pub iso_week: u32,
    pub start_date: NaiveDate,
    pub timezone: String,
    pub recipes: Vec<RecipeProposal>,
    pub meals: Vec<MealSlot>,
}

impl WeeklyPlan {
    pub fn validate(&self) -> Result<()> {
        if self.iso_year < 2000 || self.iso_week == 0 || self.iso_week > 53 {
            return Err(Error::Validation("invalid ISO week".into()));
        }
        let iso = self.start_date.iso_week();
        if (iso.year(), iso.week()) != (self.iso_year, self.iso_week) {
            return Err(Error::Validation(
                "start date must belong to the target ISO week".into(),
            ));
        }
        if self.timezone.is_empty() || self.recipes.is_empty() || self.meals.is_empty() {
            return Err(Error::Validation(
                "plan must contain timezone, recipes and meals".into(),
            ));
        }
        let recipe_ids: HashSet<_> = self.recipes.iter().map(|r| r.recipe_id.as_str()).collect();
        if recipe_ids.len() != self.recipes.len() {
            return Err(Error::Validation("recipe IDs must be unique".into()));
        }
        let slot_ids: HashSet<_> = self.meals.iter().map(|m| m.slot_id.as_str()).collect();
        if slot_ids.len() != self.meals.len() {
            return Err(Error::Validation("meal slot IDs must be unique".into()));
        }
        let cycle_end = self.start_date + Duration::days(6);
        let rollover = self.start_date + Duration::days(7);
        for recipe in &self.recipes {
            if recipe.recipe_id.is_empty()
                || recipe.title.is_empty()
                || recipe.servings <= Decimal::ZERO
                || recipe.active_minutes == 0
                || recipe.ingredients.is_empty()
                || recipe.instructions.is_empty()
            {
                return Err(Error::Validation(
                    "recipe fields must be non-empty and positive".into(),
                ));
            }
            for ingredient in &recipe.ingredients {
                if ingredient.name.is_empty() {
                    return Err(Error::Validation(
                        "ingredient name must not be empty".into(),
                    ));
                }
                ingredient.quantity.validate()?;
            }
        }
        for meal in &self.meals {
            if !recipe_ids.contains(meal.recipe_id.as_str()) {
                return Err(Error::Validation(format!(
                    "meal references unknown recipe: {}",
                    meal.recipe_id
                )));
            }
            if let Some(origin) = &meal.leftover_of
                && !slot_ids.contains(origin.as_str())
            {
                return Err(Error::Validation(format!(
                    "leftover references unknown meal slot: {origin}"
                )));
            }
            let in_cycle = meal.date >= self.start_date && meal.date <= cycle_end;
            if !in_cycle && !(meal.date == rollover && meal.leftover_of.is_some()) {
                return Err(Error::Validation(
                    "meal date is outside the target planning cycle".into(),
                ));
            }
            if meal.servings <= Decimal::ZERO {
                return Err(Error::Validation("meal servings must be positive".into()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DinnerLeftoverAllocation {
    pub recipe_id: String,
    pub recipe_servings: Decimal,
    pub dinner: MealSlot,
    pub next_day_lunch: MealSlot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShoppingLine {
    pub food_id: String,
    pub display_name: String,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalShoppingLine {
    pub food_id: String,
    pub display_name: String,
    pub dimension: UnitDimension,
    pub amount: Decimal,
    pub unit: Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunIdentity {
    pub profile_id: String,
    pub iso_year: i32,
    pub iso_week: u32,
    pub profile_version: String,
    #[serde(default = "default_schema_version")]
    pub planner_schema_version: String,
}
fn default_schema_version() -> String {
    PLANNER_SCHEMA_VERSION.into()
}

impl RunIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.profile_id.is_empty()
            || self.profile_version.is_empty()
            || NaiveDate::from_isoywd_opt(self.iso_year, self.iso_week, ChronoWeekday::Mon)
                .is_none()
        {
            Err(Error::Validation("invalid run identity".into()))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Drafting,
    NeedsReview,
    Validated,
    Publishing,
    Published,
    Failed,
    Partial,
}
impl RunState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drafting => "drafting",
            Self::NeedsReview => "needs_review",
            Self::Validated => "validated",
            Self::Publishing => "publishing",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "drafting" => Ok(Self::Drafting),
            "needs_review" => Ok(Self::NeedsReview),
            "validated" => Ok(Self::Validated),
            "publishing" => Ok(Self::Publishing),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "partial" => Ok(Self::Partial),
            _ => Err(Error::Validation("unknown run state".into())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRecord {
    pub run_key: String,
    pub identity: RunIdentity,
    pub state: RunState,
    pub input_hash: String,
    pub provider_model: Option<String>,
    pub failure_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteObject {
    pub object_type: String,
    pub remote_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FoodSourceRecord {
    pub source_id: String,
    pub source_version: String,
    pub description: String,
    pub data_type: String,
    pub nutrients_per_100g: NutrientProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FoodSearchCandidate {
    pub source_id: String,
    pub description: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MealiePlanEntry {
    pub entry_id: String,
    pub date: NaiveDate,
    pub entry_type: String,
    pub recipe_id: Option<String>,
    #[serde(default)]
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> RecipeProposal {
        RecipeProposal {
            recipe_id: "r".into(),
            title: "Bowl".into(),
            servings: Decimal::ONE,
            active_minutes: 10,
            cuisine_tags: vec![],
            ingredients: vec![IngredientProposal {
                name: "rice".into(),
                quantity: Quantity {
                    amount: Decimal::from(100),
                    unit: Unit::Gram,
                },
                preparation_note: None,
            }],
            instructions: vec!["Cook.".into()],
        }
    }

    #[test]
    fn tuesday_cycle_includes_following_monday() {
        let start = NaiveDate::from_ymd_opt(2026, 12, 29).unwrap();
        let plan = WeeklyPlan {
            iso_year: 2026,
            iso_week: 53,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe()],
            meals: vec![MealSlot {
                slot_id: "m".into(),
                date: start + Duration::days(6),
                meal_type: MealType::Lunch,
                recipe_id: "r".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_recipe_and_outside_date() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let mut plan = WeeklyPlan {
            iso_year: 2026,
            iso_week: 48,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe()],
            meals: vec![MealSlot {
                slot_id: "m".into(),
                date: start,
                meal_type: MealType::Dinner,
                recipe_id: "unknown".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        assert!(plan.validate().is_err());
        plan.meals[0].recipe_id = "r".into();
        plan.meals[0].date = start + Duration::days(8);
        assert!(plan.validate().is_err());
    }

    #[test]
    fn strict_profile_rejects_unknown_fields() {
        let yaml = "goal: x\nnutrition: {}\npreferences: {}\nplanning: {}\nunexpected: true\n";
        assert!(serde_yaml::from_str::<NutritionProfile>(yaml).is_err());
    }
}
