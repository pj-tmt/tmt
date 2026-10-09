//! Layout-only recovery bypasses identity learning and advisory capture.

use crate::{invocation::OutputMode, output::Failure, workspace_show_command::selected_socket};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace,
};

pub fn execute(socket: Option<&str>, mode: OutputMode) -> io::Result<u8> {
    let outcome: Result<_, Failure> = (|| {
        let caller = CallerEnvironment::current();
        let socket = selected_socket(socket, &caller)?;
        let paths = ConfigPaths::discover().map_err(Failure::from)?;
        let snapshot = workspace::read_snapshot(&paths, &socket).map_err(|error| {
            Failure::new(
                "WORKSPACE_RESTORE_UNAVAILABLE",
                format!("Could not read workspace recovery input: {error}"),
                1,
            )
        })?;
        let result = Host::for_caller(&caller)
            .workspace_restore_layout(&snapshot, Instant::now() + Duration::from_secs(30))
            .map_err(|error| {
                Failure::new(
                    "WORKSPACE_RESTORE_UNAVAILABLE",
                    format!("Could not restore workspace layout: {error}"),
                    1,
                )
            })?
            .ok_or_else(|| {
                Failure::new(
                    "WORKSPACE_RESTORE_UNAVAILABLE",
                    "The selected host does not support layout restoration.",
                    1,
                )
            })?;
        Ok((socket, result))
    })();
    let (socket, result) = match outcome {
        Ok(result) => result,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let partial = result.partial();
    if mode.json {
        writeln!(
            output,
            "{}",
            serde_json::json!({"version": 1, "socket": socket, "layoutOnly": true,
            "status": if partial { "partial" } else { "completed" }, "sessions": result.sessions,
            "windows": result.windows, "panes": result.panes, "failures": result.failures})
        )?;
    } else {
        let created = result
            .sessions
            .iter()
            .filter(|session| session.action == "created")
            .count();
        let skipped = result
            .sessions
            .iter()
            .filter(|session| session.action == "skip_existing")
            .count();
        writeln!(
            output,
            "Workspace on {socket}: {created} sessions created, {skipped} skipped{}.",
            if partial { ", partial restore" } else { "" }
        )?;
        for session in &result.sessions {
            writeln!(output, "  {:?}: {}", session.name, session.action)?;
            if let Some(pane) = &session.retained_bootstrap {
                writeln!(output, "    Bootstrap retained: {pane}")?;
            }
        }
        for failure in &result.failures {
            writeln!(output, "  {failure}")?;
        }
    }
    Ok(u8::from(partial))
}
