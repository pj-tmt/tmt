//! Public completion setup; exact script generation remains grammar-owned.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{
    completion_install::{self, Plan, Shell},
    config::ConfigPaths,
};
use tmt_cli_style::{Interaction, detail, message};

pub fn execute(shell: Option<&str>, install: bool, yes: bool, mode: OutputMode) -> io::Result<u8> {
    match run(shell, install, yes, mode, Interaction::detect(mode.json)) {
        Ok(()) => Ok(0),
        Err(error) => error.publish(mode),
    }
}

fn failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("COMPLETION_ERROR", error.to_string(), 1).caused_by(error)
}

fn legacy_script(explicit_shell: bool, install: bool, interaction: Interaction) -> bool {
    explicit_shell && !install && !interaction.json && !interaction.stdout
}

fn run(
    shell: Option<&str>,
    install: bool,
    yes: bool,
    mode: OutputMode,
    interaction: Interaction,
) -> Result<(), Failure> {
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if legacy_script(shell.is_some(), install, interaction) {
        return crate::grammar::completion::generate(shell.expect("explicit shell"), &mut output)
            .map_err(failure);
    }
    let detected = completion_install::detected_shell();
    let selected = shell.and_then(Shell::parse).or(detected).ok_or_else(|| {
        Failure::new(
            "COMPLETION_SHELL_UNKNOWN",
            "Cannot detect a supported shell from SHELL; specify bash, zsh, or fish.",
            1,
        )
    })?;
    let mut plan =
        Plan::inspect(selected, ConfigPaths::shell_startup(selected)?).map_err(failure)?;
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
            writeln!(output, "Place the line after compinit.").map_err(failure)?;
        }
        for warning in &plan.warnings {
            message::hint(&mut output, terminal, warning).map_err(failure)?;
        }
    }
    let mut changed = false;
    if install && !plan.configured {
        // JSON keeps exactly one stdout result; the exact proposed mutation is
        // still shown on stderr before applying explicit --yes consent.
        if mode.json {
            let mut err = tmt_cli_style::stream::stderr();
            let terminal = err.terminal();
            detail::write(
                &mut err,
                terminal,
                "Completion install plan",
                &[
                    ("file", plan.file.display().to_string()),
                    ("line", selected.line().into()),
                ],
            )
            .map_err(failure)?;
            err.flush().map_err(failure)?;
        }
        output.flush().map_err(failure)?;
        if !crate::consent::ask(
            &mut output,
            yes,
            mode,
            crate::consent::Consent {
                code: "COMPLETION_CONSENT_REQUIRED",
                refusal: "Completion installation requires consent; run interactively or pass --yes with --install.",
                question: "Append this line to this startup file",
                declined: "No changes made.",
            },
            failure,
        )? {
            return Ok(());
        }
        changed = plan.append().map_err(failure)?;
        plan = Plan::inspect(selected, plan.file.clone()).map_err(failure)?;
        if !mode.json {
            message::success(
                &mut output,
                terminal,
                "Completion line appended; reload your shell to activate it.",
            )
            .map_err(failure)?;
        }
    }
    if mode.json {
        writeln!(
            output,
            "{}",
            json!({
                "shell": selected.name(), "detectedShell": detected.map(Shell::name),
                "installed": plan.configured, "evidence": "startup-file", "file": plan.file,
                "line": selected.line(), "warnings": plan.warnings, "changed": changed,
            })
        )
        .map_err(failure)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_generation_depends_only_on_explicit_shell_pipe_and_no_setup_mode() {
        let mut facts = Interaction {
            json: false,
            stdin: false,
            stdout: false,
            stderr: false,
            dumb: true,
        };
        assert!(legacy_script(true, false, facts));
        assert!(!legacy_script(false, false, facts));
        assert!(!legacy_script(true, true, facts));
        facts.stdout = true;
        assert!(
            !legacy_script(true, false, facts),
            "TTY with redirected stdin is still a guide"
        );
        facts.stdout = false;
        facts.json = true;
        assert!(!legacy_script(true, false, facts));
    }
}
