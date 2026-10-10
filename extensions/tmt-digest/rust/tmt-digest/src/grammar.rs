//! The unpublished Digest entry exposes help and version only.

use clap::{ArgAction, Command};
use tmt_cli_style::{CommandSpec, Example, OutputModes};

const ROOT: CommandSpec = CommandSpec {
    name: "digest",
    summary: "Digest extension groundwork",
    examples: &[Example {
        command: "tmt digest --help",
        note: "Show the source-built extension's current capabilities",
    }],
    outputs: OutputModes::Human,
    details: "This source-built extension currently exposes help and version only. Member settings and delivery ticks are not available yet.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt digest")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
}
