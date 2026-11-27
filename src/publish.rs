use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::approval;
use crate::dry_run::DryRunPreview;
use crate::mealie::{MealieReadClient, MealieReader, find_destination_conflicts};
use crate::models::RunState;
use crate::storage::RunLedger;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishSummary {
    pub run_key: String,
    pub state: RunState,
    pub writes_performed: usize,
}

/// Publishes one validated immutable preview. It never deletes or overwrites remote objects.
pub async fn publish_preview(
    preview: &DryRunPreview,
    ledger: &mut RunLedger,
    mealie: &MealieReadClient,
) -> Result<PublishSummary> {
    let run = ledger
        .get_run(&preview.run_key)?
        .ok_or_else(|| Error::RunIdentityConflict("publish run does not exist".into()))?;
    if run.state == RunState::Published {
        return Ok(PublishSummary {
            run_key: preview.run_key.clone(),
            state: RunState::Published,
            writes_performed: 0,
        });
    }
    if run.state != RunState::Validated
        || preview.state != RunState::Validated
        || !preview.issues.is_empty()
        || preview.writes_performed != 0
    {
        return Err(Error::Validation(
            "only a clean validated dry-run artifact can be published".into(),
        ));
    }
    if !approval::is_approved(preview, ledger)? {
        return Err(Error::Validation(
            "exact preview artifact has not been explicitly approved".into(),
        ));
    }
    if !ledger.remote_objects(&preview.run_key)?.is_empty() {
        return Err(Error::RunIdentityConflict(
            "validated run already owns remote objects; reconcile manually".into(),
        ));
    }
    let plan = preview
        .plan
        .as_ref()
        .ok_or_else(|| Error::Validation("validated preview has no plan".into()))?;
    plan.validate()?;

    // Both destination reads happen before the first state change or remote mutation.
    let existing_meals = mealie
        .mealplans(plan.start_date, plan.start_date + chrono::Duration::days(7))
        .await?;
    let existing_lists = mealie.shopping_lists().await?;
    let conflicts = find_destination_conflicts(plan, &existing_meals);
    let list_name = format!(
        "Nutrition Assistant {}-W{:02}",
        plan.iso_year, plan.iso_week
    );
    let list_conflict = existing_lists.iter().any(|list| list.name == list_name);
    if !conflicts.is_empty() || list_conflict {
        return Err(Error::Validation(
            "Mealie target contains an occupied meal slot or shopping list; zero writes performed"
                .into(),
        ));
    }

    ledger.transition(&preview.run_key, RunState::Publishing, None)?;
    let result = match publish_objects(preview, ledger, mealie, &list_name).await {
        Ok(writes) => verify_readback(preview, ledger, mealie)
            .await
            .map(|()| writes),
        Err(error) => Err(error),
    };
    match result {
        Ok(writes) => {
            let final_run = ledger.transition(&preview.run_key, RunState::Published, None)?;
            Ok(PublishSummary {
                run_key: preview.run_key.clone(),
                state: final_run.state,
                writes_performed: writes,
            })
        }
        Err(error) => {
            ledger.transition(
                &preview.run_key,
                RunState::Partial,
                Some("MealiePublishError"),
            )?;
            Err(error)
        }
    }
}

async fn publish_objects(
    preview: &DryRunPreview,
    ledger: &mut RunLedger,
    mealie: &MealieReadClient,
    list_name: &str,
) -> Result<usize> {
    let plan = preview.plan.as_ref().expect("validated above");
    let nutrition: BTreeMap<_, _> = preview
        .nutrition
        .iter()
        .map(|item| (item.recipe_id.as_str(), item))
        .collect();
    let mut remote_recipes = BTreeMap::new();
    let mut writes = 0;
    for recipe in &plan.recipes {
        let calculated = nutrition.get(recipe.recipe_id.as_str()).ok_or_else(|| {
            Error::Validation(format!(
                "recipe {} has no nutrition record",
                recipe.recipe_id
            ))
        })?;
        let slug = mealie.create_recipe_shell(recipe).await?;
        ledger.record_remote_object(&preview.run_key, "recipe", &slug)?;
        writes += 1;
        let remote_id = mealie
            .update_recipe(&slug, recipe, calculated, &preview.run_key)
            .await?;
        remote_recipes.insert(recipe.recipe_id.as_str(), (slug, remote_id));
    }
    for meal in &plan.meals {
        let recipe = plan
            .recipes
            .iter()
            .find(|recipe| recipe.recipe_id == meal.recipe_id)
            .ok_or_else(|| Error::Validation("meal references unknown recipe".into()))?;
        let (_, remote_id) = remote_recipes
            .get(meal.recipe_id.as_str())
            .ok_or_else(|| Error::Validation("remote recipe mapping is missing".into()))?;
        let id = mealie
            .create_mealplan(meal, remote_id, recipe.servings)
            .await?;
        ledger.record_remote_object(&preview.run_key, "mealplan", &id)?;
        writes += 1;
    }
    let list_id = mealie
        .create_shopping_list(list_name, &preview.run_key)
        .await?;
    ledger.record_remote_object(&preview.run_key, "shopping_list", &list_id)?;
    writes += 1;
    for line in &preview.shopping {
        let id = mealie
            .create_shopping_item(&list_id, line, &preview.run_key)
            .await?;
        ledger.record_remote_object(&preview.run_key, "shopping_item", &id)?;
        writes += 1;
    }
    Ok(writes)
}

async fn verify_readback(
    preview: &DryRunPreview,
    ledger: &RunLedger,
    mealie: &MealieReadClient,
) -> Result<()> {
    let plan = preview.plan.as_ref().expect("validated above");
    let objects = ledger.remote_objects(&preview.run_key)?;
    for object in &objects {
        match object.object_type.as_str() {
            "recipe" => {
                mealie
                    .verify_recipe(&object.remote_id, &preview.run_key)
                    .await?
            }
            "shopping_list" => {
                mealie
                    .verify_shopping_list(&object.remote_id, &preview.run_key)
                    .await?
            }
            "shopping_item" => {
                mealie
                    .verify_shopping_item(&object.remote_id, &preview.run_key)
                    .await?
            }
            "mealplan" => {}
            _ => {
                return Err(Error::RunIdentityConflict(
                    "run owns an unknown remote object type".into(),
                ));
            }
        }
    }
    let remote_meals = mealie
        .mealplans(plan.start_date, plan.start_date + chrono::Duration::days(7))
        .await?;
    let meal_ids: std::collections::BTreeSet<_> = remote_meals
        .iter()
        .map(|entry| entry.entry_id.as_str())
        .collect();
    if objects
        .iter()
        .filter(|object| object.object_type == "mealplan")
        .any(|object| !meal_ids.contains(object.remote_id.as_str()))
    {
        return Err(Error::Provider(
            "Mealie mealplan readback is incomplete".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::models::{
        CanonicalShoppingLine, IngredientProposal, MealSlot, MealType, NutrientProfile, Quantity,
        RecipeProposal, RunIdentity, Unit, UnitDimension, WeeklyPlan,
    };
    use crate::nutrition::RecipeNutrition;

    fn setup() -> (tempfile::TempDir, RunLedger, DryRunPreview) {
        let dir = tempdir().unwrap();
        let mut ledger = RunLedger::open(&dir.path().join("planner.sqlite3")).unwrap();
        let identity = RunIdentity {
            profile_id: "synthetic".into(),
            iso_year: 2026,
            iso_week: 49,
            profile_version: "synthetic-v1".into(),
            planner_schema_version: "4".into(),
        };
        let run = ledger.begin_run(&identity, "hash", Some("test")).unwrap();
        ledger
            .transition(&run.run_key, RunState::Validated, None)
            .unwrap();
        let start = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let recipe = RecipeProposal {
            recipe_id: "recipe-1".into(),
            title: "Synthetic Bowl".into(),
            servings: Decimal::ONE,
            active_minutes: 10,
            cuisine_tags: vec!["synthetic".into()],
            ingredients: vec![IngredientProposal {
                name: "Test food".into(),
                quantity: Quantity {
                    amount: Decimal::from(100),
                    unit: Unit::Gram,
                },
                preparation_note: None,
            }],
            instructions: vec!["Prepare.".into()],
        };
        let preview = DryRunPreview {
            run_key: run.run_key,
            state: RunState::Validated,
            plan: Some(WeeklyPlan {
                iso_year: 2026,
                iso_week: 49,
                start_date: start,
                timezone: "Europe/Berlin".into(),
                recipes: vec![recipe],
                meals: vec![MealSlot {
                    slot_id: "dinner-1".into(),
                    date: start,
                    meal_type: MealType::Dinner,
                    recipe_id: "recipe-1".into(),
                    servings: Decimal::ONE,
                    leftover_of: None,
                }],
            }),
            issues: vec![],
            nutrition: vec![RecipeNutrition {
                recipe_id: "recipe-1".into(),
                servings: Decimal::ONE,
                recipe_total: NutrientProfile::default(),
                per_serving: NutrientProfile::default(),
                trace: vec![],
                method: "ingredient_based_estimate".into(),
            }],
            shopping: vec![CanonicalShoppingLine {
                food_id: "test-food".into(),
                display_name: "Test food".into(),
                dimension: UnitDimension::Mass,
                amount: Decimal::from(100),
                unit: Unit::Gram,
            }],
            destination_conflicts: vec![],
            writes_performed: 0,
        };
        (dir, ledger, preview)
    }

    #[tokio::test]
    async fn occupied_slot_blocks_all_writes() {
        let (_dir, mut ledger, preview) = setup();
        crate::approval::approve(&preview, &mut ledger).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/households/mealplans"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[{
                    "id":"existing","date":"2026-11-30","entryType":"dinner","title":"occupied"
                }]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/households/shopping/lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
            .expect(1)
            .mount(&server)
            .await;
        let client = MealieReadClient::new(server.uri(), "synthetic-token".into()).unwrap();
        assert!(
            publish_preview(&preview, &mut ledger, &client)
                .await
                .is_err()
        );
        assert_eq!(
            ledger.get_run(&preview.run_key).unwrap().unwrap().state,
            RunState::Validated
        );
        assert!(ledger.remote_objects(&preview.run_key).unwrap().is_empty());
    }

    #[tokio::test]
    async fn partial_failure_records_created_recipe_before_stopping() {
        let (_dir, mut ledger, preview) = setup();
        crate::approval::approve(&preview, &mut ledger).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/households/mealplans"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/households/shopping/lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/recipes"))
            .respond_with(ResponseTemplate::new(201).set_body_json("synthetic-bowl"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/recipes/synthetic-bowl"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let client = MealieReadClient::new(server.uri(), "synthetic-token".into()).unwrap();
        assert!(
            publish_preview(&preview, &mut ledger, &client)
                .await
                .is_err()
        );
        let run = ledger.get_run(&preview.run_key).unwrap().unwrap();
        assert_eq!(run.state, RunState::Partial);
        assert_eq!(run.failure_code.as_deref(), Some("MealiePublishError"));
        let objects = ledger.remote_objects(&preview.run_key).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].object_type, "recipe");
        assert_eq!(objects[0].remote_id, "synthetic-bowl");
    }

    #[tokio::test]
    async fn successful_rerun_is_write_free_and_readback_verified() {
        let (_dir, mut ledger, preview) = setup();
        crate::approval::approve(&preview, &mut ledger).unwrap();
        let server = MockServer::start().await;
        // The same response is safe for preflight (unknown entry type) and readback (owned ID).
        Mock::given(method("GET"))
            .and(path("/api/households/mealplans"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"items":[{"id":"7","date":"2026-11-30","entryType":"other"}]}),
            ))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/households/shopping/lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
            .expect(1)
            .mount(&server)
            .await;
        // Re-mount mutations and object readbacks after reset.
        mount_success_routes(&server, &preview.run_key).await;
        let client = MealieReadClient::new(server.uri(), "synthetic-token".into()).unwrap();
        let first = publish_preview(&preview, &mut ledger, &client)
            .await
            .unwrap();
        assert_eq!(first.writes_performed, 4);
        let second = publish_preview(&preview, &mut ledger, &client)
            .await
            .unwrap();
        assert_eq!(second.writes_performed, 0);
        assert_eq!(second.state, RunState::Published);
        assert_eq!(ledger.remote_objects(&preview.run_key).unwrap().len(), 4);
    }

    #[tokio::test]
    async fn unapproved_preview_is_blocked_before_destination_reads() {
        let (_dir, mut ledger, preview) = setup();
        let server = MockServer::start().await;
        let client = MealieReadClient::new(server.uri(), "synthetic-token".into()).unwrap();

        let error = publish_preview(&preview, &mut ledger, &client)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("explicitly approved"));
        assert_eq!(
            ledger.get_run(&preview.run_key).unwrap().unwrap().state,
            RunState::Validated
        );
        assert!(ledger.remote_objects(&preview.run_key).unwrap().is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    async fn mount_success_routes(server: &MockServer, run_key: &str) {
        Mock::given(method("POST"))
            .and(path("/api/recipes"))
            .respond_with(ResponseTemplate::new(201).set_body_json("synthetic-bowl"))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/recipes/synthetic-bowl"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"recipe-remote"})),
            )
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/households/mealplans"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id":7})))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/households/shopping/lists"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({"id":"list-remote"})),
            )
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/households/shopping/items"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(serde_json::json!({"createdItems":[{"id":"item-remote"}]})),
            )
            .expect(1)
            .mount(server)
            .await;
        for path_value in [
            "/api/recipes/synthetic-bowl",
            "/api/households/shopping/lists/list-remote",
            "/api/households/shopping/items/item-remote",
        ] {
            Mock::given(method("GET"))
                .and(path(path_value))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"extras":{"nutritionAssistantRunKey":run_key}}),
                ))
                .expect(1)
                .mount(server)
                .await;
        }
    }
}
