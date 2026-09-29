//! `tmt squad` (alias `tmt sq`): an optional extension reached through TMT's
//! external command dispatch. It keeps no state of its own.

mod action;
mod back;
mod board;
mod config;
mod consent;
mod core;
mod effects;
mod filter;
mod hotkeys;
mod member_actions;
mod membership;
mod playbook;
mod requests;
mod runner;
mod send;
mod specs;
mod squad;
mod status;
mod template;

use crate::{config::Config, core::Core, core::SquadError, membership::Outcome, squad::Squad};
use clap::{Arg, ArgAction, ArgMatches, Command, error::ErrorKind};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{IsTerminal, Write},
    process::ExitCode,
};
use tmt_cli_style::Route;

const SKILL: &str = include_str!("../../../skills/tmt-squad/SKILL.md");

fn squad_option() -> Arg {
    Arg::new("squad")
        .long("squad")
        .value_name("NAME")
        .help("Select a squad; optional when exactly one exists")
}

fn message() -> Arg {
    Arg::new("text")
        .required(true)
        .allow_hyphen_values(true)
        .help("The message, exactly as sent")
}

/// The name is fixed, never argv[0]: `tmt-squad` and its `tmt-sq` link print
/// byte-identical help, errors and completion.
fn grammar() -> Command {
    let operand = |name: &'static str, help: &'static str| Arg::new(name).required(true).help(help);
    let build = tmt_cli_style::command;
    build(specs::ROOT)
        .bin_name("tmt squad")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print one JSON document"),
        )
        .subcommand(
            build(specs::INIT)
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
            build(specs::LEAD)
                .arg(operand("name", "Saved identity to lead"))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::ADD)
                .arg(operand("names", "Identities to add").num_args(1..))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::REMOVE)
                .arg(operand("name", "Member to remove"))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::SET)
                .arg(operand("member", "Member to update"))
                .arg(operand("fields", "field=value pairs").num_args(1..))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::STATUS)
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::BOARD)
                .arg(squad_option())
                .arg(
                    Arg::new("popup")
                        .long("popup")
                        .action(ArgAction::SetTrue)
                        .help("Close after a successful jump (for a tmux popup)"),
                ),
        )
        .subcommand(
            build(specs::HOTKEYS)
                .subcommand_required(true)
                .subcommand(
                    build(specs::HOTKEYS_INSTALL)
                        .arg(
                            Arg::new("print")
                                .long("print")
                                .action(ArgAction::SetTrue)
                                .help("Print the bindings and the line; change nothing"),
                        )
                        .arg(
                            Arg::new("yes")
                                .long("yes")
                                .action(ArgAction::SetTrue)
                                .help("Consent without a prompt"),
                        )
                        .arg(
                            Arg::new("config")
                                .long("config")
                                .value_name("PATH")
                                .help("The tmux configuration to edit (absolute path)"),
                        ),
                )
                .subcommand(
                    build(specs::HOTKEYS_REMOVE)
                        .arg(
                            Arg::new("yes")
                                .long("yes")
                                .action(ArgAction::SetTrue)
                                .help("Consent without a prompt"),
                        ),
                )
                .subcommand(build(specs::HOTKEYS_SHOW)),
        )
        .subcommand(
            build(specs::JUMP)
                .arg(operand("member", "Member or lead to show"))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::TALK)
                .arg(operand("member", "Member or lead"))
                .arg(message())
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::REPLY)
                .arg(operand("member", "Member or lead"))
                .arg(message())
                .arg(
                    Arg::new("request")
                        .long("request")
                        .value_name("ID")
                        .help("Which open request, when the member waits on several"),
                )
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::ANNOTATE)
                .arg(operand("member", "The row the note is about"))
                .arg(message())
                .arg(
                    Arg::new("to")
                        .long("to")
                        .value_name("WHOM")
                        .value_parser(["lead", "member"])
                        .default_value("lead")
                        .help("Send to the squad's lead or to the member"),
                )
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::REPLIES)
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::BACK),
        )
        .subcommand(
            build(specs::OPEN)
                .arg(operand("member", "Member or lead"))
                .arg(
                    Arg::new("link")
                        .long("link")
                        .value_name("FIELD")
                        .help("The field holding the link"),
                )
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::COPY)
                .arg(operand("member", "Member or lead"))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("TEMPLATE")
                        .allow_hyphen_values(true)
                        .help("Text with {field} placeholders [default: \"{name}: {task} ({state})\"]"),
                )
                .arg(squad_option()),
        )
        .subcommand(playbook::grammar())
        .subcommand(
            build(specs::SKILL)
                .subcommand_required(true)
                .subcommand(build(specs::SKILL_SHOW)),
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

/// What the command line asks for.
enum Request {
    /// `tmt squad help [command...]`: print that command's help.
    Help(Box<Command>),
    Run(ArgMatches),
}

/// `help <command>` resolves through the shared route, so it prints what
/// `<command> -h` prints; everything else, `-h` included, is clap's.
fn request(argv: &[OsString]) -> Result<Request, clap::Error> {
    let words: Option<Vec<String>> = argv
        .iter()
        .skip(1)
        .filter(|word| *word != "--json")
        .map(|word| word.to_str().map(str::to_owned))
        .collect();
    if let Some(words) = words {
        match tmt_cli_style::route(&grammar(), &words) {
            Route::Help(command) => return Ok(Request::Help(command)),
            Route::Unknown(word) => {
                return Err(grammar().error(
                    ErrorKind::InvalidSubcommand,
                    format!("unrecognized subcommand '{word}'"),
                ));
            }
            Route::Other => {}
        }
    }
    grammar().try_get_matches_from(argv).map(Request::Run)
}

/// Completion v1: literal candidates for the unfinished word; empty output
/// lets the shell fall back to file completion.
fn complete(words: &[String]) -> Vec<String> {
    let words = words.strip_prefix(&["--".to_owned()]).unwrap_or(words);
    let (current, before) = words
        .split_last()
        .map_or(("", &[][..]), |(last, rest)| (last.as_str(), rest));
    let root = grammar();
    // `help <command>` completes the command names, never options.
    let helping = before.first().is_some_and(|word| word == "help");
    let before = if helping { &before[1..] } else { before };
    // Descend through subcommand names (`hotkeys install`); any other word
    // before the cursor is a value, which falls back to the shell.
    let mut command = &root;
    for word in before {
        match command.find_subcommand(word) {
            Some(sub) => command = sub,
            None => return Vec::new(),
        }
    }
    let mut candidates: Vec<String> = if current.starts_with('-') && helping {
        Vec::new()
    } else if current.starts_with('-') {
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
            .chain((before.is_empty() && !helping).then(|| "help".to_owned()))
            .collect()
    };
    candidates.retain(|candidate| candidate.starts_with(current));
    candidates.sort();
    candidates.dedup();
    candidates
}

fn replies_text(document: &Value) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let mut output = String::new();
    for reply in document["replies"].as_array().into_iter().flatten() {
        // Bodies are agent-written: strip escapes before the terminal sees them.
        let text = |key: &str| board::notes::sanitize(reply[key].as_str().unwrap_or_default());
        let when = reply["submittedAtMs"]
            .as_u64()
            .map_or(String::new(), |at| format!(" · {}", requests::age(now, at)));
        output.push_str(&format!("{}{when}\n  › {}\n", text("to"), text("prompt")));
        match reply["response"].as_str() {
            Some(response) => {
                for line in board::notes::sanitize(response).lines() {
                    output.push_str(&format!("  {line}\n"));
                }
            }
            None => output.push_str(&format!("  tmt result {}\n", text("requestId"))),
        }
    }
    if output.is_empty() {
        output.push_str("No replies to your squad requests yet.\n");
    }
    if document["olderRequestsNotShown"] == true {
        output.push_str("(older requests not shown)\n");
    }
    output
}

fn hotkeys_text(document: &Value) -> String {
    let path = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    if document["bindings"].is_string() && document["installed"].is_null() {
        // --print: exactly what install would write.
        return format!(
            "# {}\n{}\n# add to {}:\n{}\n",
            path(&document["squadFile"]),
            document["bindings"].as_str().unwrap_or_default(),
            path(&document["target"]),
            path(&document["line"])
        );
    }
    if document["installed"] == true && document.get("changed").is_some() {
        return if document["changed"] == true {
            format!(
                "Installed: {} sources {}.{}\n",
                path(&document["target"]),
                path(&document["squadFile"]),
                document["backup"]
                    .as_str()
                    .map_or(String::new(), |backup| format!(" Backup: {backup}."))
            )
        } else {
            "Already installed; nothing changed.\n".into()
        };
    }
    if document.get("removed").is_some() {
        return if document["changed"] == true {
            format!("Removed squad's hotkeys ({}).\n", document["removed"])
        } else {
            "No squad hotkeys were installed; nothing changed.\n".into()
        };
    }
    let mut output = format!(
        "installed: {}\nkeys: popup {}, pane {}{}\nsquad file: {}{}\n",
        document["installed"],
        path(&document["keys"]["popup"]),
        path(&document["keys"]["pane"]),
        document["keys"]["back"]
            .as_str()
            .map_or(String::new(), |key| format!(", back {key}")),
        path(&document["squadFile"]),
        if document["current"] == true {
            ""
        } else {
            " (out of date; run install)"
        },
    );
    if document["executableExists"] == false {
        output.push_str(&format!(
            "The recorded tmt {} no longer exists; run tmt squad hotkeys install.\n",
            path(&document["executable"])
        ));
    }
    output
}

fn human(command: &str, document: &Value) -> String {
    match command {
        "status" | "board" => status::text(document),
        "hotkeys" => hotkeys_text(document),
        "playbook" => playbook::text(document),
        "jump" => format!(
            "Showing {} ({}).\n{}",
            document["member"].as_str().unwrap_or_default(),
            document["focused"]["pane"].as_str().unwrap_or_default(),
            document["warning"]
                .as_str()
                .map_or(String::new(), |warning| format!("Note: {warning}\n"))
        ),
        "talk" | "annotate" => format!(
            "Sent to {} ({}).\n",
            document["to"].as_str().unwrap_or_default(),
            document["requestId"].as_str().unwrap_or_default()
        ),
        "replies" => replies_text(document),
        "reply" => format!(
            "Replied to {} ({}).\n",
            document["from"].as_str().unwrap_or_default(),
            document["requestId"].as_str().unwrap_or_default()
        ),
        "back" => match document["back"]["focused"]["pane"].as_str() {
            Some(pane) => format!("Back at {pane}.\n"),
            None => "Nothing to go back to.\n".into(),
        },
        "open" => format!(
            "Opened {}\n",
            document["opened"].as_str().unwrap_or_default()
        ),
        "copy" => format!("{}\n", document["message"].as_str().unwrap_or_default()),
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

/// `playbook`: `list` and `show` are pure; `install` and `remove` reach core
/// only through the extension-skill door, never the provider directories.
fn playbook_command(matches: &ArgMatches) -> Result<Outcome, SquadError> {
    let (action, flags) = matches.subcommand().expect("subcommand required");
    let flag = |name: &str| flags.try_get_one::<bool>(name).ok().flatten() == Some(&true);
    let name = flags
        .try_get_one::<String>("playbook")
        .ok()
        .flatten()
        .map(String::as_str)
        .unwrap_or_default();
    match action {
        "list" => Ok(playbook::catalog()),
        "show" => playbook::embedded(name),
        "install" => playbook::install(
            &Core::discover()?,
            name,
            flag("print"),
            flag("yes"),
            flag("force"),
        ),
        _ => playbook::remove(&Core::discover()?, name, flag("yes")),
    }
    .map(Outcome::from)
}

fn run(command: &str, matches: &ArgMatches) -> Result<Outcome, SquadError> {
    if command == "playbook" {
        return playbook_command(matches);
    }
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
    if command == "back" {
        return member_actions::back(&core);
    }
    let mut config = Config::load(&core)?;
    if command == "hotkeys" {
        let (action, flags) = matches.subcommand().expect("subcommand required");
        let flag = |name: &str| flags.try_get_one::<bool>(name).ok().flatten() == Some(&true);
        let explicit = flags
            .try_get_one::<String>("config")
            .ok()
            .flatten()
            .map(std::path::Path::new);
        return match action {
            "install" => hotkeys::install(&core, &config, explicit, flag("print"), flag("yes")),
            "remove" => hotkeys::remove(&core, &config, flag("yes")),
            _ => hotkeys::report(&core, &config),
        }
        .map(Outcome::from);
    }
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
        "jump" => member_actions::jump(&core, &squad, &config, text("member").unwrap_or_default()),
        "open" => member_actions::open(
            &core,
            &squad,
            &config,
            text("member").unwrap_or_default(),
            text("link"),
        ),
        "copy" => member_actions::copy(
            &core,
            &squad,
            &config,
            text("member").unwrap_or_default(),
            text("format"),
        ),
        "set" => membership::set(
            &core,
            &squad,
            text("member").unwrap_or_default(),
            &many("fields"),
        ),
        "replies" => member_actions::replies(&core, &squad, &config),
        "talk" => member_actions::talk(
            &core,
            &squad,
            &config,
            text("member").unwrap_or_default(),
            text("text").unwrap_or_default(),
        ),
        "reply" => member_actions::answer(
            &core,
            &squad,
            &config,
            text("member").unwrap_or_default(),
            text("request"),
            text("text").unwrap_or_default(),
        ),
        "annotate" => member_actions::annotate(
            &core,
            &squad,
            &config,
            text("member").unwrap_or_default(),
            text("to") == Some("lead"),
            text("text").unwrap_or_default(),
        ),
        _ => {
            let layout = config.layout(&squad.name)?;
            let sections = config.sections(&squad.name)?;
            let states = config.states(&squad.name, layout)?;
            let mut document =
                status::document(&squad, layout, &states, &sections, squad.members(&core)?);
            requests::overlay(&core, &squad, config.me()?, &mut document)?;
            Ok(document.into())
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let json = argv.iter().skip(1).any(|arg| arg == "--json");
    let matches = match request(&argv) {
        Ok(Request::Run(matches)) => matches,
        Ok(Request::Help(command)) => {
            let mut out = tmt_cli_style::stream::stdout(json);
            let text = tmt_cli_style::help_text(&command, out.terminal());
            return match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            };
        }
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
    // The board needs a terminal; otherwise it is `status`, text or JSON.
    if command == "board" && !json && std::io::stdout().is_terminal() {
        let squad = sub.get_one::<String>("squad").cloned();
        let popup = sub.get_flag("popup");
        return match Core::discover().and_then(|core| board::run(core, squad, popup)) {
            Ok(signal) => ExitCode::from(board::exit_status(signal)),
            Err(failure) => {
                let _ = writeln!(std::io::stderr(), "tmt squad: {failure}");
                ExitCode::from(1)
            }
        };
    }
    let (body, code) = match run(command, sub) {
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
mod cli_style_tests;

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
                "add", "annotate", "back", "board", "copy", "help", "hotkeys", "init", "jump",
                "lead", "open", "playbook", "remove", "replies", "reply", "set", "skill", "status",
                "talk"
            ]
        );
        assert_eq!(complete(&words("-- h")), ["help", "hotkeys"]);
        assert_eq!(
            complete(&words("-- help ho")),
            ["hotkeys"],
            "help completes command names"
        );
        assert_eq!(complete(&words("-- help hotkeys i")), ["install"]);
        assert!(complete(&words("-- help --")).is_empty());
        assert!(!complete(&words("-- help ")).contains(&"help".to_owned()));
        assert_eq!(
            complete(&words("-- status --")),
            ["--help", "--json", "--squad"]
        );
        assert_eq!(complete(&words("-- skill s")), ["show"]);
        assert_eq!(
            complete(&words("-- playbook ")),
            ["install", "list", "remove", "show"]
        );
        assert_eq!(
            complete(&words("-- playbook install --")),
            ["--force", "--help", "--json", "--print", "--yes"]
        );
        assert_eq!(
            complete(&words("-- hotkeys ")),
            ["install", "remove", "show"]
        );
        assert_eq!(
            complete(&words("-- hotkeys install --")),
            ["--config", "--help", "--json", "--print", "--yes"]
        );
        assert!(
            complete(&words("-- set auth-fix st")).is_empty(),
            "values fall back to the shell"
        );
        assert!(
            complete(&words("-- __c")).is_empty(),
            "hidden entry points stay hidden"
        );
    }

    fn argv(line: &str) -> Vec<OsString> {
        std::iter::once("tmt-squad")
            .chain(line.split(' ').filter(|word| !word.is_empty()))
            .map(OsString::from)
            .collect()
    }

    fn help_of(line: &str) -> String {
        match request(&argv(line)) {
            Ok(Request::Help(command)) => {
                tmt_cli_style::help_text(&command, tmt_cli_style::Terminal::PLAIN)
            }
            Err(error) => error.to_string(),
            Ok(Request::Run(_)) => panic!("{line:?} is not a help request"),
        }
    }

    #[test]
    fn help_prints_what_dash_h_prints_and_does_not_reach_a_command() {
        assert_eq!(help_of("help"), help_of("--help"));
        assert_eq!(help_of("help status"), help_of("status -h"));
        assert_eq!(
            help_of("help hotkeys install"),
            help_of("hotkeys install --help")
        );
        assert_eq!(help_of("help skill show --json"), help_of("skill show -h"));
        assert!(help_of("help hotkeys install").contains("Usage: tmt squad hotkeys install"));
        // A word that is no command is an error, never a command that runs.
        for line in [
            "help nope",
            "help init product",
            "help status --squad x",
            "help __complete",
        ] {
            let Err(error) = request(&argv(line)) else {
                panic!("{line:?} did not fail");
            };
            assert_eq!(error.kind(), ErrorKind::InvalidSubcommand, "{line}");
        }
    }

    #[test]
    fn a_message_that_reads_help_is_data_not_a_help_request() {
        for line in [
            "talk auth-fix -- -h",
            "talk auth-fix help",
            "annotate auth-fix -- --help",
        ] {
            let Ok(Request::Run(matches)) = request(&argv(line)) else {
                panic!("{line:?} was taken for help");
            };
            let (_, sub) = matches.subcommand().unwrap();
            assert!(sub.get_one::<String>("text").is_some(), "{line}");
        }
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
