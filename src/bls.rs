use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use async_trait::async_trait;
use calamine::{Data, Reader, open_workbook_auto};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::FoodDataProvider;
use crate::models::{FoodSearchCandidate, FoodSourceRecord, NutrientProfile};
use crate::{Error, Result};

const DATASET_LABEL: &str = "BLS 4.0 (2025)";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlsImportSummary {
    pub source_version: String,
    pub imported_records: usize,
    pub skipped_incomplete_records: usize,
    pub already_present: bool,
}

fn configuration_error(context: &str, error: impl std::fmt::Display) -> Error {
    Error::Configuration(format!("{context}: {error}"))
}

fn cell_text(cell: Option<&Data>) -> Option<String> {
    match cell? {
        Data::String(value) => Some(value.trim().to_owned()),
        Data::Int(value) => Some(value.to_string()),
        Data::Float(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn cell_decimal(cell: Option<&Data>) -> Option<Decimal> {
    let raw = match cell? {
        Data::Int(value) => return Some(Decimal::from(*value)),
        Data::Float(value) if value.is_finite() => value.to_string(),
        Data::String(value) => value.trim().replace(',', "."),
        _ => return None,
    };
    Decimal::from_str(&raw).ok()
}

fn find_column(headers: &[Data], exact: &str) -> Result<usize> {
    headers
        .iter()
        .position(|cell| cell_text(Some(cell)).as_deref() == Some(exact))
        .ok_or_else(|| Error::Configuration(format!("BLS column is missing: {exact}")))
}

fn find_nutrient_column(headers: &[Data], code: &str) -> Result<usize> {
    let prefix = format!("{code} ");
    headers
        .iter()
        .position(|cell| {
            cell_text(Some(cell))
                .as_deref()
                .is_some_and(|value| value.starts_with(&prefix) && value.contains("/100g]"))
        })
        .ok_or_else(|| Error::Configuration(format!("BLS nutrient column is missing: {code}")))
}

#[derive(Debug, Clone, Copy)]
struct Columns {
    source_id: usize,
    description: usize,
    energy: usize,
    protein: usize,
    fat: usize,
    carbohydrate: usize,
    fiber: usize,
}

impl Columns {
    fn from_headers(headers: &[Data]) -> Result<Self> {
        Ok(Self {
            source_id: find_column(headers, "BLS Code")?,
            description: find_column(headers, "Lebensmittelbezeichnung")?,
            energy: find_nutrient_column(headers, "ENERCC")?,
            protein: find_nutrient_column(headers, "PROT625")?,
            fat: find_nutrient_column(headers, "FAT")?,
            carbohydrate: find_nutrient_column(headers, "CHO")?,
            fiber: find_nutrient_column(headers, "FIBT")?,
        })
    }
}

fn record_from_row(
    row: &[Data],
    columns: Columns,
    source_version: &str,
) -> Option<FoodSourceRecord> {
    let source_id = cell_text(row.get(columns.source_id))?;
    let description = cell_text(row.get(columns.description))?;
    let nutrients = NutrientProfile {
        energy_kcal: cell_decimal(row.get(columns.energy))?,
        protein_g: cell_decimal(row.get(columns.protein))?,
        carbohydrate_g: cell_decimal(row.get(columns.carbohydrate))?,
        fat_g: cell_decimal(row.get(columns.fat))?,
        fiber_g: cell_decimal(row.get(columns.fiber))?,
    };
    if [
        nutrients.energy_kcal,
        nutrients.protein_g,
        nutrients.carbohydrate_g,
        nutrients.fat_g,
        nutrients.fiber_g,
    ]
    .into_iter()
    .any(|value| value < Decimal::ZERO)
    {
        return None;
    }
    Some(FoodSourceRecord {
        source_id,
        source_version: source_version.to_owned(),
        description,
        data_type: "BLS".into(),
        nutrients_per_100g: nutrients,
    })
}

fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS bls_datasets(
            source_version TEXT PRIMARY KEY,
            sha256 TEXT NOT NULL,
            imported_records INTEGER NOT NULL,
            skipped_incomplete_records INTEGER NOT NULL,
            imported_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bls_foods(
            source_id TEXT NOT NULL,
            source_version TEXT NOT NULL,
            description TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            PRIMARY KEY(source_id, source_version),
            FOREIGN KEY(source_version) REFERENCES bls_datasets(source_version)
        );
        CREATE INDEX IF NOT EXISTS idx_bls_foods_version_description
            ON bls_foods(source_version, description);
        "#,
    )?;
    Ok(())
}

pub fn import_bls_xlsx(database_path: &Path, input_path: &Path) -> Result<BlsImportSummary> {
    let bytes = fs::read(input_path)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let source_version = format!("{DATASET_LABEL}; sha256={sha256}");
    let mut connection = Connection::open(database_path)?;
    migrate(&connection)?;

    let existing = connection
        .query_row(
            "SELECT imported_records, skipped_incomplete_records FROM bls_datasets WHERE source_version=?1",
            [&source_version],
            |row| Ok((row.get::<_, usize>(0)?, row.get::<_, usize>(1)?)),
        )
        .optional()?;
    if let Some((imported_records, skipped_incomplete_records)) = existing {
        return Ok(BlsImportSummary {
            source_version,
            imported_records,
            skipped_incomplete_records,
            already_present: true,
        });
    }

    let mut workbook = open_workbook_auto(input_path)
        .map_err(|error| configuration_error("cannot open BLS workbook", error))?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| Error::Configuration("BLS workbook has no worksheet".into()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|error| configuration_error("cannot read BLS worksheet", error))?;
    let mut rows = range.rows();
    let headers = rows
        .next()
        .ok_or_else(|| Error::Configuration("BLS worksheet is empty".into()))?;
    let columns = Columns::from_headers(headers)?;

    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO bls_datasets(source_version,sha256,imported_records,skipped_incomplete_records,imported_at) VALUES(?1,?2,0,0,?3)",
        params![source_version, sha256, Utc::now().to_rfc3339()],
    )?;
    let mut imported_records = 0usize;
    let mut skipped_incomplete_records = 0usize;
    for row in rows {
        let Some(record) = record_from_row(row, columns, &source_version) else {
            skipped_incomplete_records += 1;
            continue;
        };
        transaction.execute(
            "INSERT INTO bls_foods(source_id,source_version,description,payload_json) VALUES(?1,?2,?3,?4)",
            params![
                record.source_id,
                record.source_version,
                record.description,
                serde_json::to_string(&record)?
            ],
        )?;
        imported_records += 1;
    }
    if imported_records == 0 {
        return Err(Error::Configuration(
            "BLS import produced no complete nutrition records".into(),
        ));
    }
    transaction.execute(
        "UPDATE bls_datasets SET imported_records=?2,skipped_incomplete_records=?3 WHERE source_version=?1",
        params![
            source_version,
            imported_records,
            skipped_incomplete_records
        ],
    )?;
    transaction.commit()?;
    Ok(BlsImportSummary {
        source_version,
        imported_records,
        skipped_incomplete_records,
        already_present: false,
    })
}

pub struct BlsProvider {
    connection: Mutex<Connection>,
    source_version: String,
}

impl BlsProvider {
    pub fn open(database_path: &Path, source_version: String) -> Result<Self> {
        let connection = Connection::open(database_path)?;
        migrate(&connection)?;
        let exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM bls_datasets WHERE source_version=?1)",
            [&source_version],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(Error::Configuration(
                "requested BLS dataset version has not been imported".into(),
            ));
        }
        Ok(Self {
            connection: Mutex::new(connection),
            source_version,
        })
    }

    pub fn open_latest(database_path: &Path) -> Result<Self> {
        let connection = Connection::open(database_path)?;
        migrate(&connection)?;
        let source_version: Option<String> = connection
            .query_row(
                "SELECT source_version FROM bls_datasets ORDER BY imported_at DESC, source_version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        drop(connection);
        Self::open(
            database_path,
            source_version
                .ok_or_else(|| Error::Configuration("no BLS dataset has been imported".into()))?,
        )
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<FoodSearchCandidate>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Provider("BLS database lock poisoned".into()))?;
        let mut statement = connection.prepare(
            "SELECT source_id,description FROM bls_foods
             WHERE source_version=?1 AND instr(lower(description),lower(?2)) > 0
             ORDER BY length(description),description LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![self.source_version, query.trim(), limit], |row| {
                Ok(FoodSearchCandidate {
                    source_id: row.get(0)?,
                    description: row.get(1)?,
                    data_type: "BLS".into(),
                })
            })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

#[async_trait]
impl FoodDataProvider for BlsProvider {
    fn source_fingerprint(&self) -> &str {
        &self.source_version
    }

    async fn get_food(&self, source_id: &str) -> Result<FoodSourceRecord> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Provider("BLS database lock poisoned".into()))?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload_json FROM bls_foods WHERE source_id=?1 AND source_version=?2",
                params![source_id, self.source_version],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|value| serde_json::from_str(&value).map_err(Error::from))
            .transpose()?
            .ok_or_else(|| {
                Error::CatalogResolution(format!(
                    "BLS source ID is missing or lacks required nutrients: {source_id}"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> Vec<Data> {
        [
            "BLS Code",
            "Lebensmittelbezeichnung",
            "ENERCC Energie (Kilokalorien) [kcal/100g]",
            "PROT625 Protein (Nx6,25) [g/100g]",
            "FAT Fett [g/100g]",
            "CHO Kohlenhydrate, verfügbar [g/100g]",
            "FIBT Ballaststoffe, gesamt [g/100g]",
        ]
        .into_iter()
        .map(|value| Data::String(value.into()))
        .collect()
    }

    #[test]
    fn parses_required_bls_components_by_code() {
        let columns = Columns::from_headers(&headers()).unwrap();
        let row = vec![
            Data::String("B123456".into()),
            Data::String("Synthetischer Reis".into()),
            Data::Int(130),
            Data::Float(2.7),
            Data::String("0,3".into()),
            Data::Int(28),
            Data::Float(0.4),
        ];
        let record = record_from_row(&row, columns, "test-version").unwrap();
        assert_eq!(record.source_id, "B123456");
        assert_eq!(record.nutrients_per_100g.energy_kcal, Decimal::from(130));
        assert_eq!(record.nutrients_per_100g.fat_g, Decimal::new(3, 1));
    }

    #[test]
    fn missing_nutrient_is_not_treated_as_zero() {
        let columns = Columns::from_headers(&headers()).unwrap();
        let mut row = vec![
            Data::String("B123456".into()),
            Data::String("Synthetischer Reis".into()),
            Data::Int(130),
            Data::Float(2.7),
            Data::Float(0.3),
            Data::Int(28),
            Data::Float(0.4),
        ];
        row[6] = Data::Empty;
        assert!(record_from_row(&row, columns, "test-version").is_none());
    }
}
