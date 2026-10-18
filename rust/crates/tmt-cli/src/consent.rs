//! Explicit consent for commands that change the user's installation or
//! settings. A non-interactive run without --yes refuses; consent is never
//! assumed. The one prompt shared by setup and the extension commands.

use crate::{invocation::OutputMode, output::Failure};
use std::io::{self, BufRead, IsTerminal, Write};
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
/// returns false. The answer is bounded to 256 bytes, and one that does not end
/// within them is declined.
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
    let accepted = read_affirmative(io::stdin().lock()).map_err(&io_failure)?;
    if !accepted {
        writeln!(output, "{}", consent.declined).map_err(&io_failure)?;
    }
    Ok(accepted)
}

/// The most bytes of an answer read, counting its newline.
const ANSWER_LIMIT: u64 = 256;

/// Reads one answer line and says whether it is `y` or `yes`. The answer counts only when its
/// newline or the end of input arrives within `ANSWER_LIMIT`: a read that used the whole limit
/// without either may be the prefix of a longer line, so it is not affirmative. The rest of an
/// over-long line is left unread, not drained.
fn read_affirmative(input: impl BufRead) -> io::Result<bool> {
    let mut answer = Vec::new();
    input.take(ANSWER_LIMIT).read_until(b'\n', &mut answer)?;
    let complete = answer.last() == Some(&b'\n') || (answer.len() as u64) < ANSWER_LIMIT;
    Ok(complete
        && matches!(
            std::str::from_utf8(&answer)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "y" | "yes"
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    fn affirmed(input: &[u8]) -> bool {
        read_affirmative(Cursor::new(input)).unwrap()
    }

    #[test]
    fn a_complete_short_answer_is_read_by_its_text() {
        for answer in [
            "y\n",
            "yes\n",
            "Y\n",
            "YES\n",
            "  yes  \n",
            "yes\r\n",
            "yes",
            "y",
            " Yes ",
        ] {
            assert!(affirmed(answer.as_bytes()), "{answer:?}");
        }
        for answer in [
            "",
            "\n",
            "n\n",
            "no\n",
            "yess\n",
            "ye\n",
            "yes please\n",
            "yes no\n",
        ] {
            assert!(!affirmed(answer.as_bytes()), "{answer:?}");
        }
    }

    #[test]
    fn invalid_utf8_is_never_an_affirmative_answer() {
        assert!(!affirmed(b"y\xff\n"));
        assert!(!affirmed(b"\xffyes\n"));
    }

    /// The answer is bounded to 256 bytes with its newline. What follows the limit was never
    /// read, so a prefix that reads as `yes` proves nothing about the whole line.
    #[test]
    fn an_answer_that_exhausts_the_limit_without_ending_is_not_affirmative() {
        let padded = format!("yes{}", " ".repeat(253));
        assert_eq!(padded.len(), 256);
        assert!(!affirmed(padded.as_bytes()), "256 bytes, no newline");
        assert!(
            !affirmed(format!("{padded}NO\n").as_bytes()),
            "the line continues with NO"
        );
        assert!(
            !affirmed(format!("{padded}\n").as_bytes()),
            "the newline is byte 257"
        );
        assert!(
            !affirmed(format!("y{}\n", " ".repeat(400)).as_bytes()),
            "an over-limit line"
        );
    }

    #[test]
    fn a_failing_reader_is_an_error_not_an_answer() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("read failed"))
            }
        }
        let reader = io::BufReader::new(Failing);
        assert!(read_affirmative(reader).is_err());
    }

    #[test]
    fn an_answer_that_ends_within_the_limit_is_read_in_full() {
        let at_limit = format!("yes{}\n", " ".repeat(252));
        assert_eq!(at_limit.len(), 256);
        assert!(affirmed(at_limit.as_bytes()), "the newline is byte 256");
        let at_eof = format!("yes{}", " ".repeat(252));
        assert_eq!(at_eof.len(), 255);
        assert!(affirmed(at_eof.as_bytes()), "end of input before the limit");
    }
}
