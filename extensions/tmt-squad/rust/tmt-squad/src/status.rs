//! `status`: one list of members, the same order the board uses. Text is
//! rendered from the JSON document so the two views cannot disagree.

use crate::{
    config::{Layout, Rank, Section, SortKey, States},
    filter::Row,
    rows::{Column as RowColumn, ListSizing, Rows},
    squad::{Member, Squad},
};
use serde_json::{Value, json};
use std::{cmp::Ordering, io::Write};
use tmt_cli_style::{
    Terminal, Token,
    grid::{self, Overflow},
    list::{self, Section as ListSection},
    mark::Mark,
    table::{Cell, Column, Table, escape},
};

/// A member's row as `ls --json` reports it; templates fill from it.
pub fn row(member: &Member) -> Value {
    member_value(member)
}

fn member_value(member: &Member) -> Value {
    let field = |name: &str| {
        member
            .fields
            .get(name)
            .map_or(Value::Null, |value| value.as_str().into())
    };
    let mut row = json!({
        "id": member.id, "name": member.name, "lifetime": member.lifetime,
        "presence": member.presence, "pane": member.pane, "activity": member.activity,
        "state": field("state"), "pending": field("pending"),
        "fields": member.fields, "failed": member.failed, "staleness": crate::staleness::unavailable("disabled"),
    });
    if !member.colors.is_empty() {
        row["colors"] = json!(member.colors);
    }
    row
}

/// Crew puts rows that owe the user a decision first; then the layout's state
/// order (unknown states after known ones); then name.
pub(crate) fn sort(rows: &mut [Member], layout: Layout, states: &States) {
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
    compare_values(
        key,
        a,
        b,
        (a.numbers.get(&key.field), b.numbers.get(&key.field)),
        (states.rank(a.value("state")), states.rank(b.value("state"))),
    )
}

pub(crate) fn compare_values(
    key: &SortKey,
    a: &impl Row,
    b: &impl Row,
    numbers: (Option<&f64>, Option<&f64>),
    ranks: (Rank, Rank),
) -> Ordering {
    let (left, right) = (a.value(&key.field), b.value(&key.field));
    let ordering = match (left, right) {
        (None, None) => return Ordering::Equal,
        (None, Some(_)) => return Ordering::Greater,
        (Some(_), None) => return Ordering::Less,
        (Some(left), Some(right)) if key.field == "state" => {
            ranks.0.cmp(&ranks.1).then_with(|| left.cmp(right))
        }
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

pub(crate) fn now_ms() -> u64 {
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

/// Each cell's color token: a column's threshold the value reaches wins over
/// a provider's suggested token; otherwise the cell has none. The value is
/// the bound number when there is one, else the field's text read as a
/// number. `state` keeps its state colors.
fn apply_colors(rows: &Rows, states: &States, members: &mut [Member]) {
    for member in members.iter_mut() {
        member.colors.remove("state");
        if let Some(token) = states.color(member.fields.get("state").map(String::as_str)) {
            member.colors.insert("state".into(), token.to_owned());
        }
        for column in rows
            .columns
            .iter()
            .filter(|column| !column.color.is_empty())
        {
            if column.field == "state" {
                continue;
            }
            let number = member.numbers.get(&column.field).copied().or_else(|| {
                member
                    .fields
                    .get(&column.field)
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .filter(|number| number.is_finite())
            });
            if let Some(token) = number.and_then(|number| column.threshold(number)) {
                member.colors.insert(column.field.clone(), token.to_owned());
            }
        }
    }
}

/// One section/filter/sort pipeline for acquired members and projected tab rows.
pub(crate) fn sections<T: Row>(
    all: &[T],
    sections: &[Section],
    compare: impl Fn(&SortKey, &T, &T) -> Ordering,
    project: impl Fn(&T) -> Value,
) -> Value {
    let rows = |items: &[&T]| items.iter().map(|row| project(row)).collect::<Vec<_>>();
    let all = all.iter().collect::<Vec<_>>();
    if sections.is_empty() {
        return json!([{"title": null, "rows": rows(&all)}]);
    }
    let mut shaped = sections
        .iter()
        .map(|section| {
            let mut matching = all
                .iter()
                .copied()
                .filter(|row| section.includes(*row))
                .collect::<Vec<_>>();
            matching.sort_by(|a, b| {
                section
                    .sort
                    .iter()
                    .map(|key| compare(key, a, b))
                    .find(|order| order.is_ne())
                    .unwrap_or(Ordering::Equal)
            });
            json!({"title": section.title, "rows": rows(&matching)})
        })
        .collect::<Vec<_>>();
    let rest = all
        .iter()
        .copied()
        .filter(|row| !sections.iter().any(|section| section.includes(*row)))
        .collect::<Vec<_>>();
    if !rest.is_empty() {
        shaped.push(json!({"title": null, "rows": rows(&rest)}));
    }
    json!(shaped)
}

pub(crate) fn prepare(row_layout: &Rows, states: &States, members: &mut [Member]) {
    apply_sources(row_layout, members, now_ms());
    apply_colors(row_layout, states, members);
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
    prepare(row_layout, states, &mut members);
    prepared_document(squad, layout, states, sections, members)
}

pub(crate) fn prepared_document(
    squad: &Squad,
    layout: Layout,
    states: &States,
    sections: &[Section],
    members: Vec<Member>,
) -> Value {
    let (leads, mut members): (Vec<_>, Vec<_>) = members.into_iter().partition(Member::is_lead);
    sort(&mut members, layout, states);
    let sections = self::sections(
        &members,
        sections,
        |key, a, b| compare(key, states, a, b),
        member_value,
    );
    json!({
        "squad": {
            "name": squad.name, "roomId": squad.room_id, "layout": layout.as_str(),
            "lead": leads.first().map_or(Value::Null, member_value), "notesStaleness": crate::staleness::unavailable("disabled"),
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

/// What the reader needs first: the decision owed and the latest annotation.
fn detail(row: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(pending) = row["pending"].as_str() {
        parts.push(format!("waiting on you: {pending}"));
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
    for failure in document["failures"].as_array().into_iter().flatten() {
        let source = failure["squad"]
            .as_str()
            .unwrap_or_else(|| failure["source"].as_str().unwrap_or("read"));
        let what = format!(
            "Partial tab: {source}: {}",
            cell(&failure["error"]["message"])
        );
        let _ = tmt_cli_style::message::warning(&mut output, terminal, &escape(&what), None);
    }
    if !squads.is_empty() && document.get("you").is_some_and(Value::is_null) {
        let _ = writeln!(output, "\n{}", terminal.paint(Token::Dim, UNKNOWN_YOU));
    }
    String::from_utf8(output).unwrap_or_default()
}

/// Table's two-cell indent, one-cell row mark and two-cell gap before data.
const LIST_PREFIX: usize = 2 + 1 + 2;

/// One squad: its header, then each section with the configured columns (the
/// board's), a leading mark and a trailing detail.
fn squad_text(document: &Value, terminal: Terminal, output: &mut Vec<u8>) {
    let squad = &document["squad"];
    let header = if let Some(tab) = document["tab"].as_str() {
        format!("tab {tab}")
    } else {
        format!(
            "squad {} · {} · layout {}",
            cell(&squad["name"]),
            squad["lead"]["name"]
                .as_str()
                .map_or_else(|| "no lead".to_owned(), |name| format!("lead {name}")),
            cell(&squad["layout"]),
        )
    };
    let _ = writeln!(output, "{}\n", terminal.paint(Token::Dim, &escape(&header)));
    if let Some(stale) = crate::staleness::label(&squad["notesStaleness"]) {
        let _ = writeln!(
            output,
            "{}\n",
            terminal.paint(Token::Dim, &format!("lead notes: {stale}"))
        );
    }
    // Flatten configured lines in order; the shared grid owns sizing and
    // fitting, while the list/table renderer still owns sections and styles.
    let mut fields = Vec::new();
    let hidden = document["hidden_columns"].as_array();
    for line in document["lines"].as_array().into_iter().flatten() {
        let mut position = 0;
        for cell in line.as_array().into_iter().flatten() {
            let end = position + cell["span"].as_u64().unwrap_or(1) as usize;
            let visible = (position..end).any(|index| {
                !hidden.is_some_and(|names| names.contains(&document["columns"][index]["field"]))
            });
            if let Some(field) = cell["field"].as_str()
                && visible
                && !fields.contains(&field)
                && !document["columns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|column| column["field"] == field)
                    .and_then(|column| column["from"].as_str())
                    .and_then(|path| crate::source::ColumnSource::parse(path, |_| true, |_| true))
                    .is_some_and(|source| source.board_only())
            {
                fields.push(field);
            }
            position = end;
        }
    }
    if fields.is_empty() && hidden.is_none() {
        fields = vec!["member", "state"];
    }
    let sections: Vec<_> = document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let rows: Vec<_> = sections
        .iter()
        .flat_map(|section| section["rows"].as_array().into_iter().flatten())
        .collect();
    let stale_column = rows.iter().any(|row| row["staleness"]["state"] == "stale");
    let mut columns: Vec<_> = fields
        .iter()
        .map(|field| {
            let value = document["columns"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|column| column["field"] == *field)
                .unwrap_or(&Value::Null);
            RowColumn::display(field, value)
        })
        .collect();
    // Value-only column metadata supplies no configured sizing to this flat list.
    let sizing = ListSizing::for_columns(&columns);
    columns.push(RowColumn::display("detail", &Value::Null));
    if stale_column {
        columns.push(RowColumn::display("staleness", &Value::Null));
    }
    let value = |row: &Value, index: usize| -> String {
        if index < fields.len() {
            column_cell(row, fields[index]).to_owned()
        } else if index == fields.len() {
            detail(row)
        } else {
            crate::staleness::label(&row["staleness"])
                .unwrap_or_default()
                .to_owned()
        }
    };
    let (shown, layout): (Vec<(usize, Option<usize>)>, Vec<Column>) = match sizing {
        ListSizing::Natural => (
            (0..columns.len()).map(|index| (index, None)).collect(),
            std::iter::once(Column::Fixed)
                .chain(columns.iter().map(|column| {
                    if column.field == "member" {
                        Column::Name
                    } else if column.width.is_some() || column.field == "staleness" {
                        Column::Fixed
                    } else {
                        Column::Detail
                    }
                }))
                .collect(),
        ),
        ListSizing::Configured => {
            let natural: Vec<_> = (0..columns.len())
                .map(|index| {
                    rows.iter()
                        .map(|row| {
                            unicode_width::UnicodeWidthStr::width(
                                escape(&value(row, index)).as_str(),
                            )
                        })
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let tracks: Vec<_> = columns
                .iter()
                .zip(&natural)
                .map(|(column, natural)| column.track(*natural))
                .collect();
            // Indent, mark, and its gap are outside the data grid. A pipe
            // uses natural data widths plus gaps, before priority hiding.
            let available = terminal
                .width
                .map(|width| usize::from(width).saturating_sub(LIST_PREFIX))
                .unwrap_or_else(|| {
                    natural.iter().sum::<usize>() + 2 * columns.len().saturating_sub(1)
                });
            let shown: Vec<_> = grid::solve(&tracks, Some(available), 2)
                .into_iter()
                .enumerate()
                .filter_map(|(index, width)| width.map(|width| (index, Some(width))))
                .collect();
            let layout = vec![Column::Fixed; shown.len() + 1];
            (shown, layout)
        }
    };
    let built: Vec<(String, usize, Table)> = sections
        .iter()
        .map(|section| {
            let rows = section["rows"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut table = Table::new(&layout);
            for row in rows {
                let mark = mark(row);
                let fitted: Vec<_> = shown
                    .iter()
                    .map(|&(index, width)| {
                        let column = &columns[index];
                        match width {
                            Some(width) => grid::fit_lines(
                                &value(row, index),
                                width,
                                column.align,
                                column.truncate,
                                column.overflow.unwrap_or(Overflow::Ellipsis),
                            ),
                            None => vec![value(row, index)],
                        }
                    })
                    .collect();
                let height = fitted.iter().map(Vec::len).max().unwrap_or(1);
                for line in 0..height {
                    let mut cells = vec![Cell::styled(
                        if line == 0 { mark.symbol() } else { " " },
                        mark.token(),
                    )];
                    for ((index, width), fitted) in shown.iter().zip(&fitted) {
                        let text = fitted
                            .get(line)
                            .cloned()
                            .unwrap_or_else(|| " ".repeat(width.unwrap_or(0)));
                        cells.push(
                            if columns[*index].field == "state"
                                || columns[*index].field == "staleness"
                            {
                                Cell::styled(text, Token::Dim)
                            } else {
                                text.into()
                            },
                        );
                    }
                    table.row(cells);
                }
            }
            (
                section["title"].as_str().unwrap_or("members").to_owned(),
                rows.len(),
                table,
            )
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
    fn cell_colors_come_from_thresholds_then_providers_and_only_when_set() {
        let directory =
            std::env::temp_dir().join(format!("squad-cell-colors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(
            &path,
            "[squad.product.rows]\ncolumns = [\n  { name = \"member\" },\n  \
             { name = \"ctx\", from = \"session.usage.tokens\", format = \"tokens\",\n    \
               color = [{ at = 400000, token = \"review\" }, { at = 600000, token = \"red\" }] },\n  \
             { name = \"score\", color = [{ at = 10, token = \"working\" }] },\n  \
             { name = \"state\", color = [{ at = 0, token = \"blocked\" }] },\n]\n",
        )
        .unwrap();
        let config = crate::config::Config::read(path).unwrap();
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let used = |name: &str, tokens: u64, fields: &[(&str, &str)]| {
            let mut member = member(name, fields);
            member.seen = json!({"resume": {"usage": {"tokens": tokens}}});
            member
        };
        // A provider suggested a color for `ctx` and for `score`.
        let suggested = |mut member: Member| {
            member.colors.insert("ctx".into(), "link".into());
            member.colors.insert("score".into(), "accent".into());
            member
        };
        let document = document(
            &squad,
            Layout::Minimal,
            &states(Layout::Minimal),
            &[],
            &config.rows("product").unwrap(),
            vec![
                used("below", 399_999, &[("score", "9")]),
                used("at", 400_000, &[("score", "10")]),
                used("over", 650_000, &[("score", "ten"), ("state", "working")]),
                suggested(used("hinted", 399_999, &[("score", "12.5")])),
                suggested(used("quiet", 100, &[])),
                member("plain", &[]),
            ],
        );
        let colors = |name: &str| {
            document["sections"][0]["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == name)
                .unwrap()
                .get("colors")
                .cloned()
        };
        // Below the first threshold, and no suggestion: no key at all.
        assert_eq!(colors("below"), None);
        // Exactly `at` reaches it; the highest reached wins; an alias stays
        // as written, a token name. Text that is not a number gets none.
        assert_eq!(
            colors("at"),
            Some(json!({"ctx": "review", "score": "working"}))
        );
        assert_eq!(colors("over"), Some(json!({"ctx": "red"})));
        // A threshold the value reaches beats the provider's suggestion; one
        // it does not reach leaves the suggestion.
        assert_eq!(
            colors("hinted"),
            Some(json!({"ctx": "link", "score": "working"}))
        );
        assert_eq!(
            colors("quiet"),
            Some(json!({"ctx": "link", "score": "accent"}))
        );
        // `state` keeps its state colors; no missing value is colored.
        assert_eq!(colors("plain"), None);
        let _ = std::fs::remove_dir_all(&directory);
    }

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
            lead_marker: None,
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
            colors: Default::default(),
            failed: Default::default(),
        }
    }

    #[test]
    fn patterns_color_and_order_the_projected_rows_and_sections() {
        let directory =
            std::env::temp_dir().join(format!("squad-state-patterns-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(
            &path,
            r#"
[squad.product.states]
custom = { sort = 9 }
[[squad.product.state_patterns]]
match = "blocked*"
color = "blocked"
sort = 0
[[squad.product.state_patterns]]
match = "review*"
color = "review"
sort = 3
[[squad.product.section]]
title = "All"
sort = ["state"]
"#,
        )
        .unwrap();
        let config = crate::config::Config::read(path).unwrap();
        let states = config.states("product", Layout::Crew).unwrap();
        let squad = Squad {
            name: "product".into(),
            room_id: "room".into(),
        };
        let members = || {
            vec![
                member("unknown", &[("state", "unknown")]),
                member("working", &[("state", "working")]),
                member("review", &[("state", "review-on-ci")]),
                member("blocked", &[("state", "blocked-on-ci")]),
                member("custom", &[("state", "custom")]),
                member("missing", &[]),
            ]
        };
        let layout = Rows::preset();
        for sections in [Vec::new(), config.sections("product").unwrap()] {
            let mut members = members();
            members[0].colors.insert("state".into(), "red".into());
            let result = document(&squad, Layout::Crew, &states, &sections, &layout, members);
            let rows = result["sections"][0]["rows"].as_array().unwrap();
            assert_eq!(
                rows.iter()
                    .map(|row| row["name"].as_str().unwrap())
                    .take(4)
                    .collect::<Vec<_>>(),
                ["blocked", "working", "review", "custom"]
            );
            assert_eq!(rows[0]["colors"]["state"], "blocked");
            assert_eq!(rows[1]["colors"]["state"], "working");
            assert_eq!(rows[2]["colors"]["state"], "review");
            for row in &rows[3..] {
                assert!(row.get("colors").is_none(), "{row}");
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn states(layout: Layout) -> States {
        States::preset(layout)
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
                &[("state", "blocked"), ("pending", "approve the plan")],
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
        assert_eq!(bob["pending"], "approve the plan");
        assert!(!bob.as_object().unwrap().contains_key("note"));
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
             \x20 ◆  bob  blocked  waiting on you: approve the plan\n\
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
    fn opted_in_piped_lists_share_fitting_without_changing_full_row_values() {
        let config: toml_edit::DocumentMut = r#"[p.rows]
columns = [{ name = "member", width = "20%" },
           { name = "task", width = "40%", overflow = "wrap", max_lines = 2 }]
"#
        .parse()
        .unwrap();
        let rows = crate::rows::read(config["p"].as_table_like(), "p").unwrap();
        let task =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron";
        let mut document = document(
            &Squad {
                name: "p".into(),
                room_id: "room-p".into(),
            },
            Layout::Minimal,
            &states(Layout::Minimal),
            &[],
            &rows,
            vec![member("worker", &[("task", task)])],
        );
        let metadata = rows.value();
        document["columns"] = metadata["columns"].clone();
        document["lines"] = metadata["lines"].clone();
        let before = document.clone();
        let rendered = text(&document, Terminal::PLAIN);
        let data: Vec<_> = rendered
            .lines()
            .filter(|line| line.starts_with("  "))
            .collect();
        assert_eq!(data.len(), 2, "{rendered}");
        assert!(
            data[0].contains("worker") && data[0].contains("alpha beta"),
            "{rendered}"
        );
        assert!(
            data[1].contains('…') && !data[1].contains("worker"),
            "{rendered}"
        );
        assert_eq!(document, before);
        assert_eq!(document["sections"][0]["rows"][0]["fields"]["task"], task);
        let narrow = text(
            &document,
            Terminal {
                width: Some(20),
                ..Terminal::PLAIN
            },
        );
        assert!(
            narrow
                .lines()
                .filter(|line| line.starts_with("  "))
                .all(|line| unicode_width::UnicodeWidthStr::width(line) <= 20)
        );
    }

    #[test]
    fn value_only_text_fields_ignore_their_unused_track_settings() {
        let mut document = json!({
            "squad": {"name": "x", "layout": "minimal"},
            "columns": [
                {"field": "member", "width": 14},
                {"field": "task", "width": "40%", "overflow": "wrap", "max_lines": 2},
                {"field": "model", "width": 200, "min": 200, "max": 200, "grow": 100, "valueOnly": true}
            ],
            "lines": [[{"field": "member", "span": 1}, {"field": "task", "span": 1}], [{"field": null,"span": 1},{"field": "model","span": 1}]],
            "sections": [{"title": null, "rows": [{"name": "worker", "presence": "offline", "fields": {"task": "long task ".repeat(20), "model": "test-model"}}]}]
        });
        let before = document.clone();
        let rendered = text(
            &document,
            Terminal {
                width: Some(80),
                ..Terminal::PLAIN
            },
        );
        for key in ["width", "min", "max", "grow"] {
            document["columns"][2].as_object_mut().unwrap().remove(key);
        }
        assert_eq!(
            rendered,
            text(
                &document,
                Terminal {
                    width: Some(80),
                    ..Terminal::PLAIN
                }
            )
        );
        assert!(rendered.contains("test-model"));
        assert_eq!(before["sections"], document["sections"]);
    }

    #[test]
    fn ls_omits_board_only_values_and_text_columns_by_source_kind() {
        let config: toml_edit::DocumentMut = "[p.rows]\ncolumns=[{name='member'}, {name='observed',from='usage.w1',width=7}]\nlines=[['member','observed']]".parse().unwrap();
        let rows = crate::rows::read(config["p"].as_table_like(), "p").unwrap();
        let mut document = document(
            &Squad {
                name: "p".into(),
                room_id: "R".into(),
            },
            Layout::Crew,
            &states(Layout::Crew),
            &[],
            &rows,
            vec![member(
                "worker",
                &[("pending", "approve"), ("observed", "42")],
            )],
        );
        let metadata = rows.value();
        document["columns"] = metadata["columns"].clone();
        document["lines"] = metadata["lines"].clone();
        assert_eq!(document["columns"][1]["from"], "usage.w1");
        assert!(document["sections"][0]["rows"][0]["fields"]["observed"].is_null());
        let mut control = document.clone();
        control["columns"].as_array_mut().unwrap().truncate(1);
        control["lines"][0].as_array_mut().unwrap().truncate(1);
        document["sections"][0]["rows"][0]["fields"]["observed"] = json!("999");
        assert_eq!(
            text(&document, Terminal::PLAIN),
            text(&control, Terminal::PLAIN)
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
            "squad product · no lead · layout crew\n\n\
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
            "squad product · no lead · layout crew\n\n\
             MEMBERS 1\n\
             \x20 ○  zed  working  cache room reads\n\
             \n\
             squad reviews · no lead · layout crew\n\n\
             MEMBERS 1\n\
             \x20 ○  amy  review\n\
             \n◆ needs to know who you are: tmt squad me <name>\n"
        );
    }
}
