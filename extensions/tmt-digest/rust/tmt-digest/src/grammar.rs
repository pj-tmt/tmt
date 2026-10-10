//! Member settings grammar, admitted before Core discovery or file effects.

use clap::{Arg, ArgAction, Command};
use tmt_cli_style::{CommandSpec, Example, OutputModes};

const ROOT: CommandSpec = CommandSpec {
    name: "digest",
    summary: "Set a member's digest mode",
    examples: &[
        Example {
            command: "tmt digest worker 5m",
            note: "Save an interval for a member",
        },
        Example {
            command: "tmt digest worker off",
            note: "Turn off the member's digest mode",
        },
        Example {
            command: "tmt digest worker default",
            note: "Remove the member setting and inherit the global default",
        },
    ],
    outputs: OutputModes::Human,
    details: "Use a duration, auto or off to save a member override. Use default to remove it and inherit the global default. Global default and flush count (flushCount) are configured in digest.toml. Run tick once a minute to apply settings and deliver due digests; Core checks whether each member can receive them.",
};

const TICK: CommandSpec = CommandSpec {
    name: "tick",
    summary: "Apply digest settings and deliver due digests during this minute",
    examples: &[Example {
        command: "tmt digest tick",
        note: "Run once a minute from Ops, cron or launchd",
    }],
    outputs: OutputModes::Human,
    details: "A concurrent tick exits immediately. This run sleeps to deadlines within the minute, then exits. Interval messages become due after the oldest held message waits its interval, or at the flush count. Auto and off use normal Core delivery in this phase.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt digest")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_negates_reqs(true)
        .subcommand(tmt_cli_style::command(&TICK))
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
        .arg(
            Arg::new("member")
                .value_name("MEMBER")
                .required(true)
                .requires("value")
                .help("Saved identity name or UUID"),
        )
        .arg(
            Arg::new("value")
                .value_name("DURATION|auto|off|default")
                .required(true)
                .requires("member")
                .help("Duration such as 30s, 5m or 1h, or auto, off or default")
                .value_parser(|text: &str| {
                    crate::settings::MemberSetting::parse(text)
                        .map(|_| text.to_owned())
                        .map_err(|error| error.message)
                }),
        )
}
