//! The #436 help guard for `tmt`, Office's facade tree included: every visible
//! command follows `docs/cli-style.md` and its examples parse through the real
//! parser, except the commands still listed in `cli_style_allowlist.rs`.

use crate::{help_output, invocation::Invocation, parser};
use std::ffi::OsString;
use tmt_cli_style::audit::{self, Probe};

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
    help_output::write(&path, &mut text).map_err(|error| error.to_string())?;
    String::from_utf8(text).map_err(|error| error.to_string())
}

fn parse(words: &[String]) -> Result<(), String> {
    parser::parse(&arguments(words))
        .map(drop)
        .map_err(|error| error.message)
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
