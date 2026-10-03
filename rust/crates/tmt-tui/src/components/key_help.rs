//! Typed data for Footer and key help. The application resolves effective keys,
//! action descriptions and names; this leaf never derives prose from verbs.
use crate::binding::Schema;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tmt_cli_style::table::escape;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct KeyHint {
    pub key: String,
    pub description: String,
}

/// Effective-key hints in priority order. At tiny widths the application puts
/// its help hint first, then quit. Whole hints step aside, never split words.
pub fn footer(hints: &[KeyHint], width: u16, separator: &str) -> String {
    let mut result = String::new();
    for hint in hints {
        let pair = format!("{} {}", escape(&hint.key), escape(&hint.description));
        let candidate = if result.is_empty() {
            pair
        } else {
            format!("{result}{separator}{pair}")
        };
        if candidate.width() <= usize::from(width) {
            result = candidate;
        }
    }
    result
}

#[derive(Debug, Clone)]
pub struct KeyHelpEntry {
    pub id: String,
    pub keys: String,
    pub description: String,
}
#[derive(Debug, Clone)]
pub struct KeyHelpSection {
    pub id: String,
    pub title: String,
    pub entries: Vec<KeyHelpEntry>,
}
#[derive(Debug, Clone, Default)]
pub struct KeyHelp {
    pub sections: Vec<KeyHelpSection>,
}

impl KeyHelp {
    /// For `tmt-key-help bind="$.help"`. IDs describe stable entries, not indices.
    pub fn schema() -> Schema {
        fn object(fields: impl IntoIterator<Item = (&'static str, Schema)>) -> Schema {
            Schema::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key.into(), value))
                    .collect::<BTreeMap<_, _>>(),
            )
        }
        object([(
            "sections",
            Schema::Collection(Box::new(object([
                ("id", Schema::StableId),
                ("title", Schema::Scalar),
                (
                    "entries",
                    Schema::Collection(Box::new(object([
                        ("id", Schema::StableId),
                        ("keys", Schema::Scalar),
                        ("description", Schema::Scalar),
                    ]))),
                ),
            ]))),
        )])
    }

    pub fn value(&self) -> Value {
        json!({"sections": self.sections.iter().map(|section| json!({
            "id": section.id, "title": section.title,
            "entries": section.entries.iter().map(|entry| json!({
                "id": entry.id, "keys": entry.keys, "description": entry.description,
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()})
    }
}
