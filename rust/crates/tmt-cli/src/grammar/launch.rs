//! Foreground launch and exact-resume command grammar.

use crate::grammar::{base, internal, operand, option};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn run() -> Command {
    base(spec!(
        "run",
        "Start a known agent, or bind this pane and run a command",
        [
            "Start an agent now and name it later with tmt this" => "tmt run claude",
            "Start an agent in this pane under a temporary name" => "tmt run worker claude",
            "Keep the identity after the pane is gone" => "tmt run --save worker claude",
        ]
    ))
    .arg(option("save"))
    .arg(
        Arg::new("resume")
            .long("resume")
            .action(ArgAction::SetTrue)
            .help("Resume the remembered session; put this option before the name"),
    )
    .arg(
        Arg::new("channel")
            .long("channel")
            .action(ArgAction::SetTrue)
            .conflicts_with("resume")
            .help("Deliver talk through the agent's message channel instead of paste (supported agents only); put this option before the name"),
    )
    .arg(
        Arg::new("run-argv")
            .value_name("AGENT [ARGS...] | NAME [COMMAND...]")
            .required(true)
            .num_args(1..)
            .trailing_var_arg(true)
            .value_parser(clap::builder::OsStringValueParser::new()),
    )
}

/// The stdio server a provider starts for a `--channel` launch.
pub(in crate::grammar) fn channel_server() -> Command {
    internal(
        "__channel-server",
        "Internal provider message-channel server",
    )
    .hide(true)
    .arg(operand("harness", true))
    .arg(operand("binding-id", true))
    .arg(operand("generation", true))
    .arg(operand("directory", true))
}

pub(in crate::grammar) fn resume() -> Command {
    base(spec!(
        "resume",
        "Resume an identity's remembered session in this pane",
        [
            "Resume an identity's last session" => "tmt resume worker",
            "Forget the remembered session" => "tmt resume --forget worker",
        ]
    ))
    .arg(
        Arg::new("forget")
            .long("forget")
            .action(ArgAction::SetTrue)
            .help("Forget the remembered session instead of resuming it"),
    )
    .arg(
        Arg::new("retry")
            .long("retry")
            .action(ArgAction::SetTrue)
            .conflicts_with("forget")
            .help("Try a session marked stale once more"),
    )
    .arg(operand("name", true).help("Identity name; TMT options go before it"))
}
