use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unit conversion failed: {0}")]
    UnitConversion(String),
    #[error("nutrition resolution failed: {0}")]
    NutritionResolution(String),
    #[error("invalid run transition: {0}")]
    InvalidRunTransition(String),
    #[error("run identity conflict: {0}")]
    RunIdentityConflict(String),
    #[error("provider request failed: {0}")]
    Provider(String),
    #[error("provider refused the request")]
    ProviderRefusal,
    #[error("provider returned an incomplete response")]
    IncompleteProviderResponse,
    #[error("catalog resolution failed: {0}")]
    CatalogResolution(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("configuration failed: {0}")]
    Configuration(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Decimal(#[from] rust_decimal::Error),
    #[error(transparent)]
    Date(#[from] chrono::ParseError),
}
