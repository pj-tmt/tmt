//! `status`: one list of members, the same order the board uses. Text is
//! rendered from the JSON document so the two views cannot disagree.

use crate::{
    config::{Layout, Section, SortKey, States},
    filter::Row,
    rows::Rows,
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

/// Missing values sort last in both directions; a bound column's numbers
/// sort as numbers, whatever their format shows.
fn compare(key: &SortKey, states: &States, a: &Member, b: &Member) -> Ordering {
    let (left, right) = (a.value(&key.field), b.value(&key.field));
    let numbers = (a.numbers.get(&key.field), b.numbers.get(&key.field));
    let ordering = match (left, right) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(left), Some(right)) if key.field == "state" => states
            .rank(Some(left))
            .cmp(&states.rank(Some(right)))
            .then_with(|| left.cmp(right)),
        (Some(left), Some(right)) => match numbers {
            (Some(left), Some(right)) => left.total_cmp(right),
            _ => left.cmp(right),
        },
    };
    if key.descending {
        ordering.reverse()
    } else {
        ordering
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// A bound or formatted column's value replaces the member's field of the
/// column's name, before anything reads it, so sections, filters, sorts, the
/// board and `ls` all see the same value. A missing value removes the field.
fn apply_sources(rows: &Rows, members: &mut [Member], now_ms: u64) {
    for column in &rows.columns {
        let Some(source) = column.source() else {
            continue;
        };
        for member in members.iter_mut() {
            match source.value(member, column.format, now_ms) {
                Some(value) => member.fields.insert(column.field.clone(), value),
                None => member.fields.remove(&column.field),
            };
            match source.number(member) {
                Some(number) => member.numbers.insert(column.field.clone(), number),
                None => member.numbers.remove(&column.field),
            };
        }
    }
}

fn rows(members: &[&Member]) -> Vec<Value> {
    members.iter().map(|member| member_value(member)).collect()
}

/// Without user-defined sections, `sections` holds exactly one untitled
/// section with every non-lead member; scripts never depend on the layout.
/// A user section shows every matching row (a row may appear in several),
/// ordered by its sort keys over the layout's default order; rows that match
/// no section follow in one untitled section, so nobody is hidden.
pub fn document(
    squad: &Squad,
    layout: Layout,
    states: &States,
    sections: &[Section],
    row_layout: &Rows,
    mut members: Vec<Member>,
) -> Value {
    apply_sources(row_layout, &mut members, now_ms());
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
            .chain({
                // Nobody is hidden: rows no section matched follow, untitled.
                let rest: Vec<&Member> = all
                    .iter()
                    .copied()
                    .filter(|member| !sections.iter().any(|section| section.includes(*member)))
                    .collect();
                (!rest.is_empty()).then(|| json!({"title": null, "rows": rows(&rest)}))
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

/// A column's value in a row: `member` is the name, `state` shows `-` when
/// unset, and any other unset field stays empty so sparse columns stay quiet.
fn column_cell<'a>(row: &'a Value, field: &str) -> &'a str {
    match field {
        "member" => cell(&row["name"]),
        "state" => cell(&row["state"]),
        field => row["fields"][field].as_str().unwrap_or_default(),
    }
}

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

/// Plain or styled for `terminal`; the same parts as every TMT list. One
/// squad's document, or `{squads: [...]}` for every squad in turn.
pub fn text(document: &Value, terminal: Terminal) -> String {
    let mut output = Vec::new();
    let squads: Vec<&Value> = match document["squads"].as_array() {
        Some(squads) => squads.iter().collect(),
        None => vec![document],
    };
    if squads.is_empty() {
        let _ = writeln!(output, "No squad exists yet.");
        let _ = tmt_cli_style::message::hint(&mut output, terminal, "tmt squad init <name>");
    }
    for (index, squad) in squads.iter().enumerate() {
        if index > 0 {
            let _ = writeln!(output);
        }
        squad_text(squad, terminal, &mut output);
    }
    if !squads.is_empty() && document.get("you").is_some_and(Value::is_null) {
        let _ = writeln!(output, "\n{}", terminal.paint(Token::Dim, UNKNOWN_YOU));
    }
    String::from_utf8(output).unwrap_or_default()
}

/// One squad: its header, then each section with the configured columns (the
/// board's), a leading mark and a trailing detail.
fn squad_text(document: &Value, terminal: Terminal, output: &mut Vec<u8>) {
    let squad = &document["squad"];
    let header = format!(
        "squad {} · lead {} · layout {}",
        cell(&squad["name"]),
        cell(&squad["lead"]["name"]),
        cell(&squad["layout"]),
    );
    let _ = writeln!(output, "{}\n", terminal.paint(Token::Dim, &escape(&header)));
    // The board's field selection and order: every field on any line, the
    // first line's first. A list stays complete, so only a configured width
    // (a short fixed value) keeps a column from truncating.
    let width = |field: &str| {
        document["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|column| column["field"] == field)
            .and_then(|column| column["width"].as_u64())
    };
    let mut fields: Vec<(&str, Option<u64>)> = Vec::new();
    let lines = document["lines"].as_array().into_iter().flatten();
    for cell in lines.flat_map(|line| line.as_array().into_iter().flatten()) {
        if let Some(field) = cell["field"].as_str()
            && !fields.iter().any(|(known, _)| *known == field)
        {
            fields.push((field, width(field)));
        }
    }
    if fields.is_empty() {
        fields = vec![("member", None), ("state", Some(10))];
    }
    let layout: Vec<Column> = std::iter::once(Column::Fixed)
        .chain(fields.iter().map(|(field, width)| match (*field, width) {
            ("member", _) => Column::Name,
            (_, None) => Column::Detail,
            (_, Some(_)) => Column::Fixed,
        }))
        .chain(std::iter::once(Column::Detail))
        .collect();
    let built: Vec<(String, usize, Table)> = document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|section| {
            let rows = section["rows"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut table = Table::new(&layout);
            for row in rows {
                let mark = mark(row);
                let cells = std::iter::once(Cell::styled(mark.symbol(), mark.token()))
                    .chain(fields.iter().map(|(field, _)| match *field {
                        "state" => Cell::styled(column_cell(row, "state"), Token::Dim),
                        field => column_cell(row, field).into(),
                    }))
                    .chain(std::iter::once(detail(row).into()));
                table.row(cells.collect::<Vec<Cell>>());
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
    let _ = list::write(output, terminal, &list);
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
            meta: Default::default(),
            seen: Value::Null,
            numbers: Default::default(),
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

    /// A bound column's value is the row's field of the column's name
    /// before sections, filters and sorts read it; it replaces what an agent
    /// wrote under that name, and a missing value leaves no field.
    #[test]
    fn a_bound_column_is_one_value_for_rows_sections_and_sorts() {
        let directory = std::env::temp_dir().join(format!("squad-bound-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(
            &path,
            "[[squad.product.section]]\ntitle = \"Busy\"\nfilter = \"ctx\"\nsort = [\"-ctx\"]\n\
             [squad.product.rows]\ncolumns = [\n  { name = \"member\" },\n  \
             { name = \"ctx\", from = \"session.usage.tokens\", format = \"tokens\" },\n  \
             { name = \"model\", from = \"session.model\" },\n]\n",
        )
        .unwrap();
        let config = crate::config::Config::read(path).unwrap();
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let used = |name: &str, tokens: u64| {
            let mut member = member(name, &[("state", "working")]);
            member.seen = json!({"resume": {"driver": "claude", "model": "opus",
                                            "usage": {"tokens": tokens}}});
            member
        };
        // An agent wrote `ctx` itself; the binding decides.
        let fresh = member("new", &[("ctx", "999k")]);
        let document = document(
            &squad,
            Layout::Crew,
            &states(Layout::Crew),
            &config.sections("product").unwrap(),
            &config.rows("product").unwrap(),
            vec![
                used("amy", 250_000),
                fresh,
                used("zed", 487_123),
                used("kai", 1_234_567),
            ],
        );
        let busy = &document["sections"][0];
        assert_eq!(busy["title"], "Busy");
        let rows: Vec<(&str, &str, &str)> = busy["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["name"].as_str().unwrap(),
                    row["fields"]["ctx"].as_str().unwrap(),
                    row["fields"]["model"].as_str().unwrap(),
                )
            })
            .collect();
        // Numbers sort as numbers: 1.2M before 487k, although "4" > "1".
        assert_eq!(
            rows,
            [
                ("kai", "1.2M", "opus"),
                ("zed", "487k", "opus"),
                ("amy", "250k", "opus")
            ]
        );
        let rest = &document["sections"][1]["rows"][0];
        assert_eq!(rest["name"], "new");
        assert_eq!(rest["fields"].get("ctx"), None, "no session, no value");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn crew_puts_pending_first_then_state_order_and_keeps_one_shape() {
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let crew = document(
            &squad,
            Layout::Crew,
            &states(Layout::Crew),
            &[],
            &Rows::preset(),
            members(),
        );
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
            &Rows::preset(),
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
            &Rows::preset(),
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

    #[test]
    fn rows_no_section_matches_follow_untitled_so_nobody_is_hidden() {
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let path = std::env::temp_dir().join(format!("tmt-squad-rest-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[squad.product.section]]\ntitle = \"Needs me\"\nfilter = \"pending\"\n",
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
            &Rows::preset(),
            members(),
        );
        let listed: Vec<(Value, Vec<&str>)> = document["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|section| {
                (
                    section["title"].clone(),
                    section["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row["name"].as_str().unwrap())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (json!("Needs me"), vec!["bob"]),
                (Value::Null, vec!["zed", "amy", "kai"]),
            ]
        );
        let all_matched = super::document(
            &squad,
            Layout::Crew,
            &states(Layout::Crew),
            &sections,
            &Rows::preset(),
            vec![member("bob", &[("pending", "x")])],
        );
        assert_eq!(
            all_matched["sections"].as_array().unwrap().len(),
            1,
            "no empty trailing section"
        );
    }

    #[test]
    fn ls_text_uses_the_configured_columns_and_lists_several_squads_in_turn() {
        let squad = |name: &str| Squad {
            name: name.into(),
            room_id: format!("room-{name}"),
        };
        let config: toml_edit::DocumentMut =
            "[p.columns]\nshow = [\"member\", \"state\", \"task\"]\n"
                .parse()
                .unwrap();
        let rows = crate::rows::read(config["p"].as_table_like(), "p")
            .unwrap()
            .value();
        let with_columns = |name: &str, members: Vec<Member>| {
            let mut document = document(
                &squad(name),
                Layout::Crew,
                &states(Layout::Crew),
                &[],
                &Rows::preset(),
                members,
            );
            document["columns"] = rows["columns"].clone();
            document["lines"] = rows["lines"].clone();
            document
        };
        let product = with_columns(
            "product",
            vec![member(
                "zed",
                &[("state", "working"), ("task", "cache room reads")],
            )],
        );
        assert_eq!(
            text(&product, Terminal::PLAIN),
            "squad product · lead - · layout crew\n\n\
             MEMBERS 1\n\
             \x20 ○  zed  working  cache room reads\n"
        );
        let both = json!({
            "squads": [product, with_columns("reviews", vec![member("amy", &[("state", "review")])])],
            "you": null,
        });
        let rendered = text(&both, Terminal::PLAIN);
        assert_eq!(
            rendered,
            "squad product · lead - · layout crew\n\n\
             MEMBERS 1\n\
             \x20 ○  zed  working  cache room reads\n\
             \n\
             squad reviews · lead - · layout crew\n\n\
             MEMBERS 1\n\
             \x20 ○  amy  review\n\
             \n◆ needs to know who you are: tmt squad me <name>\n"
        );
    }
}
