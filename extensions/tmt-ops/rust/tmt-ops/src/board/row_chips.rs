//! The chips beside a member's name: the labels its sources supplied, else the
//! Core digest policy's. Every surface asks here, so a row never shows both and
//! no painter knows which source a chip came from.
use crate::labels::Supplied;
use serde_json::{Value, json};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

const SEPARATOR: &str = " · ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub role: Role,
}

/// A row's chips in full. Supplied labels replace the policy chip; later chips
/// carry their own separator so a prefix of the list always reads whole.
pub fn of(supplied: &Supplied, row: &Value, now: u64) -> Vec<Chip> {
    let labels: Vec<_> = row["id"]
        .as_str()
        .into_iter()
        .flat_map(|id| supplied.of(id))
        .collect();
    if labels.is_empty() {
        return policy(row, now);
    }
    labels
        .into_iter()
        .enumerate()
        .map(|(index, (_, label))| {
            // Hidden bidi and format characters never reach the terminal.
            let text = super::notes::sanitize(&label.text);
            Chip {
                text: if index == 0 {
                    text
                } else {
                    format!("{SEPARATOR}{text}")
                },
                role: label.role,
            }
        })
        .collect()
}

/// Core's digest policy: the word, then what remains of it.
fn policy(row: &Value, now: u64) -> Vec<Chip> {
    let Some(label) = crate::digest::label(row, now) else {
        return Vec::new();
    };
    let (word, rest) = label.split_at(crate::digest::WORD.len());
    let mut chips = vec![Chip {
        text: word.into(),
        role: Role::Text,
    }];
    if !rest.is_empty() {
        chips.push(Chip {
            text: rest.into(),
            role: Role::Muted,
        });
    }
    chips
}

pub fn width(chips: &[Chip]) -> usize {
    chips.iter().map(|chip| chip.text.width()).sum()
}

/// How many leading chips fit `budget` columns. The first chip is never cut:
/// when it does not fit, none shows.
pub fn prefix(chips: &[Chip], budget: usize) -> usize {
    let mut used = 0;
    chips
        .iter()
        .take_while(|chip| {
            used += chip.text.width();
            used <= budget
        })
        .count()
}

/// The leading chips that fit `budget` columns, and whether that is all of them.
pub fn fitted(supplied: &Supplied, row: &Value, now: u64, budget: usize) -> (Vec<Chip>, bool) {
    let mut chips = of(supplied, row, now);
    let shown = prefix(&chips, budget);
    let all = shown == chips.len();
    chips.truncate(shown);
    (chips, all)
}

/// Template data for a row's heading: the chips that fit, the first led by the
/// one-column gap that separates them from the name.
pub fn pieces(supplied: &Supplied, row: &Value, now: u64, budget: usize) -> Value {
    json!(
        fitted(supplied, row, now, budget)
            .0
            .into_iter()
            .enumerate()
            .map(|(index, chip)| {
                let gap = if index == 0 { " " } else { "" };
                json!({"id": format!("chip-{index}"), "text": format!("{gap}{}", chip.text), "role": chip.role.name()})
            })
            .collect::<Vec<_>>()
    )
}

/// `pieces` closed by a one-column gap, for headings whose next cell follows directly.
pub fn padded(mut pieces: Value) -> Value {
    if let Some(pieces) = pieces.as_array_mut().filter(|pieces| !pieces.is_empty()) {
        let id = format!("chip-{}", pieces.len());
        pieces.push(json!({"id": id, "text": " ", "role": Role::Text.name()}));
    }
    pieces
}

/// Columns the template data takes.
pub fn pieces_width(pieces: &Value) -> usize {
    pieces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|piece| piece["text"].as_str())
        .map(UnicodeWidthStr::width)
        .sum()
}

/// The fields row detail adds when the heading could not show every chip.
pub fn detail(supplied: &Supplied, row: &Value, now: u64) -> Vec<(String, String)> {
    let Some(id) = row["id"].as_str() else {
        return Vec::new();
    };
    let mut fields: Vec<(String, Vec<&str>)> = Vec::new();
    for (source, label) in supplied.of(id) {
        match fields.last_mut() {
            Some((last, texts)) if last == source => texts.push(&label.text),
            _ => fields.push((source.to_owned(), vec![&label.text])),
        }
    }
    if fields.is_empty() {
        return crate::digest::detail(row, now)
            .map(|text| (crate::digest::WORD.to_owned(), text))
            .into_iter()
            .collect();
    }
    fields
        .into_iter()
        .map(|(source, texts)| (source, texts.join(SEPARATOR)))
        .collect()
}

/// Text that changes with the clock alone, so a frame redraws when it does.
/// Supplied labels change only when their source answers.
pub fn clock(supplied: &Supplied, row: &Value, now: u64) -> Option<String> {
    let supplied_row = row["id"]
        .as_str()
        .is_some_and(|id| supplied.of(id).next().is_some());
    if supplied_row {
        None
    } else {
        crate::digest::label(row, now)
    }
}

#[cfg(test)]
mod tests;
