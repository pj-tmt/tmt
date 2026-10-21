//! Workspace commands bypass learning, advisory capture and storage initialization.

use crate::{
    invocation::{Invocation, OutputMode},
    output::Failure,
};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace::saved,
};

pub fn eligible(invocation: &Invocation) -> bool {
    matches!(
        invocation,
        Invocation::WorkspaceShow { .. }
            | Invocation::WorkspaceRestore { .. }
            | Invocation::WorkspaceSave { .. }
            | Invocation::WorkspaceList { .. }
            | Invocation::WorkspaceRemove { .. }
    )
}

pub fn context(socket: Option<&str>) -> Result<(ConfigPaths, String, Host), Failure> {
    let caller = CallerEnvironment::current();
    let host = Host::for_workspace(&caller, socket.is_some());
    if !host.workspace_supported() {
        return Err(Failure::new(
            "WORKSPACE_HOST_UNSUPPORTED",
            "The selected host does not support workspace recovery; phase 1 supports tmux only.",
            1,
        ));
    }
    let socket = crate::workspace_show_command::selected_socket(socket, &caller)?;
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    Ok((paths, socket, host))
}

pub fn failure(error: saved::Error, unavailable: &'static str, legacy_latest: bool) -> Failure {
    let (code, exit) = match &error {
        saved::Error::NotFound if !legacy_latest => ("WORKSPACE_SNAPSHOT_NOT_FOUND", 3),
        saved::Error::Exists => ("WORKSPACE_SNAPSHOT_EXISTS", 5),
        saved::Error::Limit => ("WORKSPACE_SNAPSHOT_LIMIT", 1),
        saved::Error::Changed => ("WORKSPACE_SNAPSHOT_CHANGED", 5),
        saved::Error::Busy => ("WORKSPACE_BUSY", 5),
        _ => (unavailable, 1),
    };
    Failure::new(code, error.to_string(), exit).caused_by(error)
}

fn entry(entry: &saved::SnapshotEntry) -> serde_json::Value {
    serde_json::json!({"name": entry.name.as_ref().map(|name| name.as_str()), "digest": entry.digest, "capturedAtMs": entry.captured_at_ms,
        "generation": entry.generation.as_ref().map(|generation| generation.id.as_str()), "panes": entry.panes,
        "agents": entry.agents, "protected": entry.protected, "shrink": entry.generation.as_ref().is_some_and(|generation| generation.shrink)})
}

pub fn execute(invocation: &Invocation, mode: OutputMode) -> io::Result<u8> {
    match invocation {
        Invocation::WorkspaceShow { socket, name } => {
            return crate::workspace_show_command::execute(socket.as_deref(), name.as_ref(), mode);
        }
        Invocation::WorkspaceRestore {
            socket,
            name,
            expect,
            layout_only,
        } => {
            return crate::workspace_restore_command::execute(
                socket.as_deref(),
                name.as_ref(),
                expect.as_deref(),
                *layout_only,
                mode,
            );
        }
        _ => (),
    }
    let outcome: Result<_, Failure> = (|| {
        let (socket, name, unavailable) = match invocation {
            Invocation::WorkspaceSave { socket, name } => {
                (socket, Some(name), "WORKSPACE_SAVE_UNAVAILABLE")
            }
            Invocation::WorkspaceRemove { socket, name } => {
                (socket, Some(name), "WORKSPACE_REMOVE_UNAVAILABLE")
            }
            Invocation::WorkspaceList { socket } => (socket, None, "WORKSPACE_LIST_UNAVAILABLE"),
            _ => unreachable!("workspace commands only"),
        };
        let (paths, socket, _) = context(socket.as_deref())?;
        let mut document = serde_json::json!({"version":1,"socket":socket});
        let human = match invocation {
            Invocation::WorkspaceSave { .. } => {
                let saved = saved::save(&paths, &socket, name.expect("save name"))
                    .map_err(|error| failure(error, unavailable, false))?;
                document
                    .as_object_mut()
                    .expect("object")
                    .extend(entry(&saved).as_object().expect("entry object").clone());
                format!(
                    "Saved workspace {:?} (captured {}).",
                    name.expect("save name").as_str(),
                    tmt_cli_style::value::relative_time(
                        tmt_adapters::request_runtime::wall_time_ms()
                            .saturating_sub(saved.captured_at_ms)
                    )
                )
            }
            Invocation::WorkspaceRemove { .. } => {
                let name = name.expect("remove name");
                saved::remove(&paths, &socket, name)
                    .map_err(|error| failure(error, unavailable, false))?;
                document["name"] = name.as_str().into();
                document["removed"] = true.into();
                format!("Removed workspace {:?}.", name.as_str())
            }
            Invocation::WorkspaceList { .. } => {
                let entries = saved::list_snapshots(&paths, &socket)
                    .map_err(|error| failure(error, unavailable, false))?;
                document["snapshots"] = entries.iter().map(entry).collect::<Vec<_>>().into();
                let mut lines = vec![format!("Workspace snapshots on {socket}:")];
                for saved in entries {
                    lines.push(format!(
                        "  {} (captured {}; {} panes, {} agents{})",
                        saved
                            .name
                            .as_ref()
                            .map(|name| name.as_str())
                            .or_else(|| saved
                                .generation
                                .as_ref()
                                .map(|generation| generation.id.as_str()))
                            .unwrap_or("latest"),
                        tmt_cli_style::value::relative_time(
                            tmt_adapters::request_runtime::wall_time_ms()
                                .saturating_sub(saved.captured_at_ms)
                        ),
                        saved.panes,
                        saved.agents,
                        if saved.protected {
                            ", protected"
                        } else if saved
                            .generation
                            .as_ref()
                            .is_some_and(|generation| generation.shrink)
                        {
                            ", shrink"
                        } else {
                            ""
                        }
                    ));
                }
                lines.join("\n")
            }
            _ => unreachable!("snapshot management command"),
        };
        Ok((document, human))
    })();
    let (document, human) = match outcome {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{document}")?;
    } else {
        writeln!(output, "{human}")?;
    }
    Ok(0)
}
