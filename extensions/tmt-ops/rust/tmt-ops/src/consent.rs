//! The one consent prompt shared by squad's commands that change files outside
//! squad's own state (tmux configuration, agent skill directories).

use crate::core::SquadError;
use std::io::{BufRead, Write};
use tmt_cli_style::Mode;

fn failed(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}

/// How a command gets consent, decided once per invocation in `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// `--yes`: the plan is in the command's own output.
    Given,
    /// A person can answer on the terminal.
    Ask,
    /// Nobody can be asked (a script, a pipe, `--json`) and `--yes` is absent.
    Unavailable,
}

impl Consent {
    pub fn new(yes: bool, prompt: Mode) -> Self {
        match (yes, prompt) {
            (true, _) => Self::Given,
            (false, Mode::Interactive) => Self::Ask,
            (false, Mode::Plain) => Self::Unavailable,
        }
    }
}

/// Shows the plan and asks once when a person can answer; otherwise `--yes`
/// is the consent.
pub fn ask(consent: Consent, plan: &str, question: &str) -> Result<(), SquadError> {
    match consent {
        Consent::Given => return Ok(()),
        Consent::Unavailable => return Err(required()),
        Consent::Ask => {}
    }
    let mut prompt = tmt_cli_style::stream::stderr();
    // The prompt stays on standard error, where scripts expect it.
    let _ = write!(prompt, "{plan}\n{question} [y/N] ").and_then(|()| prompt.flush());
    drop(prompt);
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|_| {
            failed(
                "SQUAD_CONSENT_REQUIRED",
                "No answer was read; nothing was changed.",
            )
        })?;
    if matches!(answer.trim(), "y" | "Y" | "yes") {
        Ok(())
    } else {
        Err(failed(
            "SQUAD_CONSENT_REQUIRED",
            "Declined; nothing was changed.",
        ))
    }
}

/// Consent is needed but cannot be asked here.
fn required() -> SquadError {
    SquadError::hinted(
        "SQUAD_CONSENT_REQUIRED",
        "Nothing was changed.",
        " ",
        "Review the plan with --print, then run again with --yes.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_is_consent_and_without_it_only_a_person_can_be_asked() {
        assert_eq!(Consent::new(true, Mode::Plain), Consent::Given);
        assert_eq!(Consent::new(true, Mode::Interactive), Consent::Given);
        assert_eq!(Consent::new(false, Mode::Interactive), Consent::Ask);
        assert_eq!(Consent::new(false, Mode::Plain), Consent::Unavailable);
        assert_eq!(
            ask(Consent::Unavailable, "plan", "Go?").unwrap_err().code,
            "SQUAD_CONSENT_REQUIRED"
        );
        assert_eq!(ask(Consent::Given, "plan", "Go?"), Ok(()));
    }

    #[test]
    fn the_json_message_is_unchanged_and_human_output_splits_the_next_step() {
        let error = super::required();
        assert_eq!(
            error.to_json().to_string(),
            r#"{"error":{"code":"SQUAD_CONSENT_REQUIRED","message":"Nothing was changed. Review the plan with --print, then run again with --yes."}}"#
        );
        assert_eq!(
            error.human(),
            (
                "Nothing was changed.",
                Some("Review the plan with --print, then run again with --yes.")
            )
        );
    }
}
