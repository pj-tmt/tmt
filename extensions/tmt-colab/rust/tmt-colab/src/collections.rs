//! Owner-only collections: display metadata and references, never page authority or originals.
//! The browser's existing All-pages search owns evaluation; this module validates its canonical
//! saved form and the bounded mutation inputs. The Store owns atomic changes and capacity.
use serde::{Deserialize, Serialize};
use tmt_colab_model::values;

pub mod ipc;

pub use crate::limits::{
    COLLECTION_ITEMS as ITEMS, COLLECTION_NAME_POINTS as NAME_POINTS,
    COLLECTION_NOTE_BYTES as NOTE_BYTES, COLLECTION_PERSON_BYTES as PERSON_BYTES,
    COLLECTION_SPACE_ITEMS as SPACE_ITEMS, COLLECTIONS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    Manual,
    Updated,
    Title,
}

/// Optional wire fields are omitted, never explicit null.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Canonical existing All-pages search, not another query language. Omitted defaults stay omitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by: Vec<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub sort: Option<Sort>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub archived: Option<bool>,
}

/// JavaScript String.trim's whitespace set, as used by validateHomeSearch. Rust trim also takes
/// U+0085 and does not take U+FEFF, so it cannot validate the browser's saved bytes by itself.
fn search_space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

impl Search {
    pub fn valid(&self) -> bool {
        self.q.as_deref().is_none_or(|q| {
            !q.is_empty() && q.trim_matches(search_space) == q && q.encode_utf16().count() <= 256
        }) && names(&self.label, crate::labels::valid)
            && names(&self.by, |name| display(name, PERSON_BYTES, false))
            && matches!(self.sort, None | Some(Sort::Title))
            && matches!(self.archived, None | Some(true))
    }
}

fn names(names: &[String], valid: impl Fn(&str) -> bool) -> bool {
    names.len() <= 8
        && names.iter().all(|name| valid(name))
        && names
            .iter()
            .enumerate()
            .all(|(index, name)| !names[..index].contains(name))
}

pub fn name_valid(name: &str) -> bool {
    !name.trim_matches(search_space).is_empty()
        && name.chars().count() <= NAME_POINTS
        && !name.chars().any(char::is_control)
}

pub fn section_valid(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(b))
}

fn display(value: &str, max: usize, empty: bool) -> bool {
    (empty || !value.is_empty()) && value.len() <= max && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Definition {
    Manual {
        sort: Sort,
    },
    Smart {
        search: Search,
        #[serde(
            default,
            rename = "needsYou",
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        needs_you: Option<bool>,
    },
}
impl Definition {
    pub fn valid(&self) -> bool {
        match self {
            Self::Manual { .. } => true,
            Self::Smart { search, needs_you } => {
                search.valid() && matches!(needs_you, None | Some(true))
            }
        }
    }
    pub fn manual(&self) -> bool {
        matches!(self, Self::Manual { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub maintainer: String,
    pub archived: bool,
    pub definition: Definition,
}
impl Collection {
    pub fn valid(&self) -> bool {
        values::generated_id(&self.id).is_ok()
            && name_valid(&self.name)
            && display(&self.maintainer, PERSON_BYTES, true)
            && self.definition.valid()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub page_id: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub anchor: Option<String>,
    pub note: String,
    pub audience: String,
}
impl Item {
    pub fn valid(&self) -> bool {
        values::generated_id(&self.id).is_ok()
            && values::generated_id(&self.page_id).is_ok()
            && self.anchor.as_deref().is_none_or(section_valid)
            && display(&self.note, NOTE_BYTES, true)
            && display(&self.audience, PERSON_BYTES, true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Seen {
    pub id: String,
    pub at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub collection: Collection,
    pub items: Vec<Item>,
    pub seen: Vec<Seen>,
}
impl Detail {
    pub fn valid(&self) -> bool {
        let manual = self.collection.definition.manual();
        self.collection.valid()
            && self.items.len() <= ITEMS
            && (manual || self.items.is_empty())
            && self.items.iter().all(Item::valid)
            && self
                .items
                .iter()
                .map(|item| &item.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.items.len()
            && self.seen.len() <= if manual { ITEMS } else { crate::limits::PAGES }
            && self.seen.iter().all(|mark| {
                values::generated_id(&mark.id).is_ok()
                    && values::time(mark.at_ms).is_ok()
                    && (!manual || self.items.iter().any(|item| item.id == mark.id))
            })
            && self
                .seen
                .iter()
                .map(|mark| &mark.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.seen.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub detail: Detail,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Invalid,
    Capacity,
    Conflict,
    NotFound,
    Unavailable,
    Unknown,
    ServerMismatch,
}
impl Fault {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "COLAB_INPUT_INVALID",
            Self::Capacity => "COLAB_CAPACITY",
            Self::Conflict => "COLAB_CONFLICT",
            Self::NotFound => "COLAB_COLLECTION_NOT_FOUND",
            Self::Unavailable => "COLAB_UNAVAILABLE",
            Self::Unknown => "COLAB_OUTCOME_UNKNOWN",
            Self::ServerMismatch => "COLAB_SERVER_MISMATCH",
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "Invalid collection change. Nothing was changed.",
            Self::Capacity => "The collection limit was reached. Nothing was changed.",
            Self::Conflict => {
                "This ID already names different collection data. Read it before changing it."
            }
            Self::NotFound => {
                "Collection not found. List the collections and choose an existing ID."
            }
            Self::Unavailable => "Collections are unavailable. Open the collection again.",
            Self::Unknown => "The collection change is unconfirmed. Use collection ls and show to inspect it before changing it again. No retry was attempted.",
            Self::ServerMismatch => "This CLI and the running Colab service use different collection protocols. Restart the service with the installed build before changing collections.",
        })
    }
}
impl std::error::Error for Fault {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Reply {
    Directory { collections: Vec<Collection> },
    Detail { detail: Detail, changed: bool },
}
impl Reply {
    pub fn matches(&self, command: &Command) -> bool {
        match (self, command.collection_id()) {
            (Self::Directory { .. }, None) => matches!(command, Command::List {}),
            (Self::Detail { detail, .. }, Some(id)) => detail.collection.id == id,
            _ => false,
        }
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Directory { collections } => {
                collections.len() <= COLLECTIONS
                    && collections.iter().all(Collection::valid)
                    && collections
                        .iter()
                        .map(|collection| &collection.id)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == collections.len()
            }
            Self::Detail { detail, .. } => detail.valid(),
        }
    }
}

/// A root-local request may inspect or maintain collections, but it never marks a person's Seen.
/// The browser admission owner explicitly enables only the last two person actions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Command {
    List {},
    Show {
        id: String,
    },
    Create {
        collection: Collection,
    },
    Add {
        id: String,
        item: Item,
    },
    Remove {
        id: String,
        item: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Maintainer {
        id: String,
        maintainer: String,
    },
    Sort {
        id: String,
        sort: Sort,
    },
    Query {
        id: String,
        search: Search,
        #[serde(
            default,
            rename = "needsYou",
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        needs_you: Option<bool>,
    },
    Archive {
        id: String,
        archived: bool,
    },
    Seen {
        id: String,
        target: String,
    },
    ClearSeen {
        id: String,
    },
}
impl Command {
    pub fn collection_id(&self) -> Option<&str> {
        match self {
            Self::List {} => None,
            Self::Create { collection } => Some(&collection.id),
            Self::Show { id }
            | Self::Add { id, .. }
            | Self::Remove { id, .. }
            | Self::Rename { id, .. }
            | Self::Maintainer { id, .. }
            | Self::Sort { id, .. }
            | Self::Query { id, .. }
            | Self::Archive { id, .. }
            | Self::Seen { id, .. }
            | Self::ClearSeen { id } => Some(id),
        }
    }
    pub fn valid(&self, person: bool) -> bool {
        let id = match self {
            Self::List {} => return true,
            Self::Create { collection } => return collection.valid(),
            Self::Show { id }
            | Self::Add { id, .. }
            | Self::Remove { id, .. }
            | Self::Rename { id, .. }
            | Self::Maintainer { id, .. }
            | Self::Sort { id, .. }
            | Self::Query { id, .. }
            | Self::Archive { id, .. }
            | Self::Seen { id, .. }
            | Self::ClearSeen { id } => id,
        };
        values::generated_id(id).is_ok()
            && match self {
                Self::Add { item, .. } => item.valid(),
                Self::Remove { item, .. } => values::generated_id(item).is_ok(),
                Self::Rename { name, .. } => name_valid(name),
                Self::Maintainer { maintainer, .. } => display(maintainer, PERSON_BYTES, true),
                Self::Query {
                    search, needs_you, ..
                } => search.valid() && matches!(needs_you, None | Some(true)),
                Self::Seen { target, .. } => person && values::generated_id(target).is_ok(),
                Self::ClearSeen { .. } => person,
                _ => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn native_and_browser_share_canonical_names_sections_and_list_search() {
        let vector: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/collection-v1.json"
        ))
        .unwrap();
        for (set, expected) in [("valid", true), ("invalid", false)] {
            for name in vector["names"][set].as_array().unwrap() {
                assert_eq!(name_valid(name.as_str().unwrap()), expected, "{name}");
            }
            for section in vector["sections"][set].as_array().unwrap() {
                assert_eq!(
                    section_valid(section.as_str().unwrap()),
                    expected,
                    "{section}"
                );
            }
            for search in vector["search"][set].as_array().unwrap() {
                let valid =
                    serde_json::from_value::<Search>(search.clone()).is_ok_and(|q| q.valid());
                assert_eq!(valid, expected, "{search}");
            }
        }
    }

    #[test]
    fn only_browser_admission_may_mark_or_clear_seen() {
        let id = "10000000-0000-4000-8000-000000000001";
        for value in [
            json!({"op":"seen","id":id,"target":id}),
            json!({"op":"clear-seen","id":id}),
        ] {
            let command: Command = serde_json::from_value(value).unwrap();
            assert!(!command.valid(false));
            assert!(command.valid(true));
        }
        assert!(serde_json::from_value::<Command>(json!({"op":"notify","id":id})).is_err());
        assert!(
            serde_json::from_value::<Command>(json!({"op":"clear-seen","id":id,"agent":"PM"}))
                .is_err()
        );
    }

    #[test]
    fn reference_labels_do_not_carry_notification_or_page_authority() {
        let item = Item {
            id: "10000000-0000-4000-8000-000000000001".into(),
            page_id: "10000000-0000-4000-8000-000000000002".into(),
            anchor: Some("hosting".into()),
            note: "x".repeat(NOTE_BYTES),
            audience: "Ben".into(),
        };
        assert!(item.valid());
        let mut long = item.clone();
        long.note.push('x');
        assert!(!long.valid());
        long = item;
        long.anchor = Some("[id=hosting]".into());
        assert!(!long.valid());
    }
}
