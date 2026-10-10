//! The limits a pane's statusline footer shows. Some providers expose them
//! nowhere Core reads, so the only source without touching provider settings is
//! the footer text. Only one exact form is read, and anything else is no reading
//! rather than a guess:
//!
//! `5h <percent left>% (<time to reset>)  7d <percent left>% (<time to reset>)`
//!
//! with a reset such as `4d03h`, `3h14m` or `45m`.
use super::{Reading, Window};

const SHORT_MINUTES: u64 = 5 * 60;
const WEEK_MINUTES: u64 = 7 * 24 * 60;

/// The bottom-most statusline in `capture`, read as of `observed_at_ms`.
pub(super) fn read(capture: &str, observed_at_ms: u64) -> Option<Reading> {
    let line = capture
        .lines()
        .rev()
        .find(|line| line.contains("5h ") && line.contains("7d "))?;
    let mut words = line.split_whitespace();
    let (mut short, mut weekly) = (None, None);
    while let Some(word) = words.next() {
        let slot = match word {
            "5h" => &mut short,
            "7d" => &mut weekly,
            _ => continue,
        };
        let left = percent(words.next()?)?;
        let reset = minutes(words.next()?)?;
        if slot.replace((left, reset)).is_some() {
            return None;
        }
    }
    let (short, weekly) = (short?, weekly?);
    if short.1 > SHORT_MINUTES || weekly.1 > WEEK_MINUTES {
        return None;
    }
    let window = |(left, minutes): (f64, u64)| Window {
        left,
        resets_at_ms: observed_at_ms.saturating_add(minutes * 60_000).max(1),
    };
    let reading = Reading {
        observed_at_ms,
        weekly: window(weekly),
        short: Some(window(short)),
    };
    reading.valid().then_some(reading)
}

/// `98.0%`: a plain decimal, no sign or exponent.
fn percent(word: &str) -> Option<f64> {
    let number = word.strip_suffix('%')?;
    let mut parts = number.split('.');
    let (whole, fraction) = (parts.next()?, parts.next());
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|f| !digits(f)) || parts.next().is_some() {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    (0.0..=100.0).contains(&value).then_some(value)
}

/// `(4d03h)`, `(3h14m)` or `(45m)`: whole units in descending order.
fn minutes(word: &str) -> Option<u64> {
    let inner = word.strip_prefix('(')?.strip_suffix(')')?;
    let (mut total, mut rank, mut digits) = (0u64, 3u8, String::new());
    for ch in inner.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let (next, unit) = match ch {
            'd' => (2, 24 * 60),
            'h' => (1, 60),
            'm' => (0, 1),
            _ => return None,
        };
        if digits.is_empty() || digits.len() > 4 || next >= rank {
            return None;
        }
        total += digits.parse::<u64>().ok()? * unit;
        (rank, digits) = (next, String::new());
    }
    (digits.is_empty() && rank < 3).then_some(total)
}

#[cfg(test)]
mod tests;
