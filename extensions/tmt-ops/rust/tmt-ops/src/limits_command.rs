//! `limits`: the weekly-limit samples the board keeps, read-only. It reads one cache file; it
//! needs no Core, config or board, and it never writes (sampling happens only while a board is open).
use crate::{board, cache};
use serde_json::Value;
use tmt_cli_style::{
    Terminal, Token,
    list::Section,
    table::{Cell, Column, Table},
};

pub fn run() -> Value {
    board::limits_history(cache::directory("limits"))
}

fn span(first: u64, last: u64) -> String {
    let minutes = last.saturating_sub(first) / 60_000;
    match (minutes / 1440, minutes / 60 % 24) {
        (0, 0) => format!("{minutes}m"),
        (0, hours) => format!("{hours}h"),
        (days, hours) => format!("{days}d {hours}h"),
    }
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    let mut rows = Table::new(&[Column::Name, Column::Fixed, Column::Fixed, Column::Fixed]);
    let providers = document["limitHistory"].as_array().into_iter().flatten();
    let mut count = 0;
    for provider in providers {
        let points = provider["points"].as_array().map_or(&[][..], Vec::as_slice);
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            continue;
        };
        count += 1;
        rows.row([
            Cell::from(provider["provider"].as_str().unwrap_or_default()),
            Cell::from(points.len().to_string()),
            Cell::from(format!("{}% left", last[1])),
            Cell::styled(
                span(
                    first[0].as_u64().unwrap_or(0),
                    last[0].as_u64().unwrap_or(0),
                ),
                Token::Dim,
            ),
        ]);
    }
    let note =
        (count == 0).then_some("No samples yet: the board samples limits only while it is open.");
    let mut output = Vec::new();
    let _ = Section {
        title: "WEEKLY LIMITS",
        count: Some(count),
        rows,
        note,
        hint: None,
    }
    .write(&mut output, terminal);
    String::from_utf8(output).unwrap_or_default()
}
