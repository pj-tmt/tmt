//! Shared JSON admission and projection for conditional metadata updates.

use serde::Deserialize;
use serde_json::{Value, json};
use tmt_core::identity_metadata::{
    ApplyMetadataResult, MetadataAction, MetadataChange, MetadataChanges, MetadataExpectation,
    MetadataKey, MetadataValidationError, MetadataValue,
};

// Enough for 64 keys and two maximally escaped values per key. Stdin callers
// enforce this before decoding, independently of the larger API envelope bound.
pub const APPLY_INPUT_LIMIT: usize = 64 * (64 + 2 * 1024) * 6 + 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    key: String,
    expect: Expectation,
    then: Action,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Expectation {
    Value(String),
    Absent,
    Any,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Set(String),
    Remove,
}

#[derive(Debug)]
pub enum ApplyInputError {
    Json(serde_json::Error),
    Invalid(MetadataValidationError),
    TooLarge,
}

impl std::fmt::Display for ApplyInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::Json(_) => formatter.write_str("Expected a JSON list of metadata changes."),
            Self::TooLarge => formatter.write_str("Metadata changes exceed the input byte limit."),
        }
    }
}

impl std::error::Error for ApplyInputError {}

pub fn decode_changes(input: &str) -> Result<MetadataChanges, ApplyInputError> {
    if input.len() > APPLY_INPUT_LIMIT {
        return Err(ApplyInputError::TooLarge);
    }
    let wire: Vec<Change> = serde_json::from_str(input).map_err(ApplyInputError::Json)?;
    let changes = wire
        .into_iter()
        .map(|change| {
            Ok(MetadataChange {
                key: MetadataKey::parse(&change.key)?,
                expect: match change.expect {
                    Expectation::Value(value) => {
                        MetadataExpectation::Value(MetadataValue::parse(&value)?)
                    }
                    Expectation::Absent => MetadataExpectation::Absent,
                    Expectation::Any => MetadataExpectation::Any,
                },
                then: match change.then {
                    Action::Set(value) => MetadataAction::Set(MetadataValue::parse(&value)?),
                    Action::Remove => MetadataAction::Remove,
                },
            })
        })
        .collect::<Result<Vec<_>, MetadataValidationError>>()
        .map_err(ApplyInputError::Invalid)?;
    MetadataChanges::new(changes).map_err(ApplyInputError::Invalid)
}

pub fn applied_value(result: &ApplyMetadataResult) -> Value {
    json!({"identityId": result.identity_id, "changed": result.changed})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_changes_preserve_literal_sentinel_values_and_reject_invalid_batches() {
        let changes = decode_changes(
            r#"[{"key":"key","expect":{"value":"absent"},"then":{"set":"remove"}}]"#,
        )
        .unwrap();
        assert_eq!(
            changes.changes()[0].expect,
            MetadataExpectation::Value(MetadataValue::parse("absent").unwrap())
        );
        assert_eq!(
            changes.changes()[0].then,
            MetadataAction::Set(MetadataValue::parse("remove").unwrap())
        );
        for body in [
            "[]",
            "{}",
            "null",
            r#"[{"key":"key","expect":"absent","then":"remove","extra":1}]"#,
            r#"[{"key":"Upper","expect":"any","then":"remove"}]"#,
            r#"[{"key":"key","expect":{"value":""},"then":"remove"}]"#,
            r#"[{"key":"key","expect":"any","then":{"set":"line\nfeed"}}]"#,
            r#"[{"key":"key","expect":"any","then":"remove"},{"key":"key","expect":"absent","then":{"set":"v"}}]"#,
            r#"[{"key":"key","key":"other","expect":"any","then":"remove"}]"#,
            r#"[{"key":"key","expect":{"value":"old","extra":true},"then":"remove"}]"#,
        ] {
            assert!(decode_changes(body).is_err(), "{body}");
        }
        let oversized = (0..65)
            .map(|index| format!(r#"{{"key":"k{index}","expect":"any","then":"remove"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        assert!(decode_changes(&format!("[{oversized}]")).is_err());
        assert!(matches!(
            decode_changes(&" ".repeat(APPLY_INPUT_LIMIT + 1)),
            Err(ApplyInputError::TooLarge)
        ));
    }
}
