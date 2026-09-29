//! What `-h`, `--help` and `tmt help <command>` print, for a path the parser
//! has already resolved and validated.

use crate::{extension_command::Discovered, grammar};
use std::io::{self, Write};

/// Root help also lists `discovered`; dispatch passes the real `PATH`
/// discovery, and tests pass a fixed list so help never depends on the machine.
pub fn write(path: &[String], discovered: &Discovered, output: &mut impl Write) -> io::Result<()> {
    if path.is_empty() {
        writeln!(
            output,
            "TMT native alpha — collaborate with terminal agents through durable exchanges.\nRun tmt install to set up agent skills; managed installations use tmt upgrade.\n"
        )?;
    }
    grammar::help_command(path)
        .map_err(io::Error::other)?
        .write_help(output)?;
    writeln!(output)?;
    if path.is_empty() {
        discovered.write(output)?;
    }
    Ok(())
}
