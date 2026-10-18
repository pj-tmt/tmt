//! Member settings grammar, admitted before Core discovery or file effects.

use clap::{Arg, ArgAction, ArgGroup, Command};
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
    details: "Use a duration, auto or off to save a member override. Use default to remove it and inherit the global default. Global default and flush count (flushCount) are configured in digest.toml. Run tmt digest tick once a minute to apply settings and deliver due digests; Core still checks that each member is ready to receive them.",
};

const TICK: CommandSpec = CommandSpec {
    name: "tick",
    summary: "Apply digest settings and deliver due digests during this minute",
    examples: &[Example {
        command: "tmt digest tick",
        note: "Run once a minute from an Ops squad cron, cron or launchd",
    }],
    outputs: OutputModes::Human,
    details: "If another tick is already running, this one exits at once. Each run stays up to one minute to deliver digests as they fall due, then exits. With an interval, held messages are delivered once the oldest has waited that long, or sooner when the flush count is reached. auto and off currently deliver messages normally.",
};

const SKILL: CommandSpec = CommandSpec {
    name: "skill",
    summary: "Print the bundled Digest skill",
    examples: &[Example {
        command: "tmt digest skill",
        note: "Read the bundled instructions",
    }],
    outputs: OutputModes::Human,
    details: "Prints the exact bundled skill without Core discovery or storage access.",
};

const STATUS: CommandSpec = CommandSpec {
    name: "status",
    summary: "Read digest settings, held messages and labels for all members",
    examples: &[Example {
        command: "tmt digest status --json",
        note: "Read one status document without changing settings or delivering messages",
    }],
    outputs: OutputModes::Json,
    details: "Reads Core identities and held-message stats in batches. Unavailable stats stay unknown. Savings estimate context inputs avoided since Digest collection began, never billed tokens. This command does not tick or deliver messages.",
};

const DECISIONS: CommandSpec = CommandSpec {
    name: "decisions",
    summary: "List the delivery decisions each tick recorded, or report what the digests saved",
    examples: &[
        Example {
            command: "tmt digest decisions --json",
            note: "List the last 24 hours of recorded decisions as one JSON document",
        },
        Example {
            command: "tmt digest decisions --report --json",
            note: "Turns saved and delay against delivering every message on its own",
        },
        Example {
            command: "tmt digest decisions --report --recompute 20m --json",
            note: "Also estimate what a 20m interval would have done",
        },
    ],
    outputs: OutputModes::Json,
    details: "Each row names the rule that decided a member (interval_deadline, flush_count, auto_idle_point, off or not_due), the held count and oldest held age it saw, and Core's answer to the delivery opportunity. Rows hold counts and times only, never message text. A row is written when a member's rule, answer or oldest held message changes, and holds until that member's next row. With --report, every member and the total show the messages delivered, the deliveries that took them (the baseline is one delivery per message), the turns saved, and the delay from arrival to delivery; --recompute INTERVAL adds the same figures for that interval, estimated from arrival times only, so its delay is a lower bound and it compares rules rather than predicting them. This command does not tick or deliver messages.",
};

const USAGE: CommandSpec = CommandSpec {
    name: "usage",
    summary: "Report token usage, account limits and message flow",
    examples: &[Example {
        command: "tmt digest usage report --json",
        note: "Read the last 24 hours as one JSON document",
    }],
    outputs: OutputModes::Human,
    details: "Reads Core only; Digest keeps no usage store.",
};

const REPORT: CommandSpec = CommandSpec {
    name: "report",
    summary: "Report token usage, account limits and message flow for all members",
    examples: &[
        Example {
            command: "tmt digest usage report --json",
            note: "Read the last 24 hours as one JSON document",
        },
        Example {
            command: "tmt digest usage report --since 7d --csv usage",
            note: "Export the week's token usage per member and model as CSV",
        },
    ],
    outputs: OutputModes::Json,
    details: "Windows end at the last completed 5-minute boundary (window.toMs). Missing evidence is reported as unavailable, never as zero; Claude account limits are unavailable because Core has no source for them. CSV rows equal the JSON rows; a text cell starting with =, +, -, @, tab or carriage return gets a leading ' so a spreadsheet never reads it as a formula.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt digest")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_negates_reqs(true)
        .subcommand(tmt_cli_style::command(&TICK))
        .subcommand(tmt_cli_style::command(&SKILL))
        .subcommand(
            tmt_cli_style::command(&USAGE)
                .subcommand_required(true)
                .subcommand(
                    tmt_cli_style::command(&REPORT)
                        .arg(
                            Arg::new("since")
                                .long("since")
                                .default_value("24h")
                                .value_parser(["24h", "7d", "30d"])
                                .help("Window ending at the last completed 5-minute boundary"),
                        )
                        .arg(
                            Arg::new("json")
                                .long("json")
                                .action(ArgAction::SetTrue)
                                .help("Output one JSON document"),
                        )
                        .arg(
                            Arg::new("csv")
                                .long("csv")
                                .value_name("TABLE")
                                .value_parser(["usage", "credits", "exchange"])
                                .help("Output one CSV table: usage, credits or exchange"),
                        )
                        .group(ArgGroup::new("format").args(["json", "csv"]).required(true)),
                ),
        )
        .subcommand(
            tmt_cli_style::command(&DECISIONS)
                .arg(
                    Arg::new("since")
                        .long("since")
                        .default_value("24h")
                        .value_parser(["24h", "7d", "30d"])
                        .help("Window ending now"),
                )
                .arg(
                    Arg::new("report")
                        .long("report")
                        .action(ArgAction::SetTrue)
                        .help("Report the turns the delivered digests saved, instead of listing decisions"),
                )
                .arg(
                    Arg::new("recompute")
                        .long("recompute")
                        .value_name("INTERVAL")
                        .requires("report")
                        .help("Also estimate the same arrivals under another interval, such as 20m")
                        .value_parser(|text: &str| match crate::settings::Mode::parse(text) {
                            Ok(crate::settings::Mode::Interval { .. }) => Ok(text.to_owned()),
                            Ok(_) => Err("auto and off cannot be recomputed; give a duration such as 20m".to_owned()),
                            Err(error) => Err(error.message),
                        }),
                )
                .arg(
                    Arg::new("flush-count")
                        .long("flush-count")
                        .value_name("N")
                        .requires("recompute")
                        .help("Flush count for the recomputed interval (default: the configured one)")
                        .value_parser(clap::value_parser!(u64).range(1..=1_000_000)),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .required(true)
                        .action(ArgAction::SetTrue)
                        .help("Output one JSON document"),
                ),
        )
        .subcommand(
            tmt_cli_style::command(&STATUS)
                .arg(
                    Arg::new("window")
                        .long("window")
                        .default_value("24h")
                        .value_parser(["24h", "7d", "lifetime"])
                        .help("Savings window ending at the collected evidence watermark"),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .required(true)
                        .action(ArgAction::SetTrue)
                        .help("Output one JSON document"),
                ),
        )
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
