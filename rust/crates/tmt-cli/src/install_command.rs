//! Native skill installation composition; filesystem policy stays in adapters.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    error::Error,
    io::{self, Write},
    path::Path,
};
use tmt_adapters::{
    config::ConfigPaths,
    skill_installation::{self, InstallReport, ProviderEnvironment},
};
use tmt_command_output::guidance::{report_document, write_report_human};

fn failure(error: impl Error + 'static) -> Failure {
    Failure::new("ERROR", error.to_string(), 1).caused_by(error)
}

fn run(
    provider: Option<&str>,
    directory: Option<&str>,
    force: bool,
) -> Result<InstallReport, Failure> {
    let environment = ProviderEnvironment::capture().map_err(failure)?;
    let paths = ConfigPaths::discover()?;
    skill_installation::install(
        &environment,
        &paths.global_dir,
        provider,
        directory.map(Path::new),
        force,
    )
    .map_err(failure)
}

pub fn execute(
    provider: Option<String>,
    directory: Option<String>,
    force: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let report = match run(provider.as_deref(), directory.as_deref(), force) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let terminal = output.terminal();
    if mode.json {
        let value = report_document(&report);
        writeln!(output, "{value}")?;
    } else {
        write_report_human(&report, &mut output)?;
        tmt_cli_style::message::hint(
            &mut output,
            terminal,
            "reload or restart your agent to use the current skill; existing conversations can read tmt learn --skill",
        )?;
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "reload or restart your agent to use the current skill; existing conversations can read tmt learn --skill",
        &[""],
        &[],
    ),
];
