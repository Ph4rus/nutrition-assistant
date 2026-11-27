use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{
    CanonicalShoppingLine, MealSlot, MealiePlanEntry, NutrientProfile, RecipeProposal, Unit,
    WeeklyPlan,
};
use crate::nutrition::RecipeNutrition;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationConflict {
    pub date: NaiveDate,
    pub entry_type: String,
    pub existing_entry_id: String,
}

#[async_trait]
pub trait MealieReader: Send + Sync {
    async fn mealplans(
        &self,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<MealiePlanEntry>>;
}

pub struct MealieReadClient {
    base_url: String,
    api_token: String,
    client: reqwest::Client,
    max_attempts: usize,
}
impl MealieReadClient {
    pub fn new(base_url: String, api_token: String) -> Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').into(),
            api_token,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()?,
            max_attempts: 3,
        })
    }

    async fn mutate(&self, method: reqwest::Method, path: &str, body: Value) -> Result<Value> {
        let response = self
            .client
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Provider(format!(
                "Mealie mutation returned HTTP {}",
                response.status()
            )));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| Error::Provider("Mealie mutation returned malformed JSON".into()))
    }

    async fn read_one(&self, path: &str) -> Result<Value> {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Provider(format!(
                "Mealie readback returned HTTP {}",
                response.status()
            )));
        }
        response
            .json::<Value>()
            .await
            .map_err(|_| Error::Provider("Mealie readback returned malformed JSON".into()))
    }

    pub async fn shopping_lists(&self) -> Result<Vec<MealieShoppingList>> {
        let payload = self
            .read_one("/api/households/shopping/lists?perPage=100")
            .await?;
        Ok(payload
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some(MealieShoppingList {
                    id: item.get("id")?.as_str()?.into(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    run_key: item
                        .get("extras")
                        .and_then(|extras| extras.get("nutritionAssistantRunKey"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect())
    }

    pub async fn create_recipe_shell(&self, recipe: &RecipeProposal) -> Result<String> {
        let payload = self
            .mutate(
                reqwest::Method::POST,
                "/api/recipes",
                serde_json::json!({"name": recipe.title}),
            )
            .await?;
        payload
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Provider("Mealie recipe creation returned no slug".into()))
    }

    pub async fn update_recipe(
        &self,
        slug: &str,
        recipe: &RecipeProposal,
        nutrition: &RecipeNutrition,
        run_key: &str,
    ) -> Result<String> {
        let ingredients: Vec<_> = recipe
            .ingredients
            .iter()
            .map(|ingredient| {
                serde_json::json!({
                    "quantity": 0,
                    "display": format_quantity(
                        ingredient.quantity.amount,
                        ingredient.quantity.unit,
                        &ingredient.name,
                        ingredient.preparation_note.as_deref(),
                    ),
                    "note": ingredient.preparation_note,
                })
            })
            .collect();
        let instructions: Vec<_> = recipe
            .instructions
            .iter()
            .map(|text| serde_json::json!({"text": text}))
            .collect();
        let payload = self
            .mutate(
                reqwest::Method::PUT,
                &format!("/api/recipes/{slug}"),
                serde_json::json!({
                    "name": recipe.title,
                    "recipeServings": decimal_number(recipe.servings),
                    "prepTimeSeconds": u64::from(recipe.active_minutes) * 60,
                    "recipeIngredient": ingredients,
                    "recipeInstructions": instructions,
                    "nutrition": nutrient_payload(&nutrition.per_serving),
                    "extras": {
                        "nutritionAssistantRunKey": run_key,
                        "nutritionAssistantRecipeId": recipe.recipe_id,
                        "nutritionMethod": nutrition.method,
                    }
                }),
            )
            .await?;
        payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Provider("Mealie recipe update returned no ID".into()))
    }

    pub async fn create_mealplan(
        &self,
        meal: &MealSlot,
        remote_recipe_id: &str,
        recipe_servings: rust_decimal::Decimal,
    ) -> Result<String> {
        let payload = self
            .mutate(
                reqwest::Method::POST,
                "/api/households/mealplans",
                serde_json::json!({
                    "date": meal.date,
                    "entryType": meal.meal_type.as_str(),
                    "recipeId": remote_recipe_id,
                    "recipeScale": decimal_number(meal.servings / recipe_servings),
                }),
            )
            .await?;
        payload
            .get("id")
            .and_then(|value| value.as_i64().map(|id| id.to_string()))
            .ok_or_else(|| Error::Provider("Mealie mealplan creation returned no ID".into()))
    }

    pub async fn create_shopping_list(&self, name: &str, run_key: &str) -> Result<String> {
        let payload = self
            .mutate(
                reqwest::Method::POST,
                "/api/households/shopping/lists",
                serde_json::json!({
                    "name": name,
                    "extras": {"nutritionAssistantRunKey": run_key},
                }),
            )
            .await?;
        payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Provider("Mealie shopping list creation returned no ID".into()))
    }

    pub async fn create_shopping_item(
        &self,
        list_id: &str,
        line: &CanonicalShoppingLine,
        run_key: &str,
    ) -> Result<String> {
        let payload = self
            .mutate(
                reqwest::Method::POST,
                "/api/households/shopping/items",
                serde_json::json!({
                    "shoppingListId": list_id,
                    "quantity": 0,
                    "display": format_quantity(line.amount, line.unit, &line.display_name, None),
                    "note": format_quantity(line.amount, line.unit, &line.display_name, None),
                    "extras": {
                        "nutritionAssistantRunKey": run_key,
                        "nutritionAssistantFoodId": line.food_id,
                    }
                }),
            )
            .await?;
        payload
            .get("createdItems")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Provider("Mealie shopping item creation returned no ID".into()))
    }

    pub async fn verify_recipe(&self, slug: &str, run_key: &str) -> Result<()> {
        let payload = self.read_one(&format!("/api/recipes/{slug}")).await?;
        verify_run_key(&payload, run_key, "recipe")
    }

    pub async fn verify_shopping_list(&self, id: &str, run_key: &str) -> Result<()> {
        let payload = self
            .read_one(&format!("/api/households/shopping/lists/{id}"))
            .await?;
        verify_run_key(&payload, run_key, "shopping list")
    }

    pub async fn verify_shopping_item(&self, id: &str, run_key: &str) -> Result<()> {
        let payload = self
            .read_one(&format!("/api/households/shopping/items/{id}"))
            .await?;
        verify_run_key(&payload, run_key, "shopping item")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MealieShoppingList {
    pub id: String,
    pub name: String,
    pub run_key: Option<String>,
}

fn decimal_number(value: rust_decimal::Decimal) -> Value {
    value
        .to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(value.to_string()))
}

fn unit_name(value: Unit) -> &'static str {
    match value {
        Unit::Gram => "g",
        Unit::Kilogram => "kg",
        Unit::Milliliter => "ml",
        Unit::Liter => "l",
        Unit::Piece => "Stück",
    }
}

fn format_quantity(
    amount: rust_decimal::Decimal,
    unit: Unit,
    name: &str,
    note: Option<&str>,
) -> String {
    let mut display = format!("{} {} {name}", amount.normalize(), unit_name(unit));
    if let Some(note) = note.filter(|note| !note.is_empty()) {
        display.push_str(&format!(" ({note})"));
    }
    display
}

fn nutrient_payload(value: &NutrientProfile) -> Value {
    serde_json::json!({
        "calories": format!("{} kcal", value.energy_kcal),
        "proteinContent": format!("{} g", value.protein_g),
        "carbohydrateContent": format!("{} g", value.carbohydrate_g),
        "fatContent": format!("{} g", value.fat_g),
        "fiberContent": format!("{} g", value.fiber_g),
    })
}

fn verify_run_key(payload: &Value, run_key: &str, object: &str) -> Result<()> {
    if payload
        .get("extras")
        .and_then(|extras| extras.get("nutritionAssistantRunKey"))
        .and_then(Value::as_str)
        == Some(run_key)
    {
        Ok(())
    } else {
        Err(Error::Provider(format!(
            "Mealie {object} readback lost ownership marker"
        )))
    }
}
#[async_trait]
impl MealieReader for MealieReadClient {
    async fn mealplans(
        &self,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<MealiePlanEntry>> {
        let mut last = String::new();
        let mut payload = None;
        for _ in 0..self.max_attempts {
            match self
                .client
                .get(format!("{}/api/households/mealplans", self.base_url))
                .bearer_auth(&self.api_token)
                .query(&[
                    ("start_date", start_date.to_string()),
                    ("end_date", end_date.to_string()),
                    ("perPage", "100".into()),
                ])
                .send()
                .await
            {
                Ok(response) if response.status().is_server_error() => {
                    last = format!("HTTP {}", response.status())
                }
                Ok(response) if !response.status().is_success() => {
                    return Err(Error::Provider(format!(
                        "Mealie returned HTTP {}",
                        response.status()
                    )));
                }
                Ok(response) => {
                    payload =
                        Some(response.json::<Value>().await.map_err(|_| {
                            Error::Provider("Mealie returned malformed JSON".into())
                        })?);
                    break;
                }
                Err(error) if error.is_timeout() || error.is_connect() => last = error.to_string(),
                Err(error) => return Err(Error::Provider(error.to_string())),
            }
        }
        let payload = payload.ok_or_else(|| {
            Error::Provider(format!(
                "Mealie request failed after bounded retries: {last}"
            ))
        })?;
        Ok(payload
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some(MealiePlanEntry {
                    entry_id: value_id(item.get("id")?)?,
                    date: NaiveDate::parse_from_str(item.get("date")?.as_str()?, "%Y-%m-%d")
                        .ok()?,
                    entry_type: item
                        .get("entryType")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    recipe_id: item
                        .get("recipeId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    title: item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                })
            })
            .collect())
    }
}

fn value_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
}

pub fn find_destination_conflicts(
    plan: &WeeklyPlan,
    existing: &[MealiePlanEntry],
) -> Vec<DestinationConflict> {
    let mut conflicts = Vec::new();
    for meal in &plan.meals {
        if let Some(entry) = existing
            .iter()
            .find(|v| v.date == meal.date && v.entry_type == meal.meal_type.as_str())
        {
            conflicts.push(DestinationConflict {
                date: meal.date,
                entry_type: meal.meal_type.as_str().into(),
                existing_entry_id: entry.entry_id.clone(),
            });
        }
    }
    conflicts.sort_by(|a, b| (&a.date, &a.entry_type).cmp(&(&b.date, &b.entry_type)));
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn reader_uses_v3_household_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/households/mealplans"))
            .and(header("authorization", "Bearer synthetic-token"))
            .and(query_param("start_date", "2026-11-24"))
            .and(query_param("end_date", "2026-12-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[{
                "id":"entry-1","date":"2026-11-24","entryType":"dinner","recipeId":"remote-recipe"}]})))
            .expect(1)
            .mount(&server)
            .await;
        let client = MealieReadClient::new(server.uri(), "synthetic-token".into()).unwrap();
        let entries = client
            .mealplans(
                NaiveDate::from_ymd_opt(2026, 11, 24).unwrap(),
                NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(entries[0].entry_type, "dinner");
    }
}
