//! Explicit consent for commands that change the user's installation. A
//! non-interactive run without --yes refuses; consent is never assumed.

use crate::{invocation::OutputMode, output::Failure};
use std::io::{self, BufRead, IsTerminal, Read, Write};

pub fn ask(yes: bool, mode: OutputMode, code: &'static str, action: &str) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    if mode.json || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Failure::new(
            code,
            format!("{action} requires explicit --yes; no changes were made."),
            1,
        ));
    }
    let failure = |error: io::Error| {
        Failure::new("IO_ERROR", "Could not ask for consent.", 1).caused_by(error)
    };
    let mut stderr = io::stderr().lock();
    write!(stderr, "{action}? [y/N] ")
        .and_then(|()| stderr.flush())
        .map_err(failure)?;
    let mut answer = Vec::new();
    io::stdin()
        .lock()
        .take(256)
        .read_until(b'\n', &mut answer)
        .map_err(failure)?;
    Ok(matches!(
        std::str::from_utf8(&answer)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes"
    ))
}
