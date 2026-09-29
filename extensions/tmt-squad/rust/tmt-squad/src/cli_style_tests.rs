//! The #436 help guard for `tmt squad`: every visible command follows
//! `docs/cli-style.md` and its examples parse through Squad's grammar, except
//! the commands still listed in `cli_style_allowlist.rs`.

use tmt_cli_style::{
    Terminal,
    audit::{self, Probe},
    help_text,
};

#[path = "cli_style_allowlist.rs"]
mod allowlist;

fn request(words: &[String]) -> Result<super::Request, clap::Error> {
    let argv: Vec<std::ffi::OsString> = std::iter::once("tmt-squad".into())
        .chain(words.iter().map(Into::into))
        .collect();
    super::request(&argv)
}

/// The text `-h`, `--help` and `help <command>` print, through the same
/// `request` that `main` dispatches.
fn help(words: &[String]) -> Result<String, String> {
    match request(words) {
        Ok(super::Request::Help(command)) => Ok(help_text(&command, Terminal::PLAIN)),
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => Ok(error.to_string()),
        Err(error) => Err(error.kind().to_string()),
        Ok(super::Request::Run(_)) => Err("not a help request".into()),
    }
}

fn parse(words: &[String]) -> Result<(), String> {
    match request(words) {
        Ok(super::Request::Run(_)) => Ok(()),
        Ok(super::Request::Help(_)) => Err("a help request".into()),
        Err(error) => Err(error.kind().to_string()),
    }
}

#[test]
fn every_command_follows_the_help_style_or_is_still_migrating() {
    let program = ["tmt", "squad"];
    let violations = audit::walk(
        &super::grammar(),
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
