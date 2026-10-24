//! Public prepared messages reach the existing serving owner, never a new transport composition.
use clap::{Arg, ArgAction, Command};
use std::io::{Read, Write};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{control, error::RemoteError, limits};
const MESSAGE: CommandSpec = CommandSpec {
    name: "message",
    summary: "Carry one prepared cloud message",
    examples: &[Example {
        command: "tmt remote message --json < prepared-message.json",
        note: "Read one prepared cloud request",
    }],
    outputs: OutputModes::Json,
    details: "Reads one bounded version-1 request from stdin and writes one JSON answer through the running Remote owner. Requires an already joined cloud peer. Nothing opens, signs in or starts automatically, and an uncertain message is never sent again automatically.",
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
    // Reject invalid input before even asking Core for the storage root.
    let request = tmt_remote::cloud_command::Request::read(&bytes)?;
    request.selected(explicit)?;
    let root = super::discovery_root()?;
    let answer = control::cloud_message(&root.join("remote"), &bytes, explicit)?;
    writeln!(tmt_cli_style::stream::stdout(true), "{answer}")?;
    Ok(())
}
