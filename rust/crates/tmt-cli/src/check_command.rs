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
    host::{Host, HostError},
    storage::Storage,
};
use tmt_cli_style::Token;
use tmt_core::{binding::PaneIdentity, host::HostKind, request::RequestEndpoint};

struct Report {
    target: String,
    observed: PaneIdentity,
    lines: u64,
    output: String,
}

fn run(target: String, lines: Option<u64>, capture_only: bool) -> Result<Report, Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let lines = lines.unwrap_or(settings.capture_lines);
    // Names resolve through stored bindings; each is probed on its own host.
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No pane input occurred; retrying the identical command is safe.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })?;
    let pending = target::resolve(&mut storage, &target).and_then(|observed| {
        // A capture-only read is for display data and may be repeated by a tool,
        // so it never hands a retained checklist to the pane.
        if let Some(identity) = observed.identity.as_ref().filter(|_| !capture_only) {
            tmt_adapters::digest::flush_idle(&mut storage,&identity.id,std::time::Duration::from_secs_f64(settings.paste_enter_delay_ms/1000.0)).map_err(|e|Failure::new("DIGEST_DELIVERY_ERROR","Could not confirm Digest checklist delivery; inspect the retained checklist before retrying.",1).caused_by(e))?;
        }
        let endpoint = RequestEndpoint {
            server: observed.server.clone(),
            pane_id: observed.pane.id.clone(),
            pane_pid: observed.pane.pane_pid,
        };
        let output = Host::for_server(&observed.server)
            .capture(&endpoint, lines)
            .map_err(|error| capture_failure(observed.server.host, &observed.pane.id, error))?;
        Ok(Report {
            target,
            observed,
            lines,
            output,
        })
    });
    after_cleanup(pending, || storage.close())
}

/// A failed capture. Only tmux gets the tmux hint; an external host names
/// itself and its driver's own answer, which is otherwise only the cause.
fn capture_failure(host: HostKind, pane: &str, error: HostError) -> Failure {
    if error.socket_permission_denied() {
        return socket_failure(error);
    }
    let message = match (host, &error) {
        (HostKind::Tmux, _) => format!("Failed to capture pane {pane}. Is tmux running?"),
        (
            HostKind::External(host),
            HostError::Refused {
                driver,
                code,
                message,
            },
        ) => {
            // The code's wire name, such as `bad_request`.
            let code = serde_json::to_value(code).unwrap_or_default();
            format!(
                "Failed to capture pane {pane} on host {}: the {driver} driver answered {}: {message}",
                host.as_str(),
                code.as_str().unwrap_or("an error")
            )
        }
        (HostKind::External(host), error) => {
            format!(
                "Failed to capture pane {pane} on host {}: {error}",
                host.as_str()
            )
        }
    };
    Failure::new("ERROR", message, 1).caused_by(error)
}

/// The pane's text as its host captured it, for an agent to read verbatim.
/// A capture carries no escape sequences, so it is never escaped.
fn write_captured(output: &mut impl Write, text: &str) -> io::Result<()> {
    writeln!(output, "{text}")
}

pub fn execute(
    target: String,
    lines: Option<u64>,
    capture_only: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let report = match run(target, lines, capture_only) {
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
