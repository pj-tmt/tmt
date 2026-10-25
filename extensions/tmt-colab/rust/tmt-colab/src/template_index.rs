//! Which pages are templates and what each page was created from (#5066), read once for the whole
//! space. A page's marks are in its content metadata (`template_marks.rs`), so reading them folds
//! the page: the answers are kept by `page_index.rs`, and a name's lookup or `template ls` folds
//! only the pages whose progress moved. The index is a cache of the pages' own marks, never an
//! authority: the page's fold is what a create or a save reads when it matters.
use crate::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    page_index::{self, Kept},
    store::Store,
    template_marks,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A page's marks: neither one for most pages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub template: Option<Template>,
    pub from: Option<Origin>,
}

/// The page is a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub description: String,
    /// The page's layout kind (`freeform` when it names none).
    pub layout: String,
    /// The page's original author, when its source records one.
    pub author: Option<String>,
}

/// What the page was created from: a template's name and, for a user template, its revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub name: String,
    pub revision: Option<String>,
}

/// The marks of a validated content metadata, from a fold that already happened.
pub fn marks_of(meta: &Value, author: Option<&str>) -> Marks {
    Marks {
        template: template_marks::mark_of_meta(meta).map(|(name, description)| Template {
            name: name.to_owned(),
            description: description.to_owned(),
            layout: crate::decoder::layout_of_meta(meta)
                .map_or("freeform", |layout| layout.kind.as_str())
                .to_owned(),
            author: author.map(str::to_owned),
        }),
        from: template_marks::source_of_meta(meta).map(|(name, revision)| Origin {
            name: name.to_owned(),
            revision: revision.map(str::to_owned),
        }),
    }
}

impl Kept for Marks {
    const FILE: &'static str = "template-index.json";
    const LOCK: &'static str = "template-index.lock";
    const VERSION: u64 = 1;
    fn to_json(&self) -> Value {
        let mut entry = json!({});
        if let Some(template) = &self.template {
            entry["template"] = json!({
                "name": template.name,
                "description": template.description,
                "layout": template.layout,
            });
            if let Some(author) = &template.author {
                entry["template"]["author"] = json!(author);
            }
        }
        if let Some(from) = &self.from {
            entry["from"] = json!({"name": from.name});
            if let Some(revision) = &from.revision {
                entry["from"]["revision"] = json!(revision);
            }
        }
        entry
    }
    fn from_json(_page: &str, entry: &Value) -> Option<Self> {
        let template = match entry.get("template") {
            None => None,
            Some(mark) => Some(Template {
                name: mark["name"].as_str()?.to_owned(),
                description: mark["description"].as_str()?.to_owned(),
                layout: mark["layout"].as_str()?.to_owned(),
                author: match mark.get("author") {
                    None => None,
                    Some(author) => Some(author.as_str()?.to_owned()),
                },
            }),
        };
        let from = match entry.get("from") {
            None => None,
            Some(from) => Some(Origin {
                name: from["name"].as_str()?.to_owned(),
                revision: match from.get("revision") {
                    None => None,
                    Some(revision) => Some(revision.as_str()?.to_owned()),
                },
            }),
        };
        Some(Self { template, from })
    }
}

/// The marks of each given page, from the index only: a page whose entry is missing or stale is
/// left out, without a fold, lock or write.
pub fn cached(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    pages: &[String],
) -> Result<BTreeMap<String, Marks>> {
    page_index::cached(layout, store, key, pages)
}

/// The marks of each given page, folding only the pages whose progress moved since the last scan. A
/// shell has none; a page that cannot be read now is left out and asked again next time.
pub fn scan(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    pages: &[String],
) -> Result<BTreeMap<String, Marks>> {
    page_index::scan(layout, store, key, pages, |id| {
        let snapshot = page::snapshot(store, key, id, false).ok()?;
        let view = snapshot.materialize(key, id, decoder).ok()?;
        Some(if view.meta["pageType"].as_str() == Some("shell") {
            Marks::default()
        } else {
            marks_of(&view.meta, view.original_author.as_deref())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_read_from_the_metadata_and_survive_the_file() {
        let meta = json!({
            "layout": "column",
            "template": {"name": "weekly-review", "description": "Weekly review"},
            "fromTemplate": {"name": "starter"},
        });
        let marks = marks_of(&meta, Some("ben"));
        assert_eq!(
            marks.template,
            Some(Template {
                name: "weekly-review".into(),
                description: "Weekly review".into(),
                layout: "column".into(),
                author: Some("ben".into()),
            })
        );
        assert_eq!(
            marks.from,
            Some(Origin {
                name: "starter".into(),
                revision: None
            })
        );
        let entry = page_index::entry_json(&marks, "f1");
        assert_eq!(entry["fingerprint"], "f1");
        assert_eq!(Marks::from_json("p", &entry), Some(marks));
    }

    #[test]
    fn a_page_without_marks_keeps_an_empty_entry_and_a_damaged_one_is_dropped() {
        let none = marks_of(&json!({"layout": "column"}), None);
        assert_eq!(none, Marks::default());
        assert_eq!(
            Marks::from_json("p", &page_index::entry_json(&none, "f")),
            Some(Marks::default())
        );
        // A page with no layout is a freeform page.
        let plain = marks_of(
            &json!({"template": {"name": "notes", "description": "d"}}),
            None,
        );
        assert_eq!(plain.template.unwrap().layout, "freeform");
        assert_eq!(
            Marks::from_json("p", &json!({"fingerprint":"f","template":{"name":"x"}})),
            None
        );
        assert_eq!(
            Marks::from_json("p", &json!({"fingerprint":"f","from":{"revision":"1"}})),
            None
        );
    }
}
