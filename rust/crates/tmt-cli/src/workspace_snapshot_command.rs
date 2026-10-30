//! Workspace commands bypass learning, advisory capture and storage initialization.

use crate::{
    invocation::{Invocation, OutputMode},
    output::Failure,
};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    workspace::{Store, saved},
};
use tmt_core::{
    host::HostKind,
    workspace::{WorkspaceSnapshot, builtin::BuiltinSnapshot},
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
    let host = crate::caller_context::workspace_host(&caller, socket.is_some())?;
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

/// Without `--socket` or `--host`, a caller on the built-in host, or in no tmux server at all,
/// selects the built-in store. A caller inside tmux keeps its socket and an external host is
/// refused as before, so only the plain-terminal case changes.
fn implicit_builtin(invocation: &Invocation) -> Result<bool, Failure> {
    let (socket, builtin) = match invocation {
        Invocation::WorkspaceShow {
            socket, builtin, ..
        }
        | Invocation::WorkspaceRestore {
            socket, builtin, ..
        }
        | Invocation::WorkspaceSave {
            socket, builtin, ..
        }
        | Invocation::WorkspaceRemove {
            socket, builtin, ..
        }
        | Invocation::WorkspaceList { socket, builtin } => (socket, *builtin),
        _ => return Ok(false),
    };
    if builtin || socket.is_some() {
        return Ok(false);
    }
    let caller = CallerEnvironment::current();
    let host = crate::caller_context::workspace_host(&caller, false)?;
    Ok(host.kind() == HostKind::pty()
        || host.kind() == HostKind::Tmux && matches!(caller.selected_server_socket(), Ok(None)))
}

fn select_builtin(invocation: &Invocation) -> Invocation {
    let mut selected = invocation.clone();
    match &mut selected {
        Invocation::WorkspaceShow { builtin, .. }
        | Invocation::WorkspaceRestore { builtin, .. }
        | Invocation::WorkspaceSave { builtin, .. }
        | Invocation::WorkspaceRemove { builtin, .. }
        | Invocation::WorkspaceList { builtin, .. } => *builtin = true,
        _ => unreachable!("workspace commands only"),
    }
    selected
}

/// An implicit built-in selection with no built-in snapshot, while tmux stores exist (a tmux
/// user in a plain terminal after their server ended): say so and name the `--socket` form.
/// The selected store never changes; `socketStores` only lists what exists.
fn without_builtin_snapshot(invocation: &Invocation, mode: OutputMode) -> Option<io::Result<u8>> {
    let paths = ConfigPaths::discover().ok()?;
    let empty = saved::list_snapshots::<BuiltinSnapshot>(&paths, Store::Builtin)
        .is_ok_and(|entries| entries.is_empty());
    if !empty {
        return None;
    }
    let sockets = saved::socket_stores(&paths);
    let first = sockets.first()?;
    let command = match invocation {
        Invocation::WorkspaceShow { .. } => "show",
        Invocation::WorkspaceRestore { .. } => "restore",
        Invocation::WorkspaceSave { .. } => "save",
        Invocation::WorkspaceRemove { .. } => "rm",
        _ => "ls",
    };
    let hint = format!(
        "tmux workspace snapshots exist; select one with tmt workspace {command} --socket {}",
        crate::output::shell_word(first)
    );
    if command == "ls" {
        let mut output = tmt_cli_style::stream::stdout(mode.json);
        let written = if mode.json {
            writeln!(
                output,
                "{}",
                serde_json::json!({"version": 1, "host": HostKind::pty().as_str(), "snapshots": [], "socketStores": sockets})
            )
        } else {
            writeln!(output, "No built-in member snapshots. {hint}.")
        };
        return Some(written.map(|()| 0));
    }
    let failure = Failure::new(
        "WORKSPACE_SNAPSHOT_NOT_FOUND",
        "No built-in member snapshot exists. Nothing was changed.",
        3,
    )
    .suggestion(hint);
    if !mode.json {
        return Some(failure.publish(mode));
    }
    let mut document = failure.document();
    document["socketStores"] = sockets.into();
    Some(writeln!(tmt_cli_style::stream::stdout(true), "{document}").map(|()| failure.status))
}

pub fn execute(invocation: &Invocation, mode: OutputMode) -> io::Result<u8> {
    let selected;
    let invocation = match implicit_builtin(invocation) {
        Ok(false) => invocation,
        Ok(true) => {
            if let Some(published) = without_builtin_snapshot(invocation, mode) {
                return published;
            }
            selected = select_builtin(invocation);
            &selected
        }
        Err(error) => return error.publish(mode),
    };
    match invocation {
        Invocation::WorkspaceShow {
            socket,
            builtin,
            name,
        } => {
            if *builtin {
                return crate::workspace_show_command::execute_builtin(name.as_ref(), mode);
            }
            return crate::workspace_show_command::execute(socket.as_deref(), name.as_ref(), mode);
        }
        Invocation::WorkspaceRestore {
            builtin: true,
            name,
            expect,
            fresh,
            ..
        } => {
            return crate::workspace_restore_command::execute_builtin(
                name.as_ref(),
                expect.as_deref(),
                *fresh,
                mode,
            );
        }
        Invocation::WorkspaceRestore {
            socket,
            name,
            expect,
            layout_only,
            ..
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
        let (socket, builtin, name, unavailable) = match invocation {
            Invocation::WorkspaceSave {
                socket,
                builtin,
                name,
            } => (socket, *builtin, Some(name), "WORKSPACE_SAVE_UNAVAILABLE"),
            Invocation::WorkspaceRemove {
                socket,
                builtin,
                name,
            } => (socket, *builtin, Some(name), "WORKSPACE_REMOVE_UNAVAILABLE"),
            Invocation::WorkspaceList { socket, builtin } => {
                (socket, *builtin, None, "WORKSPACE_LIST_UNAVAILABLE")
            }
            _ => unreachable!("workspace commands only"),
        };
        // The built-in store needs no host or socket: its members are read only by capture.
        let (paths, socket) = if builtin {
            (ConfigPaths::discover().map_err(Failure::from)?, None)
        } else {
            let (paths, socket, _) = context(socket.as_deref())?;
            (paths, Some(socket))
        };
        let store = socket.as_deref().map_or(Store::Builtin, Store::Socket);
        let mut document = match &socket {
            Some(socket) => {
                serde_json::json!({"version":1,"host":HostKind::Tmux.as_str(),"socket":socket})
            }
            None => serde_json::json!({"version":1,"host":HostKind::pty().as_str()}),
        };
        let human = match invocation {
            Invocation::WorkspaceSave { .. } => {
                let name = name.expect("save name");
                let saved = if builtin {
                    saved::save::<BuiltinSnapshot>(&paths, store, name)
                } else {
                    saved::save::<WorkspaceSnapshot>(&paths, store, name)
                }
                .map_err(|error| failure(error, unavailable, false))?;
                document
                    .as_object_mut()
                    .expect("object")
                    .extend(entry(&saved).as_object().expect("entry object").clone());
                format!(
                    "Saved workspace {:?} (captured {}).",
                    name.as_str(),
                    tmt_cli_style::value::relative_time(
                        tmt_adapters::request_runtime::wall_time_ms()
                            .saturating_sub(saved.captured_at_ms)
                    )
                )
            }
            Invocation::WorkspaceRemove { .. } => {
                let name = name.expect("remove name");
                if builtin {
                    saved::remove::<BuiltinSnapshot>(&paths, store, name)
                } else {
                    saved::remove::<WorkspaceSnapshot>(&paths, store, name)
                }
                .map_err(|error| failure(error, unavailable, false))?;
                document["name"] = name.as_str().into();
                document["removed"] = true.into();
                format!("Removed workspace {:?}.", name.as_str())
            }
            Invocation::WorkspaceList { .. } => {
                let entries = if builtin {
                    saved::list_snapshots::<BuiltinSnapshot>(&paths, store)
                } else {
                    saved::list_snapshots::<WorkspaceSnapshot>(&paths, store)
                }
                .map_err(|error| failure(error, unavailable, false))?;
                document["snapshots"] = entries.iter().map(entry).collect::<Vec<_>>().into();
                let mut lines = vec![match &socket {
                    Some(socket) => format!("Workspace snapshots on {socket}:"),
                    None => "Built-in member snapshots:".to_owned(),
                }];
                for saved in entries {
                    lines.push(format!(
                        "  {} (captured {}; {} {}, {} agents{})",
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
                        if builtin { "members" } else { "panes" },
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

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "tmux workspace snapshots exist; select one with tmt workspace {command} --socket {}",
        &[""],
        &[
            ("{command}", "ls"),
            ("--socket {}", "--socket /tmp/hint.sock"),
        ],
    )];
