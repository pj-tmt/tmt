//! The chips beside a member's name: the labels its sources supplied, else the
//! Core digest policy's. Every surface asks here, so a row never shows both and
//! no painter knows which source a chip came from.
use crate::labels::Supplied;
use serde_json::{Value, json};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

const SEPARATOR: &str = " · ";
/// Ends the chip of a label that offers a choice.
const CHOICE: &str = "▾";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub role: Role,
    /// A source's name: it shows only together with the label that follows it.
    pub head: bool,
}

/// A row's chips in full. Each source's labels follow its name and replace the
/// policy chip; later chips carry their own separator so a prefix of the list
/// always reads whole.
pub fn of(supplied: &Supplied, row: &Value, now: u64) -> Vec<Chip> {
    let mut chips = Vec::new();
    let mut source = None;
    for (name, label) in row["id"]
        .as_str()
        .into_iter()
        .flat_map(|id| supplied.of(id))
    {
        // Hidden bidi and format characters never reach the terminal.
        let text = super::notes::sanitize(&label.text);
        let text = if label.action.is_some() {
            format!("{text} {CHOICE}")
        } else {
            text
        };
        if source == Some(name) {
            chips.push(Chip {
                text: format!("{SEPARATOR}{text}"),
                role: label.role,
                head: false,
            });
            continue;
        }
        source = Some(name);
        let separator = if chips.is_empty() { "" } else { SEPARATOR };
        chips.push(Chip {
            text: format!("{separator}{} ", super::notes::sanitize(name)),
            role: Role::Muted,
            head: true,
        });
        chips.push(Chip {
            text,
            role: label.role,
            head: false,
        });
    }
    if chips.is_empty() {
        return policy(row, now);
    }
    chips
}

/// Whether a chip is the one that offers a choice.
pub fn offers_choice(text: &str) -> bool {
    text.ends_with(CHOICE)
}

/// The id of the heading piece that offers a choice, among `shown`'s pieces.
pub fn choice_piece(pieces: &Value) -> Option<String> {
    pieces
        .as_array()?
        .iter()
        .find(|piece| piece["text"].as_str().is_some_and(offers_choice))
        .and_then(|piece| piece["id"].as_str())
        .map(str::to_owned)
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
        head: false,
    }];
    if !rest.is_empty() {
        chips.push(Chip {
            text: rest.into(),
            role: Role::Muted,
            head: false,
        });
    }
    chips
}

pub fn width(chips: &[Chip]) -> usize {
    chips.iter().map(|chip| chip.text.width()).sum()
}

/// How many leading chips fit `budget` columns. A source's name goes with its
/// first label, and the first of them is never cut: when it does not fit, none
/// shows.
pub fn prefix(chips: &[Chip], budget: usize) -> usize {
    let mut used = 0;
    let mut shown = 0;
    while shown < chips.len() {
        let unit = if chips[shown].head { 2 } else { 1 }.min(chips.len() - shown);
        used += width(&chips[shown..shown + unit]);
        if used > budget {
            break;
        }
        shown += unit;
    }
    shown
}

/// A heading's chips as template data.
pub struct Shown {
    pub pieces: Value,
    /// Columns the pieces take, gaps included.
    pub width: usize,
    /// Whether the heading had room for every chip.
    pub all: bool,
}

/// The chips that fit `room` columns, the first led by the one-column gap that
/// separates them from the name; `closed` ends them with another gap, for
/// headings whose next cell follows directly. Trailing labels drop whole.
pub fn shown(supplied: &Supplied, row: &Value, now: u64, room: usize, closed: bool) -> Shown {
    let mut chips = of(supplied, row, now);
    let count = prefix(&chips, room.saturating_sub(1 + usize::from(closed)));
    let all = count == chips.len();
    chips.truncate(count);
    let mut pieces: Vec<_> = chips
        .iter()
        .enumerate()
        .map(|(index, chip)| {
            let gap = if index == 0 { " " } else { "" };
            json!({"id": format!("chip-{index}"), "text": format!("{gap}{}", chip.text), "role": chip.role.name()})
        })
        .collect();
    if closed && !pieces.is_empty() {
        pieces.push(json!({"id": format!("chip-{count}"), "text": " ", "role": Role::Text.name()}));
    }
    let width = pieces
        .iter()
        .filter_map(|piece| piece["text"].as_str())
        .map(UnicodeWidthStr::width)
        .sum();
    Shown {
        pieces: json!(pieces),
        width,
        all,
    }
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
