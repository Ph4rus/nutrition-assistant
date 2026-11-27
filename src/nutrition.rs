use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::{NutrientProfile, ResolvedIngredient, ResolvedRecipe};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngredientNutritionTrace {
    pub food_id: String,
    pub source_id: String,
    pub source_version: String,
    pub edible_grams: Decimal,
    pub nutrients: NutrientProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecipeNutrition {
    pub recipe_id: String,
    pub servings: Decimal,
    pub recipe_total: NutrientProfile,
    pub per_serving: NutrientProfile,
    pub trace: Vec<IngredientNutritionTrace>,
    pub method: String,
}

fn scale(value: &NutrientProfile, factor: Decimal) -> NutrientProfile {
    NutrientProfile {
        energy_kcal: value.energy_kcal * factor,
        protein_g: value.protein_g * factor,
        carbohydrate_g: value.carbohydrate_g * factor,
        fat_g: value.fat_g * factor,
        fiber_g: value.fiber_g * factor,
    }
}
fn add(left: &NutrientProfile, right: &NutrientProfile) -> NutrientProfile {
    NutrientProfile {
        energy_kcal: left.energy_kcal + right.energy_kcal,
        protein_g: left.protein_g + right.protein_g,
        carbohydrate_g: left.carbohydrate_g + right.carbohydrate_g,
        fat_g: left.fat_g + right.fat_g,
        fiber_g: left.fiber_g + right.fiber_g,
    }
}

pub fn ingredient_nutrition(ingredient: &ResolvedIngredient) -> Result<IngredientNutritionTrace> {
    if ingredient.source_id.is_empty()
        || ingredient.source_version.is_empty()
        || ingredient.edible_grams <= Decimal::ZERO
    {
        return Err(Error::NutritionResolution(
            "ingredient nutrition provenance is incomplete".into(),
        ));
    }
    Ok(IngredientNutritionTrace {
        food_id: ingredient.food_id.clone(),
        source_id: ingredient.source_id.clone(),
        source_version: ingredient.source_version.clone(),
        edible_grams: ingredient.edible_grams,
        nutrients: scale(
            &ingredient.nutrients_per_100g,
            ingredient.edible_grams / Decimal::from(100),
        ),
    })
}

pub fn calculate_recipe_nutrition(recipe: &ResolvedRecipe) -> Result<RecipeNutrition> {
    if recipe.ingredients.is_empty() || recipe.servings <= Decimal::ZERO {
        return Err(Error::NutritionResolution(
            "a recipe requires ingredients and positive servings".into(),
        ));
    }
    let trace: Vec<_> = recipe
        .ingredients
        .iter()
        .map(ingredient_nutrition)
        .collect::<Result<_>>()?;
    let total = trace.iter().fold(NutrientProfile::default(), |sum, item| {
        add(&sum, &item.nutrients)
    });
    Ok(RecipeNutrition {
        recipe_id: recipe.recipe_id.clone(),
        servings: recipe.servings,
        per_serving: scale(&total, Decimal::ONE / recipe.servings),
        recipe_total: total,
        trace,
        method: "ingredient_based_estimate".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calculates_traceable_per_serving_values() {
        let recipe = ResolvedRecipe {
            recipe_id: "r".into(),
            servings: Decimal::from(2),
            ingredients: vec![ResolvedIngredient {
                name: "rice".into(),
                food_id: "rice".into(),
                source_id: "1".into(),
                source_version: "v1".into(),
                edible_grams: Decimal::from(200),
                nutrients_per_100g: NutrientProfile {
                    energy_kcal: Decimal::from(130),
                    ..Default::default()
                },
            }],
        };
        let result = calculate_recipe_nutrition(&recipe).unwrap();
        assert_eq!(result.recipe_total.energy_kcal, Decimal::from(260));
        assert_eq!(result.per_serving.energy_kcal, Decimal::from(130));
    }
}
