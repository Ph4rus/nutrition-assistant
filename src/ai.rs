use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::{
    IngredientProposal, MealSlot, MealType, NutritionProfile, Quantity, RecipeProposal, Unit,
    WeeklyPlan,
};
use crate::planning_cycle::cycle_start_for;
use crate::validation::violates_dairy_preference;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiIngredient {
    pub name: String,
    pub amount: String,
    pub unit: AiMassUnit,
    pub preparation_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum AiMassUnit {
    #[serde(rename = "g")]
    #[schemars(rename = "g")]
    Gram,
    #[serde(rename = "kg")]
    #[schemars(rename = "kg")]
    Kilogram,
}

impl From<AiMassUnit> for Unit {
    fn from(value: AiMassUnit) -> Self {
        match value {
            AiMassUnit::Gram => Self::Gram,
            AiMassUnit::Kilogram => Self::Kilogram,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiRecipe {
    pub recipe_id: String,
    pub title: String,
    pub servings: String,
    pub active_minutes: u32,
    pub cuisine_tags: Vec<String>,
    pub ingredients: Vec<AiIngredient>,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiMealSlot {
    pub slot_id: String,
    pub date: String,
    pub meal_type: MealType,
    pub recipe_id: String,
    pub servings: String,
    pub leftover_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiWeeklyPlan {
    pub iso_year: i32,
    pub iso_week: u32,
    pub start_date: String,
    pub timezone: String,
    pub recipes: Vec<AiRecipe>,
    pub meals: Vec<AiMealSlot>,
}

impl TryFrom<AiRecipe> for RecipeProposal {
    type Error = Error;
    fn try_from(recipe: AiRecipe) -> Result<Self> {
        Ok(Self {
            recipe_id: recipe.recipe_id,
            title: recipe.title,
            servings: Decimal::from_str(&recipe.servings)?,
            active_minutes: recipe.active_minutes,
            cuisine_tags: recipe.cuisine_tags,
            ingredients: recipe
                .ingredients
                .into_iter()
                .map(|item| {
                    Ok(IngredientProposal {
                        name: item.name,
                        quantity: Quantity {
                            amount: Decimal::from_str(&item.amount)?,
                            unit: item.unit.into(),
                        },
                        preparation_note: item.preparation_note,
                    })
                })
                .collect::<Result<_>>()?,
            instructions: recipe.instructions,
        })
    }
}

impl TryFrom<AiWeeklyPlan> for WeeklyPlan {
    type Error = Error;
    fn try_from(value: AiWeeklyPlan) -> Result<Self> {
        let plan = Self {
            iso_year: value.iso_year,
            iso_week: value.iso_week,
            start_date: NaiveDate::parse_from_str(&value.start_date, "%Y-%m-%d")?,
            timezone: value.timezone,
            recipes: value
                .recipes
                .into_iter()
                .map(RecipeProposal::try_from)
                .collect::<Result<_>>()?,
            meals: value
                .meals
                .into_iter()
                .map(|meal| {
                    Ok(MealSlot {
                        slot_id: meal.slot_id,
                        date: NaiveDate::parse_from_str(&meal.date, "%Y-%m-%d")?,
                        meal_type: meal.meal_type,
                        recipe_id: meal.recipe_id,
                        servings: Decimal::from_str(&meal.servings)?,
                        leftover_of: meal.leftover_of,
                    })
                })
                .collect::<Result<_>>()?,
        };
        plan.validate()?;
        Ok(plan)
    }
}

pub fn planning_context(
    profile: &NutritionProfile,
    iso_year: i32,
    iso_week: u32,
    allowed_ingredient_names: &[String],
) -> Result<Value> {
    let planning = &profile.planning;
    let dislikes: Vec<_> = profile
        .preferences
        .dislikes
        .iter()
        .chain(profile.learned.avoid_ingredients.iter())
        .map(|value| value.to_lowercase())
        .collect();
    let allowed_ingredient_names: Vec<_> = allowed_ingredient_names
        .iter()
        .filter(|name| {
            let normalized = name.to_lowercase();
            !dislikes.iter().any(|term| normalized.contains(term))
                && !violates_dairy_preference(profile, &normalized)
        })
        .collect();
    let cycle_start = planning
        .shopping_day
        .map(|day| cycle_start_for(iso_year, iso_week, day))
        .transpose()?
        .map(|date| date.to_string());
    Ok(json!({
        "language": profile.language, "goal": profile.goal, "iso_year": iso_year, "iso_week": iso_week,
        "cycle_start_date": cycle_start, "timezone": planning.schedule.timezone,
        "nutrition": { "daily_kcal_reference": profile.nutrition.provisional_daily_kcal_reference.map(|v| v.to_string()),
            "automatic_target_adjustment": false, "prioritize_satiety": profile.nutrition.prioritize_satiety,
            "prefer_large_portions": profile.nutrition.prefer_large_portions },
        "preferences": { "cuisines": profile.preferences.cuisines, "likes": profile.preferences.likes,
            "vegetables": profile.preferences.vegetables, "dislikes": profile.preferences.dislikes,
            "dairy": profile.preferences.dairy,
            "allergies": profile.preferences.allergies, "eggs": profile.preferences.eggs,
            "breakfast": profile.preferences.breakfast, "equipment": profile.preferences.equipment },
        "learned_preferences": profile.learned,
        "allowed_ingredient_names": allowed_ingredient_names,
        "planning": { "household_size": planning.household_size, "weekday_max_active_minutes": planning.weekday_max_active_minutes,
            "weekend_max_active_minutes": planning.weekend_max_active_minutes,
            "breakfast_max_morning_minutes": planning.breakfast_max_morning_minutes,
            "lunch_from_previous_dinner": planning.lunch_from_previous_dinner,
            "emergency_meals_minimum": planning.emergency_meals_minimum, "shopping_day": planning.shopping_day }
    }))
}

#[async_trait]
pub trait WeeklyPlanGenerator: Send + Sync {
    fn provider_model(&self) -> String;
    async fn generate(
        &self,
        profile: &NutritionProfile,
        iso_year: i32,
        iso_week: u32,
        allowed_ingredient_names: &[String],
    ) -> Result<WeeklyPlan>;
}

#[async_trait]
pub trait RecipeRefiner: Send + Sync {
    fn provider_model(&self) -> String;
    async fn refine(
        &self,
        profile: &NutritionProfile,
        plan: &WeeklyPlan,
        recipe: &RecipeProposal,
        instruction: &str,
        allowed_ingredient_names: &[String],
    ) -> Result<RecipeProposal>;
}

pub struct OpenAiWeeklyPlanGenerator {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
    max_attempts: usize,
}

impl OpenAiWeeklyPlanGenerator {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        Self::with_base_url(api_key, model, "https://api.openai.com/v1".into())
    }
    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()?;
        Ok(Self {
            api_key,
            model,
            base_url: base_url.trim_end_matches('/').into(),
            client,
            max_attempts: 3,
        })
    }
    async fn request(&self, body: &Value) -> Result<Value> {
        let mut last = None;
        for _ in 0..self.max_attempts {
            match self
                .client
                .post(format!("{}/responses", self.base_url))
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await
            {
                Ok(response) if response.status().is_server_error() => {
                    last = Some(format!("HTTP {}", response.status()));
                }
                Ok(response) if !response.status().is_success() => {
                    return Err(Error::Provider(format!(
                        "OpenAI returned HTTP {}",
                        response.status()
                    )));
                }
                Ok(response) => {
                    return response
                        .json()
                        .await
                        .map_err(|_| Error::Provider("OpenAI returned malformed JSON".into()));
                }
                Err(error) if error.is_timeout() || error.is_connect() => {
                    last = Some(if error.is_timeout() {
                        "timeout".into()
                    } else {
                        "connect".into()
                    })
                }
                Err(error) => return Err(Error::Provider(error.to_string())),
            }
        }
        Err(Error::Provider(format!(
            "OpenAI request failed after bounded retries: {}",
            last.unwrap_or_default()
        )))
    }
    fn parse_response<T: DeserializeOwned>(value: Value) -> Result<T> {
        if value.get("status").and_then(Value::as_str) == Some("incomplete") {
            return Err(Error::IncompleteProviderResponse);
        }
        for output in value
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for content in output
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                match content.get("type").and_then(Value::as_str) {
                    Some("refusal") => return Err(Error::ProviderRefusal),
                    Some("output_text") => {
                        let text =
                            content.get("text").and_then(Value::as_str).ok_or_else(|| {
                                Error::Provider("OpenAI output_text has no text".into())
                            })?;
                        return serde_json::from_str(text).map_err(|_| {
                            Error::Provider(
                                "OpenAI structured output could not be deserialized".into(),
                            )
                        });
                    }
                    _ => {}
                }
            }
        }
        Err(Error::Provider(
            "OpenAI returned no structured output".into(),
        ))
    }

    fn strict_schema<T: JsonSchema>() -> Value {
        fn close(value: &mut Value) {
            if let Some(object) = value.as_object_mut() {
                if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                    let required = properties.keys().cloned().map(Value::String).collect();
                    object.insert("required".into(), Value::Array(required));
                    object.insert("additionalProperties".into(), Value::Bool(false));
                }
                for child in object.values_mut() {
                    close(child);
                }
            } else if let Some(array) = value.as_array_mut() {
                for child in array {
                    close(child);
                }
            }
        }
        let mut schema =
            serde_json::to_value(schema_for!(T)).expect("generated JSON schema is serializable");
        close(&mut schema);
        schema
    }
}

#[async_trait]
impl WeeklyPlanGenerator for OpenAiWeeklyPlanGenerator {
    fn provider_model(&self) -> String {
        format!("openai:{}", self.model)
    }
    async fn generate(
        &self,
        profile: &NutritionProfile,
        iso_year: i32,
        iso_week: u32,
        allowed_ingredient_names: &[String],
    ) -> Result<WeeklyPlan> {
        let system = "Create a complete weekly meal-plan proposal from the supplied planning context. The plan starts exactly on cycle_start_date. Include exactly one breakfast, lunch, and dinner on each of the seven cycle days; snacks are optional. Every dinner must provide reserved leftovers for exactly one lunch on the following day, including the seventh dinner and its rollover lunch on day eight. The first cycle-day lunch may be a standalone recipe. A leftover lunch must use the same recipe_id, reference the originating dinner slot_id in leftover_of, and occur exactly one day later. For every recipe, declared servings must exactly equal the sum of servings in all meal slots that reference it. Every ingredient name must exactly equal one entry in allowed_ingredient_names. Ingredient quantities must be edible mass in g or kg only, never volume or piece counts. Treat explicit dislikes, learned avoid rules and dairy preferences as hard constraints. Use learned preferences to favor, reduce or avoid dishes and ingredients without changing nutrition targets. When ordinary yogurt or similar dairy is disliked and soy alternatives are preferred, use an allowed soy alternative instead. Prefer varied, protein- and fiber-oriented meals that support comfortable fullness between meals, never eating beyond satiety, and favor the requested cuisines while respecting active-time limits. If daily_kcal_reference is present, choose conservative ingredient amounts so the seven complete cycle days average no more than that explicit reference and no individual cycle day exceeds it by more than 150 kcal; the day-eight rollover lunch is not a complete cycle day. Application code recalculates and enforces these limits from ingredient data. Never output nutrition values and never alter calorie or macro targets. If the egg preference is unknown, do not include egg ingredients. Return only the requested structured data.";
        let body = json!({ "model": self.model, "store": false, "max_output_tokens": 24000,
            "input": [{"role":"system","content":system}, {"role":"user","content": planning_context(profile, iso_year, iso_week, allowed_ingredient_names)?.to_string()}],
            "text": { "format": { "type":"json_schema", "name":"weekly_plan", "strict":true,
                "schema": Self::strict_schema::<AiWeeklyPlan>() } } });
        let mut last_error = None;
        for _ in 0..self.max_attempts {
            let result =
                Self::parse_response::<AiWeeklyPlan>(self.request(&body).await?).and_then(|dto| {
                    dto.try_into().map_err(|error| {
                        Error::Provider(format!("OpenAI output failed domain validation: {error}"))
                    })
                });
            match result {
                Ok(plan) => return Ok(plan),
                Err(
                    error @ (Error::ProviderRefusal
                    | Error::IncompleteProviderResponse
                    | Error::Provider(_)),
                ) => last_error = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            Error::Provider("OpenAI generation exhausted its bounded retries".into())
        }))
    }
}

#[async_trait]
impl RecipeRefiner for OpenAiWeeklyPlanGenerator {
    fn provider_model(&self) -> String {
        format!("openai:{}", self.model)
    }

    async fn refine(
        &self,
        profile: &NutritionProfile,
        plan: &WeeklyPlan,
        recipe: &RecipeProposal,
        instruction: &str,
        allowed_ingredient_names: &[String],
    ) -> Result<RecipeProposal> {
        if instruction.trim().is_empty() {
            return Err(Error::Validation(
                "recipe refinement instruction must not be empty".into(),
            ));
        }
        let system = "Replace exactly one recipe according to the user's refinement instruction. Preserve recipe_id and servings exactly. Use only allowed_ingredient_names and edible mass in g or kg. Respect all hard dislikes, learned preferences, dairy rules and active-time limits. Return only the replacement recipe. Never provide nutrition values.";
        let body = json!({
            "model": self.model,
            "store": false,
            "max_output_tokens": 6000,
            "input": [
                {"role":"system","content":system},
                {"role":"user","content":json!({
                    "planning_context": planning_context(profile, plan.iso_year, plan.iso_week, allowed_ingredient_names)?,
                    "existing_recipe": recipe,
                    "refinement_instruction": instruction.trim(),
                    "required_recipe_id": recipe.recipe_id,
                    "required_servings": recipe.servings.to_string(),
                }).to_string()}
            ],
            "text": {"format": {"type":"json_schema","name":"recipe_replacement","strict":true,
                "schema": Self::strict_schema::<AiRecipe>()}}
        });
        let mut last_error = None;
        for _ in 0..self.max_attempts {
            let result = Self::parse_response::<AiRecipe>(self.request(&body).await?)
                .and_then(RecipeProposal::try_from);
            match result {
                Ok(replacement) => return Ok(replacement),
                Err(
                    error @ (Error::ProviderRefusal
                    | Error::IncompleteProviderResponse
                    | Error::Provider(_)),
                ) => last_error = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            Error::Provider("OpenAI recipe refinement exhausted its bounded retries".into())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::load_profile;
    use std::path::Path;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    #[test]
    fn schema_is_closed() {
        let schema = OpenAiWeeklyPlanGenerator::strict_schema::<AiWeeklyPlan>();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["$defs"]["AiIngredient"]["additionalProperties"],
            false
        );
        assert!(
            schema["$defs"]["AiIngredient"]["required"]
                .as_array()
                .unwrap()
                .contains(&Value::String("preparation_note".into()))
        );
    }
    #[test]
    fn refusal_fails_closed() {
        let value = json!({"status":"completed","output":[{"content":[{"type":"refusal","refusal":"no"}]}]});
        assert!(matches!(
            OpenAiWeeklyPlanGenerator::parse_response::<AiWeeklyPlan>(value),
            Err(Error::ProviderRefusal)
        ));
    }
    #[test]
    fn incomplete_fails_closed() {
        assert!(matches!(
            OpenAiWeeklyPlanGenerator::parse_response::<AiWeeklyPlan>(
                json!({"status":"incomplete","output":[]})
            ),
            Err(Error::IncompleteProviderResponse)
        ));
    }

    #[test]
    fn planning_context_excludes_body_and_includes_catalog_allowlist() {
        let mut profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        profile.preferences.dairy = Some(crate::models::DairyPreferences {
            milk: Some("accepted".into()),
            cream: Some("accepted".into()),
            butter: Some("accepted".into()),
            most_other_dairy: Some("disliked".into()),
            soy_alternatives: Some("preferred".into()),
        });
        profile.body = Some(crate::models::BodyProfile {
            age: Some(99),
            sex: None,
            height_cm: Some(Decimal::from(199)),
            weight_kg: Some(Decimal::from(199)),
            activity: None,
            target_weight_kg: None,
        });
        profile.preferences.dislikes.push("sweet_potato".into());
        let context = planning_context(
            &profile,
            2026,
            48,
            &[
                "rice".into(),
                "sweet_potato".into(),
                "skyr".into(),
                "soy_yogurt".into(),
                "peanut_butter".into(),
            ],
        )
        .unwrap();
        assert!(context.get("body").is_none());
        assert!(!context.to_string().contains("199"));
        assert_eq!(
            context["allowed_ingredient_names"],
            json!(["rice", "soy_yogurt", "peanut_butter"])
        );
        assert_eq!(
            context["preferences"]["dairy"]["most_other_dairy"],
            "disliked"
        );
        assert_eq!(
            context["preferences"]["dairy"]["soy_alternatives"],
            "preferred"
        );
    }

    #[tokio::test]
    async fn responses_contract_disables_storage_and_uses_bearer_auth() {
        let server = MockServer::start().await;
        let plan = json!({
            "iso_year": 2026, "iso_week": 48, "start_date": "2026-11-23", "timezone": "Europe/Berlin",
            "recipes": [{"recipe_id":"r","title":"Bowl","servings":"1","active_minutes":10,"cuisine_tags":[],
                "ingredients":[{"name":"rice","amount":"100","unit":"g","preparation_note":null}],"instructions":["Cook."]}],
            "meals": [{"slot_id":"s","date":"2026-11-23","meal_type":"dinner","recipe_id":"r","servings":"1","leftover_of":null}]
        });
        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("authorization", "Bearer synthetic-key"))
            .and(body_partial_json(json!({"model":"synthetic-model","store":false,"text":{"format":{"type":"json_schema","strict":true}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"completed","output":[{"content":[{"type":"output_text","text":plan.to_string()}]}]})))
            .expect(1)
            .mount(&server)
            .await;
        let generator = OpenAiWeeklyPlanGenerator::with_base_url(
            "synthetic-key".into(),
            "synthetic-model".into(),
            server.uri(),
        )
        .unwrap();
        let result = generator
            .generate(
                &load_profile(Path::new("config/profile.example.yaml")).unwrap(),
                2026,
                48,
                &["rice".into()],
            )
            .await
            .unwrap();
        assert_eq!(
            result.recipes[0].ingredients[0].quantity.amount,
            Decimal::from(100)
        );
    }

    #[tokio::test]
    async fn recipe_refinement_uses_strict_structured_output() {
        let server = MockServer::start().await;
        let replacement = json!({
            "recipe_id":"r","title":"Revised Bowl","servings":"1","active_minutes":8,
            "cuisine_tags":["synthetic"],
            "ingredients":[{"name":"rice","amount":"120","unit":"g","preparation_note":null}],
            "instructions":["Cook differently."]
        });
        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("authorization", "Bearer synthetic-key"))
            .and(body_partial_json(json!({"store":false,"text":{"format":{"name":"recipe_replacement","strict":true}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"completed","output":[{"content":[{"type":"output_text","text":replacement.to_string()}]}]})))
            .expect(1)
            .mount(&server)
            .await;
        let generator = OpenAiWeeklyPlanGenerator::with_base_url(
            "synthetic-key".into(),
            "synthetic-model".into(),
            server.uri(),
        )
        .unwrap();
        let recipe = RecipeProposal {
            recipe_id: "r".into(),
            title: "Old Bowl".into(),
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
        };
        let plan = WeeklyPlan {
            iso_year: 2026,
            iso_week: 48,
            start_date: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(),
            timezone: "Europe/Berlin".into(),
            recipes: vec![recipe.clone()],
            meals: vec![MealSlot {
                slot_id: "s".into(),
                date: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(),
                meal_type: MealType::Dinner,
                recipe_id: "r".into(),
                servings: Decimal::ONE,
                leftover_of: None,
            }],
        };
        let result = generator
            .refine(
                &load_profile(Path::new("config/profile.example.yaml")).unwrap(),
                &plan,
                &recipe,
                "change it",
                &["rice".into()],
            )
            .await
            .unwrap();
        assert_eq!(result.title, "Revised Bowl");
        assert_eq!(result.recipe_id, "r");
        assert_eq!(result.servings, Decimal::ONE);
    }

    #[tokio::test]
    async fn incomplete_generation_retries_are_bounded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status":"incomplete","output":[]})),
            )
            .expect(3)
            .mount(&server)
            .await;
        let generator = OpenAiWeeklyPlanGenerator::with_base_url(
            "synthetic-key".into(),
            "synthetic-model".into(),
            server.uri(),
        )
        .unwrap();
        let error = generator
            .generate(
                &load_profile(Path::new("config/profile.example.yaml")).unwrap(),
                2026,
                48,
                &["rice".into()],
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::IncompleteProviderResponse));
    }
}
