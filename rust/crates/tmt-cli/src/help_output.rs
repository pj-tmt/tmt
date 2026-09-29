//! What `-h`, `--help` and `tmt help <command>` print, for a path the parser
//! has already resolved and validated.

use crate::{extension_command::Discovered, grammar};
use std::io::{self, Write};
use tmt_cli_style::Terminal;

/// Root help also lists `discovered`; dispatch passes the real `PATH`
/// discovery, and tests pass a fixed list so help never depends on the machine.
pub fn write(
    path: &[String],
    discovered: &Discovered,
    terminal: Terminal,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut command = grammar::help_command(path).map_err(io::Error::other)?;
    if path.is_empty() {
        command = tmt_cli_style::apply(command, grammar::ROOT, &discovered.sections());
    }
    output.write_all(tmt_cli_style::help_text(&command, terminal).as_bytes())
}
