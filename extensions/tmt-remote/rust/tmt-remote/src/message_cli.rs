//! Provisional public boundary: bounded input, no enrollment or provider bootstrap shortcut.
use clap::{Arg, ArgAction, Command};
use std::io::Read;
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{cloud_command, error::RemoteError, limits};
const MESSAGE: CommandSpec = CommandSpec {
    name: "message",
    summary: "Carry one prepared cloud message (pilot)",
    examples: &[Example {
        command: "tmt remote message --json < prepared-message.json",
        note: "Read one prepared cloud request",
    }],
    outputs: OutputModes::Json,
    details: "Reads one bounded version-1 request from stdin and writes one JSON answer. Cloud enrollment and the Core command alignment are not enabled yet. Nothing opens and an uncertain message is never sent again automatically.",
};
pub(crate) fn command() -> Command {
    tmt_cli_style::command(&MESSAGE)
        .arg(
            Arg::new("json")
                .long("json")
                .required(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("no-open")
                .long("no-open")
                .action(ArgAction::SetTrue)
                .hide(true),
        )
}
pub(crate) fn run(explicit: Option<&str>) -> Result<(), RemoteError> {
    let mut bytes = vec![];
    std::io::stdin()
        .take((limits::CLOUD_COMMAND_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let request = cloud_command::Request::read(&bytes)?;
    if explicit.is_some()
        && request.remote.as_deref().is_some()
        && explicit != request.remote.as_deref()
    {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "The command and prepared request name different remotes.",
        ));
    }
    // Final argv/schema alignment belongs to #4536, enrollment to #4538. The emulator
    // composes the same Request/Sender boundary with fixture-only seeded peers. Never
    // turn this provisional command into a production grant or name-based fallback.
    Err(cloud_command::upgrade())
}
