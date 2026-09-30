//! Identity records, notes and profile command grammar.

use crate::grammar::{general, operand, storage, with_options};
use clap::Command;

pub(in crate::grammar) fn rename() -> Command {
    storage(spec!(
        "rename",
        "Rename an identity; its UUID, session, profile, notes and history stay",
        [
            "Give an identity a new name" => "tmt rename worker reviewer",
        ]
    ))
    .arg(operand("old", true))
    .arg(operand("new", true))
}

pub(in crate::grammar) fn preamble() -> Command {
    general(spec!(
        "preamble",
        "Manage identity-owned preambles",
        [
            "Set an agent's preamble" => "tmt preamble set worker \"Answer in one paragraph\"",
            "Show preambles" => "tmt preamble show",
        ]
    ))
    .subcommand(
        general(spec!(
            "show",
            "Show preambles",
            [
                "Show every preamble" => "tmt preamble show",
                "Show one agent's preamble" => "tmt preamble show worker",
            ]
        ))
        .arg(operand("agent", false)),
    )
    .subcommand(
        general(spec!(
            "set",
            "Set a preamble",
            [
                "Set an agent's preamble" => "tmt preamble set worker \"Answer in one paragraph\"",
            ]
        ))
        .arg(operand("agent", true))
        .arg(operand("content", true).num_args(1..)),
    )
    .subcommand(
        general(spec!(
            "clear",
            "Clear a preamble",
            [
                "Clear an agent's preamble" => "tmt preamble clear worker",
            ]
        ))
        .arg(operand("agent", true)),
    )
}

pub(in crate::grammar) fn identity() -> Command {
    storage(spec!(
            "identity",
            "Manage identity records, metadata and self-reported status",
            [
                "Create a saved identity" => "tmt identity create reviewer",
                "Show an identity" => "tmt identity show reviewer",
            ]
        ))
        .subcommand_required(true)
        .subcommand(storage(spec!(
            "create",
            "Create or save an identity",
            [
                "Create a saved identity" => "tmt identity create reviewer",
            ]
        )).arg(operand("name", true)))
        .subcommand(
            storage(spec!(
                "rename",
                "Rename an identity; its UUID, session, profile, notes and history stay",
                [
                    "Give an identity a new name" => "tmt identity rename worker reviewer",
                ]
            ))
            .arg(operand("old", true))
            .arg(operand("new", true)),
        )
        .subcommand(storage(spec!(
            "show",
            "Show an identity by name or UUID, or the verified caller",
            [
                "Show an identity" => "tmt identity show reviewer",
                "Show this pane's identity" => "tmt identity show",
            ]
        )).arg(operand("name", false)))
        .subcommand(with_options(
            storage(spec!(
                "list",
                "List non-retired identities",
                [
                    "List identities" => "tmt identity list",
                    "Only those with matching metadata" => "tmt identity list --where team=infra",
                ]
            )),
            &["where", "has"],
        ))
        .subcommand(
            storage(spec!(
                "status",
                "Manage expiring self-reported activity, not endpoint presence",
                [
                    "Report what you are doing" => "tmt identity status set \"Reviewing PR 444\"",
                    "Show your status" => "tmt identity status show",
                ]
            ))
            .subcommand_required(true)
            .subcommand(with_options(
                storage(spec!(
                    "show",
                    "Show current or stale status",
                    [
                        "Show your status" => "tmt identity status show",
                        "Show another identity's status" => "tmt identity status show --identity reviewer",
                    ]
                )),
                &["identity"],
            ))
            .subcommand(
                with_options(
                    storage(spec!(
                        "set",
                        "Replace status and renew its expiry",
                        [
                            "Report what you are doing" => "tmt identity status set \"Reviewing PR 444\"",
                            "For thirty minutes" => "tmt identity status set \"Running the suite\" --for 30m",
                        ]
                    )),
                    &["identity", "mood", "for"],
                )
                .arg(operand("activity", true)),
            )
            .subcommand(with_options(
                storage(spec!(
                    "clear",
                    "Clear this identity's self-reported status",
                    [
                        "Clear your status" => "tmt identity status clear",
                    ]
                )),
                &["identity"],
            )),
        )
        .subcommand(
            storage(spec!(
                "meta",
                "Manage descriptive identity metadata",
                [
                    "Set a metadata value" => "tmt identity meta set team infra",
                    "List metadata" => "tmt identity meta list",
                ]
            ))
                .subcommand_required(true)
                .subcommand(
                    with_options(storage(spec!(
                        "set",
                        "Set a metadata value",
                        [
                            "Set a metadata value" => "tmt identity meta set team infra",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true))
                        .arg(operand("value", true)),
                )
                .subcommand(
                    with_options(storage(spec!(
                        "get",
                        "Get a metadata value",
                        [
                            "Read one value" => "tmt identity meta get team",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true)),
                )
                .subcommand(with_options(
                    storage(spec!(
                        "list",
                        "List metadata values",
                        [
                            "List metadata" => "tmt identity meta list",
                            "For another identity" => "tmt identity meta list --identity reviewer",
                        ]
                    )),
                    &["identity"],
                ))
                .subcommand(
                    with_options(storage(spec!(
                        "rm",
                        "Remove a metadata value",
                        [
                            "Remove a value" => "tmt identity meta rm team",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true)),
                ),
        )
}

pub(in crate::grammar) fn notes() -> Command {
    storage(spec!(
            "notes",
            "Access saved identity notes",
            [
                "Print the path of your notes file" => "tmt notes path",
            ]
        ))
            .subcommand_required(true)
            .subcommand(with_options(
                storage(spec!(
                    "path",
                    "Initialize and print the local Markdown path",
                    details = "Saved identities only. Edit the returned Markdown file directly; an Office\nnotebook object reads this same file. Office reads never create missing notes.",
                    [
                        "Print the path of your notes file" => "tmt notes path",
                        "For another saved identity" => "tmt notes path --identity reviewer",
                    ]
                )),
                &["identity"],
            ))
}

pub(in crate::grammar) fn role() -> Command {
    with_options(
        general(spec!(
            "role",
            "Manage role profiles",
            [
                "Set your role" => "tmt role set \"Reviews Rust changes\"",
                "Show your role" => "tmt role show",
            ]
        )),
        &["identity"],
    )
    .subcommand_required(true)
    .subcommand(with_options(
        general(spec!(
            "show",
            "Show a role",
            [
                "Show your role" => "tmt role show",
                "Show another identity's role" => "tmt role show --identity reviewer",
            ]
        )),
        &["identity"],
    ))
    .subcommand(
        with_options(
            general(spec!(
                "set",
                "Set a role from inline text or file",
                [
                    "Set your role" => "tmt role set \"Reviews Rust changes\"",
                    "Read it from a file" => "tmt role set --file role.md",
                ]
            )),
            &["identity", "file"],
        )
        .arg(operand("content", false)),
    )
    .subcommand(with_options(
        general(spec!(
            "clear",
            "Clear a role",
            [
                "Clear your role" => "tmt role clear",
            ]
        )),
        &["identity"],
    ))
}
