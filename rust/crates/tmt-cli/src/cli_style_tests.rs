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
