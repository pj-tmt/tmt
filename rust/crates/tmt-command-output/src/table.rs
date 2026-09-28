//! Plain human tables only; structured output and exact bodies bypass this owner.

use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

fn cell(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
            rendered.extend(ch.escape_default());
        } else {
            rendered.push(ch);
        }
    }
    rendered
}

/// Preserve full values and align by displayed columns, not bytes or tab stops.
/// Const-sized rows prevent silently missing or extra cells at call sites.
pub fn write<const N: usize>(
    output: &mut impl Write,
    headers: [&str; N],
    rows: impl IntoIterator<Item = [String; N]>,
) -> io::Result<()> {
    let rows: Vec<_> = std::iter::once(headers.map(str::to_owned))
        .chain(rows)
        .map(|row| row.map(|value| cell(&value)))
        .collect();
    let widths: [usize; N] =
        std::array::from_fn(|index| rows.iter().map(|row| row[index].width()).max().unwrap_or(0));
    for row in rows {
        // Empty final cells must not leave trailing padding on a line.
        let end = row.iter().rposition(|value| !value.is_empty());
        if let Some(end) = end {
            for index in 0..=end {
                write!(output, "{}", row[index])?;
                if index < end {
                    write!(
                        output,
                        "{}",
                        " ".repeat(widths[index] - row[index].width() + 2)
                    )?;
                }
            }
        }
        writeln!(output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
