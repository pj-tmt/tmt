//! Foreground launch, exact-resume and message-channel enrollment command grammar.

use crate::grammar::{base, internal, operand, option, storage};
use clap::{Arg, ArgAction, ArgGroup, Command};

pub(in crate::grammar) fn run() -> Command {
    let command = base(spec!(
        "run",
        "Start a known agent, or bind this pane and run a command",
        [
            "Start and name an agent" => "tmt run worker claude",
            "Save the agent for later" => "tmt run --save worker claude",
            "Use plain paste delivery" => "tmt run --no-channel worker codex",
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
            .value_name("AGENT [ARGS...] | NAME [COMMAND...]")
            .required(true)
            .num_args(1..)
            .trailing_var_arg(true)
            .value_parser(clap::builder::OsStringValueParser::new()),
    );
    channel_options(command, false)
}

/// Inspection and recovery of one `tmt run --channel` enrollment.
pub(in crate::grammar) fn channel() -> Command {
    storage(spec!(
        "channel",
        "Inspect or recover a message-channel enrollment",
        [
            "Show what an agent's enrollment recorded" => "tmt channel inspect worker",
            "Recover the abandoned enrollment talk named" => "tmt channel recover --binding 11111111-1111-4111-8111-111111111111 --generation 22222222-2222-4222-8222-222222222222",
        ]
    ))
    .subcommand_required(true)
    .subcommand(enrollment(storage(spec!(
        "inspect",
        "Show an enrollment and whether each process it recorded still runs",
        details = "Read-only: it changes no file and signals no process.",
        [
            "For an active agent" => "tmt channel inspect worker",
            "For the binding a talk error named" => "tmt channel inspect --binding 11111111-1111-4111-8111-111111111111",
        ]
    ))))
    .subcommand(
        enrollment(storage(spec!(
            "recover",
            "Remove an abandoned enrollment so its pane is pasted to again",
            details = "Removes only the named generation, and only when every process it recorded is gone.\nIt never signals a process, sends, resends or pastes. Run it again safely.\nWhen the launch never recorded its agent, check the pane yourself first.",
            [
                "Recover the abandoned enrollment talk named" => "tmt channel recover --binding 11111111-1111-4111-8111-111111111111 --generation 22222222-2222-4222-8222-222222222222",
                "For an active agent's binding" => "tmt channel recover worker --generation 22222222-2222-4222-8222-222222222222",
            ]
        )))
        .arg(
            Arg::new("generation")
                .long("generation")
                .required(true)
                .help("The exact enrollment generation, as inspect or talk printed it"),
        ),
    )
}

/// The enrollment to act on: an identity target through the shared resolver, or
/// an exact binding ID that may no longer be current.
fn enrollment(command: Command) -> Command {
    command
        .arg(
            operand("target", false)
                .help("Identity name, UUID or pane of an active binding")
                .conflicts_with("binding"),
        )
        .arg(
            Arg::new("binding")
                .long("binding")
                .help("Exact binding ID, as a talk error printed it"),
        )
        .group(
            ArgGroup::new("enrollment")
                .args(["target", "binding"])
                .required(true),
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
    let command = base(spec!(
        "resume",
        "Resume an identity's remembered session in this pane",
        [
            "Resume an identity's last session" => "tmt resume worker",
            "Require a channel for exact resume" => "tmt resume --channel worker",
            "Forget an identity's last session" => "tmt resume --forget worker",
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
    .arg(operand("name", true).help("Identity name; TMT options go before it"));
    channel_options(command, true)
}

/// Both foreground commands expose the same mutually exclusive launch policy.
fn channel_options(command: Command, forget: bool) -> Command {
    let required = Arg::new("channel")
        .long("channel")
        .action(ArgAction::SetTrue)
        .conflicts_with("no-channel")
        .help("Require the agent's message channel; fail if unavailable (put before the name)");
    let disabled = Arg::new("no-channel")
        .long("no-channel")
        .action(ArgAction::SetTrue)
        .help("Use plain paste delivery for this launch (put before the name)");
    if forget {
        command
            .arg(required.conflicts_with("forget"))
            .arg(disabled.conflicts_with("forget"))
    } else {
        command.arg(required).arg(disabled)
    }
}
