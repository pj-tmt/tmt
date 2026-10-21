//! CLI flag and interaction inputs for the shared browser opener.
use crate::{control, error::RemoteError};
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use tmt_invoke::open::{Decision, Env};
pub use tmt_invoke::open::{Flag, Outcome};

/// Only public short entries on the selected running local door are open targets.
pub(crate) fn valid_target(target: &str) -> bool {
    crate::pages::app_route(target)
        || crate::pages::short_page(target).is_some()
        || crate::mount::Mounts::valid_short_route(target)
}

#[derive(Serialize)]
pub struct PageOpen {
    pub version: u8,
    pub link: String,
    pub opened: bool,
    pub state: &'static str,
}

/// Resolve the platform opener before creating an offer. Its streams stay null in `tmt-invoke`.
pub fn open_page(
    directory: &Path,
    target: &str,
    flag: Flag,
    setting: bool,
    json: bool,
    interactive: bool,
) -> Result<PageOpen, RemoteError> {
    let env = Env::current(interactive);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let executable = tmt_invoke::open::find_opener(&env, &path);
    let launch = |link: &str| {
        tmt_invoke::open::launch(executable.as_deref().expect("selected opener"), link)
    };
    open_page_with(
        directory,
        target,
        flag,
        setting,
        json,
        &env,
        executable
            .as_ref()
            .map(|_| &launch as &dyn Fn(&str) -> Outcome),
    )
}

/// Test/extension composition injects the existing opener outcome, never a secret in the reply.
pub fn open_page_with(
    directory: &Path,
    target: &str,
    flag: Flag,
    setting: bool,
    json_output: bool,
    env: &Env,
    launch: Option<&dyn Fn(&str) -> Outcome>,
) -> Result<PageOpen, RemoteError> {
    if !valid_target(target) {
        return Err(crate::operations::invalid());
    }
    let status = control::status(directory, false)?.ok_or_else(|| {
        RemoteError::new(
            "REMOTE_NOT_RUNNING",
            "Remote is not running; start it with tmt remote serve.",
        )
    })?;
    let origin = status["origin"]
        .as_str()
        .ok_or_else(crate::operations::invalid)?;
    let mut result = PageOpen {
        version: 1,
        link: format!("{origin}{target}"),
        opened: false,
        state: "skipped",
    };
    if matches!(
        tmt_invoke::open::decide(flag, setting, json_output, env),
        Decision::Skip(_)
    ) {
        return Ok(result);
    }
    let Some(launch) = launch else {
        result.state = "no-opener";
        return Ok(result);
    };
    let answer = match control::request(directory, &json!({"op":"auto-pair","target":target})) {
        Ok(Some(answer)) => Some(answer),
        // No offer was made: open plainly without changing the manual ceremony.
        Err(error) if error.code == "REMOTE_PAIRING_BUSY" => None,
        Ok(None) => return Err(control::outdated_serve()),
        Err(error) => {
            return Err(match error.code.as_str() {
                "REMOTE_CONTROL_UNSUPPORTED" => control::outdated_serve(),
                _ => RemoteError::new(
                    "REMOTE_OPEN_UNAVAILABLE",
                    "Could not get the browser link ready, so nothing was opened. Try again.",
                ),
            });
        }
    };
    // This private control answer is never serialized or used in a diagnostic.
    let (id, link) = if let Some(answer) = answer.as_ref() {
        let id = answer["offerId"]
            .as_str()
            .filter(|id| crate::canonical::uuid(id).is_ok())
            .ok_or_else(control::outdated_serve)?;
        let ephemeral = answer["link"].as_str().filter(|link| {
            link.strip_prefix(&format!("{origin}/remote/pair#auto="))
                .is_some_and(|code| {
                    code.len() == 26 && crate::canonical::pairing_code(code).is_ok()
                })
        });
        let Some(ephemeral) = ephemeral else {
            let _ = control::request(directory, &json!({"op":"auto-cancel","offerId":id}));
            return Err(control::outdated_serve());
        };
        (Some(id), ephemeral)
    } else {
        (None, result.link.as_str())
    };
    match launch(link) {
        Outcome::Opened => {
            result.opened = true;
            result.state = "opened";
        }
        // Opener detail may contain arbitrary strings; never copy it into output.
        _ => {
            if let Some(id) = id {
                let _ = control::request(directory, &json!({"op":"auto-cancel","offerId":id}));
            }
            result.state = "failed";
        }
    }
    Ok(result)
}

pub fn flag(args: &clap::ArgMatches) -> Flag {
    match (args.get_flag("open"), args.get_flag("no-open")) {
        (_, true) => Flag::NoOpen,
        (true, false) => Flag::Open,
        _ => Flag::Unset,
    }
}

pub fn open_link(link: &str, flag: Flag, setting: bool, json: bool) -> Outcome {
    tmt_invoke::open::open_link(
        link,
        flag,
        setting,
        json,
        tmt_cli_style::Interaction::detect(false).stdout,
    )
}
