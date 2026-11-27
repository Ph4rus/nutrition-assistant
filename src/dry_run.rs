use chrono::Duration;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::ai::{WeeklyPlanGenerator, planning_context};
use crate::catalog::{CuratedFoodCatalog, FoodDataProvider};
use crate::mealie::{DestinationConflict, MealieReader, find_destination_conflicts};
use crate::models::{
    CanonicalShoppingLine, NutritionProfile, ResolvedRecipe, RunIdentity, RunState, ShoppingLine,
    WeeklyPlan,
};
use crate::nutrition::{RecipeNutrition, calculate_recipe_nutrition};
use crate::profile::profile_version;
use crate::shopping::consolidate_shopping;
use crate::storage::RunLedger;
use crate::validation::{
    IssueSeverity, ValidationIssue, validate_energy_budget, validate_profile_readiness,
    validate_weekly_plan,
};
use crate::{Error, Result};

const ARTIFACT_TYPE: &str = "dry_run_preview_v7";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DryRunPreview {
    pub run_key: String,
    pub state: RunState,
    pub plan: Option<WeeklyPlan>,
    pub issues: Vec<ValidationIssue>,
    pub nutrition: Vec<RecipeNutrition>,
    pub shopping: Vec<CanonicalShoppingLine>,
    pub destination_conflicts: Vec<DestinationConflict>,
    pub writes_performed: u8,
}

pub struct PlanEvaluation {
    pub issues: Vec<ValidationIssue>,
    pub nutrition: Vec<RecipeNutrition>,
    pub shopping: Vec<CanonicalShoppingLine>,
}

pub async fn evaluate_plan(
    plan: &WeeklyPlan,
    profile: &NutritionProfile,
    catalog: &CuratedFoodCatalog,
    food_provider: &dyn FoodDataProvider,
) -> Result<PlanEvaluation> {
    plan.validate()?;
    let mut issues = validate_weekly_plan(plan, profile);
    let mut nutrition = Vec::new();
    let mut shopping_lines = Vec::new();
    for recipe in &plan.recipes {
        let mut resolved = Vec::new();
        for (index, ingredient) in recipe.ingredients.iter().enumerate() {
            match catalog.resolve(ingredient, food_provider).await {
                Ok(item) => {
                    resolved.push(item);
                    let entry = catalog.entry_for(&ingredient.name)?;
                    shopping_lines.push(ShoppingLine {
                        food_id: entry.food_id.clone(),
                        display_name: ingredient.name.clone(),
                        quantity: ingredient.quantity.clone(),
                    });
                }
                Err(Error::CatalogResolution(_) | Error::UnitConversion(_)) => {
                    issues.push(ValidationIssue {
                        severity: IssueSeverity::Blocker,
                        code: "ingredient_unresolved".into(),
                        path: format!("recipes.{}.ingredients.{index}", recipe.recipe_id),
                        message: "Ingredient has no usable curated nutrition mapping.".into(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        if resolved.len() == recipe.ingredients.len() {
            nutrition.push(calculate_recipe_nutrition(&ResolvedRecipe {
                recipe_id: recipe.recipe_id.clone(),
                servings: recipe.servings,
                ingredients: resolved,
            })?);
        }
    }
    issues.extend(validate_energy_budget(plan, profile, &nutrition));
    Ok(PlanEvaluation {
        issues,
        nutrition,
        shopping: consolidate_shopping(&shopping_lines, &[]),
    })
}

fn input_hash(
    profile: &NutritionProfile,
    iso_year: i32,
    iso_week: u32,
    provider_model: &str,
    catalog: &CuratedFoodCatalog,
    nutrition_source_fingerprint: &str,
) -> Result<String> {
    let payload = json!({"planning_context":planning_context(profile,iso_year,iso_week,&catalog.generation_names())?,"provider_model":provider_model,
        "catalog_version":catalog.version(),"nutrition_source_fingerprint":nutrition_source_fingerprint,"prompt_version":"7"});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&payload)?)
    ))
}

pub struct DryRunPlanner<'a> {
    pub generator: &'a dyn WeeklyPlanGenerator,
    pub food_provider: &'a dyn FoodDataProvider,
    pub catalog: &'a CuratedFoodCatalog,
    pub ledger: &'a mut RunLedger,
    pub mealie: Option<&'a dyn MealieReader>,
}

impl DryRunPlanner<'_> {
    pub async fn run(
        &mut self,
        profile: &NutritionProfile,
        iso_year: i32,
        iso_week: u32,
    ) -> Result<DryRunPreview> {
        let provider_model = self.generator.provider_model();
        let identity = RunIdentity {
            profile_id: profile.profile_id.clone(),
            iso_year,
            iso_week,
            profile_version: profile_version(profile)?,
            planner_schema_version: crate::models::PLANNER_SCHEMA_VERSION.into(),
        };
        let hash = input_hash(
            profile,
            iso_year,
            iso_week,
            &provider_model,
            self.catalog,
            self.food_provider.source_fingerprint(),
        )?;
        let run = self
            .ledger
            .begin_run(&identity, &hash, Some(&provider_model))?;
        if let Some(stored) = self.ledger.get_artifact(&run.run_key, ARTIFACT_TYPE)? {
            return Ok(serde_json::from_str(&stored)?);
        }
        if run.state != RunState::Drafting {
            return Err(Error::RunIdentityConflict(
                "non-drafting run is missing its dry-run artifact".into(),
            ));
        }
        let readiness = validate_profile_readiness(profile);
        if readiness
            .iter()
            .any(|v| v.severity == IssueSeverity::Blocker)
        {
            let final_run = self
                .ledger
                .transition(&run.run_key, RunState::NeedsReview, None)?;
            let preview = DryRunPreview {
                run_key: run.run_key,
                state: final_run.state,
                plan: None,
                issues: readiness,
                nutrition: vec![],
                shopping: vec![],
                destination_conflicts: vec![],
                writes_performed: 0,
            };
            self.ledger.put_artifact(
                &preview.run_key,
                ARTIFACT_TYPE,
                &serde_json::to_string(&preview)?,
            )?;
            return Ok(preview);
        }
        let result = self
            .generate_preview(profile, iso_year, iso_week, &run.run_key)
            .await;
        if let Err(error) = &result {
            self.ledger
                .transition(&run.run_key, RunState::Failed, Some(error_code(error)))?;
        }
        result
    }
    async fn generate_preview(
        &mut self,
        profile: &NutritionProfile,
        iso_year: i32,
        iso_week: u32,
        run_key: &str,
    ) -> Result<DryRunPreview> {
        let allowed_ingredient_names = self.catalog.generation_names();
        let plan = self
            .generator
            .generate(profile, iso_year, iso_week, &allowed_ingredient_names)
            .await?;
        let evaluation = evaluate_plan(&plan, profile, self.catalog, self.food_provider).await?;
        let mut issues = evaluation.issues;
        let mut conflicts = Vec::new();
        if let Some(mealie) = self.mealie {
            let existing = mealie
                .mealplans(plan.start_date, plan.start_date + Duration::days(7))
                .await?;
            conflicts = find_destination_conflicts(&plan, &existing);
            for conflict in &conflicts {
                issues.push(ValidationIssue {
                    severity: IssueSeverity::Blocker,
                    code: "destination_occupied".into(),
                    path: format!("meals.{}.{}", conflict.date, conflict.entry_type),
                    message: "Mealie already contains an entry in this meal slot.".into(),
                });
            }
        }
        issues
            .sort_by(|a, b| (&a.severity, &a.code, &a.path).cmp(&(&b.severity, &b.code, &b.path)));
        let target = if issues.iter().any(|v| v.severity == IssueSeverity::Blocker) {
            RunState::NeedsReview
        } else {
            RunState::Validated
        };
        let final_run = self.ledger.transition(run_key, target, None)?;
        let preview = DryRunPreview {
            run_key: run_key.into(),
            state: final_run.state,
            plan: Some(plan),
            issues,
            nutrition: evaluation.nutrition,
            shopping: evaluation.shopping,
            destination_conflicts: conflicts,
            writes_performed: 0,
        };
        self.ledger
            .put_artifact(run_key, ARTIFACT_TYPE, &serde_json::to_string(&preview)?)?;
        Ok(preview)
    }
}
fn error_code(error: &Error) -> &'static str {
    match error {
        Error::ProviderRefusal => "ProviderRefusal",
        Error::IncompleteProviderResponse => "IncompleteProviderResponse",
        Error::Provider(_) | Error::Http(_) => "ProviderError",
        Error::CatalogResolution(_) => "CatalogResolutionError",
        Error::NutritionResolution(_) => "NutritionResolutionError",
        Error::Validation(_) => "ValidationError",
        _ => "PlannerError",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use tempfile::tempdir;

    use super::*;
    use crate::catalog::{CatalogDocument, CatalogEntry};
    use crate::models::{
        FoodPreferences, FoodSourceRecord, IngredientProposal, MealSlot, MealType, NutrientProfile,
        NutritionPreferences, PlanningPreferences, Quantity, RecipeProposal, SchedulePreferences,
        StringOrList, Unit, Weekday,
    };

    fn profile(ready: bool) -> NutritionProfile {
        NutritionProfile {
            profile_id: "synthetic".into(),
            language: "de".into(),
            private: true,
            goal: "balanced".into(),
            body: None,
            nutrition: NutritionPreferences::default(),
            preferences: FoodPreferences {
                allergies: ready.then(|| StringOrList::List(vec![])),
                ..Default::default()
            },
            learned: Default::default(),
            planning: PlanningPreferences {
                household_size: ready.then_some(1),
                shopping_day: Some(Weekday::Tuesday),
                schedule: SchedulePreferences {
                    confirmed: ready,
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }
    fn plan() -> WeeklyPlan {
        let start = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let mut meals = Vec::new();
        for offset in 0..7 {
            let date = start + chrono::Duration::days(offset);
            meals.push(MealSlot {
                slot_id: format!("breakfast-{offset}"),
                date,
                meal_type: MealType::Breakfast,
                recipe_id: "recipe-1".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            });
            meals.push(MealSlot {
                slot_id: format!("lunch-{offset}"),
                date,
                meal_type: MealType::Lunch,
                recipe_id: "recipe-1".into(),
                servings: Decimal::ONE,
                leftover_of: (offset > 0).then(|| format!("dinner-{}", offset - 1)),
            });
            meals.push(MealSlot {
                slot_id: format!("dinner-{offset}"),
                date,
                meal_type: MealType::Dinner,
                recipe_id: "recipe-1".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            });
        }
        meals.push(MealSlot {
            slot_id: "rollover-lunch".into(),
            date: start + chrono::Duration::days(7),
            meal_type: MealType::Lunch,
            recipe_id: "recipe-1".into(),
            servings: Decimal::ONE,
            leftover_of: Some("dinner-6".into()),
        });
        WeeklyPlan {
            iso_year: 2026,
            iso_week: 48,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![RecipeProposal {
                recipe_id: "recipe-1".into(),
                title: "Synthetic bowl".into(),
                servings: Decimal::from(22),
                active_minutes: 5,
                cuisine_tags: vec![],
                ingredients: vec![IngredientProposal {
                    name: "synthetic rice".into(),
                    quantity: Quantity {
                        amount: Decimal::from(2200),
                        unit: Unit::Gram,
                    },
                    preparation_note: None,
                }],
                instructions: vec!["Prepare.".into()],
            }],
            meals,
        }
    }
    fn catalog() -> CuratedFoodCatalog {
        CuratedFoodCatalog::new(CatalogDocument {
            foods: vec![CatalogEntry {
                food_id: "synthetic-rice".into(),
                source_id: "123".into(),
                names: vec!["synthetic rice".into()],
                conversion: None,
            }],
        })
        .unwrap()
    }
    struct FakeGenerator {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl WeeklyPlanGenerator for FakeGenerator {
        fn provider_model(&self) -> String {
            "fake:model".into()
        }
        async fn generate(
            &self,
            _profile: &NutritionProfile,
            _iso_year: i32,
            _iso_week: u32,
            allowed_ingredient_names: &[String],
        ) -> Result<WeeklyPlan> {
            assert!(allowed_ingredient_names.contains(&"synthetic rice".to_owned()));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(plan())
        }
    }
    struct FakeFood;
    #[async_trait]
    impl FoodDataProvider for FakeFood {
        fn source_fingerprint(&self) -> &str {
            "synthetic-source-v1"
        }

        async fn get_food(&self, source_id: &str) -> Result<FoodSourceRecord> {
            Ok(FoodSourceRecord {
                source_id: source_id.into(),
                source_version: "v1".into(),
                description: "Synthetic rice".into(),
                data_type: "Foundation".into(),
                nutrients_per_100g: NutrientProfile {
                    energy_kcal: Decimal::from(130),
                    ..Default::default()
                },
            })
        }
    }

    struct FailingGenerator;
    #[async_trait]
    impl WeeklyPlanGenerator for FailingGenerator {
        fn provider_model(&self) -> String {
            "fake:failing".into()
        }
        async fn generate(
            &self,
            _profile: &NutritionProfile,
            _iso_year: i32,
            _iso_week: u32,
            _allowed_ingredient_names: &[String],
        ) -> Result<WeeklyPlan> {
            Err(Error::Provider("synthetic failure".into()))
        }
    }

    #[tokio::test]
    async fn clean_run_is_cached_and_write_free() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("runs.db")).unwrap();
        let generator = FakeGenerator {
            calls: AtomicUsize::new(0),
        };
        let catalog = catalog();
        let first = {
            let mut planner = DryRunPlanner {
                generator: &generator,
                food_provider: &FakeFood,
                catalog: &catalog,
                ledger: &mut ledger,
                mealie: None,
            };
            planner.run(&profile(true), 2026, 48).await.unwrap()
        };
        let repeated = {
            let mut planner = DryRunPlanner {
                generator: &generator,
                food_provider: &FakeFood,
                catalog: &catalog,
                ledger: &mut ledger,
                mealie: None,
            };
            planner.run(&profile(true), 2026, 48).await.unwrap()
        };
        assert_eq!(first.state, RunState::Validated);
        assert_eq!(first.writes_performed, 0);
        assert_eq!(
            first.nutrition[0].per_serving.energy_kcal.round_dp(6),
            Decimal::from(130)
        );
        assert_eq!(first, repeated);
        assert_eq!(generator.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn readiness_blocks_provider_before_generation() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("runs.db")).unwrap();
        let generator = FakeGenerator {
            calls: AtomicUsize::new(0),
        };
        let catalog = catalog();
        let preview = DryRunPlanner {
            generator: &generator,
            food_provider: &FakeFood,
            catalog: &catalog,
            ledger: &mut ledger,
            mealie: None,
        }
        .run(&profile(false), 2026, 48)
        .await
        .unwrap();
        assert_eq!(preview.state, RunState::NeedsReview);
        assert!(preview.plan.is_none());
        assert_eq!(generator.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn provider_failure_is_persisted_with_redacted_code() {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("runs.db")).unwrap();
        let catalog = catalog();
        let error = DryRunPlanner {
            generator: &FailingGenerator,
            food_provider: &FakeFood,
            catalog: &catalog,
            ledger: &mut ledger,
            mealie: None,
        }
        .run(&profile(true), 2026, 48)
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Provider(_)));
        let identity = RunIdentity {
            profile_id: "synthetic".into(),
            iso_year: 2026,
            iso_week: 48,
            profile_version: profile_version(&profile(true)).unwrap(),
            planner_schema_version: crate::models::PLANNER_SCHEMA_VERSION.into(),
        };
        let run = ledger
            .get_run(&crate::storage::build_run_key(&identity).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(run.state, RunState::Failed);
        assert_eq!(run.failure_code.as_deref(), Some("ProviderError"));
        assert!(!run.failure_code.unwrap().contains("synthetic"));
    }
}
