use std::path::Path;
use std::str::FromStr;

use chrono::{NaiveDate, Utc, Weekday};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::NutritionProfile;
use crate::{Error, Result};

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS preference_feedback (
 feedback_key TEXT PRIMARY KEY,
 scope TEXT NOT NULL,
 target_kind TEXT NOT NULL,
 signal TEXT NOT NULL,
 target TEXT NOT NULL,
 reason TEXT,
 iso_year INTEGER,
 iso_week INTEGER,
 active INTEGER NOT NULL DEFAULT 1,
 created_at TEXT NOT NULL,
 CHECK(scope IN ('permanent','cycle')),
 CHECK(target_kind IN ('ingredient','dish','cuisine')),
 CHECK(signal IN ('avoid','prefer','less_often','more_often')),
 CHECK((scope='permanent' AND iso_year IS NULL AND iso_week IS NULL) OR
       (scope='cycle' AND iso_year IS NOT NULL AND iso_week IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_preference_feedback_active_cycle
ON preference_feedback(active, scope, iso_year, iso_week);
"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackScope {
    Permanent,
    Cycle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackTargetKind {
    Ingredient,
    Dish,
    Cuisine,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSignal {
    Avoid,
    Prefer,
    LessOften,
    MoreOften,
}

macro_rules! string_enum {
    ($type:ty, {$($variant:path => $value:literal),+ $(,)?}) => {
        impl $type {
            pub const fn as_str(self) -> &'static str {
                match self {$($variant => $value),+}
            }
        }
        impl FromStr for $type {
            type Err = Error;
            fn from_str(value: &str) -> Result<Self> {
                match value {$($value => Ok($variant),)+ _ => Err(Error::Validation(format!("unknown feedback value: {value}")))}
            }
        }
    };
}

string_enum!(FeedbackScope, {
    FeedbackScope::Permanent => "permanent",
    FeedbackScope::Cycle => "cycle",
});
string_enum!(FeedbackTargetKind, {
    FeedbackTargetKind::Ingredient => "ingredient",
    FeedbackTargetKind::Dish => "dish",
    FeedbackTargetKind::Cuisine => "cuisine",
});
string_enum!(FeedbackSignal, {
    FeedbackSignal::Avoid => "avoid",
    FeedbackSignal::Prefer => "prefer",
    FeedbackSignal::LessOften => "less_often",
    FeedbackSignal::MoreOften => "more_often",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeedbackRecord {
    pub feedback_key: String,
    pub scope: FeedbackScope,
    pub target_kind: FeedbackTargetKind,
    pub signal: FeedbackSignal,
    pub target: String,
    pub reason: Option<String>,
    pub iso_year: Option<i32>,
    pub iso_week: Option<u32>,
    pub active: bool,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewFeedback {
    pub scope: FeedbackScope,
    pub target_kind: FeedbackTargetKind,
    pub signal: FeedbackSignal,
    pub target: String,
    pub reason: Option<String>,
    pub iso_year: Option<i32>,
    pub iso_week: Option<u32>,
}

impl NewFeedback {
    fn validate(&self) -> Result<()> {
        if self.target.trim().is_empty() {
            return Err(Error::Validation(
                "feedback target must not be empty".into(),
            ));
        }
        match self.scope {
            FeedbackScope::Permanent if self.iso_year.is_some() || self.iso_week.is_some() => Err(
                Error::Validation("permanent feedback cannot have a cycle".into()),
            ),
            FeedbackScope::Cycle
                if self.iso_year.is_none()
                    || !self
                        .iso_year
                        .zip(self.iso_week)
                        .is_some_and(|(year, week)| {
                            NaiveDate::from_isoywd_opt(year, week, Weekday::Mon).is_some()
                        }) =>
            {
                Err(Error::Validation(
                    "cycle feedback requires a valid ISO year and week".into(),
                ))
            }
            _ => Ok(()),
        }
    }

    fn key(&self) -> Result<String> {
        self.validate()?;
        let identity = serde_json::json!({
            "scope": self.scope.as_str(),
            "target_kind": self.target_kind.as_str(),
            "signal": self.signal.as_str(),
            "target": self.target.trim().to_lowercase(),
            "iso_year": self.iso_year,
            "iso_week": self.iso_week,
        });
        Ok(format!(
            "feedback:{:x}",
            Sha256::digest(serde_json::to_vec(&identity)?)
        ))
    }
}

pub struct FeedbackStore {
    connection: Connection,
}

impl FeedbackStore {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(MIGRATION)?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(3, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(Self { connection })
    }

    pub fn add(&mut self, feedback: &NewFeedback) -> Result<(FeedbackRecord, bool)> {
        let key = feedback.key()?;
        let changed = self.connection.execute(
            "INSERT OR IGNORE INTO preference_feedback(feedback_key,scope,target_kind,signal,target,reason,iso_year,iso_week,active,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,?9)",
            params![
                key,
                feedback.scope.as_str(),
                feedback.target_kind.as_str(),
                feedback.signal.as_str(),
                feedback.target.trim(),
                feedback.reason.as_deref().map(str::trim).filter(|v| !v.is_empty()),
                feedback.iso_year,
                feedback.iso_week,
                Utc::now().to_rfc3339(),
            ],
        )?;
        let record = self
            .get(&key)?
            .ok_or_else(|| Error::Validation("stored feedback disappeared".into()))?;
        Ok((record, changed == 1))
    }

    pub fn deactivate(&mut self, key: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE preference_feedback SET active=0 WHERE feedback_key=?1 AND active=1",
            [key],
        )? == 1)
    }

    pub fn active_for_cycle(&self, iso_year: i32, iso_week: u32) -> Result<Vec<FeedbackRecord>> {
        self.query(
            "WHERE active=1 AND (scope='permanent' OR (scope='cycle' AND iso_year=?1 AND iso_week=?2))",
            params![iso_year, iso_week],
        )
    }

    pub fn list(&self) -> Result<Vec<FeedbackRecord>> {
        self.query("", params![])
    }

    fn get(&self, key: &str) -> Result<Option<FeedbackRecord>> {
        let sql = "SELECT feedback_key,scope,target_kind,signal,target,reason,iso_year,iso_week,active,created_at FROM preference_feedback WHERE feedback_key=?1";
        self.connection
            .query_row(sql, [key], row_to_record)
            .optional()
            .map_err(Into::into)
    }

    fn query<P: rusqlite::Params>(&self, clause: &str, params: P) -> Result<Vec<FeedbackRecord>> {
        let sql = format!(
            "SELECT feedback_key,scope,target_kind,signal,target,reason,iso_year,iso_week,active,created_at FROM preference_feedback {clause} ORDER BY created_at,feedback_key"
        );
        let mut statement = self.connection.prepare(&sql)?;
        Ok(statement
            .query_map(params, row_to_record)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackRecord> {
    let scope: String = row.get(1)?;
    let target_kind: String = row.get(2)?;
    let signal: String = row.get(3)?;
    Ok(FeedbackRecord {
        feedback_key: row.get(0)?,
        scope: scope.parse().map_err(to_sql_error)?,
        target_kind: target_kind.parse().map_err(to_sql_error)?,
        signal: signal.parse().map_err(to_sql_error)?,
        target: row.get(4)?,
        reason: row.get(5)?,
        iso_year: row.get(6)?,
        iso_week: row.get(7)?,
        active: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
    })
}

fn to_sql_error(error: Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

pub fn apply_feedback(profile: &mut NutritionProfile, records: &[FeedbackRecord]) {
    for record in records.iter().filter(|record| record.active) {
        let destination = match (record.target_kind, record.signal) {
            (FeedbackTargetKind::Ingredient, FeedbackSignal::Avoid) => {
                &mut profile.learned.avoid_ingredients
            }
            (FeedbackTargetKind::Ingredient, FeedbackSignal::Prefer) => {
                &mut profile.learned.prefer_ingredients
            }
            (FeedbackTargetKind::Dish, FeedbackSignal::Avoid) => &mut profile.learned.avoid_dishes,
            (FeedbackTargetKind::Dish, FeedbackSignal::Prefer) => {
                &mut profile.learned.prefer_dishes
            }
            (_, FeedbackSignal::LessOften) => &mut profile.learned.less_often,
            (_, FeedbackSignal::MoreOften) => &mut profile.learned.more_often,
            (FeedbackTargetKind::Cuisine, FeedbackSignal::Avoid) => {
                &mut profile.learned.avoid_dishes
            }
            (FeedbackTargetKind::Cuisine, FeedbackSignal::Prefer) => {
                &mut profile.learned.prefer_dishes
            }
        };
        if !destination
            .iter()
            .any(|value| value.eq_ignore_ascii_case(record.target.trim()))
        {
            destination.push(record.target.trim().to_owned());
        }
    }
    for values in [
        &mut profile.learned.avoid_ingredients,
        &mut profile.learned.prefer_ingredients,
        &mut profile.learned.avoid_dishes,
        &mut profile.learned.prefer_dishes,
        &mut profile.learned.less_often,
        &mut profile.learned.more_often,
    ] {
        values.sort_by_key(|value| value.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::profile::{load_profile, profile_version};

    #[test]
    fn feedback_is_idempotent_scoped_and_changes_profile() {
        let dir = tempdir().unwrap();
        let database = dir.path().join("planner.sqlite3");
        crate::storage::RunLedger::open(&database).unwrap();
        let mut store = FeedbackStore::open(&database).unwrap();
        let permanent = NewFeedback {
            scope: FeedbackScope::Permanent,
            target_kind: FeedbackTargetKind::Ingredient,
            signal: FeedbackSignal::Avoid,
            target: "Synthetic food".into(),
            reason: Some("test".into()),
            iso_year: None,
            iso_week: None,
        };
        assert!(store.add(&permanent).unwrap().1);
        assert!(!store.add(&permanent).unwrap().1);
        let cycle = NewFeedback {
            scope: FeedbackScope::Cycle,
            target_kind: FeedbackTargetKind::Dish,
            signal: FeedbackSignal::LessOften,
            target: "Synthetic bowl".into(),
            reason: None,
            iso_year: Some(2026),
            iso_week: Some(48),
        };
        let (cycle_record, _) = store.add(&cycle).unwrap();
        assert_eq!(store.active_for_cycle(2026, 49).unwrap().len(), 1);
        let records = store.active_for_cycle(2026, 48).unwrap();
        assert_eq!(records.len(), 2);
        let mut profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        let before = profile_version(&profile).unwrap();
        apply_feedback(&mut profile, &records);
        assert_ne!(before, profile_version(&profile).unwrap());
        assert_eq!(profile.learned.avoid_ingredients, ["Synthetic food"]);
        assert_eq!(profile.learned.less_often, ["Synthetic bowl"]);
        assert!(store.deactivate(&cycle_record.feedback_key).unwrap());
        assert!(!store.deactivate(&cycle_record.feedback_key).unwrap());
    }
}
