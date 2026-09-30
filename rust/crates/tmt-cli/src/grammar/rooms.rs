//! Shared communication room command grammar.

use super::{operand, storage, with_options};
use clap::{Arg, Command};

pub(super) fn room() -> Command {
    storage(spec!(
            "room",
            "Manage shared communication rooms (not access controls)",
            [
                "Create a room" => "tmt room create reviewers",
                "Ask every member of a room" => "tmt room send reviewers \"Please review PR 444\"",
            ]
        ))
        .subcommand_required(true)
        .subcommand(storage(spec!(
            "create",
            "Create an empty room",
            [
                "Create a room" => "tmt room create reviewers",
            ]
        )).arg(operand("name", true)))
        .subcommand(storage(spec!(
            "list",
            "List rooms and membership counts",
            [
                "List rooms and their member counts" => "tmt room list",
                "The same, as JSON" => "tmt room list --json",
            ]
        )).visible_alias("ls"))
        .subcommand(
            storage(spec!(
                "retire",
                "Stop new room work while retaining content and history",
                [
                    "Retire a room, keeping its history" => "tmt room retire reviewers",
                ]
            ))
            .arg(operand("room", true)),
        )
        .subcommand(
            storage(spec!(
                "show",
                "Show a room by UUID or unique exact name",
                [
                    "Show a room and its members" => "tmt room show reviewers",
                ]
            )).arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "join",
                    "Join a room without changing other members",
                    [
                        "Join a room as this pane's identity" => "tmt room join reviewers",
                        "Add another identity" => "tmt room join reviewers --identity worker",
                    ]
                )),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "leave",
                    "Leave a room without deleting its history",
                    [
                        "Leave a room" => "tmt room leave reviewers",
                    ]
                )),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommands(
            [
                spec!(
                    "send",
                    "Queue one replyable inbox request per room member",
                    [
                        "Ask every member for a reply" => "tmt room send reviewers \"Please review PR 444\"",
                    ]
                ),
                spec!(
                    "broadcast",
                    "Queue a no-reply announcement per room member",
                    [
                        "Announce without expecting replies" => "tmt room broadcast reviewers \"Main is green again\"",
                    ]
                ),
            ]
            .into_iter()
            .map(|spec| {
                with_options(storage(spec), &["identity"])
                    .arg(operand("room", true))
                    .arg(operand("message", true))
                    .arg(
                        Arg::new("operation-id")
                            .long("operation-id")
                            .help("Reuse a UUID for safe retry of the same composition"),
                    )
            }),
        )
}
