//! Workspace initialization and setting command grammar.

use crate::grammar::{general, operand, with_options};
use clap::Command;

pub(in crate::grammar) fn init() -> Command {
    general(spec!(
        "init",
        "Create workspace settings",
        [
            "Create settings for this workspace" => "tmt init",
        ]
    ))
}

pub(in crate::grammar) fn config() -> Command {
    general(spec!(
        "config",
        "View or modify settings",
        [
            "Show settings" => "tmt config show",
            "Change a setting" => "tmt config set preambleEvery 2",
        ]
    ))
    .subcommand(general(spec!(
        "show",
        "Show settings",
        [
            "Show settings and where each comes from" => "tmt config show",
        ]
    )))
    .subcommand(
        with_options(
            general(spec!(
                "set",
                "Set a setting",
                [
                    "Change a workspace setting" => "tmt config set preambleEvery 2",
                    "Change a global setting" => "tmt config set --global pasteEnterDelayMs 300",
                ]
            )),
            &["global"],
        )
        .arg(operand("key", true))
        .arg(operand("value", true).allow_negative_numbers(true)),
    )
    .subcommand(
        general(spec!(
            "rm",
            "Reset local setting overrides; global settings and defaults stay",
            details = "With a key, remove that workspace override; without a key, remove all local\noverrides. Global settings and built-in defaults are retained.",
            [
                "Clear a workspace setting" => "tmt config rm preambleEvery",
            ]
        ))
        .alias("clear")
        .arg(operand("key", false)),
    )
}
