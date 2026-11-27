use std::collections::HashMap;
use std::fs;
use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{FoodMassConversion, FoodSourceRecord, IngredientProposal, ResolvedIngredient};
use crate::units::to_edible_grams;
use crate::{Error, Result};

#[async_trait]
pub trait FoodDataProvider: Send + Sync {
    fn source_fingerprint(&self) -> &str;
    async fn get_food(&self, source_id: &str) -> Result<FoodSourceRecord>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub food_id: String,
    pub source_id: String,
    pub names: Vec<String>,
    pub conversion: Option<FoodMassConversion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CatalogDocument {
    pub foods: Vec<CatalogEntry>,
}

#[derive(Debug, Clone)]
pub struct CuratedFoodCatalog {
    version: String,
    by_name: HashMap<String, CatalogEntry>,
    entries: Vec<CatalogEntry>,
}

pub fn normalize_food_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl CuratedFoodCatalog {
    pub fn new(document: CatalogDocument) -> Result<Self> {
        let version = format!("{:x}", Sha256::digest(serde_json::to_vec(&document)?));
        let mut by_name = HashMap::new();
        let entries = document.foods;
        for entry in &entries {
            if entry.food_id.is_empty() || entry.source_id.is_empty() || entry.names.is_empty() {
                return Err(Error::Validation("catalog entries must be complete".into()));
            }
            if entry.conversion.as_ref().is_some_and(|conversion| {
                conversion
                    .density_g_per_ml
                    .is_some_and(|value| value <= rust_decimal::Decimal::ZERO)
                    || conversion
                        .grams_per_piece
                        .is_some_and(|value| value <= rust_decimal::Decimal::ZERO)
            }) {
                return Err(Error::Validation(
                    "food mass conversions must be positive".into(),
                ));
            }
            for name in &entry.names {
                let normalized = normalize_food_name(name);
                if by_name.insert(normalized.clone(), entry.clone()).is_some() {
                    return Err(Error::Validation(format!(
                        "duplicate catalog name: {normalized}"
                    )));
                }
            }
        }
        Ok(Self {
            version,
            by_name,
            entries,
        })
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::new(serde_yaml::from_str(&fs::read_to_string(path)?)?)
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }
    pub fn generation_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.by_name.keys().cloned().collect();
        names.sort();
        names
    }
    pub fn entry_for(&self, name: &str) -> Result<&CatalogEntry> {
        self.by_name.get(&normalize_food_name(name)).ok_or_else(|| {
            Error::CatalogResolution(format!(
                "ingredient has no curated mapping: {}",
                normalize_food_name(name)
            ))
        })
    }
    pub async fn resolve(
        &self,
        ingredient: &IngredientProposal,
        provider: &dyn FoodDataProvider,
    ) -> Result<ResolvedIngredient> {
        let entry = self.entry_for(&ingredient.name)?;
        let source = provider.get_food(&entry.source_id).await?;
        Ok(ResolvedIngredient {
            name: ingredient.name.clone(),
            food_id: entry.food_id.clone(),
            source_id: source.source_id,
            source_version: source.source_version,
            edible_grams: to_edible_grams(&ingredient.quantity, entry.conversion.as_ref())?,
            nutrients_per_100g: source.nutrients_per_100g,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_names_are_explicit_and_unique() {
        let doc = CatalogDocument {
            foods: vec![CatalogEntry {
                food_id: "x".into(),
                source_id: "1".into(),
                names: vec!["  Rice ".into()],
                conversion: None,
            }],
        };
        let catalog = CuratedFoodCatalog::new(doc).unwrap();
        assert!(catalog.entry_for("rice").is_ok());
        assert!(catalog.entry_for("rices").is_err());
        assert_eq!(catalog.generation_names(), vec!["rice"]);
    }
}
