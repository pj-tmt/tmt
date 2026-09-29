//! What `-h`, `--help` and `tmt help <command>` print, for a path the parser
//! has already resolved and validated.

use crate::{extension_command, grammar};
use std::io::{self, Write};

pub fn write(path: &[String], output: &mut impl Write) -> io::Result<()> {
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
        extension_command::write_discovered(output)?;
    }
    Ok(())
}
