//! The one consent prompt shared by squad's commands that change files outside
//! squad's own state (tmux configuration, agent skill directories).

use crate::core::SquadError;
use std::io::{BufRead, IsTerminal};

fn failed(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}

/// Shows the plan and asks once on the terminal; without one, `--yes` is
/// the consent (the plan is then in the command's own output).
pub fn consent(yes: bool, plan: &str, question: &str) -> Result<(), SquadError> {
    if yes {
        return Ok(());
    }
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Err(failed(
            "SQUAD_CONSENT_REQUIRED",
            "Nothing was changed. Review the plan with --print, then run again with --yes.",
        ));
    }
    eprint!("{plan}\n{question} [y/N] ");
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
