//! Identity bindings, live presence and pane-view command grammar.

use crate::grammar::{general, list_filter, operand, option, with_options};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn list_command() -> Command {
    general(spec!(
        "ls",
        "List global identities, lifetime and live presence",
        [
            "List identities and whether they are live" => "tmt ls",
            "List the members of a room" => "tmt ls --room reviewers",
            "The same, as JSON" => "tmt ls --json",
        ]
    ))
    .alias("list")
    .arg(operand("target", false).conflicts_with("room"))
    .arg(option("room"))
    .arg(list_filter("saved", "Only saved identities").conflicts_with("temp"))
    .arg(list_filter("temp", "Only temporary identities"))
    .arg(list_filter("here", "Only agents in this tmux session"))
    .arg(list_filter("all", "Show offline identities one per row"))
}

pub(in crate::grammar) fn add() -> Command {
    with_options(
        general(spec!(
            "add",
            "Bind an explicit pane; temporary unless saved",
            [
                "Bind a pane by its tmux id" => "tmt add %3 worker",
                "Keep the identity after the pane is gone" => "tmt add --save %3 worker",
            ]
        )),
        &["save"],
    )
    .arg(operand("pane-target", true))
    .arg(operand("name", true))
}

pub(in crate::grammar) fn name() -> Command {
    with_options(
        general(spec!(
            "name",
            "Bind this pane; temporary unless saved",
            [
                "Name this pane" => "tmt name worker",
                "Keep the identity after the pane is gone" => "tmt name --save worker",
            ]
        )),
        &["save"],
    )
    .visible_alias("this")
    .arg(operand("name", true))
}

pub(in crate::grammar) fn marked() -> Command {
    with_options(
        general(spec!(
            "marked",
            "Bind the pane explicitly marked in tmux; temporary unless saved",
            [
                "Bind the pane you marked in tmux" => "tmt marked worker",
            ]
        )),
        &["save"],
    )
    .arg(operand("name", true))
}

pub(in crate::grammar) fn remove() -> Command {
    with_options(
        general(spec!(
            "rm",
            "Retire identity and remove role/preamble; keep pane/exchanges (--force for saved)",
            [
                "Retire a temporary identity" => "tmt rm worker",
                "Retire a saved identity" => "tmt rm --force worker",
            ]
        )),
        &["force"],
    )
    .alias("remove")
    .arg(operand("name", true))
}

pub(in crate::grammar) fn check() -> Command {
    with_options(
        general(spec!(
            "check",
            "Capture diagnostic pane output",
            details = "A check touching an expired/off Focus target may hand off one retained checklist after verifying that exact live session is idle; --capture-only never does. Capture never proves a final response.",
            [
                "Show recent output from an agent's pane" => "tmt check worker",
                "Show the last 50 lines" => "tmt check worker 50",
                "Read a pane without delivering a retained checklist" => "tmt check worker --capture-only",
            ]
        )),
        &["lines", "capture-only"],
    )
    .visible_alias("read")
    .arg(operand("target", true))
    .arg(operand("capture-lines", false))
}

pub(in crate::grammar) fn focus() -> Command {
    general(spec!(
        "focus",
        "Show an identity's or pane's view in your tmux client",
        [
            "Show an agent's pane in your tmux client" => "tmt focus worker",
            "Name your client and the pane it shows" => "tmt focus --client",
        ]
    ))
    .arg(operand("target", false).required_unless_present("client"))
    .arg(
        Arg::new("client")
            .long("client")
            .action(ArgAction::SetTrue)
            .conflicts_with("target")
            .help("Name your tmux client and the pane it shows; takes no target, switches nothing"),
    )
}

pub(in crate::grammar) fn whoami() -> Command {
    general(spec!(
        "whoami",
        "Show this pane's verified identity",
        [
            "Show this pane's identity" => "tmt whoami",
            "Include pending work" => "tmt whoami --context",
        ]
    ))
    .arg(
        Arg::new("context")
            .long("context")
            .action(ArgAction::SetTrue)
            .help("Read bounded identity and pending-work context without changing state"),
    )
}

pub(in crate::grammar) fn unbind() -> Command {
    general(spec!(
        "unbind",
        "Detach this pane; retire temporary identity",
        [
            "Detach this pane from its identity" => "tmt unbind",
        ]
    ))
}
