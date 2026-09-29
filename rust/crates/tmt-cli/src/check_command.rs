//! Diagnostic capture composition; terminal content never completes a request.

use crate::{
    binding_error::socket_failure,
    invocation::OutputMode,
    output::{Failure, after_cleanup},
    target,
};
use std::io::{self, Write};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    storage::Storage,
    tmux::Tmux,
};
use tmt_cli_style::Token;
use tmt_core::binding::PaneIdentity;

struct Report {
    target: String,
    observed: PaneIdentity,
    lines: u64,
    output: String,
}

fn run(target: String, lines: Option<u64>) -> Result<Report, Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let lines = lines.unwrap_or(settings.capture_lines);
    let tmux = Tmux::default();
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No pane input occurred; retrying the identical command is safe.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })?;
    let pending = target::resolve(&mut storage, &tmux, &target).and_then(|observed| {
        let output = tmux
            .capture_on(&observed.server.socket_path, &observed.pane.id, lines)
            .map_err(|error| {
                if error.socket_permission_denied() {
                    return socket_failure(error);
                }
                Failure::new(
                    "ERROR",
                    format!(
                        "Failed to capture pane {}. Is tmux running?",
                        observed.pane.id
                    ),
                    1,
                )
                .caused_by(error)
            })?;
        Ok(Report {
            target,
            observed,
            lines,
            output,
        })
    });
    after_cleanup(pending, || storage.close())
}

/// The pane's text as tmux captured it, for an agent to read verbatim. A
/// capture without `-e` carries no escape sequences, so it is never escaped.
fn write_captured(output: &mut impl Write, text: &str) -> io::Result<()> {
    writeln!(output, "{text}")
}

pub fn execute(target: String, lines: Option<u64>, mode: OutputMode) -> io::Result<u8> {
    let report = match run(target, lines) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        let mut document = serde_json::json!({
            "target": report.target,
            "pane": report.observed.pane.id,
            "lines": report.lines,
            "output": report.output,
        });
        if let Some(identity) = report.observed.identity {
            document["identity"] = serde_json::json!({"name": identity.name, "canonicalName": identity.canonical_name});
        }
        writeln!(stdout, "{document}")?;
    } else {
        let terminal = stdout.terminal();
        let title = format!(
            "OUTPUT {} {}",
            tmt_cli_style::table::escape(&report.target),
            report.observed.pane.id
        );
        writeln!(stdout, "{}", terminal.paint(Token::Title, &title))?;
        write_captured(&mut stdout, &report.output)?;
    }
    Ok(0)
}
