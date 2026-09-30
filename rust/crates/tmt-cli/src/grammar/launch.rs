//! Foreground launch and exact-resume command grammar.

use crate::grammar::{base, operand, option};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn run() -> Command {
    base(spec!(
        "run",
        "Bind this pane and run a command with its original arguments",
        [
            "Start an agent in this pane under a temporary name" => "tmt run worker claude",
            "Keep the identity after the pane is gone" => "tmt run --save worker claude",
            "Resume the remembered session" => "tmt run --resume worker",
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
        Arg::new("run-argv")
            .value_name("NAME [COMMAND...]")
            .required(true)
            .num_args(1..)
            .trailing_var_arg(true)
            .value_parser(clap::builder::OsStringValueParser::new()),
    )
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
