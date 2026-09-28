//! `tmt squad` (alias `tmt sq`): an optional extension reached through TMT's
//! external command dispatch. It keeps no state of its own.

mod board;
mod config;
mod core;
mod filter;
mod membership;
mod runner;
mod squad;
mod status;

use crate::{config::Config, core::Core, core::SquadError, membership::Outcome, squad::Squad};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{IsTerminal, Write},
    process::ExitCode,
};

const SKILL: &str = include_str!("../../../skills/tmt-squad/SKILL.md");

fn squad_option() -> Arg {
    Arg::new("squad")
        .long("squad")
        .value_name("NAME")
        .help("Select a squad; optional when exactly one exists")
}

/// The name is fixed, never argv[0]: `tmt-squad` and its `tmt-sq` link print
/// byte-identical help, errors and completion.
fn grammar() -> Command {
    let operand = |name: &'static str, help: &'static str| Arg::new(name).required(true).help(help);
    Command::new("squad")
        .bin_name("tmt squad")
        .about("Leads, members and one board for a team of agents (alias: tmt sq)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .disable_help_subcommand(true)
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print one JSON document"),
        )
        .subcommand(
            Command::new("init")
                .about("Create the squad room squad-<name>; record which saved identity is you")
                .arg(operand(
                    "name",
                    "Squad name: [a-z][a-z0-9-], up to 24 characters",
                ))
                .arg(
                    Arg::new("me")
                        .long("me")
                        .value_name("NAME")
                        .help("Your saved identity (asked interactively when unset)"),
                ),
        )
        .subcommand(
            Command::new("lead")
                .about("Make a saved identity the squad's lead")
                .arg(operand("name", "Saved identity to lead"))
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("add")
                .about("Add running agents to the squad")
                .arg(operand("names", "Identities to add").num_args(1..))
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("remove")
                .about("Remove a member and clear its squad fields; the agent keeps running")
                .arg(operand("name", "Member to remove"))
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("set")
                .about(
                    "Set member fields such as state, task, pending, note or links (field= clears)",
                )
                .arg(operand("member", "Member to update"))
                .arg(operand("fields", "field=value pairs").num_args(1..))
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("status")
                .about("Show the squad as text, or JSON with --json")
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("board")
                .about("Open the terminal board (prints status without a terminal)")
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("skill")
                .about("The tmt-squad skill for lead agents")
                .subcommand_required(true)
                .subcommand(Command::new("show").about("Print the skill")),
        )
        .subcommand(
            Command::new("__complete").hide(true).arg(
                Arg::new("words")
                    .num_args(0..)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true),
            ),
        )
}

/// Completion v1: literal candidates for the unfinished word; empty output
/// lets the shell fall back to file completion.
fn complete(words: &[String]) -> Vec<String> {
    let words = words.strip_prefix(&["--".to_owned()]).unwrap_or(words);
    let (current, before) = words
        .split_last()
        .map_or(("", &[][..]), |(last, rest)| (last.as_str(), rest));
    let root = grammar();
    let command = match before {
        [] => &root,
        [name, ..] => match root.find_subcommand(name) {
            Some(command) if before.len() == 1 || name == "skill" => command,
            _ => return Vec::new(),
        },
    };
    let mut candidates: Vec<String> = if current.starts_with('-') {
        command
            .get_arguments()
            .filter_map(|arg| arg.get_long().map(|long| format!("--{long}")))
            .chain(["--json".to_owned()])
            .collect()
    } else {
        command
            .get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .map(|sub| sub.get_name().to_owned())
            .collect()
    };
    candidates.retain(|candidate| candidate.starts_with(current));
    candidates.sort();
    candidates.dedup();
    candidates
}

fn human(command: &str, document: &Value) -> String {
    match command {
        "status" | "board" => status::text(document),
        "init" => format!(
            "Squad {} {} (room squad-{}); you are {}.\n",
            document["squad"]["name"].as_str().unwrap_or_default(),
            if document["created"] == true {
                "created"
            } else {
                "already exists"
            },
            document["squad"]["name"].as_str().unwrap_or_default(),
            document["me"].as_str().unwrap_or("unset"),
        ),
        _ => format!(
            "{}\n",
            serde_json::to_string_pretty(document).unwrap_or_default()
        ),
    }
}

fn run(command: &str, matches: &ArgMatches) -> Result<Outcome, SquadError> {
    let core = Core::discover()?;
    let text = |name: &str| matches.get_one::<String>(name).map(String::as_str);
    let many = |name: &str| {
        matches
            .get_many::<String>(name)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
    };
    // The board needs a terminal; otherwise it is `status`, text or JSON.
    let json = matches.get_flag("json");
    if command == "board" && !json && std::io::stdout().is_terminal() {
        board::run(core, text("squad").map(str::to_owned))?;
        return Ok(Value::Null.into());
    }
    let mut config = Config::load(&core)?;
    if command == "init" {
        let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        return membership::init(
            &core,
            &mut config,
            text("name").unwrap_or_default(),
            text("me"),
            interactive,
        );
    }
    let squad = Squad::resolve(&core, text("squad"))?;
    match command {
        "lead" => membership::lead(&core, &squad, text("name").unwrap_or_default()),
        "add" => membership::add(&core, &squad, config.layout(&squad.name)?, &many("names")),
        "remove" => membership::remove(&core, &squad, text("name").unwrap_or_default()),
        "set" => membership::set(
            &core,
            &squad,
            text("member").unwrap_or_default(),
            &many("fields"),
        ),
        _ => {
            let layout = config.layout(&squad.name)?;
            let sections = config.sections(&squad.name)?;
            Ok(status::document(&squad, layout, &sections, squad.members(&core)?).into())
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let json = argv.iter().skip(1).any(|arg| arg == "--json");
    let matches = match grammar().try_get_matches_from(&argv) {
        Ok(matches) => matches,
        Err(error) if json && error.use_stderr() => {
            let failure = SquadError::new("USAGE_ERROR", error.kind().to_string());
            println!("{}", failure.to_json());
            return ExitCode::from(2);
        }
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(if error.use_stderr() { 2 } else { 0 });
        }
    };
    let (command, sub) = matches.subcommand().expect("subcommand required");
    match command {
        "__complete" => {
            let words: Vec<String> = sub
                .get_many::<String>("words")
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            for candidate in complete(&words) {
                println!("{candidate}");
            }
            return ExitCode::SUCCESS;
        }
        "skill" => {
            print!("{SKILL}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let (body, code) = match run(command, sub) {
        // The interactive board has already drawn everything it shows.
        Ok(outcome) if outcome.document.is_null() => (String::new(), 0),
        Ok(outcome) => {
            let rendered = if json {
                format!("{}\n", outcome.document)
            } else {
                human(command, &outcome.document)
            };
            (rendered, if outcome.complete { 0 } else { 1 })
        }
        Err(failure) if json => (format!("{}\n", failure.to_json()), 1),
        Err(failure) => {
            let _ = writeln!(std::io::stderr(), "tmt squad: {failure}");
            (String::new(), 1)
        }
    };
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(body.as_bytes())
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn completion_offers_literal_subcommands_and_options_only() {
        assert_eq!(complete(&words("-- s")), ["set", "skill", "status"]);
        assert_eq!(
            complete(&words("-- ")),
            [
                "add", "board", "init", "lead", "remove", "set", "skill", "status"
            ]
        );
        assert_eq!(complete(&words("-- status --")), ["--json", "--squad"]);
        assert_eq!(complete(&words("-- skill s")), ["show"]);
        assert!(
            complete(&words("-- set auth-fix st")).is_empty(),
            "values fall back to the shell"
        );
        assert!(
            complete(&words("-- __c")).is_empty(),
            "hidden entry points stay hidden"
        );
    }

    #[test]
    fn help_names_the_command_not_the_executable() {
        let help = grammar().render_help().to_string();
        assert!(
            help.contains("Usage: tmt squad [OPTIONS] <COMMAND>"),
            "{help}"
        );
        assert!(help.contains("alias: tmt sq"));
        assert!(!help.contains("__complete"));
        grammar().debug_assert();
    }
}
