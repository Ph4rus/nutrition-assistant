use serde_json::json;
use sha2::{Digest, Sha256};

use crate::ai::RecipeRefiner;
use crate::catalog::{CuratedFoodCatalog, FoodDataProvider};
use crate::dry_run::{DryRunPreview, evaluate_plan};
use crate::models::{NutritionProfile, RecipeProposal, RunIdentity, RunState, WeeklyPlan};
use crate::profile::profile_version;
use crate::storage::RunLedger;
use crate::validation::IssueSeverity;
use crate::{Error, Result};

const ARTIFACT_TYPE: &str = "recipe_revision_v1";

pub fn replace_recipe(
    plan: &WeeklyPlan,
    target_recipe_id: &str,
    replacement: RecipeProposal,
) -> Result<WeeklyPlan> {
    let existing = plan
        .recipes
        .iter()
        .find(|recipe| recipe.recipe_id == target_recipe_id)
        .ok_or_else(|| Error::Validation("refinement target recipe does not exist".into()))?;
    if replacement.recipe_id != existing.recipe_id {
        return Err(Error::Validation(
            "replacement must preserve recipe_id".into(),
        ));
    }
    if replacement.servings != existing.servings {
        return Err(Error::Validation(
            "replacement must preserve declared servings".into(),
        ));
    }
    if &replacement == existing {
        return Err(Error::Validation("replacement recipe is unchanged".into()));
    }
    let mut revised = plan.clone();
    let target = revised
        .recipes
        .iter_mut()
        .find(|recipe| recipe.recipe_id == target_recipe_id)
        .expect("target checked above");
    *target = replacement;
    revised.validate()?;
    Ok(revised)
}

pub struct RevisionPlanner<'a> {
    pub refiner: &'a dyn RecipeRefiner,
    pub food_provider: &'a dyn FoodDataProvider,
    pub catalog: &'a CuratedFoodCatalog,
    pub ledger: &'a mut RunLedger,
}

impl RevisionPlanner<'_> {
    pub async fn revise(
        &mut self,
        parent: &DryRunPreview,
        profile: &NutritionProfile,
        target_recipe_id: &str,
        instruction: &str,
    ) -> Result<DryRunPreview> {
        if parent.state != RunState::Validated
            || !parent.issues.is_empty()
            || parent.writes_performed != 0
        {
            return Err(Error::Validation(
                "only a clean validated preview can be refined".into(),
            ));
        }
        if instruction.trim().is_empty() {
            return Err(Error::Validation(
                "recipe refinement instruction must not be empty".into(),
            ));
        }
        let plan = parent
            .plan
            .as_ref()
            .ok_or_else(|| Error::Validation("parent preview has no plan".into()))?;
        let recipe = plan
            .recipes
            .iter()
            .find(|recipe| recipe.recipe_id == target_recipe_id)
            .ok_or_else(|| Error::Validation("refinement target recipe does not exist".into()))?;
        let base_profile_version = profile_version(profile)?;
        let effective_profile_version = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&json!({
                "base_profile_version": base_profile_version,
                "parent_run_key": parent.run_key,
                "target_recipe_id": target_recipe_id,
                "instruction": instruction.trim(),
                "revision_contract": "recipe_revision_v1",
            }))?)
        );
        let provider_model = self.refiner.provider_model();
        let input_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&json!({
                "effective_profile_version": effective_profile_version,
                "provider_model": provider_model,
                "catalog_version": self.catalog.version(),
                "nutrition_source": self.food_provider.source_fingerprint(),
            }))?)
        );
        let identity = RunIdentity {
            profile_id: profile.profile_id.clone(),
            iso_year: plan.iso_year,
            iso_week: plan.iso_week,
            profile_version: effective_profile_version,
            planner_schema_version: crate::models::PLANNER_SCHEMA_VERSION.into(),
        };
        let run = self
            .ledger
            .begin_run(&identity, &input_hash, Some(&provider_model))?;
        if let Some(stored) = self.ledger.get_artifact(&run.run_key, ARTIFACT_TYPE)? {
            return Ok(serde_json::from_str(&stored)?);
        }
        if run.state != RunState::Drafting {
            return Err(Error::RunIdentityConflict(
                "non-drafting revision is missing its artifact".into(),
            ));
        }

        let result = self
            .build_revision(parent, profile, recipe, instruction, &run.run_key)
            .await;
        if result.is_err() {
            self.ledger
                .transition(&run.run_key, RunState::Failed, Some("RecipeRevisionError"))?;
        }
        result
    }

    async fn build_revision(
        &mut self,
        parent: &DryRunPreview,
        profile: &NutritionProfile,
        recipe: &RecipeProposal,
        instruction: &str,
        run_key: &str,
    ) -> Result<DryRunPreview> {
        let parent_plan = parent.plan.as_ref().expect("validated above");
        let replacement = self
            .refiner
            .refine(
                profile,
                parent_plan,
                recipe,
                instruction,
                &self.catalog.generation_names(),
            )
            .await?;
        let plan = replace_recipe(parent_plan, &recipe.recipe_id, replacement)?;
        let mut evaluation =
            evaluate_plan(&plan, profile, self.catalog, self.food_provider).await?;
        evaluation
            .issues
            .sort_by(|a, b| (&a.severity, &a.code, &a.path).cmp(&(&b.severity, &b.code, &b.path)));
        let state = if evaluation
            .issues
            .iter()
            .any(|issue| issue.severity == IssueSeverity::Blocker)
        {
            RunState::NeedsReview
        } else {
            RunState::Validated
        };
        let final_run = self.ledger.transition(run_key, state, None)?;
        let preview = DryRunPreview {
            run_key: run_key.into(),
            state: final_run.state,
            plan: Some(plan),
            issues: evaluation.issues,
            nutrition: evaluation.nutrition,
            shopping: evaluation.shopping,
            destination_conflicts: vec![],
            writes_performed: 0,
        };
        self.ledger
            .put_artifact(run_key, ARTIFACT_TYPE, &serde_json::to_string(&preview)?)?;
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::*;
    use crate::models::{IngredientProposal, MealSlot, MealType, Quantity, Unit};

    fn recipe(id: &str, title: &str) -> RecipeProposal {
        RecipeProposal {
            recipe_id: id.into(),
            title: title.into(),
            servings: Decimal::ONE,
            active_minutes: 10,
            cuisine_tags: vec![],
            ingredients: vec![IngredientProposal {
                name: "synthetic".into(),
                quantity: Quantity {
                    amount: Decimal::from(100),
                    unit: Unit::Gram,
                },
                preparation_note: None,
            }],
            instructions: vec!["Prepare.".into()],
        }
    }

    #[test]
    fn replacement_preserves_all_other_recipes_and_meals() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let original = WeeklyPlan {
            iso_year: 2026,
            iso_week: 49,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe("target", "Old"), recipe("locked", "Locked")],
            meals: vec![MealSlot {
                slot_id: "slot".into(),
                date: start,
                meal_type: MealType::Breakfast,
                recipe_id: "target".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        let mut replacement = recipe("target", "New");
        replacement.instructions = vec!["Prepare differently.".into()];
        let revised = replace_recipe(&original, "target", replacement).unwrap();
        assert_eq!(revised.meals, original.meals);
        assert_eq!(revised.recipes[1], original.recipes[1]);
        assert_eq!(revised.recipes[0].title, "New");
    }

    #[test]
    fn replacement_cannot_change_identity_or_servings() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let original = WeeklyPlan {
            iso_year: 2026,
            iso_week: 49,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe("target", "Old")],
            meals: vec![MealSlot {
                slot_id: "slot".into(),
                date: start,
                meal_type: MealType::Breakfast,
                recipe_id: "target".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        let mut wrong = recipe("other", "New");
        assert!(replace_recipe(&original, "target", wrong.clone()).is_err());
        wrong.recipe_id = "target".into();
        wrong.servings = Decimal::from(2);
        assert!(replace_recipe(&original, "target", wrong).is_err());
    }
}
