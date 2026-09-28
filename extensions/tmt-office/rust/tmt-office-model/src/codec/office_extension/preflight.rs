//! Bounded data-only authoring protocol. No storage, catalog mutation or host execution.

use super::{ExtensionError, validate_pair};
use crate::office_block::INPUT_LIMIT;
use serde::{Deserialize, Serialize};

// Two raw documents carried as JSON strings preserve duplicate keys. A source
// byte needs at most six bytes of JSON escaping; leave room for envelope keys.
pub const PROTOCOL_INPUT_LIMIT: usize = INPUT_LIMIT * 2 * 6 + 128;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationInput {
    pub definition: String,
    pub instance: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidationScope {
    StructureOnly,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum ValidationReport {
    Valid {
        definition: String,
        instance: String,
        scope: ValidationScope,
    },
    Invalid {
        problem: ExtensionError,
    },
}

pub fn execute(bytes: &[u8]) -> Vec<u8> {
    let result = (|| {
        if bytes.len() > PROTOCOL_INPUT_LIMIT {
            return Err(ExtensionError::InvalidInput);
        }
        let input: ValidationInput =
            serde_json::from_slice(bytes).map_err(|_| ExtensionError::InvalidInput)?;
        let (definition, instance) =
            validate_pair(input.definition.as_bytes(), input.instance.as_bytes())?;
        Ok(ValidationReport::Valid {
            definition: definition.id,
            instance: instance.id,
            scope: ValidationScope::StructureOnly,
        })
    })();
    serde_json::to_vec(&result.unwrap_or_else(|problem| ValidationReport::Invalid { problem }))
        .expect("the validation report contains only bounded strings and enums")
}

#[cfg(test)]
mod tests;
