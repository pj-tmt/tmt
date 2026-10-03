//! Skill, hook and managed-product installation command grammar.

use crate::grammar::{
    channel_option, extension_target, general, internal, operand, option, with_options,
};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn extension() -> Command {
    general(spec!(
        "extension",
        "Manage consented extension integrations",
        [
            "List official extensions" => "tmt extension ls",
            "Install Squad" => "tmt extension install squad",
        ]
    ))
    .subcommand_required(true)
    .subcommand(
        general(spec!(
            "hooks",
            "Manage lifecycle hooks for trusted extensions",
            [
                "List extensions with hooks enabled" => "tmt extension hooks ls",
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
        .subcommand(
            general(spec!(
                "ls",
                "List extensions with enabled hooks",
                [
                    "List extensions with hooks enabled" => "tmt extension hooks ls",
                ]
            ))
            .alias("list"),
        ),
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
        .arg(
            Arg::new("archive")
                .long("archive")
                .requires("manifest")
                .help("Install from a local release archive; requires --manifest"),
        )
        .arg(
            Arg::new("manifest")
                .long("manifest")
                .requires("archive")
                .help("Verify the local archive with this release manifest; requires --archive"),
        )
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
        .arg(option("to"))
        .arg(option("unpin")),
    )
    .subcommand(extension_target(
        general(spec!(
            "rm",
            "Remove an extension's commands; releases and data are kept",
            [
                "Remove Squad's commands" => "tmt extension rm squad",
            ]
        ))
        .alias("uninstall"),
    ))
    .subcommand(
        general(spec!(
            "ls",
            "List official extensions, versions and PATH shadowing",
            [
                "List official extensions" => "tmt extension ls",
                "Also check for newer releases" => "tmt extension ls --check",
            ]
        ))
        .alias("list")
        .arg(option("prefix"))
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
            .help("Remove the selected provider's unchanged TMT session hooks")
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
    .arg(option("yes"))
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
        "Upgrade TMT, managed skills and installed official extensions",
        [
            "Update to the latest release on your channel" => "tmt upgrade",
            "Switch to the stable channel" => "tmt upgrade --channel stable",
            "Install an exact version" => "tmt upgrade --to 5.0.0-alpha.8",
        ]
    ))
    .visible_alias("update")
    .arg(option("yes"))
    .arg(channel_option())
    .arg(option("to"))
    .arg(option("unpin"))
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
    .arg(option("yes"))
    .arg(
        Arg::new("prefix")
            .long("prefix")
            .help("The installation prefix (default: this installation's)"),
    )
}

pub(in crate::grammar) fn refresh_skills() -> Command {
    internal("__native-refresh-skills", "Internal managed skill refresh").hide(true)
}

pub(in crate::grammar) fn upgrade_extensions() -> Command {
    internal(
        "__native-upgrade-extensions",
        "Internal extension upgrade plan/apply",
    )
    .hide(true)
    .arg(
        Arg::new("plan")
            .long("plan")
            .action(ArgAction::SetTrue)
            .conflicts_with("yes")
            .required_unless_present("yes"),
    )
    .arg(
        Arg::new("yes")
            .long("yes")
            .action(ArgAction::SetTrue)
            .required_unless_present("plan"),
    )
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

/// Consented host drivers (#570): terminal hosts TMT doesn't build in.
pub(in crate::grammar) fn driver() -> Command {
    general(spec!(
        "driver",
        "Manage consented drivers",
        [
            "List approved drivers" => "tmt driver ls",
            "Approve the shipped Herdr driver" => "tmt driver install herdr",
        ]
    ))
    .subcommand_required(true)
    .subcommand(
        general(spec!(
            "install",
            "Approve a driver after showing what it declares",
            [
                "Approve the shipped Herdr driver" => "tmt driver install herdr",
                "Approve a driver executable" => "tmt driver install ./tmt-driver-screen",
                "Approve without a prompt" => "tmt driver install herdr --yes",
            ]
        ))
        .arg(operand("path", true))
        .arg(option("yes")),
    )
    .subcommand(
        general(spec!(
            "ls",
            "List approved drivers and whether each still runs as approved",
            [
                "List approved drivers" => "tmt driver ls",
            ]
        ))
        .alias("list"),
    )
    .subcommand(
        general(spec!(
            "rm",
            "Withdraw approval of a host driver",
            [
                "Remove the Herdr driver" => "tmt driver rm herdr",
            ]
        ))
        .arg(operand("name", true)),
    )
}
