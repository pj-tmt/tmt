//! `status`: one list of members, the same order the board uses. Text is
//! rendered from the JSON document so the two views cannot disagree.

use crate::{
    config::{Layout, Section, SortKey},
    filter::Row,
    squad::{Member, Squad},
};
use serde_json::{Value, json};
use std::cmp::Ordering;

fn member_value(member: &Member) -> Value {
    let field = |name: &str| {
        member
            .fields
            .get(name)
            .map_or(Value::Null, |value| value.as_str().into())
    };
    json!({
        "id": member.id, "name": member.name, "lifetime": member.lifetime,
        "presence": member.presence, "pane": member.pane, "activity": member.activity,
        "state": field("state"), "pending": field("pending"), "note": field("note"),
        "fields": member.fields,
    })
}

/// Crew puts rows that owe the user a decision first; then the layout's state
/// order (unknown states after known ones); then name.
fn sort(rows: &mut [Member], layout: Layout) {
    let rank = |member: &Member| {
        let pending = layout.pending_first() && !member.fields.contains_key("pending");
        let state = member.fields.get("state").map(String::as_str);
        let order = state_rank(layout, state);
        (
            pending,
            order,
            state.unwrap_or_default().to_owned(),
            member.name.clone(),
        )
    };
    rows.sort_by_key(rank);
}

fn state_rank(layout: Layout, state: Option<&str>) -> usize {
    state
        .and_then(|state| layout.states().iter().position(|known| *known == state))
        .unwrap_or(layout.states().len())
}

/// Missing values sort last in both directions.
fn compare(key: &SortKey, layout: Layout, a: &Member, b: &Member) -> Ordering {
    let (left, right) = (a.value(&key.field), b.value(&key.field));
    let ordering = match (left, right) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(left), Some(right)) if key.field == "state" => state_rank(layout, Some(left))
            .cmp(&state_rank(layout, Some(right)))
            .then_with(|| left.cmp(right)),
        (Some(left), Some(right)) => left.cmp(right),
    };
    if key.descending {
        ordering.reverse()
    } else {
        ordering
    }
}

fn rows(members: &[&Member]) -> Vec<Value> {
    members.iter().map(|member| member_value(member)).collect()
}

/// Without user-defined sections, `sections` holds exactly one untitled
/// section with every non-lead member; scripts never depend on the layout.
/// A user section shows every matching row (a row may appear in several),
/// ordered by its sort keys over the layout's default order.
pub fn document(
    squad: &Squad,
    layout: Layout,
    sections: &[Section],
    members: Vec<Member>,
) -> Value {
    let (leads, mut members): (Vec<_>, Vec<_>) = members.into_iter().partition(Member::is_lead);
    sort(&mut members, layout);
    let all: Vec<&Member> = members.iter().collect();
    let sections: Vec<Value> = if sections.is_empty() {
        vec![json!({"title": null, "rows": rows(&all)})]
    } else {
        sections
            .iter()
            .map(|section| {
                let mut matching: Vec<&Member> = all
                    .iter()
                    .copied()
                    .filter(|member| section.includes(*member))
                    .collect();
                matching.sort_by(|a, b| {
                    section
                        .sort
                        .iter()
                        .map(|key| compare(key, layout, a, b))
                        .find(|ordering| ordering.is_ne())
                        .unwrap_or(Ordering::Equal)
                });
                json!({"title": section.title, "rows": rows(&matching)})
            })
            .collect()
    };
    json!({
        "squad": {
            "name": squad.name, "roomId": squad.room_id, "layout": layout.as_str(),
            "lead": leads.first().map_or(Value::Null, member_value),
        },
        "sections": sections,
    })
}

fn cell(value: &Value) -> &str {
    value.as_str().unwrap_or("-")
}

pub fn text(document: &Value) -> String {
    let squad = &document["squad"];
    let mut output = format!(
        "squad {}  lead: {}  layout: {}\n",
        cell(&squad["name"]),
        cell(&squad["lead"]["name"]),
        cell(&squad["layout"]),
    );
    for section in document["sections"].as_array().into_iter().flatten() {
        if let Some(title) = section["title"].as_str() {
            output.push_str(&format!("\n{title}\n"));
        }
        let rows = section["rows"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if rows.is_empty() {
            output.push_str("  (no members)\n");
        }
        for row in rows {
            let marker = if row["pending"].is_string() {
                '◆'
            } else {
                ' '
            };
            output.push_str(&format!(
                "{marker} {:<24} {:<10} {}\n",
                cell(&row["name"]),
                cell(&row["state"]),
                cell(&row["presence"]),
            ));
            if let Some(pending) = row["pending"].as_str() {
                output.push_str(&format!("    waiting on you: {pending}\n"));
            }
            if let Some(note) = row["note"].as_str() {
                output.push_str(&format!("    note: {note}\n"));
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, fields: &[(&str, &str)]) -> Member {
        Member {
            id: format!("id-{name}"),
            name: name.into(),
            lifetime: "temporary".into(),
            presence: "offline".into(),
            pane: Value::Null,
            activity: Value::Null,
            fields: fields
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }

    fn names(document: &Value) -> Vec<&str> {
        document["sections"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect()
    }

    fn members() -> Vec<Member> {
        vec![
            member("zed", &[("state", "working")]),
            member(
                "sol",
                &[
                    ("role", "lead"),
                    ("pending", "lead items stay in the header"),
                ],
            ),
            member("amy", &[("state", "review")]),
            member("kai", &[("state", "custom")]),
            member(
                "bob",
                &[
                    ("state", "blocked"),
                    ("pending", "approve the plan"),
                    ("note", "needs a call"),
                ],
            ),
        ]
    }

    #[test]
    fn crew_puts_pending_first_then_state_order_and_keeps_one_shape() {
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let crew = document(&squad, Layout::Crew, &[], members());
        assert_eq!(names(&crew), ["bob", "zed", "amy", "kai"]);
        assert_eq!(crew["squad"]["lead"]["name"], "sol");
        assert_eq!(crew["sections"].as_array().unwrap().len(), 1);
        assert_eq!(crew["sections"][0]["title"], Value::Null);
        let bob = &crew["sections"][0]["rows"][0];
        assert_eq!(
            (bob["pending"].as_str(), bob["note"].as_str()),
            (Some("approve the plan"), Some("needs a call"))
        );
        assert_eq!(crew["sections"][0]["rows"][1]["pending"], Value::Null);

        let minimal = document(&squad, Layout::Minimal, &[], members());
        // No vocabulary: equal states group together, then names.
        assert_eq!(names(&minimal), ["bob", "kai", "amy", "zed"]);

        let rendered = text(&crew);
        assert!(rendered.starts_with("squad product  lead: sol  layout: crew\n"));
        assert!(rendered.contains("◆ bob"));
        assert!(
            rendered.contains("    waiting on you: approve the plan\n    note: needs a call\n")
        );
        assert!(rendered.contains("  zed"));
    }

    #[test]
    fn user_sections_filter_and_sort_independently() {
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let path =
            std::env::temp_dir().join(format!("tmt-squad-status-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[squad.product.section]]\ntitle = \"Needs me\"\nfilter = \"pending or state = blocked\"\n\
             [[squad.product.section]]\ntitle = \"Everyone\"\nsort = [\"-name\"]\n\
             [[squad.product.section]]\ntitle = \"Empty\"\nfilter = \"state = merged\"\n",
        )
        .unwrap();
        let sections = crate::config::Config::read(path.clone())
            .unwrap()
            .sections("product")
            .unwrap();
        let _ = std::fs::remove_file(&path);
        let document = document(&squad, Layout::Crew, &sections, members());
        let titles: Vec<_> = document["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|section| section["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["Needs me", "Everyone", "Empty"]);
        let names = |index: usize| -> Vec<&str> {
            document["sections"][index]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["name"].as_str().unwrap())
                .collect()
        };
        assert_eq!(
            names(0),
            ["bob"],
            "the lead's own pending stays in the header"
        );
        assert_eq!(names(1), ["zed", "kai", "bob", "amy"]);
        assert!(names(2).is_empty());
        assert!(text(&document).contains("\nNeeds me\n◆ bob"));
    }
}
