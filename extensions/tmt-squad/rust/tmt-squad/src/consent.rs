//! The one consent prompt shared by squad's commands that change files outside
//! squad's own state (tmux configuration, agent skill directories).

use crate::core::SquadError;
use std::io::{BufRead, IsTerminal, Write};

fn failed(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}

/// Shows the plan and asks once on the terminal; without one, `--yes` is
/// the consent (the plan is then in the command's own output).
pub fn consent(yes: bool, plan: &str, question: &str) -> Result<(), SquadError> {
    if yes {
        return Ok(());
    }
    let mut prompt = tmt_cli_style::stream::stderr();
    if !(std::io::stdin().is_terminal() && prompt.is_terminal()) {
        return Err(required());
    }
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
