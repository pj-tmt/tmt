//! Session-only Claude hooks. Setup owns persistent settings and hook ownership;
//! this driver owns CLI settings composition and Stop continuation semantics.
use crate::{
    runtime::{
        RuntimeCommand,
        hook_protocol::{CONTEXT_LIMIT, HOOK_TIMEOUT_SECONDS, LaunchHooks},
    },
    setup,
};
use serde_json::json;
use std::{io, path::Path};
use tmt_core::binding::session::ProviderSessionId;

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn prepare(plan: &LaunchHooks<'_>) -> io::Result<RuntimeCommand> {
    let launcher = plan
        .tmt
        .to_str()
        .filter(|path| Path::new(path).is_absolute() && !path.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("Invalid hook launcher."))?;
    if !plan.launch.valid() {
        return Err(io::Error::other("Invalid hook launch coordinates."));
    }
    let mut command = plan.command.clone();
    let boundary = command
        .args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(command.args.len());
    let mut selected = None;
    let mut sources = None;
    let mut index = 0;
    while index < boundary {
        let argument = command.args[index].to_str();
        if matches!(argument, Some("--bare" | "--safe-mode")) {
            return Err(io::Error::other(
                "The provider launch explicitly disables hooks.",
            ));
        }
        let source = if argument == Some("--setting-sources") {
            let value = command
                .args
                .get(index + 1)
                .and_then(|value| value.to_str())
                .ok_or_else(|| io::Error::other("--setting-sources requires a UTF-8 value."))?;
            Some((2, value.to_owned()))
        } else {
            argument
                .and_then(|arg| arg.strip_prefix("--setting-sources="))
                .map(|value| (1, value.to_owned()))
        };
        if let Some((count, value)) = source {
            if sources.is_some()
                || value
                    .split(',')
                    .any(|source| !matches!(source, "" | "user" | "project" | "local"))
            {
                return Err(io::Error::other(
                    "Ambiguous or unknown --setting-sources selection.",
                ));
            }
            sources = Some(value);
            index += count;
            continue;
        }
        let setting = if argument == Some("--settings") {
            let value = command
                .args
                .get(index + 1)
                .and_then(|value| value.to_str())
                .ok_or_else(|| io::Error::other("--settings requires UTF-8 settings."))?;
            Some((index, 2, value.to_owned()))
        } else {
            argument
                .and_then(|arg| arg.strip_prefix("--settings="))
                .map(|value| (index, 1, value.to_owned()))
        };
        if let Some(setting) = setting {
            if selected.is_some() {
                return Err(io::Error::other(
                    "Multiple --settings values cannot be composed safely.",
                ));
            }
            index += setting.1;
            selected = Some(setting);
        } else {
            index += 1;
        }
    }
    let input = match &selected {
        None => "{}".to_owned(),
        Some((_, _, value)) if value.trim_start().starts_with('{') => value.clone(),
        Some((_, _, value)) => {
            setup::read_settings(&plan.environment.resolve(Path::new(value)))?
                .ok_or_else(|| io::Error::other("The explicit settings file does not exist."))?
        }
    };
    if input.len() > setup::SETTINGS_LIMIT {
        return Err(io::Error::other("Launch settings exceed 1 MiB."));
    }
    let global = if sources
        .as_ref()
        .is_none_or(|sources| sources.split(',').any(|source| source == "user"))
    {
        setup::read_settings(&super::config_root(plan.environment).join("settings.json"))?
            .unwrap_or_else(|| "{}".to_owned())
    } else {
        "{}".to_owned()
    };
    let observer = crate::runtime::hook_protocol::command_entry(super::NAME, launcher);
    let observations = ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop"].map(|event| {
        let mut entry = observer.clone();
        if event == "Stop" {
            entry["hooks"][0]["command"] = json!(format!(
                "{} __hook {} --activity-only",
                quote(launcher),
                super::NAME
            ));
        }
        (event.to_owned(), entry)
    });
    let scope = serde_json::to_string(plan.launch).map_err(io::Error::other)?;
    let digest = json!({"hooks":[{"type":"command", "command":format!("{} __digest-hook {} --launch {}", quote(launcher), super::NAME, quote(&scope)), "timeout":HOOK_TIMEOUT_SECONDS}]});
    let settings =
        setup::compose_launch_hooks(&super::DRIVER, &input, &global, &observations, &digest)
            .map_err(io::Error::other)?;
    match selected {
        Some((index, count, _)) => {
            command
                .args
                .splice(index..index + count, ["--settings".into(), settings.into()]);
        }
        None => {
            command
                .args
                .splice(boundary..boundary, ["--settings".into(), settings.into()]);
        }
    }
    Ok(command)
}

pub(super) fn decode(bytes: &[u8]) -> Option<ProviderSessionId> {
    #[derive(serde::Deserialize)]
    struct Stop {
        hook_event_name: String,
        session_id: String,
        stop_hook_active: bool,
    }
    if bytes.len() > super::HOOK_INPUT_LIMIT {
        return None;
    }
    let stop: Stop = serde_json::from_slice(bytes).ok()?;
    (stop.hook_event_name == "Stop" && !stop.stop_hook_active).then_some(())?;
    ProviderSessionId::new(&stop.session_id).ok()
}

pub(super) fn encode(digest: &str) -> Option<String> {
    (!digest.is_empty() && digest.len() <= CONTEXT_LIMIT)
        .then(|| json!({"decision":"block", "reason":digest}).to_string())
}

#[cfg(test)]
mod tests;
