//! The #436 help guard for `tmt`, Office's facade tree included: every visible
//! command follows `design/cli-style.md` and its examples parse through the real
//! parser, except the commands still listed in `cli_style_allowlist.rs`.

use crate::{extension_command::Discovered, help_output, invocation::Invocation, parser};
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
