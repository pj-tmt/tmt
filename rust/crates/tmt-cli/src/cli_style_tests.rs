//! The #436 help guard for `tmt`, Office's facade tree included: every visible
//! command follows `design/cli-style.md` and its examples parse through the real
//! parser, except the commands still listed in `cli_style_allowlist.rs`.

use crate::{
    extension_command::Discovered,
    help_output,
    invocation::{ConfigRequest, Invocation},
    parser,
};
use std::ffi::OsString;
use tmt_cli_style::audit::{self, Probe};

#[test]
fn every_listing_command_uses_ls_with_the_list_alias() {
    let report = audit::list_spelling_report(&crate::grammar::grammar(), &["tmt"]);
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[test]
fn hidden_commands_are_listed_with_a_reason_and_double_underscore_commands_are_hidden() {
    let report = audit::hidden_report(&crate::grammar::grammar(), &["tmt"], allowlist::HIDDEN);
    assert!(report.is_empty(), "{}", report.join("\n"));
}

fn option_description_gaps(command: &clap::Command, path: &str) -> Vec<String> {
    let mut gaps = Vec::new();
    for option in command.get_arguments().filter(|arg| !arg.is_positional()) {
        let description = option
            .get_help()
            .map(ToString::to_string)
            .unwrap_or_default();
        if description.trim().is_empty() || description.contains(['\n', '\r']) {
            gaps.push(format!(
                "{path}: {} needs a one-line description",
                option.get_id()
            ));
        }
    }
    for child in command.get_subcommands() {
        // Office is mounted by core but its grammar belongs to the extension.
        if path == "tmt" && child.get_name() == "office" {
            continue;
        }
        gaps.extend(option_description_gaps(
            child,
            &format!("{path} {}", child.get_name()),
        ));
    }
    gaps
}

#[test]
fn every_public_core_option_has_a_one_line_description() {
    let public = crate::grammar::public_grammar(&crate::grammar::grammar(), true);
    let gaps = option_description_gaps(&public, "tmt");
    assert!(gaps.is_empty(), "{}", gaps.join("\n"));
}

#[test]
fn option_description_guard_checks_nested_core_options_only() {
    use clap::{Arg, Command};
    let grammar = Command::new("tmt")
        .subcommand(
            Command::new("core").subcommand(
                Command::new("nested")
                    .arg(Arg::new("operand"))
                    .arg(Arg::new("good").long("good").help("One line"))
                    .arg(Arg::new("missing").short('m'))
                    .arg(Arg::new("blank").long("blank").help("  "))
                    .arg(Arg::new("multiline").long("multiline").help("Two\nlines"))
                    .arg(Arg::new("hidden").long("hidden").hide(true)),
            ),
        )
        .subcommand(Command::new("office").arg(Arg::new("external").long("external")))
        .subcommand(
            Command::new("internal")
                .hide(true)
                .arg(Arg::new("private").long("private")),
        );
    let public = crate::grammar::public_grammar(&grammar, true);
    assert_eq!(
        option_description_gaps(&public, "tmt"),
        [
            "tmt core nested: missing needs a one-line description",
            "tmt core nested: blank needs a one-line description",
            "tmt core nested: multiline needs a one-line description",
        ]
    );
}

#[path = "cli_style_allowlist.rs"]
mod allowlist;

fn arguments(words: &[String]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

fn help(words: &[String]) -> Result<String, String> {
    help_for_terminal(words, tmt_cli_style::Terminal::PLAIN)
}

fn help_for_terminal(
    words: &[String],
    terminal: tmt_cli_style::Terminal,
) -> Result<String, String> {
    let parsed = parser::parse(&arguments(words)).map_err(|error| error.message)?;
    let Invocation::Help(path) = parsed.invocation else {
        return Err("not a help request".into());
    };
    let mut text = Vec::new();
    // No `PATH` discovery: the walk must not depend on what the machine has installed.
    help_output::write(&path, &Discovered::default(), terminal, &mut text)
        .map_err(|error| error.to_string())?;
    String::from_utf8(text).map_err(|error| error.to_string())
}

#[test]
fn core_help_forms_share_terminal_wrapping_and_intact_examples() {
    let terminal = tmt_cli_style::Terminal {
        width: Some(80),
        ..tmt_cli_style::Terminal::PLAIN
    };
    for path in [vec![], vec!["completion".to_owned()]] {
        let mut dash_h = path.clone();
        dash_h.push("-h".to_owned());
        let mut long = path.clone();
        long.push("--help".to_owned());
        let mut routed = vec!["help".to_owned()];
        routed.extend(path);
        let plain = help(&dash_h).unwrap();
        let wrapped = help_for_terminal(&dash_h, terminal).unwrap();
        assert_eq!(help_for_terminal(&long, terminal).unwrap(), wrapped);
        assert_eq!(help_for_terminal(&routed, terminal).unwrap(), wrapped);
        assert_eq!(
            tmt_cli_style::examples(&wrapped),
            tmt_cli_style::examples(&plain)
        );
        assert!(
            wrapped
                .split("Examples:")
                .next()
                .unwrap()
                .lines()
                .all(|line| line.is_ascii() && line.len() <= 80)
        );
    }
}

fn parse(words: &[String]) -> Result<(), String> {
    let parsed = parser::parse_core(&arguments(words)).map_err(|error| error.message)?;
    match parsed.invocation {
        Invocation::Config(ConfigRequest::Set { key, value, global }) => {
            let scope = if global {
                tmt_core::settings::Scope::Global
            } else {
                tmt_core::settings::Scope::Local
            };
            tmt_core::settings::Setting::edit(&key, &value, scope).map(drop)
        }
        Invocation::Config(ConfigRequest::Clear { key }) => {
            tmt_core::settings::LocalClear::parse(key.as_deref()).map(drop)
        }
        _ => Ok(()),
    }
}

#[test]
fn every_command_follows_the_help_style_or_is_still_migrating() {
    let program = ["tmt"];
    let violations = audit::walk(
        &crate::grammar::grammar(),
        &Probe {
            program: &program,
            help: &help,
            parse: &parse,
        },
    );
    let report = audit::allowlist_report(&violations, &program, allowlist::MIGRATING);
    assert!(
        report.is_empty(),
        "Help style allowlist differs from the grammar walk:\n{}\n\n{violations:#?}",
        report.join("\n")
    );
}

/// #1079: executable examples cannot be waived by a presentation migration.
#[test]
fn every_printed_hint_and_help_example_is_runnable() {
    let program = ["tmt"];
    let violations = audit::walk(
        &crate::grammar::grammar(),
        &Probe {
            program: &program,
            help: &help,
            parse: &parse,
        },
    );
    let mut failures: Vec<String> = violations
        .iter()
        .filter(|violation| violation.rule == audit::Rule::ExampleParse)
        .map(|violation| format!("{}: {}", violation.command(&program), violation.detail))
        .collect();
    for command in crate::context_command::hint_commands()
        .into_iter()
        .chain(crate::channel_command::hint_commands())
    {
        let example = tmt_cli_style::help::ShownExample {
            note: String::new(),
            command,
        };
        let result = example.argv().and_then(|argv| parse(&argv[1..]));
        if let Err(error) = result {
            failures.push(format!("rendered hint {:?}: {error}", example.command));
        }
    }
    assert!(
        failures.is_empty(),
        "Printed command guard:\n{}",
        failures.join("\n")
    );
}

/// Extension changes are consent-gated and a non-interactive run refuses without
/// `--yes`, so each shown example of one must carry it (#1569).
#[test]
fn extension_change_examples_carry_the_consent_flag() {
    for subcommand in ["install", "upgrade", "rm"] {
        let text = help(&["help".into(), "extension".into(), subcommand.into()]).unwrap();
        let examples: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with(&format!("tmt extension {subcommand} ")))
            .collect();
        assert!(!examples.is_empty(), "{subcommand} shows no examples");
        for example in examples {
            assert!(example.contains(" --yes"), "{example:?} would be refused");
        }
    }
}

#[test]
fn printed_command_validation_rejects_the_reported_regressions() {
    for command in [
        "tmt x --incoming --identity worker --json",
        "tmt config set timeout 120",
        "tmt config set --global captureLines 200",
        "tmt config set unknownKey 1",
        "tmt inbox --unknown-flag",
        "tmt config unknown-subcommand",
    ] {
        let argv = tmt_cli_style::Example {
            command,
            note: "negative control",
        }
        .argv()
        .unwrap();
        assert!(parse(&argv[1..]).is_err(), "accepted {command:?}");
    }
    for command in [
        "tmt inbox --identity worker --json",
        "tmt config set preambleEvery 2",
        "tmt config set --global pasteEnterDelayMs 300",
        "tmt config rm preambleEvery",
    ] {
        let argv = tmt_cli_style::Example {
            command,
            note: "positive control",
        }
        .argv()
        .unwrap();
        parse(&argv[1..]).unwrap_or_else(|error| panic!("{command:?}: {error}"));
    }
}

mod hints;
pub(crate) use hints::HintSpec;

fn hint_samples() -> Vec<(&'static str, &'static [HintSpec])> {
    vec![
        ("answer_command.rs", crate::answer_command::PRINTED_HINTS),
        ("appearance.rs", crate::appearance::PRINTED_HINTS),
        ("binding_command.rs", crate::binding_command::PRINTED_HINTS),
        (
            "binding_command/presentation.rs",
            crate::binding_command::PRESENTATION_HINTS,
        ),
        ("channel_command.rs", crate::channel_command::PRINTED_HINTS),
        ("config_command.rs", crate::config_command::PRINTED_HINTS),
        (
            "context_command/presentation.rs",
            crate::context_command::PRESENTATION_HINTS,
        ),
        ("driver_command.rs", crate::driver_command::PRINTED_HINTS),
        (
            "exchange_command/listen.rs",
            crate::exchange_command::LISTEN_HINTS,
        ),
        (
            "exchange_command/presentation.rs",
            crate::exchange_command::PRESENTATION_HINTS,
        ),
        (
            "extension_command.rs",
            crate::extension_command::PRINTED_HINTS,
        ),
        (
            "extension_install_command.rs",
            crate::extension_install_command::PRINTED_HINTS,
        ),
        (
            "extension_install_command/list_upgrade.rs",
            crate::extension_install_command::LIST_UPGRADE_HINTS,
        ),
        (
            "extension_install_command/skills.rs",
            crate::extension_install_command::SKILLS_HINTS,
        ),
        (
            "extension_install_command/upgrade_all.rs",
            crate::extension_install_command::UPGRADE_ALL_HINTS,
        ),
        (
            "guidance_command.rs",
            crate::guidance_command::PRINTED_HINTS,
        ),
        (
            "identity_command.rs",
            crate::identity_command::PRINTED_HINTS,
        ),
        ("install_command.rs", crate::install_command::PRINTED_HINTS),
        ("main.rs", crate::PRINTED_HINTS),
        (
            "native_upgrade_command.rs",
            crate::native_upgrade_command::PRINTED_HINTS,
        ),
        (
            "native_upgrade_command/extensions.rs",
            crate::native_upgrade_command::EXTENSIONS_HINTS,
        ),
        (
            "native_upgrade_command/rename.rs",
            crate::native_upgrade_command::RENAME_HINTS,
        ),
        ("parser.rs", crate::parser::PRINTED_HINTS),
        (
            "reply_notice_command.rs",
            crate::reply_notice_command::PRINTED_HINTS,
        ),
        ("room_command.rs", crate::room_command::PRINTED_HINTS),
        (
            "room_command/dispatch.rs",
            crate::room_command::DISPATCH_HINTS,
        ),
        ("run_command/resume.rs", crate::run_command::RESUME_HINTS),
        ("run_command/run.rs", crate::run_command::RUN_HINTS),
        ("setup_command.rs", crate::setup_command::PRINTED_HINTS),
        (
            "setup_command/guided.rs",
            crate::setup_command::GUIDED_HINTS,
        ),
        (
            "skill_refresh_command.rs",
            crate::skill_refresh_command::PRINTED_HINTS,
        ),
        ("skill_reminder.rs", crate::skill_reminder::PRINTED_HINTS),
        ("talk_command.rs", crate::talk_command::PRINTED_HINTS),
        (
            "talk_command/preparation.rs",
            crate::talk_command::PREPARATION_HINTS,
        ),
        (
            "talk_command/presentation.rs",
            crate::talk_command::PRESENTATION_HINTS,
        ),
        (
            "uninstall_command.rs",
            crate::uninstall_command::PRINTED_HINTS,
        ),
    ]
}

#[test]
fn printed_hint_templates_have_source_coverage() {
    use std::collections::BTreeSet;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let samples = hint_samples();
    let mut paths = Vec::new();
    hints::source_files(&root, &mut paths);
    let mut failures = Vec::new();
    for path in paths {
        let relative = path.strip_prefix(&root).unwrap().to_str().unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        let templates = hints::literals(&source);
        let registered = samples
            .iter()
            .find(|(owner, _)| *owner == relative)
            .map(|(_, specs)| *specs)
            .unwrap_or(&[]);
        let expected: BTreeSet<String> = registered
            .iter()
            .map(|spec| spec.template.to_owned())
            .collect();
        for missing in templates.difference(&expected) {
            failures.push(format!("{relative}: uncollected {missing:?}"));
        }
        for stale in expected.difference(&templates) {
            failures.push(format!("{relative}: stale sample {stale:?}"));
        }
        for spec in registered {
            match hints::commands(spec) {
                Err(error) => failures.push(format!("{relative}: {error}")),
                Ok(commands) => {
                    for command in commands {
                        let example = tmt_cli_style::help::ShownExample {
                            note: String::new(),
                            command,
                        };
                        if let Err(error) = example.argv().and_then(|argv| parse(&argv[1..])) {
                            failures.push(format!("{relative}: {:?}: {error}", example.command));
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Printed hint source guard:\n{}",
        failures.join("\n")
    );
}
