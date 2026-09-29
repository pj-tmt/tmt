//! `status`: one list of members, the same order the board uses. Text is
//! rendered from the JSON document so the two views cannot disagree.

use crate::{
    config::{Layout, Section, SortKey, States},
    filter::Row,
    squad::{Member, Squad},
};
use serde_json::{Value, json};
use std::{cmp::Ordering, io::Write};
use tmt_cli_style::{
    Terminal, Token,
    list::{self, Section as ListSection},
    mark::Mark,
    table::{Cell, Column, Table, escape},
};

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
fn sort(rows: &mut [Member], layout: Layout, states: &States) {
    let rank = |member: &Member| {
        let pending = layout.pending_first() && !member.fields.contains_key("pending");
        let state = member.fields.get("state").map(String::as_str);
        let order = states.rank(state);
        (
            pending,
            order,
            state.unwrap_or_default().to_owned(),
            member.name.clone(),
        )
    };
    rows.sort_by_key(rank);
}

/// Missing values sort last in both directions.
fn compare(key: &SortKey, states: &States, a: &Member, b: &Member) -> Ordering {
    let (left, right) = (a.value(&key.field), b.value(&key.field));
    let ordering = match (left, right) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(left), Some(right)) if key.field == "state" => states
            .rank(Some(left))
            .cmp(&states.rank(Some(right)))
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
    states: &States,
    sections: &[Section],
    members: Vec<Member>,
) -> Value {
    let (leads, mut members): (Vec<_>, Vec<_>) = members.into_iter().partition(Member::is_lead);
    sort(&mut members, layout, states);
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
                        .map(|key| compare(key, states, a, b))
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

const COLUMNS: [Column; 4] = [Column::Fixed, Column::Name, Column::Fixed, Column::Detail];

/// One leading mark: `◆` when the member waits on the reader's decision,
/// otherwise its presence.
fn mark(row: &Value) -> Mark {
    if row["pending"].is_string() {
        Mark::Decision
    } else {
        match row["presence"].as_str() {
            Some("active") => Mark::Running,
            Some("offline") => Mark::Offline,
            _ => Mark::Idle,
        }
    }
}

/// What the reader needs first: the decision owed, the note, the latest
/// annotation.
fn detail(row: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(pending) = row["pending"].as_str() {
        parts.push(format!("waiting on you: {pending}"));
    }
    if let Some(note) = row["note"].as_str() {
        parts.push(format!("note: {note}"));
    }
    if let Some(text) = row["annotation"]["text"].as_str() {
        parts.push(format!("✎ to {}: {text}", cell(&row["annotation"]["to"])));
    }
    parts.join(" · ")
}

/// Plain or styled for `terminal`; the same parts as every TMT list.
pub fn text(document: &Value, terminal: Terminal) -> String {
    let squad = &document["squad"];
    let mut output = Vec::new();
    let header = format!(
        "squad {} · lead {} · layout {}",
        cell(&squad["name"]),
        cell(&squad["lead"]["name"]),
        cell(&squad["layout"]),
    );
    let _ = writeln!(output, "{}\n", terminal.paint(Token::Dim, &escape(&header)));
    let sections: Vec<&Value> = document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let built: Vec<(String, usize, Table)> = sections
        .iter()
        .map(|section| {
            let rows = section["rows"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut table = Table::new(&COLUMNS);
            for row in rows {
                let mark = mark(row);
                table.row([
                    Cell::styled(mark.symbol(), mark.token()),
                    cell(&row["name"]).into(),
                    Cell::styled(cell(&row["state"]), Token::Dim),
                    detail(row).into(),
                ]);
            }
            let title = section["title"].as_str().unwrap_or("members").to_owned();
            (title, rows.len(), table)
        })
        .collect();
    let older = (document["olderRequestsNotShown"] == true).then_some("older requests not shown");
    let last = built.len().saturating_sub(1);
    let list: Vec<ListSection<'_>> = built
        .iter()
        .enumerate()
        .map(|(index, (title, count, table))| ListSection {
            title,
            count: Some(*count),
            rows: table.clone(),
            note: if table.is_empty() {
                Some("(no members)")
            } else if index == last {
                older
            } else {
                None
            },
            hint: None,
        })
        .collect();
    let _ = list::write(&mut output, terminal, &list);
    if document.get("you").is_some_and(Value::is_null) {
        let _ = writeln!(output, "\n{}", terminal.paint(Token::Dim, UNKNOWN_YOU));
    }
    String::from_utf8(output).unwrap_or_default()
}

/// Shown when no one is "you": ◆ for requests needs a recorded identity or a
/// saved identity bound to the calling pane.
pub const UNKNOWN_YOU: &str = "◆ needs to know who you are: tmt squad me <name>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decision_owed_leads_the_row_over_presence() {
        let row = |presence: &str, pending: Option<&str>| json!({"presence": presence, "pending": pending});
        assert_eq!(mark(&row("active", Some("approve"))), Mark::Decision);
        assert_eq!(mark(&row("offline", Some("approve"))), Mark::Decision);
        assert_eq!(mark(&row("active", None)), Mark::Running);
        assert_eq!(mark(&row("offline", None)), Mark::Offline);
        assert_eq!(mark(&row("unknown", None)), Mark::Idle);
    }

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

    fn states(layout: Layout) -> States {
        States {
            order: layout
                .states()
                .iter()
                .map(|state| (*state).to_owned())
                .collect(),
            colors: Default::default(),
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
        let crew = document(&squad, Layout::Crew, &states(Layout::Crew), &[], members());
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

        let minimal = document(
            &squad,
            Layout::Minimal,
            &states(Layout::Minimal),
            &[],
            members(),
        );
        // No vocabulary: equal states group together, then names.
        assert_eq!(names(&minimal), ["bob", "kai", "amy", "zed"]);

        // One leading mark: ◆ when the member waits on you, else presence.
        assert_eq!(
            text(&crew, Terminal::PLAIN),
            "squad product · lead sol · layout crew\n\n\
             MEMBERS 4\n\
             \x20 ◆  bob  blocked  waiting on you: approve the plan · note: needs a call\n\
             \x20 ○  zed  working\n\
             \x20 ○  amy  review\n\
             \x20 ○  kai  custom\n"
        );
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
        let document = document(
            &squad,
            Layout::Crew,
            &states(Layout::Crew),
            &sections,
            members(),
        );
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
        let rendered = text(&document, Terminal::PLAIN);
        assert!(rendered.contains("NEEDS ME 1\n  ◆  bob  blocked  waiting on you"));
        assert!(rendered.ends_with("EMPTY 0\n    (no members)\n"));
    }
}
