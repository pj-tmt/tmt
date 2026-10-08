//! Version 1 JSON codec. Strict bounds and topology validation precede use.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::io;
use tmt_core::{endpoint::ProcessIncarnation, workspace::*};

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Malformed or unsupported workspace snapshot.",
    )
}
fn text(value: &Value, key: &str) -> io::Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| text.len() <= 32 * 1024 && !text.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn number(value: &Value, key: &str) -> io::Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or_else(invalid)
}
fn optional_text(value: &Value, key: &str) -> io::Result<Option<String>> {
    match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(_)) => text(value, key).map(Some),
        _ => Err(invalid()),
    }
}
fn optional_bool(value: &Value, key: &str) -> io::Result<Option<bool>> {
    match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(invalid()),
    }
}
fn array<'a>(value: &'a Value, key: &str) -> io::Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .filter(|array| array.len() <= MAX_PANES)
        .map(Vec::as_slice)
        .ok_or_else(invalid)
}
fn process(value: &Value) -> io::Result<ProcessIncarnation> {
    ProcessIncarnation::new(number(value, "pid")?, &text(value, "start")?).map_err(|_| invalid())
}
fn process_value(value: &ProcessIncarnation) -> Value {
    json!({"pid": value.pid(), "start": value.start_identity()})
}
fn command_value(value: &ExternalCommand) -> Value {
    json!({"argv": value.argv, "owner": process_value(&value.owner)})
}
fn command(value: &Value) -> io::Result<ExternalCommand> {
    let argv = array(value, "argv")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|text| text.len() <= 32 * 1024 && !text.contains('\0'))
                .map(str::to_owned)
                .ok_or_else(invalid)
        })
        .collect::<io::Result<Vec<_>>>()?;
    if argv.len() < 2
        || argv[0] != "tmt"
        || !tmt_core::extension_command::valid_extension_name(&argv[1])
    {
        return Err(invalid());
    }
    Ok(ExternalCommand {
        argv,
        owner: process(value.get("owner").ok_or_else(invalid)?)?,
    })
}

/// The marker uses a format-safe alphabet, so a tmux format comparison can
/// atomically remove only this exact value without evaluating user argv.
pub fn encode_command(value: &ExternalCommand) -> io::Result<String> {
    let bytes = serde_json::to_vec(&command_value(value)).map_err(io::Error::other)?;
    if bytes.len() > 32 * 1024 {
        return Err(invalid());
    }
    command(&serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid())?)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn decode_command(text: &str) -> Option<ExternalCommand> {
    if text.len() > 48 * 1024 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
    command(&crate::json_document::parse(std::str::from_utf8(&bytes).ok()?).ok()?).ok()
}

pub fn encode(snapshot: &Snapshot) -> io::Result<Vec<u8>> {
    let document = json!({
        "version": VERSION, "capturedAtMs": snapshot.captured_at_ms,
        "server": { "socket": snapshot.server.socket, "process": process_value(&snapshot.server.process), "id": snapshot.server.id },
        "sessions": snapshot.sessions.iter().map(|session| json!({ "id": session.id, "name": session.name, "windows": session.windows.iter().map(|link| json!({ "index": link.index, "window": link.window, "active": link.active })).collect::<Vec<_>>() })).collect::<Vec<_>>(),
        "windows": snapshot.windows.iter().map(|window| json!({ "id": window.id, "name": window.name, "layout": window.layout, "visibleLayout": window.visible_layout, "width": window.width, "height": window.height, "activePane": window.active_pane })).collect::<Vec<_>>(),
        "panes": snapshot.panes.iter().map(|pane| json!({ "id": pane.id, "window": pane.window, "index": pane.index, "left": pane.left, "top": pane.top, "width": pane.width, "height": pane.height, "cwd": pane.cwd,
            "identity": pane.identity.as_ref().map(|value| json!({"id": value.id, "name": value.name, "lifetime": value.lifetime, "binding": value.binding, "harness": value.harness, "session": value.session, "mode": value.mode, "channel": value.channel})),
            "command": pane.command.as_ref().map(command_value) })).collect::<Vec<_>>(),
    });
    let bytes = serde_json::to_vec(&document).map_err(io::Error::other)?;
    decode(&bytes)?;
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> io::Result<Snapshot> {
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    let document = crate::json_document::parse(std::str::from_utf8(bytes).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    if number(&document, "version")? != VERSION {
        return Err(invalid());
    }
    let server = document.get("server").ok_or_else(invalid)?;
    let snapshot = Snapshot {
        captured_at_ms: number(&document, "capturedAtMs")?,
        server: Server {
            socket: text(server, "socket")?,
            process: process(server.get("process").ok_or_else(invalid)?)?,
            id: optional_text(server, "id")?,
        },
        sessions: array(&document, "sessions")?
            .iter()
            .map(|value| {
                Ok(WorkspaceSession {
                    id: text(value, "id")?,
                    name: text(value, "name")?,
                    windows: array(value, "windows")?
                        .iter()
                        .map(|value| {
                            Ok(WindowLink {
                                index: number(value, "index")?,
                                window: text(value, "window")?,
                                active: value
                                    .get("active")
                                    .and_then(Value::as_bool)
                                    .ok_or_else(invalid)?,
                            })
                        })
                        .collect::<io::Result<_>>()?,
                })
            })
            .collect::<io::Result<_>>()?,
        windows: array(&document, "windows")?
            .iter()
            .map(|value| {
                Ok(Window {
                    id: text(value, "id")?,
                    name: text(value, "name")?,
                    layout: text(value, "layout")?,
                    visible_layout: text(value, "visibleLayout")?,
                    width: number(value, "width")?,
                    height: number(value, "height")?,
                    active_pane: text(value, "activePane")?,
                })
            })
            .collect::<io::Result<_>>()?,
        panes: array(&document, "panes")?
            .iter()
            .map(|value| {
                Ok(Pane {
                    id: text(value, "id")?,
                    window: text(value, "window")?,
                    index: number(value, "index")?,
                    left: number(value, "left")?,
                    top: number(value, "top")?,
                    width: number(value, "width")?,
                    height: number(value, "height")?,
                    cwd: text(value, "cwd")?,
                    identity: match value.get("identity") {
                        Some(Value::Null) => None,
                        Some(identity) => Some(Identity {
                            id: text(identity, "id")?,
                            name: text(identity, "name")?,
                            lifetime: text(identity, "lifetime")?,
                            binding: text(identity, "binding")?,
                            harness: optional_text(identity, "harness")?,
                            session: optional_text(identity, "session")?,
                            mode: optional_text(identity, "mode")?,
                            channel: optional_bool(identity, "channel")?,
                        }),
                        None => return Err(invalid()),
                    },
                    command: match value.get("command") {
                        Some(Value::Null) => None,
                        Some(value) => Some(command(value)?),
                        None => return Err(invalid()),
                    },
                })
            })
            .collect::<io::Result<_>>()?,
    };
    validate(&snapshot)?;
    Ok(snapshot)
}

fn validate(snapshot: &Snapshot) -> io::Result<()> {
    if snapshot.is_consistent() {
        Ok(())
    } else {
        Err(invalid())
    }
}
