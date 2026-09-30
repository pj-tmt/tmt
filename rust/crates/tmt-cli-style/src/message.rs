//! One-line outcomes: `✓ <done>` on success, `error: <what>` on failure,
//! `warning: <what>` for a non-fatal problem, each optionally followed by
//! `hint: <next>`. Exit codes stay with the caller.

use crate::{
    mark::Mark,
    palette::{Terminal, Token},
    table::escape,
};
use std::io::{self, Write};

/// A single final period is punctuation noise in a one-line message; the
/// stored text (and `--json`) keeps it. An ellipsis is not a period.
fn line(text: &str) -> String {
    let text = if text.ends_with("..") {
        text
    } else {
        text.strip_suffix('.').unwrap_or(text)
    };
    escape(text)
}

/// `✓ <past-tense verb> <object>`.
pub fn success(output: &mut impl Write, terminal: Terminal, done: &str) -> io::Result<()> {
    let mark = Mark::Done;
    writeln!(
        output,
        "{} {}",
        terminal.paint(mark.token(), mark.symbol()),
        line(done)
    )
}

/// `error: <what>`, then `hint: <next>` when there is a next step.
pub fn error(
    output: &mut impl Write,
    terminal: Terminal,
    what: &str,
    hint: Option<&str>,
) -> io::Result<()> {
    writeln!(
        output,
        "{} {}",
        terminal.paint(Token::Error, "error:"),
        line(what)
    )?;
    match hint {
        Some(next) => self::hint(output, terminal, next),
        None => Ok(()),
    }
}

/// `warning: <what>` for a non-fatal problem: the command still did its work.
/// Then `hint: <next>` when there is a next step.
pub fn warning(
    output: &mut impl Write,
    terminal: Terminal,
    what: &str,
    hint: Option<&str>,
) -> io::Result<()> {
    writeln!(
        output,
        "{} {}",
        terminal.paint(Token::Warn, "warning:"),
        line(what)
    )?;
    match hint {
        Some(next) => self::hint(output, terminal, next),
        None => Ok(()),
    }
}

/// `hint: <next command>`, only when an action is possible.
pub fn hint(output: &mut impl Write, terminal: Terminal, next: &str) -> io::Result<()> {
    writeln!(
        output,
        "{} {}",
        terminal.paint(Token::Accent, "hint:"),
        line(next)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut output = Vec::new();
        write(&mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn a_warning_is_labeled_and_may_carry_a_hint() {
        let plain = Terminal::PLAIN;
        assert_eq!(
            text(|out| warning(out, plain, "Badge not updated.", None)),
            "warning: Badge not updated\n"
        );
        assert_eq!(
            text(|out| warning(
                out,
                plain,
                "Using an anonymous sender.",
                Some("pass --identity <name>")
            )),
            "warning: Using an anonymous sender\nhint: pass --identity <name>\n"
        );
        let color = Terminal {
            color: true,
            width: None,
            theme: None,
        };
        assert!(text(|out| warning(out, color, "x", None)).starts_with("\u{1b}[33mwarning:"));
    }

    #[test]
    fn messages_drop_one_final_period_and_escape_controls() {
        let plain = Terminal::PLAIN;
        assert_eq!(
            text(|out| success(out, plain, "Named worker.")),
            "✓ Named worker\n"
        );
        assert_eq!(
            text(|out| error(
                out,
                plain,
                "Identity 'x\u{1b}' was not found.",
                Some("tmt ls")
            )),
            "error: Identity 'x\\u{1b}' was not found\nhint: tmt ls\n"
        );
        assert_eq!(
            text(|out| error(out, plain, "Wait... no.", None)),
            "error: Wait... no\n"
        );
        assert_eq!(
            text(|out| success(out, plain, "Loading...")),
            "✓ Loading...\n"
        );
    }
}
