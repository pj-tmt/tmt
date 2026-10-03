//! Shared approval and bounded invocation for both driver kinds.
pub mod process;
pub mod registry;

use serde::{Deserialize, Deserializer, Serialize};
use tmt_driver_protocol::{Capabilities, RuntimeCapabilities};

/// Stored as the original capability object, with no extra envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Declaration {
    Host(Capabilities),
    Runtime(RuntimeCapabilities),
}

impl<'de> Deserialize<'de> for Declaration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value.get("kind").and_then(serde_json::Value::as_str) {
            Some("runtime") => serde_json::from_value(value).map(Self::Runtime),
            Some("host") | None => serde_json::from_value(value).map(Self::Host),
            _ => return Err(serde::de::Error::custom("unknown driver kind")),
        }
        .map_err(serde::de::Error::custom)
    }
}

impl Declaration {
    pub fn host(&self) -> Option<&Capabilities> {
        match self {
            Self::Host(value) => Some(value),
            Self::Runtime(_) => None,
        }
    }
    pub fn kind(&self) -> &str {
        match self {
            Self::Host(_) => "host",
            Self::Runtime(_) => "runtime",
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Host(value) => &value.name,
            Self::Runtime(value) => &value.name,
        }
    }
    pub fn version(&self) -> &str {
        match self {
            Self::Host(value) => &value.version,
            Self::Runtime(value) => &value.version,
        }
    }
    pub fn ops(&self) -> &[String] {
        match self {
            Self::Host(value) => &value.ops,
            Self::Runtime(value) => &value.ops,
        }
    }
}
