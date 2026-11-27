use chrono::{Datelike, Duration, NaiveDate};

use crate::models::Weekday;
use crate::{Error, Result};

pub fn cycle_start_for(iso_year: i32, iso_week: u32, shopping_day: Weekday) -> Result<NaiveDate> {
    NaiveDate::from_isoywd_opt(iso_year, iso_week, shopping_day.chrono())
        .ok_or_else(|| Error::Validation("invalid ISO week".into()))
}

pub fn next_cycle_start(today: NaiveDate, shopping_day: Weekday) -> NaiveDate {
    let mut days = (i64::from(shopping_day.iso_number())
        - i64::from(today.weekday().number_from_monday()))
    .rem_euclid(7);
    if days == 0 {
        days = 7;
    }
    today + Duration::days(days)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_iso_identity_to_tuesday() {
        assert_eq!(
            cycle_start_for(2026, 49, Weekday::Tuesday).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );
    }
    #[test]
    fn chooses_upcoming_tuesday() {
        assert_eq!(
            next_cycle_start(
                NaiveDate::from_ymd_opt(2026, 11, 29).unwrap(),
                Weekday::Tuesday
            ),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );
    }
    #[test]
    fn does_not_regenerate_current_cycle() {
        assert_eq!(
            next_cycle_start(
                NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
                Weekday::Tuesday
            ),
            NaiveDate::from_ymd_opt(2026, 12, 8).unwrap()
        );
    }
}
