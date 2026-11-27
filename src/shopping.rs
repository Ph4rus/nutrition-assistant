use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::models::{CanonicalShoppingLine, ShoppingLine, Unit, UnitDimension};
use crate::units::canonicalize;

type ShoppingKey = (String, UnitDimension);
type Total = (String, Decimal, Unit);

fn totals(lines: &[ShoppingLine]) -> BTreeMap<ShoppingKey, Total> {
    let mut result = BTreeMap::new();
    for line in lines {
        let (dimension, amount, unit) = canonicalize(&line.quantity);
        result
            .entry((line.food_id.clone(), dimension))
            .and_modify(|current: &mut Total| {
                current.0 = current.0.clone().min(line.display_name.clone());
                current.1 += amount;
            })
            .or_insert((line.display_name.clone(), amount, unit));
    }
    result
}

pub fn consolidate_shopping(
    lines: &[ShoppingLine],
    pantry: &[ShoppingLine],
) -> Vec<CanonicalShoppingLine> {
    let requested = totals(lines);
    let stocked = totals(pantry);
    let mut result = Vec::new();
    for ((food_id, dimension), (display_name, amount, unit)) in requested {
        let pantry_amount = stocked
            .get(&(food_id.clone(), dimension))
            .map_or(Decimal::ZERO, |v| v.1);
        let remaining = amount - pantry_amount;
        if remaining > Decimal::ZERO {
            result.push(CanonicalShoppingLine {
                food_id,
                display_name,
                dimension,
                amount: remaining,
                unit,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    fn line(amount: i64, unit: Unit) -> ShoppingLine {
        ShoppingLine {
            food_id: "rice".into(),
            display_name: "Rice".into(),
            quantity: crate::models::Quantity {
                amount: Decimal::from(amount),
                unit,
            },
        }
    }
    proptest! { #[test] fn aggregation_is_order_independent(a in 1i64..1000, b in 1i64..1000) {
        let first = consolidate_shopping(&[line(a, Unit::Gram), line(b, Unit::Gram)], &[]);
        let second = consolidate_shopping(&[line(b, Unit::Gram), line(a, Unit::Gram)], &[]);
        prop_assert_eq!(first, second);
    }}
    #[test]
    fn does_not_mix_dimensions() {
        assert_eq!(
            consolidate_shopping(&[line(1, Unit::Kilogram), line(2, Unit::Piece)], &[]).len(),
            2
        );
    }
}
