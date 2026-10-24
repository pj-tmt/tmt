//! Shared worker-side host observations and frozen literal argv preparation.
use crate::launch::display_executable;
use crate::{
    config::Config,
    core::{Core, SquadError},
};
pub(in crate::board) struct Loaded {
    pub hosts: Vec<crate::launch::Host>,
    pub config: Config,
    pub executable: String,
    pub display_executable: String,
}

pub(in crate::board) fn unavailable(text: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_LAUNCH_UNAVAILABLE", text)
}
pub(in crate::board) fn read(core: &Core) -> Result<Loaded, SquadError> {
    let hosts = crate::launch::hosts(&core.json(&["host", "templates"])?).map_err(unavailable)?;
    let executable = core
        .executable()
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| unavailable("Core executable cannot be copied."))?
        .into();
    Ok(Loaded {
        hosts,
        config: Config::load(core)?,
        executable,
        display_executable: display_executable(
            core.executable(),
            std::env::var_os("PATH").as_deref(),
        ),
    })
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
