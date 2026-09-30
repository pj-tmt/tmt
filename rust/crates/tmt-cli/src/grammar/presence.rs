//! Identity bindings, live presence and pane-view command grammar.

use super::{general, list_filter, operand, option, with_options};
use clap::{Arg, ArgAction, Command};

pub(super) fn list() -> Command {
    general(spec!(
        "list",
        "List global identities, lifetime and live presence",
        [
            "List identities and whether they are live" => "tmt list",
            "List the members of a room" => "tmt list --room reviewers",
            "The same, as JSON" => "tmt list --json",
        ]
    ))
    .visible_alias("ls")
    .arg(operand("target", false).conflicts_with("room"))
    .arg(option("room"))
    .arg(list_filter("saved", "Only saved identities").conflicts_with("temp"))
    .arg(list_filter("temp", "Only temporary identities"))
    .arg(list_filter("here", "Only agents in this tmux session"))
    .arg(list_filter("all", "Show offline identities one per row"))
}

pub(super) fn add() -> Command {
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

pub(super) fn name() -> Command {
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

pub(super) fn marked() -> Command {
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

pub(super) fn remove() -> Command {
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
    .visible_alias("remove")
    .arg(operand("name", true))
}

pub(super) fn check() -> Command {
    with_options(
        general(spec!(
            "check",
            "Capture diagnostic pane output",
            [
                "Show recent output from an agent's pane" => "tmt check worker",
                "Show the last 50 lines" => "tmt check worker 50",
            ]
        )),
        &["lines"],
    )
    .visible_alias("read")
    .arg(operand("target", true))
    .arg(operand("capture-lines", false))
}

pub(super) fn focus() -> Command {
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

pub(super) fn whoami() -> Command {
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

pub(super) fn unbind() -> Command {
    general(spec!(
        "unbind",
        "Detach this pane; retire temporary identity",
        [
            "Detach this pane from its identity" => "tmt unbind",
        ]
    ))
}
