//! The #436 help guard for `tmt squad`: every visible command follows
//! `docs/cli-style.md` and its examples parse through Squad's grammar, except
//! the commands still listed in `cli_style_allowlist.rs`.

use tmt_cli_style::audit::{self, Probe};

#[path = "cli_style_allowlist.rs"]
mod allowlist;

fn matches(words: &[String]) -> Result<clap::ArgMatches, clap::Error> {
    super::grammar()
        .try_get_matches_from(std::iter::once("tmt-squad".to_owned()).chain(words.iter().cloned()))
}

fn help(words: &[String]) -> Result<String, String> {
    match matches(words) {
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => Ok(error.to_string()),
        Err(error) => Err(error.kind().to_string()),
        Ok(_) => Err("not a help request".into()),
    }
}

fn parse(words: &[String]) -> Result<(), String> {
    matches(words)
        .map(drop)
        .map_err(|error| error.kind().to_string())
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
