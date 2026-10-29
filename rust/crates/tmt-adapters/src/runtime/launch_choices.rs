//! Best-effort model and effort choices for a launch (#5539). Every list is a
//! suggestion: a launch still accepts any value the provider itself accepts.
use serde::Serialize;

/// Where a driver's model list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    /// Read from the provider's own files.
    Provider,
    /// TMT's maintained list, used when the provider offers none.
    Fallback,
}

/// The raw ids a provider's own `--model` accepts and the values of its effort
/// setting. Either list may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchChoices {
    pub source: ModelSource,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
}

impl LaunchChoices {
    pub fn fallback(models: &[&str], efforts: &[&str]) -> Self {
        Self {
            source: ModelSource::Fallback,
            models: models.iter().map(|&model| model.to_owned()).collect(),
            efforts: efforts.iter().map(|&effort| effort.to_owned()).collect(),
        }
    }
}
