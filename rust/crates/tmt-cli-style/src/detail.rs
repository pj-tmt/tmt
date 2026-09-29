//! A detail view: a bold title, then one row per field with a dimmed key and
//! the full value. Values are never truncated, whatever the terminal width,
//! because a detail view is where the whole value lives. The title, keys and
//! values are escaped like every table cell.

use crate::{
    palette::{Terminal, Token},
    table::{Cell, Column, Table, escape},
};
use std::io::{self, Write};

pub fn write(
    output: &mut impl Write,
    terminal: Terminal,
    title: &str,
    fields: &[(&str, String)],
) -> io::Result<()> {
    writeln!(output, "{}", terminal.paint(Token::Title, &escape(title)))?;
    let mut table = Table::new(&[Column::Fixed, Column::Detail]);
    for (key, value) in fields {
        table.row([Cell::styled(key, Token::Dim), value.into()]);
    }
    table.write(
        output,
        Terminal {
            width: None,
            ..terminal
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_keep_their_full_values_on_a_narrow_terminal() {
        let mut output = Vec::new();
        let narrow = Terminal {
            color: false,
            width: Some(20),
        };
        write(
            &mut output,
            narrow,
            "work\ter",
            &[
                ("lifetime", "saved".into()),
                ("cwd", "/Users/ada/dev/a/very/long/path".into()),
                ("note", "line\nbreak".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "work\\ter\n  lifetime  saved\n  cwd       /Users/ada/dev/a/very/long/path\n  note      line\\nbreak\n"
        );
    }
}
