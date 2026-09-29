//! Whether a person can take part in this invocation: see a full-screen view
//! or answer a question. Decided once per invocation, here, from the process's
//! own streams and environment; commands receive the decision and never test a
//! handle themselves. `docs/cli-style.md` owns the rule.

use std::io::{self, IsTerminal};

/// One answer to "may this invocation involve a person?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A person is at a terminal: show the view, ask the question.
    Interactive,
    /// Scripts, pipes, CI, `--json`: print the plain result, never ask.
    Plain,
}

/// What the decision depends on, as observed. Tests build it directly;
/// commands get it from [`Interaction::detect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interaction {
    pub json: bool,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    /// `TERM=dumb`: a terminal that cannot draw a full-screen view.
    pub dumb: bool,
}

impl Interaction {
    /// The facts of this process, read once. `json` is the invocation's
    /// `--json`, which always means a document and never a person.
    pub fn detect(json: bool) -> Self {
        Self {
            json,
            stdin: io::stdin().is_terminal(),
            stdout: io::stdout().is_terminal(),
            stderr: io::stderr().is_terminal(),
            dumb: std::env::var_os("TERM").is_some_and(|term| term == "dumb"),
        }
    }

    /// A full-screen view (a TUI) reads keys from stdin and draws on stdout.
    pub fn view(self) -> Mode {
        if !self.json && self.stdin && self.stdout && !self.dumb {
            Mode::Interactive
        } else {
            Mode::Plain
        }
    }

    /// A question is asked on stderr, so stdout stays the command's result,
    /// and answered on stdin.
    pub fn prompt(self) -> Mode {
        if !self.json && self.stdin && self.stderr {
            Mode::Interactive
        } else {
            Mode::Plain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERSON: Interaction = Interaction {
        json: false,
        stdin: true,
        stdout: true,
        stderr: true,
        dumb: false,
    };

    #[test]
    fn a_person_at_a_terminal_gets_both_the_view_and_the_question() {
        assert_eq!(PERSON.view(), Mode::Interactive);
        assert_eq!(PERSON.prompt(), Mode::Interactive);
    }

    #[test]
    fn json_is_always_plain() {
        let json = Interaction {
            json: true,
            ..PERSON
        };
        assert_eq!((json.view(), json.prompt()), (Mode::Plain, Mode::Plain));
    }

    #[test]
    fn each_question_needs_its_own_streams() {
        let piped_out = Interaction {
            stdout: false,
            ..PERSON
        };
        assert_eq!(piped_out.view(), Mode::Plain, "the view draws on stdout");
        assert_eq!(
            piped_out.prompt(),
            Mode::Interactive,
            "a question still reaches the person on stderr"
        );
        let no_input = Interaction {
            stdin: false,
            ..PERSON
        };
        assert_eq!(
            (no_input.view(), no_input.prompt()),
            (Mode::Plain, Mode::Plain)
        );
        let quiet_err = Interaction {
            stderr: false,
            ..PERSON
        };
        assert_eq!(
            (quiet_err.view(), quiet_err.prompt()),
            (Mode::Interactive, Mode::Plain)
        );
    }

    #[test]
    fn a_dumb_terminal_answers_questions_but_draws_no_view() {
        let dumb = Interaction {
            dumb: true,
            ..PERSON
        };
        assert_eq!(
            (dumb.view(), dumb.prompt()),
            (Mode::Plain, Mode::Interactive)
        );
    }
}
