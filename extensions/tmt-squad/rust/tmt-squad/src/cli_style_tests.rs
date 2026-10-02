//! The #436 help guard for `tmt squad`: every visible command follows
//! `design/cli-style.md` and its examples parse through Squad's grammar, except
//! the commands still listed in `cli_style_allowlist.rs`.

use tmt_cli_style::{
    Terminal,
    audit::{self, Probe},
    help_text,
};

#[path = "cli_style_allowlist.rs"]
mod allowlist;

#[test]
fn every_listing_command_uses_ls_with_the_list_alias() {
    let report = audit::list_spelling_report(&super::grammar(), &["tmt", "squad"]);
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[test]
fn concise_commands_and_aliases_reach_the_same_squad_dispatch() {
    let cases: &[(&[&str], &[&str], &[&str])] = &[
        (&["remove"], &["rm"], &["worker", "--squad", "product"]),
        (&["list"], &["ls"], &["--refresh-fields"]),
        (&["status"], &["ls"], &[]),
        (&["hotkeys", "remove"], &["hotkeys", "rm"], &["--yes"]),
        (&["playbook", "list"], &["playbook", "ls"], &[]),
        (
            &["playbook", "remove"],
            &["playbook", "rm"],
            &["tmux-squad", "--yes"],
        ),
    ];
    for &(old, primary, operands) in cases {
        for json in [false, true] {
            let words = |path: &[&str]| {
                path.iter()
                    .chain(operands)
                    .copied()
                    .chain(json.then_some("--json"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            let run = |path: &[&str]| match request(&words(path)).unwrap() {
                super::Request::Run(matches) => matches,
                super::Request::Help(_) => panic!("expected dispatch"),
            };
            assert_eq!(run(old), run(primary), "{old:?}");
        }
        let shown = |path: &[&str]| {
            help(
                &std::iter::once("help")
                    .chain(path.iter().copied())
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let text = shown(primary);
        assert_eq!(shown(old), text);
        assert!(!text.contains(&format!("tmt squad {}", old.join(" "))));
        assert!(text.contains(&format!("Usage: tmt squad {}", primary.join(" "))));
    }
}

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
