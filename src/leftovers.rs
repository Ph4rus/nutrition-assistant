use chrono::{Duration, NaiveDate};
use rust_decimal::Decimal;

use crate::models::{DinnerLeftoverAllocation, MealSlot, MealType};

pub fn allocate_dinner_and_next_day_lunch(
    dinner_date: NaiveDate,
    recipe_id: &str,
    dinner_servings: Decimal,
    lunch_servings: Decimal,
) -> DinnerLeftoverAllocation {
    let dinner_slot_id = format!("{dinner_date}:dinner:{recipe_id}");
    let lunch_date = dinner_date + Duration::days(1);
    let dinner = MealSlot {
        slot_id: dinner_slot_id.clone(),
        date: dinner_date,
        meal_type: MealType::Dinner,
        recipe_id: recipe_id.into(),
        servings: dinner_servings,
        leftover_of: None,
    };
    let lunch = MealSlot {
        slot_id: format!("{lunch_date}:lunch:{recipe_id}"),
        date: lunch_date,
        meal_type: MealType::Lunch,
        recipe_id: recipe_id.into(),
        servings: lunch_servings,
        leftover_of: Some(dinner_slot_id),
    };
    DinnerLeftoverAllocation {
        recipe_id: recipe_id.into(),
        recipe_servings: dinner_servings + lunch_servings,
        dinner,
        next_day_lunch: lunch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crosses_iso_week_without_duplicate_recipe() {
        let result = allocate_dinner_and_next_day_lunch(
            NaiveDate::from_ymd_opt(2027, 1, 3).unwrap(),
            "r",
            Decimal::ONE,
            Decimal::ONE,
        );
        assert_eq!(
            result.next_day_lunch.date,
            NaiveDate::from_ymd_opt(2027, 1, 4).unwrap()
        );
        assert_eq!(result.recipe_servings, Decimal::from(2));
    }
}
