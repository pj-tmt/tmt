//! The one display order of a squad document's rows: the lead first, a members
//! rule, then the sections as authored. The board's cursor, sizing, paint and
//! hits and the text `ls` all read this projection; `--json` documents keep the
//! lead outside `sections` and never see it.

use serde_json::Value;
use std::collections::BTreeSet;

/// Where a displayed row came from. Section slots are authored layout scopes,
/// not member positions; the lead has one occurrence of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Lead,
    Section(usize),
}

impl Slot {
    /// The authored section, which alone can override bindings.
    pub fn section(self) -> Option<usize> {
        match self {
            Self::Lead => None,
            Self::Section(index) => Some(index),
        }
    }

    /// The stable scope of the row's admitted occurrence.
    pub fn scope(self) -> String {
        match self {
            Self::Lead => "lead".into(),
            Self::Section(index) => format!("section-{index}"),
        }
    }
}

/// The rule between the lead and the members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Distinct members shown, the lead excluded.
    pub members: usize,
    /// The squad has no member at all (a search never causes this).
    pub none_yet: bool,
}

impl Rule {
    pub fn label(&self) -> String {
        if self.none_yet {
            "members · 0 · none yet".into()
        } else {
            format!("members · {}", self.members)
        }
    }
}

/// Visits the lead and every section row; a member can appear in several
/// user sections, and each copy gets the same values.
pub fn each_row(document: &mut Value, mut visit: impl FnMut(&mut Value)) {
    let lead = &mut document["squad"]["lead"];
    if lead.is_object() {
        visit(lead);
    }
    for section in document["sections"].as_array_mut().into_iter().flatten() {
        for row in section["rows"].as_array_mut().into_iter().flatten() {
            visit(row);
        }
    }
}

pub enum Item<'a> {
    /// An authored section title.
    Header(&'a str),
    Rule(Rule),
    Row(Slot, &'a Value),
}

/// Rows `keep` admits in display order. A squad with a lead shows it first and
/// the rule after it, even when the search hides the lead or every member; a
/// squad without a lead keeps its sections as they are.
pub fn project<'a>(document: &'a Value, keep: impl Fn(&Value) -> bool) -> Vec<Item<'a>> {
    let mut items = Vec::new();
    let lead = &document["squad"]["lead"];
    let has_lead = lead.is_object();
    if has_lead && keep(lead) {
        items.push(Item::Row(Slot::Lead, lead));
    }
    let rule = items.len();
    let mut shown = BTreeSet::new();
    let mut unnamed = 0;
    let mut members = 0;
    for (index, section) in document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let rows = section["rows"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        members += rows.len();
        if let Some(title) = section["title"].as_str() {
            items.push(Item::Header(title));
        }
        for row in rows.iter().filter(|row| keep(row)) {
            match row["id"].as_str() {
                Some(id) => {
                    shown.insert(id);
                }
                None => unnamed += 1,
            }
            items.push(Item::Row(Slot::Section(index), row));
        }
    }
    if has_lead {
        items.insert(
            rule,
            Item::Rule(Rule {
                members: shown.len() + unnamed,
                none_yet: members == 0,
            }),
        );
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(lead: Value, sections: Value) -> Value {
        json!({"squad": {"name": "product", "lead": lead}, "sections": sections})
    }

    fn shape(items: &[Item<'_>]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Header(title) => format!("# {title}"),
                Item::Rule(rule) => format!("-- {}", rule.label()),
                Item::Row(slot, row) => format!("{slot:?} {}", row["name"].as_str().unwrap()),
            })
            .collect()
    }

    fn all(_: &Value) -> bool {
        true
    }

    #[test]
    fn the_lead_comes_first_then_the_rule_then_the_sections() {
        let document = document(
            json!({"id": "s", "name": "sol"}),
            json!([
                {"title": "busy", "rows": [{"id": "a", "name": "amy"}, {"id": "b", "name": "bob"}]},
                {"title": null, "rows": [{"id": "b", "name": "bob"}]},
            ]),
        );
        assert_eq!(
            shape(&project(&document, all)),
            [
                "Lead sol",
                "-- members · 2",
                "# busy",
                "Section(0) amy",
                "Section(0) bob",
                "Section(1) bob",
            ]
        );
    }

    #[test]
    fn a_squad_without_a_lead_keeps_its_sections() {
        let document = document(
            Value::Null,
            json!([{"title": null, "rows": [{"id": "a", "name": "amy"}]}]),
        );
        assert_eq!(shape(&project(&document, all)), ["Section(0) amy"]);
    }

    #[test]
    fn a_lead_with_no_members_says_so() {
        let document = document(
            json!({"id": "s", "name": "sol"}),
            json!([{"title": null, "rows": []}]),
        );
        assert_eq!(
            shape(&project(&document, all)),
            ["Lead sol", "-- members · 0 · none yet"]
        );
    }

    #[test]
    fn a_search_filters_the_lead_too_but_keeps_the_rule() {
        let document = document(
            json!({"id": "s", "name": "sol"}),
            json!([{"title": null, "rows": [{"id": "a", "name": "amy"}]}]),
        );
        let only = |name: &'static str| move |row: &Value| row["name"] == name;
        assert_eq!(
            shape(&project(&document, only("amy"))),
            ["-- members · 1", "Section(0) amy"]
        );
        assert_eq!(
            shape(&project(&document, only("sol"))),
            ["Lead sol", "-- members · 0"]
        );
        assert_eq!(shape(&project(&document, |_| false)), ["-- members · 0"]);
    }
}
