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
    let providers: Vec<&Value> = document["limitHistory"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    // The account is named only where a driver has more than one; the plain driver beside them is
    // the sessions that name none (#5275).
    let accounts = |driver: &str| {
        providers
            .iter()
            .filter(|p| p["provider"] == driver && p["account"].is_object())
            .count()
    };
    let mut count = 0;
    for provider in providers.iter().copied() {
        let points = provider["points"].as_array().map_or(&[][..], Vec::as_slice);
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            continue;
        };
        count += 1;
        let driver = provider["provider"].as_str().unwrap_or_default();
        let name = match (provider["account"]["label"].as_str(), accounts(driver) > 1) {
            (Some(label), true) => format!("{driver} · {label}"),
            (None, true) => format!("{driver} · unknown account"),
            _ => driver.to_owned(),
        };
        rows.row([
            Cell::from(name),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(provider: &str, account: Option<(&str, &str)>) -> Value {
        let mut entry = json!({"provider": provider, "windowMinutes": 10080,
            "points": [[1_792_000_000_000u64, 80.5, 1_792_500_000_000u64]]});
        if let Some((key, label)) = account {
            entry["account"] = json!({"key": key, "label": label});
        }
        entry
    }

    fn names(document: Value) -> Vec<String> {
        text(&document, Terminal::PLAIN)
            .lines()
            .filter(|line| line.contains("% left"))
            .map(|line| {
                line.split_whitespace()
                    .take_while(|word| !word.chars().all(|c| c.is_ascii_digit()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn a_driver_without_accounts_or_with_one_prints_its_name_as_before() {
        let plain =
            json!({"version": 1, "limitHistory": [entry("claude", None), entry("codex", None)]});
        assert_eq!(names(plain), ["claude", "codex"]);
        let one = json!({"version": 1, "limitHistory": [entry("claude", Some(("0123456789ab", "work")))]});
        assert_eq!(names(one), ["claude"]);
    }

    #[test]
    fn several_accounts_print_their_labels_and_the_plain_driver_beside_them_is_the_unknown_one() {
        let several = json!({"version": 1, "limitHistory": [
            entry("claude", None),
            entry("claude", Some(("0123456789ab", "work"))),
            entry("claude", Some(("ba9876543210", "home"))),
            entry("codex", None),
        ]});
        assert_eq!(
            names(several),
            [
                "claude · unknown account",
                "claude · work",
                "claude · home",
                "codex"
            ]
        );
    }
}
