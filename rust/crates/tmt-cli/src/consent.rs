//! Explicit consent for commands that change the user's installation or
//! settings. A non-interactive run without --yes refuses; consent is never
//! assumed. The one prompt shared by setup and the extension commands.

use crate::{invocation::OutputMode, output::Failure};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use tmt_cli_style::stream::Stream;

pub struct Consent<'a> {
    /// Failure code when consent is required but cannot be asked.
    pub code: &'static str,
    /// The whole refusal message, in the caller's wording.
    pub refusal: &'a str,
    /// The yes/no question, without the "[y/N]" suffix.
    pub question: &'a str,
    /// What a decline prints.
    pub declined: &'a str,
}

/// Whether a question can be asked on `output`: stdin and that stream are
/// terminals and output is not JSON.
pub fn interactive<W: Write>(mode: OutputMode, output: &Stream<W>) -> bool {
    !mode.json && io::stdin().is_terminal() && output.is_terminal()
}

/// Asks on `output` (the caller's stdout) only when stdin and that stream are
/// both terminals and output is not JSON. A decline prints `declined` and
/// returns false. The answer is bounded to 256 bytes.
pub fn ask(
    output: &mut Stream<impl Write>,
    yes: bool,
    mode: OutputMode,
    consent: Consent<'_>,
    io_failure: impl Fn(io::Error) -> Failure,
) -> Result<bool, Failure> {
    if yes {
        return Ok(true);
    }
    if !interactive(mode, output) {
        return Err(Failure::new(consent.code, consent.refusal, 1));
    }
    write!(output, "{}? [y/N] ", consent.question)
        .and_then(|()| output.flush())
        .map_err(&io_failure)?;
    let mut answer = Vec::new();
    io::stdin()
        .lock()
        .take(256)
        .read_until(b'\n', &mut answer)
        .map_err(&io_failure)?;
    let accepted = matches!(
        std::str::from_utf8(&answer)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes"
    );
    if !accepted {
        writeln!(output, "{}", consent.declined).map_err(&io_failure)?;
    }
    Ok(accepted)
}
