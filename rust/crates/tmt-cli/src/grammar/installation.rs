//! Skill, hook and managed-product installation command grammar.

use crate::grammar::{channel_option, extension_target, general, internal, operand, with_options};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn extension() -> Command {
    general(spec!(
        "extension",
        "Manage consented extension integrations",
        [
            "List official extensions" => "tmt extension list",
            "Install Squad" => "tmt extension install squad",
        ]
    ))
    .subcommand_required(true)
    .subcommand(
        general(spec!(
            "hooks",
            "Manage lifecycle hooks for trusted extensions",
            [
                "List extensions with hooks enabled" => "tmt extension hooks list",
                "Deliver lifecycle hooks to Squad" => "tmt extension hooks enable squad",
            ]
        ))
        .subcommand_required(true)
        .subcommand(
            general(spec!(
                "enable",
                "Trust tmt-<name> on PATH to receive lifecycle observations",
                [
                    "Deliver lifecycle hooks to Squad" => "tmt extension hooks enable squad",
                ]
            ))
            .arg(operand("name", true)),
        )
        .subcommand(
            general(spec!(
                "disable",
                "Stop delivering hooks to an extension",
                [
                    "Stop hooks for Squad" => "tmt extension hooks disable squad",
                ]
            ))
            .arg(operand("name", true)),
        )
        .subcommand(general(spec!(
            "list",
            "List extensions with enabled hooks",
            [
                "List extensions with hooks enabled" => "tmt extension hooks list",
            ]
        ))),
    )
    .subcommand(
        extension_target(general(spec!(
            "install",
            "Install an official extension (office, squad)",
            [
                "Install Squad" => "tmt extension install squad",
                "Install without a prompt" => "tmt extension install squad --yes",
            ]
        )))
        .arg(channel_option())
        .arg(
            Arg::new("repair")
                .long("repair")
                .action(ArgAction::SetTrue)
                .conflicts_with("channel")
                .help("Restore the exact recorded release; retain the damaged files"),
        )
        .arg(Arg::new("archive").long("archive").requires("manifest"))
        .arg(Arg::new("manifest").long("manifest").requires("archive"))
        .arg(
            Arg::new("skills")
                .long("skills")
                .action(ArgAction::SetTrue)
                .help("Also publish the agent skills the extension bundles"),
        ),
    )
    .subcommand(
        extension_target(general(spec!(
            "upgrade",
            "Update an installed official extension",
            [
                "Update Squad" => "tmt extension upgrade squad",
                "Install an exact version" => "tmt extension upgrade squad --to 0.1.0-alpha.2",
            ]
        )))
        .arg(channel_option())
        .arg(Arg::new("to").long("to").conflicts_with("unpin"))
        .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue)),
    )
    .subcommand(extension_target(general(spec!(
        "uninstall",
        "Remove an extension's commands; releases and data are kept",
        [
            "Remove Squad's commands" => "tmt extension uninstall squad",
        ]
    ))))
    .subcommand(
        general(spec!(
            "list",
            "List official extensions, versions and PATH shadowing",
            [
                "List official extensions" => "tmt extension list",
                "Also check for newer releases" => "tmt extension list --check",
            ]
        ))
        .arg(Arg::new("prefix").long("prefix"))
        .arg(
            Arg::new("check")
                .long("check")
                .action(ArgAction::SetTrue)
                .help("Also check for a newer release (uses the network)"),
        ),
    )
}

pub(in crate::grammar) fn install(names: Vec<&'static str>) -> Command {
    with_options(
        general(spec!(
            "install",
            "Install or refresh agent skills",
            [
                "Install skills for every agent" => "tmt install",
                "Only for Claude" => "tmt install claude",
            ]
        )),
        &["force", "dir"],
    )
    .arg(
        operand("agent", false)
            .value_parser(names.into_iter().chain(["all"]).collect::<Vec<_>>())
            .ignore_case(true),
    )
}

pub(in crate::grammar) fn setup(hooked: Vec<&'static str>) -> Command {
    general(spec!(
        "setup",
        "Set up every detected agent: skills and session hooks, after one approval",
        [
            "Review and apply what is missing" => "tmt setup",
            "Only Claude's session hooks" => "tmt setup claude --yes",
            "Remove Claude's session hooks" => "tmt setup claude --remove",
        ]
    ))
    .arg(operand("provider", false).value_parser(hooked))
    .arg(
        Arg::new("remove")
            .long("remove")
            .action(ArgAction::SetTrue)
            .requires("provider"),
    )
    .arg(
        Arg::new("usage")
            .long("usage")
            .action(ArgAction::SetTrue)
            .help("Also install the turn-end hook that records context usage")
            .conflicts_with_all(["no-usage", "remove"]),
    )
    .arg(
        Arg::new("no-usage")
            .long("no-usage")
            .action(ArgAction::SetTrue)
            .help("Remove only the turn-end usage hook")
            .conflicts_with("remove"),
    )
    .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue))
}

pub(in crate::grammar) fn hook(hooked: Vec<&'static str>) -> Command {
    internal("__hook", "Internal bounded provider lifecycle callback")
        .hide(true)
        .arg(operand("provider", true).value_parser(hooked))
        .arg(
            Arg::new("worker")
                .long("worker")
                .hide(true)
                .action(ArgAction::SetTrue),
        )
}

pub(in crate::grammar) fn upgrade() -> Command {
    general(spec!(
        "upgrade",
        "Upgrade the native CLI and refresh managed skills",
        [
            "Update to the latest release on your channel" => "tmt upgrade",
            "Switch to the stable channel" => "tmt upgrade --channel stable",
            "Install an exact version" => "tmt upgrade --to 5.0.0-alpha.8",
        ]
    ))
    .visible_alias("update")
    .arg(
        Arg::new("channel")
            .long("channel")
            .value_parser(tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str())),
    )
    .arg(Arg::new("to").long("to").conflicts_with("unpin"))
    .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue))
}

pub(in crate::grammar) fn uninstall() -> Command {
    general(spec!(
        "uninstall",
        "Remove TMT from this machine; your data is kept unless --purge",
        [
            "Review and remove TMT" => "tmt uninstall",
            "Also delete identities, messages and notes" => "tmt uninstall --purge",
        ]
    ))
    .arg(
        Arg::new("purge")
            .long("purge")
            .help("Also delete TMT's data directory")
            .action(ArgAction::SetTrue),
    )
    .arg(
        Arg::new("yes")
            .long("yes")
            .help("Approve without a prompt")
            .action(ArgAction::SetTrue),
    )
    .arg(
        Arg::new("prefix")
            .long("prefix")
            .help("The installation prefix (default: this installation's)"),
    )
}

pub(in crate::grammar) fn refresh_skills() -> Command {
    internal("__native-refresh-skills", "Internal managed skill refresh").hide(true)
}

pub(in crate::grammar) fn native_install() -> Command {
    internal("__native-install", "Internal offline native installation")
        .hide(true)
        .arg(
            Arg::new("product")
                .long("product")
                .default_value("cli")
                .value_parser(
                    tmt_core::native_install::Product::ALL.map(|product| product.as_str()),
                ),
        )
        .arg(Arg::new("archive").long("archive").required(true))
        .arg(Arg::new("manifest").long("manifest").required(true))
        .arg(Arg::new("prefix").long("prefix").required(true))
        .arg(
            Arg::new("channel")
                .long("channel")
                .required(true)
                .value_parser(
                    tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
                ),
        )
        .arg(
            Arg::new("pin")
                .long("pin")
                .action(ArgAction::SetTrue)
                .conflicts_with("unpin"),
        )
        .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue))
}

pub(in crate::grammar) fn learn() -> Command {
    with_options(
        general(spec!(
            "learn",
            "Read agent guidance",
            [
                "Read the tmux-team guidance" => "tmt learn",
                "Print a bundled skill" => "tmt learn --skill tmt-inbox",
            ]
        )),
        &["skill"],
    )
}
