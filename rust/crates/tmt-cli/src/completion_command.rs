//! Public completion setup; exact script generation remains grammar-owned.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{
    completion_install::{self, Plan, Shell},
    config::ConfigPaths,
};
use tmt_cli_style::{detail, message};

pub fn execute(shell: Option<&str>, mode: OutputMode) -> io::Result<u8> {
    match run(shell, mode) {
        Ok(()) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("COMPLETION_ERROR", error.to_string(), 1).caused_by(error)
}

fn run(shell: Option<&str>, mode: OutputMode) -> Result<(), Failure> {
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let detected = completion_install::detected_shell();
    let selected = shell.and_then(Shell::parse).or(detected).ok_or_else(|| {
        Failure::new(
            "COMPLETION_SHELL_UNKNOWN",
            "Cannot detect a supported shell from SHELL; specify bash, zsh, or fish.",
            1,
        )
    })?;
    let plan = Plan::inspect(selected, ConfigPaths::shell_startup(selected)?).map_err(failure)?;
    let terminal = output.terminal();
    if !mode.json {
        detail::write(
            &mut output,
            terminal,
            "Shell completion",
            &[
                ("shell", selected.name().into()),
                ("detected", detected.map_or("unknown", Shell::name).into()),
                ("file", plan.file.display().to_string()),
                ("line", selected.line().into()),
                (
                    "configured",
                    if plan.configured {
                        "already installed (startup-file evidence)"
                    } else {
                        "not installed (startup-file evidence)"
                    }
                    .into(),
                ),
            ],
        )
        .map_err(failure)?;
        writeln!(
            output,
            "This checks startup-file text, not functions loaded in your current shell."
        )
        .map_err(failure)?;
        if selected == Shell::Zsh {
            writeln!(output, "Place the line after zsh completion initialization (compinit or your shell framework).").map_err(failure)?;
        }
        for warning in &plan.warnings {
            message::hint(&mut output, terminal, warning).map_err(failure)?;
        }
    }
    if mode.json {
        writeln!(
            output,
            "{}",
            json!({
                "shell": selected.name(), "detectedShell": detected.map(Shell::name),
                "installed": plan.configured, "evidence": "startup-file", "file": plan.file,
                "line": selected.line(), "warnings": plan.warnings,
            })
        )
        .map_err(failure)?;
    }
    Ok(())
}

/// Help remains available when shell detection or startup-file inspection fails.
pub fn needs_setup() -> bool {
    let Some(shell) = completion_install::detected_shell() else {
        return false;
    };
    ConfigPaths::shell_startup(shell)
        .ok()
        .and_then(|file| Plan::inspect(shell, file).ok())
        .is_some_and(|plan| !plan.configured)
}
