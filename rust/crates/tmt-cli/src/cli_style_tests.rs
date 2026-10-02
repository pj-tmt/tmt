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

#[path = "cli_style_allowlist.rs"]
mod allowlist;

fn arguments(words: &[String]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

fn help(words: &[String]) -> Result<String, String> {
    let parsed = parser::parse(&arguments(words)).map_err(|error| error.message)?;
    let Invocation::Help(path) = parsed.invocation else {
        return Err("not a help request".into());
    };
    let mut text = Vec::new();
    // No `PATH` discovery: the walk must not depend on what the machine has installed.
    help_output::write(
        &path,
        &Discovered::default(),
        tmt_cli_style::Terminal::PLAIN,
        &mut text,
    )
    .map_err(|error| error.to_string())?;
    String::from_utf8(text).map_err(|error| error.to_string())
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
    for command in crate::context_command::hint_commands() {
        let example = tmt_cli_style::help::ShownExample {
            note: String::new(),
            command,
        };
        let result = example.argv().and_then(|argv| parse(&argv[1..]));
        if let Err(error) = result {
            failures.push(format!("context hint {:?}: {error}", example.command));
        }
    }
    assert!(
        failures.is_empty(),
        "Printed command guard:\n{}",
        failures.join("\n")
    );
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
        "tmt config set --global pasteEnterDelayMs 500",
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
        ("run_command/channel.rs", crate::run_command::CHANNEL_HINTS),
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
