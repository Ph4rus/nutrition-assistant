use rust_decimal::Decimal;

use crate::models::{FoodMassConversion, Quantity, Unit, UnitDimension};
use crate::{Error, Result};

pub const fn dimension_for(unit: Unit) -> UnitDimension {
    match unit {
        Unit::Gram | Unit::Kilogram => UnitDimension::Mass,
        Unit::Milliliter | Unit::Liter => UnitDimension::Volume,
        Unit::Piece => UnitDimension::Count,
    }
}

pub fn canonicalize(quantity: &Quantity) -> (UnitDimension, Decimal, Unit) {
    match quantity.unit {
        Unit::Gram => (UnitDimension::Mass, quantity.amount, Unit::Gram),
        Unit::Kilogram => (
            UnitDimension::Mass,
            quantity.amount * Decimal::from(1000),
            Unit::Gram,
        ),
        Unit::Milliliter => (UnitDimension::Volume, quantity.amount, Unit::Milliliter),
        Unit::Liter => (
            UnitDimension::Volume,
            quantity.amount * Decimal::from(1000),
            Unit::Milliliter,
        ),
        Unit::Piece => (UnitDimension::Count, quantity.amount, Unit::Piece),
    }
}

pub fn to_edible_grams(
    quantity: &Quantity,
    conversion: Option<&FoodMassConversion>,
) -> Result<Decimal> {
    let (dimension, amount, _) = canonicalize(quantity);
    match dimension {
        UnitDimension::Mass => Ok(amount),
        UnitDimension::Volume => conversion
            .and_then(|c| c.density_g_per_ml)
            .map(|density| amount * density)
            .ok_or_else(|| Error::UnitConversion("volume requires density_g_per_ml".into())),
        UnitDimension::Count => conversion
            .and_then(|c| c.grams_per_piece)
            .map(|grams| amount * grams)
            .ok_or_else(|| Error::UnitConversion("piece count requires grams_per_piece".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn converts_within_dimensions() {
        let q = Quantity {
            amount: Decimal::new(15, 1),
            unit: Unit::Kilogram,
        };
        assert_eq!(
            canonicalize(&q),
            (UnitDimension::Mass, Decimal::from(1500), Unit::Gram)
        );
    }

    #[test]
    fn refuses_implicit_cross_dimension_conversion() {
        let q = Quantity {
            amount: Decimal::from(2),
            unit: Unit::Piece,
        };
        assert!(matches!(
            to_edible_grams(&q, None),
            Err(Error::UnitConversion(_))
        ));
    }
}
