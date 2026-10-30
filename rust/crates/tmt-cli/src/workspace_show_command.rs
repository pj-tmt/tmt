//! Preview recovery without initializing storage, observing providers or publishing.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    time::Instant,
};
use tmt_adapters::{
    host::CallerEnvironment,
    workspace::{self, plan},
};

pub fn execute(
    socket: Option<&str>,
    name: Option<&tmt_core::workspace::SnapshotSelector>,
    mode: OutputMode,
) -> io::Result<u8> {
    let outcome: Result<_, Failure> = (|| {
        let (paths, socket, host) = crate::workspace_snapshot_command::context(socket)?;
        let document = workspace::saved::acquire_selected(
            &paths,
            workspace::Store::Socket(&socket),
            name,
            None,
        )
        .map_err(|error| {
            crate::workspace_snapshot_command::failure(
                error,
                "WORKSPACE_PLAN_UNAVAILABLE",
                name.is_none(),
            )
        })?;
        let input = plan::read_document(
            &paths,
            &socket,
            &host,
            Instant::now() + workspace::CAPTURE_BUDGET,
            name.cloned(),
            document,
        )
        .map_err(|error| {
            Failure::new(
                "WORKSPACE_PLAN_UNAVAILABLE",
                format!("Could not read the workspace plan: {error}"),
                1,
            )
        })?;
        let document = plan::document(&input).map_err(|error| {
            Failure::new(
                "WORKSPACE_PLAN_UNAVAILABLE",
                format!("Could not encode the workspace plan: {error}"),
                1,
            )
        })?;
        Ok((input, document))
    })();
    let (input, document) = match outcome {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{document}")?;
    } else {
        let preview = tmt_core::workspace::plan::restore_plan(
            &input.snapshot,
            &input.identities,
            input.current_sessions.as_deref(),
        );
        writeln!(
            output,
            "Workspace on {} (captured {})",
            input.snapshot.server.socket,
            tmt_cli_style::value::relative_time(
                tmt_adapters::request_runtime::wall_time_ms()
                    .saturating_sub(input.snapshot.captured_at_ms)
            )
        )?;
        for (id, action) in &preview.sessions {
            let session = input
                .snapshot
                .sessions
                .iter()
                .find(|session| session.id == *id)
                .expect("plan retains snapshot session");
            writeln!(output, "  session {:?}: {}", session.name, action.as_str())?;
            for link in &session.windows {
                let window = input
                    .snapshot
                    .windows
                    .iter()
                    .find(|window| window.id == link.window)
                    .expect("validated window link");
                writeln!(
                    output,
                    "    window {} {:?}: layout {}",
                    link.index, window.name, window.layout
                )?;
                for pane in input
                    .snapshot
                    .panes
                    .iter()
                    .filter(|pane| pane.window == window.id)
                {
                    let planned = preview
                        .panes
                        .iter()
                        .find(|planned| planned.pane == pane.id)
                        .expect("plan retains snapshot pane");
                    let identity = planned
                        .identity
                        .map(|record| record.entry.identity.name.as_str());
                    writeln!(
                        output,
                        "      pane {} in {:?}: {}{}",
                        pane.index,
                        pane.cwd,
                        pane_description(planned.action),
                        identity
                            .map(|name| format!(" ({name:?})"))
                            .unwrap_or_default()
                    )?;
                    if let Some(command) = &pane.command {
                        writeln!(output, "        argv: {:?}", command.argv)?;
                    }
                }
            }
        }
        writeln!(
            output,
            "Preview only; recovery rechecks live state before each effect."
        )?;
    }
    Ok(0)
}

fn pane_description(action: tmt_core::workspace::plan::WorkspacePaneAction) -> &'static str {
    use tmt_core::workspace::plan::WorkspacePaneAction;
    match action {
        WorkspacePaneAction::Shell => "shell",
        WorkspacePaneAction::RelaunchCommand => "relaunch recorded command",
        WorkspacePaneAction::IdentityMissing => "identity missing",
        WorkspacePaneAction::NoRememberedSession => "no remembered session",
        WorkspacePaneAction::Resumable => "resume remembered session",
        WorkspacePaneAction::StaleRequiresRetry => "stale remembered session",
        WorkspacePaneAction::Floating => "floating pane, not restored",
    }
}

/// A built-in preview: the recorded members, each with what a restore would do now. Read
/// only: no capture, publication or storage initialization.
pub fn execute_builtin(
    name: Option<&tmt_core::workspace::SnapshotSelector>,
    mode: OutputMode,
) -> io::Result<u8> {
    use tmt_core::workspace::builtin::{BuiltinSnapshot, member_plan};
    let outcome: Result<_, Failure> = (|| {
        let paths = tmt_adapters::config::ConfigPaths::discover().map_err(Failure::from)?;
        let document = workspace::saved::acquire_selected::<BuiltinSnapshot>(
            &paths,
            workspace::Store::Builtin,
            name,
            None,
        )
        .map_err(|error| {
            crate::workspace_snapshot_command::failure(error, "WORKSPACE_PLAN_UNAVAILABLE", false)
        })?;
        let (identities, live) = workspace::builtin_plan_input(
            &paths,
            &document.snapshot,
            Instant::now() + workspace::CAPTURE_BUDGET,
        )
        .map_err(|error| {
            Failure::new(
                "WORKSPACE_PLAN_UNAVAILABLE",
                format!("Could not read the workspace plan: {error}"),
                1,
            )
        })?;
        let plan = member_plan(&document.snapshot, &identities, &live, |cwd| {
            std::path::Path::new(cwd).is_dir()
        });
        let members = plan
            .iter()
            .map(|row| {
                serde_json::json!({
                    "identityId": row.member.identity.id,
                    "name": row.identity.map_or(row.member.identity.name.as_str(), |record| record.entry.identity.name.as_str()),
                    "cwd": row.member.cwd,
                    "harness": row.member.identity.harness,
                    "action": row.action.as_str(),
                })
            })
            .collect::<Vec<_>>();
        let human = std::iter::once(format!(
            "Built-in members (captured {}):",
            tmt_cli_style::value::relative_time(
                tmt_adapters::request_runtime::wall_time_ms()
                    .saturating_sub(document.snapshot.captured_at_ms)
            )
        ))
        .chain(plan.iter().map(|row| {
            format!(
                "  {}  {}  {}",
                row.identity
                    .map_or(row.member.identity.name.as_str(), |record| record
                        .entry
                        .identity
                        .name
                        .as_str()),
                row.action.as_str().replace('_', " "),
                row.member.cwd.as_deref().unwrap_or("(folder unknown)")
            )
        }))
        .collect::<Vec<_>>()
        .join("\n");
        let json = serde_json::json!({
            "version": 1,
            "host": tmt_core::host::HostKind::pty().as_str(),
            "name": name.map(|name| name.as_str()),
            "digest": document.digest,
            "capturedAtMs": document.snapshot.captured_at_ms,
            "generation": document.generation.as_ref().map(|generation| generation.id.as_str()),
            "protected": document.protected,
            "members": members,
        });
        Ok((json, human))
    })();
    let (json, human) = match outcome {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{json}")?;
    } else {
        writeln!(output, "{human}")?;
    }
    Ok(0)
}

pub(crate) fn selected_socket(
    socket: Option<&str>,
    caller: &CallerEnvironment,
) -> Result<String, Failure> {
    let socket = socket
        .map(str::to_owned)
        .or_else(|| {
            caller
                .selected_server_socket()
                .ok()
                .flatten()
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            Failure::new(
                "WORKSPACE_SELECTION_REQUIRED",
                "Select the original tmux socket with --socket PATH.",
                1,
            )
        })?;
    if socket.is_empty() || !std::path::Path::new(&socket).is_absolute() || socket.contains('\0') {
        return Err(Failure::new(
            "USAGE_ERROR",
            "--socket requires an absolute tmux socket path.",
            1,
        ));
    }
    Ok(socket)
}
