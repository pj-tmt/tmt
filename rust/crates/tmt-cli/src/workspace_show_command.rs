//! Preview recovery without initializing storage, observing providers or publishing.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    time::Instant,
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace::{self, plan},
};

pub fn execute(socket: Option<&str>, mode: OutputMode) -> io::Result<u8> {
    let outcome = (|| {
        let caller = CallerEnvironment::current();
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
        if socket.is_empty()
            || !std::path::Path::new(&socket).is_absolute()
            || socket.contains('\0')
        {
            return Err(Failure::new(
                "USAGE_ERROR",
                "--socket requires an absolute tmux socket path.",
                1,
            ));
        }
        let paths = ConfigPaths::discover().map_err(Failure::from)?;
        let input = plan::read(
            &paths,
            &socket,
            &Host::for_caller(&caller),
            Instant::now() + workspace::CAPTURE_BUDGET,
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
            "Workspace on {} (captured {} ms)",
            input.snapshot.server.socket, input.snapshot.captured_at_ms
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
        WorkspacePaneAction::StaleRequiresRetry => "stale session (needs --retry)",
    }
}
