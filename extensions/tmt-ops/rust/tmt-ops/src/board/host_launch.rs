//! Shared worker-side host observations and frozen literal argv preparation.
use crate::launch::display_executable;
use crate::{
    config::Config,
    core::{Core, SquadError},
};
use std::collections::BTreeMap;
pub(in crate::board) struct Loaded {
    pub hosts: Vec<crate::launch::Host>,
    /// The host Ops runs in, as Core reads its locators; only ever a preselection.
    pub caller_host: CallerHost,
    pub config: Config,
    pub executable: String,
    pub display_executable: String,
    /// Each driver's models and efforts, by driver name; empty from an older Core.
    pub choices: BTreeMap<String, LaunchChoices>,
}
/// `callerHost` of `tmt host templates`: advisory, never evidence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) enum CallerHost {
    /// An older Core does not say, or the locators name more than one host.
    #[default]
    Unknown,
    /// No locator names a host: a plain terminal.
    Plain,
    Named(String),
}

impl CallerHost {
    pub(in crate::board) fn of(document: &serde_json::Value) -> Self {
        match document.get("callerHost") {
            None => Self::Unknown,
            Some(serde_json::Value::String(host)) => Self::Named(host.clone()),
            Some(serde_json::Value::Null) => Self::Plain,
            // Not a value this Ops knows: keep the default.
            Some(_) => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct LaunchChoices {
    pub models: Vec<String>,
    pub efforts: Vec<String>,
}

pub(in crate::board) fn unavailable(text: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_LAUNCH_UNAVAILABLE", text)
}
pub(in crate::board) fn read(core: &Core) -> Result<Loaded, SquadError> {
    let templates = core.json(&["host", "templates"])?;
    let hosts = crate::launch::hosts(&templates).map_err(unavailable)?;
    let executable = core
        .executable()
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| unavailable("Core executable cannot be copied."))?
        .into();
    Ok(Loaded {
        hosts,
        caller_host: CallerHost::of(&templates),
        config: Config::load(core)?,
        choices: launch_choices(core),
        executable,
        display_executable: display_executable(
            core.executable(),
            std::env::var_os("PATH").as_deref(),
        ),
    })
}

/// `drivers.models`, read once with the form. It only adds suggestions, so an older Core
/// or a failed read leaves the observed models and free-form entry.
fn launch_choices(core: &Core) -> BTreeMap<String, LaunchChoices> {
    let words = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|word| word.as_str())
            .filter(|word| !word.is_empty() && !word.chars().any(char::is_control))
            .map(str::to_owned)
            .collect()
    };
    core.api("drivers.models", serde_json::json!({}))
        .ok()
        .and_then(|document| document["drivers"].as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                row["driver"].as_str()?.to_owned(),
                LaunchChoices {
                    models: words(&row["models"]),
                    efforts: words(&row["efforts"]),
                },
            ))
        })
        .collect()
}

impl Loaded {
    pub fn prepare(
        &self,
        host: &str,
        session: &str,
        command: &[String],
    ) -> Result<Option<Vec<String>>, SquadError> {
        let host_row = self
            .hosts
            .iter()
            .find(|h| h.name == host && matches!(h.state.as_str(), "builtin" | "ok"))
            .ok_or_else(|| unavailable("Choose an available published host."))?;
        let argv = self
            .config
            .launch_override(host)?
            .or_else(|| host_row.argv.clone());
        let line = crate::launch::line(command);
        crate::launch::fill(
            &["host".into(), "{session}".into(), "{command}".into()],
            session,
            &line,
        )
        .map_err(unavailable)?;
        argv.as_ref()
            .map(|argv| crate::launch::fill(argv, session, &line).map_err(unavailable))
            .transpose()
    }
}
pub(in crate::board) fn recheck(
    core: &Core,
    host: &str,
    session: &str,
    command: &[String],
    outer: &[String],
) -> Result<(), SquadError> {
    let loaded = read(core)?;
    if loaded.executable != command[0]
        || loaded.prepare(host, session, command)?.as_deref() != Some(outer)
    {
        return Err(unavailable("Host launch changed; review again."));
    }
    Ok(())
}
