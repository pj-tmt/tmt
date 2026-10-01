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
            "Change a setting" => "tmt config set timeout 120",
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
                    "Change a workspace setting" => "tmt config set timeout 120",
                    "Change a global setting" => "tmt config set --global captureLines 200",
                ]
            )),
            &["global"],
        )
        .arg(operand("key", true))
        .arg(operand("value", true).allow_negative_numbers(true)),
    )
    .subcommand(
        general(spec!(
            "clear",
            "Clear a local setting",
            [
                "Clear a workspace setting" => "tmt config clear timeout",
            ]
        ))
        .arg(operand("key", false)),
    )
}
