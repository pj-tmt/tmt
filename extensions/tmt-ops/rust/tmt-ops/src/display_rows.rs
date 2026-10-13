//! Independent squad-document row order: lead first, then authored sections.
//! Text `ls` consumes this projection; JSON keeps the lead outside `sections`.
//! Interactive workspace geometry uses UUID roster targets instead.

use serde_json::Value;

/// Where a displayed row came from. Section slots are authored layout scopes,
/// not member positions; the lead has one occurrence of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowOrigin {
    Lead,
    Section(usize),
}

impl RowOrigin {
    /// The authored section, which alone can override bindings.
    pub fn section(self) -> Option<usize> {
        match self {
            Self::Lead => None,
            Self::Section(index) => Some(index),
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
    /// An authored section starts, with its title when it has one.
    Section(Option<&'a str>),
    Row(RowOrigin, &'a Value),
}

/// Rows `keep` admits in source order. Filter the lead independently and retain
/// authored section headers even when their admitted rows are empty.
pub fn project<'a>(document: &'a Value, keep: impl Fn(&Value) -> bool) -> Vec<Item<'a>> {
    let mut items = Vec::new();
    let lead = &document["squad"]["lead"];
    let has_lead = lead.is_object();
    if has_lead && keep(lead) {
        items.push(Item::Row(RowOrigin::Lead, lead));
    }
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
        items.push(Item::Section(section["title"].as_str()));
        for row in rows.iter().filter(|row| keep(row)) {
            items.push(Item::Row(RowOrigin::Section(index), row));
        }
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
            .filter_map(|item| match item {
                Item::Section(title) => title.map(|title| format!("# {title}")),
                Item::Row(slot, row) => Some(format!("{slot:?} {}", row["name"].as_str().unwrap())),
            })
            .collect()
    }

    fn all(_: &Value) -> bool {
        true
    }

    #[test]
    fn mutation_visits_lead_and_each_authored_occurrence_without_rewriting_membership() {
        let mut document = document(
            json!({"id":"lead","name":"sol"}),
            json!([
                {"title":"first","rows":[{"id":"a","name":"amy"}]},
                {"title":"second","rows":[{"id":"a","name":"amy"}]},
            ]),
        );
        let mut visits = 0;
        each_row(&mut document, |row| {
            visits += 1;
            row["visited"] = json!(visits);
        });
        assert_eq!(visits, 3);
        assert_eq!(document["squad"]["lead"]["visited"], 1);
        assert_eq!(document["sections"][0]["rows"][0]["visited"], 2);
        assert_eq!(document["sections"][1]["rows"][0]["visited"], 3);
        assert_eq!(
            shape(&project(&document, all)),
            [
                "Lead sol",
                "# first",
                "Section(0) amy",
                "# second",
                "Section(1) amy"
            ]
        );
        assert_eq!(document["sections"][0]["rows"][0]["id"], "a");
        assert_eq!(document["sections"][1]["rows"][0]["id"], "a");
    }

    #[test]
    fn the_lead_comes_first_then_the_authored_sections() {
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
    fn a_lead_with_no_members_remains_a_source_row() {
        let document = document(
            json!({"id": "s", "name": "sol"}),
            json!([{"title": null, "rows": []}]),
        );
        assert_eq!(shape(&project(&document, all)), ["Lead sol"]);
    }

    #[test]
    fn a_search_filters_lead_and_members_without_reordering_sections() {
        let document = document(
            json!({"id": "s", "name": "sol"}),
            json!([{"title": null, "rows": [{"id": "a", "name": "amy"}]}]),
        );
        let only = |name: &'static str| move |row: &Value| row["name"] == name;
        assert_eq!(shape(&project(&document, only("amy"))), ["Section(0) amy"]);
        assert_eq!(shape(&project(&document, only("sol"))), ["Lead sol"]);
        assert!(shape(&project(&document, |_| false)).is_empty());
    }
}
