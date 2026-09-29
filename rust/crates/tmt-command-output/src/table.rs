//! Plain human tables only; structured output and exact bodies bypass this owner.

use std::io::{self, Write};
use tmt_cli_style::{
    Terminal,
    table::{Column, Table},
};

/// Preserve full values and align by displayed columns, not bytes or tab stops.
/// Const-sized rows prevent silently missing or extra cells at call sites.
/// Rendering, escaping and width measurement belong to `tmt-cli-style`; the
/// header row and plain, unindented layout remain until each caller migrates
/// to the list style.
pub fn write<const N: usize>(
    output: &mut impl Write,
    headers: [&str; N],
    rows: impl IntoIterator<Item = [String; N]>,
) -> io::Result<()> {
    let mut table = Table::new(&[Column::Name; N]).indent(0);
    table.row(headers);
    for row in rows {
        table.row(row);
    }
    table.write(output, Terminal::PLAIN)
}

#[cfg(test)]
mod tests;
