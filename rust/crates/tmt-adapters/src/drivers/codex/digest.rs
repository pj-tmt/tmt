//! Invocation-only Codex hooks. Definitions contain no launch coordinates:
//! exact native binding lookup admits the hook when it actually runs.
use crate::{
    runtime::{
        RuntimeCommand,
        hook_protocol::{CONTEXT_LIMIT, HOOK_INPUT_LIMIT, HOOK_TIMEOUT_SECONDS, LaunchHooks},
    },
    setup,
};
use serde_json::{Value, json};
use std::{io, path::Path};
use tmt_core::binding::session::ProviderSessionId;
use toml_edit::{DocumentMut, Item};

mod settings;

fn unavailable() -> io::Error {
    io::Error::other("Codex hook sources cannot be composed safely.")
}

pub(super) fn prepare(plan: &LaunchHooks<'_>) -> io::Result<RuntimeCommand> {
    let launcher = plan
        .tmt
        .to_str()
        .filter(|s| Path::new(s).is_absolute() && !s.chars().any(char::is_control))
        .ok_or_else(unavailable)?;
    if !plan.launch.valid() {
        return Err(unavailable());
    }
    let (session, cwd) = settings::launch_settings(plan)?;
    let sources = settings::sources(plan.environment, &cwd, Path::new("/etc/codex"))?;
    compose(plan.command, launcher, session, &sources)
}

fn compose(
    command: &RuntimeCommand,
    launcher: &str,
    session: Value,
    sources: &[String],
) -> io::Result<RuntimeCommand> {
    let session_text = session.to_string();
    let mut hooks = session.get("hooks").cloned().unwrap_or_else(|| json!({}));
    let observer = crate::runtime::hook_protocol::command_entry(super::NAME, launcher);
    for event in ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop"] {
        let mut owned = usize::from(
            setup::owned_event(&super::DRIVER, &session_text, event).map_err(io::Error::other)?,
        );
        for source in sources {
            owned += usize::from(
                setup::owned_event(&super::DRIVER, source, event).map_err(io::Error::other)?,
            );
        }
        if owned > 1 {
            return Err(unavailable());
        }
        if owned == 0 {
            let mut entry = observer.clone();
            if event == "Stop" {
                entry["hooks"][0]["command"] = json!(format!(
                    "'{}' __hook {} --activity-only",
                    launcher.replace('\'', "'\\''"),
                    super::NAME
                ));
            }
            append(&mut hooks, event, entry)?;
        }
    }
    // The trust hash covers this definition. Never embed identity, binding,
    // process start, session, environment values, or an ephemeral file path.
    let digest = json!({"hooks":[{"type":"command", "command":format!("'{}' __digest-hook {} --discover-launch", launcher.replace('\'', "'\\''"), super::NAME), "timeout":HOOK_TIMEOUT_SECONDS}]});
    append(&mut hooks, "Stop", digest)?;
    let value = settings::toml(&hooks)?;
    if value.len() > setup::SETTINGS_LIMIT {
        return Err(unavailable());
    }
    let mut result = command.clone();
    let boundary = result
        .args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(result.args.len());
    let insertion = if result.args.first().is_some_and(|arg| arg == "resume") {
        1
    } else {
        boundary
    };
    result.args.splice(
        insertion..insertion,
        ["-c".into(), format!("hooks={value}").into()],
    );
    Ok(result)
}

fn append(hooks: &mut Value, event: &str, entry: Value) -> io::Result<()> {
    let map = hooks.as_object_mut().ok_or_else(unavailable)?;
    map.entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(unavailable)?
        .push(entry);
    Ok(())
}

pub(super) fn decode(bytes: &[u8]) -> Option<ProviderSessionId> {
    #[derive(serde::Deserialize)]
    struct Stop {
        hook_event_name: String,
        session_id: String,
        turn_id: String,
        stop_hook_active: bool,
    }
    if bytes.len() > HOOK_INPUT_LIMIT {
        return None;
    }
    let stop: Stop = serde_json::from_slice(bytes).ok()?;
    (stop.hook_event_name == "Stop" && !stop.stop_hook_active).then_some(())?;
    ProviderSessionId::new(&stop.turn_id).ok()?;
    ProviderSessionId::new(&stop.session_id).ok()
}

pub(super) fn encode(digest: &str) -> Option<String> {
    (!digest.is_empty() && digest.len() <= CONTEXT_LIMIT)
        .then(|| json!({"decision":"block", "reason":digest}).to_string())
}

#[cfg(test)]
mod tests;
