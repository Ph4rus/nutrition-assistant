use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, NaiveDate, Weekday};
use nutrition_assistant::dry_run::DryRunPreview;
use nutrition_assistant::mealie::MealieReadClient;
use nutrition_assistant::models::{
    CanonicalShoppingLine, IngredientProposal, MealSlot, MealType, NutrientProfile, Quantity,
    RecipeProposal, RunIdentity, RunState, Unit, UnitDimension, WeeklyPlan,
};
use nutrition_assistant::nutrition::RecipeNutrition;
use nutrition_assistant::publish::publish_preview;
use nutrition_assistant::storage::RunLedger;
use rust_decimal::Decimal;
use tempfile::tempdir;

#[tokio::test]
#[ignore = "requires an explicitly configured disposable Mealie on loopback"]
async fn publishes_and_reads_back_against_disposable_mealie() {
    let base_url = env::var("MEALIE_DISPOSABLE_URL").expect("MEALIE_DISPOSABLE_URL is required");
    assert!(
        base_url.starts_with("http://127.0.0.1:") || base_url.starts_with("http://localhost:"),
        "live integration test refuses non-loopback Mealie URLs"
    );
    let token = env::var("MEALIE_DISPOSABLE_TOKEN").expect("MEALIE_DISPOSABLE_TOKEN is required");
    let client = MealieReadClient::new(base_url, token).unwrap();

    let dir = tempdir().unwrap();
    let mut ledger = RunLedger::open(&dir.path().join("planner.sqlite3")).unwrap();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let iso_week = 2 + (unique % 51) as u32;
    let identity = RunIdentity {
        profile_id: "synthetic-disposable".into(),
        iso_year: 2098,
        iso_week,
        profile_version: format!("synthetic-{unique}"),
        planner_schema_version: "4".into(),
    };
    let run = ledger
        .begin_run(&identity, "synthetic-live-hash", Some("integration-test"))
        .unwrap();
    ledger
        .transition(&run.run_key, RunState::Validated, None)
        .unwrap();
    let start = NaiveDate::from_isoywd_opt(2098, iso_week, Weekday::Mon).unwrap();
    assert_eq!(
        (start.iso_week().year(), start.iso_week().week()),
        (2098, iso_week)
    );
    let recipe = RecipeProposal {
        recipe_id: "synthetic-bowl".into(),
        title: format!("Synthetic Disposable Bowl {unique}"),
        servings: Decimal::from(2),
        active_minutes: 10,
        cuisine_tags: vec!["synthetic".into()],
        ingredients: vec![IngredientProposal {
            name: "Synthetic rice".into(),
            quantity: Quantity {
                amount: Decimal::from(200),
                unit: Unit::Gram,
            },
            preparation_note: Some("test only".into()),
        }],
        instructions: vec!["Prepare synthetic test food.".into()],
    };
    let preview = DryRunPreview {
        run_key: run.run_key,
        state: RunState::Validated,
        plan: Some(WeeklyPlan {
            iso_year: 2098,
            iso_week,
            start_date: start,
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe],
            meals: vec![MealSlot {
                slot_id: "synthetic-dinner".into(),
                date: start,
                meal_type: MealType::Dinner,
                recipe_id: "synthetic-bowl".into(),
                servings: Decimal::from(2),
                leftover_of: None,
            }],
        }),
        issues: vec![],
        nutrition: vec![RecipeNutrition {
            recipe_id: "synthetic-bowl".into(),
            servings: Decimal::from(2),
            recipe_total: NutrientProfile {
                energy_kcal: Decimal::from(400),
                protein_g: Decimal::from(20),
                carbohydrate_g: Decimal::from(60),
                fat_g: Decimal::from(8),
                fiber_g: Decimal::from(6),
            },
            per_serving: NutrientProfile {
                energy_kcal: Decimal::from(200),
                protein_g: Decimal::from(10),
                carbohydrate_g: Decimal::from(30),
                fat_g: Decimal::from(4),
                fiber_g: Decimal::from(3),
            },
            trace: vec![],
            method: "ingredient_based_estimate".into(),
        }],
        shopping: vec![CanonicalShoppingLine {
            food_id: "synthetic-rice".into(),
            display_name: "Synthetic rice".into(),
            dimension: UnitDimension::Mass,
            amount: Decimal::from(200),
            unit: Unit::Gram,
        }],
        destination_conflicts: vec![],
        writes_performed: 0,
    };

    let first = publish_preview(&preview, &mut ledger, &client)
        .await
        .unwrap();
    assert_eq!(first.state, RunState::Published);
    assert_eq!(first.writes_performed, 4);
    let second = publish_preview(&preview, &mut ledger, &client)
        .await
        .unwrap();
    assert_eq!(second.state, RunState::Published);
    assert_eq!(second.writes_performed, 0);
    assert_eq!(ledger.remote_objects(&preview.run_key).unwrap().len(), 4);
}
