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
mod squad;
mod status;
mod template;

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
    Command::new("squad")
        .bin_name("tmt squad")
        .about("Leads, members and one board for a team of agents (alias: tmt sq)")
        .version(env!("CARGO_PKG_VERSION"))
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
                .arg(squad_option())
                .arg(
                    Arg::new("popup")
                        .long("popup")
                        .action(ArgAction::SetTrue)
                        .help("Close after a successful jump (for a tmux popup)"),
                ),
        )
        .subcommand(
            Command::new("hotkeys")
                .about("tmux prefix keys that open the board (added to tmux.conf only with consent)")
                .subcommand_required(true)
                .subcommand(
                    Command::new("install")
                        .about("Show the plan, then add squad's source-file line and bindings")
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
                    Command::new("remove")
                        .about("Remove only squad's line and squad's bindings")
                        .arg(
                            Arg::new("yes")
                                .long("yes")
                                .action(ArgAction::SetTrue)
                                .help("Consent without a prompt"),
                        ),
                )
                .subcommand(Command::new("show").about("Report the hotkeys' state; change nothing")),
        )
        .subcommand(
            Command::new("jump")
                .about("Show a member's pane in your tmux client (tmt focus)")
                .arg(operand("member", "Member or lead to show"))
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("talk")
                .about("Send a detached request to a member, in the squad room")
                .arg(operand("member", "Member or lead"))
                .arg(message())
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("reply")
                .about("Answer what a member is waiting on you for")
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
            Command::new("annotate")
                .about("Send a note about a member's row to the lead (or the member)")
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
            Command::new("replies")
                .about("Show finals to your squad requests, newest first (never acknowledges)")
                .arg(squad_option()),
        )
        .subcommand(
            Command::new("back")
                .about("Return your tmux client to where its last squad jump came from"),
        )
        .subcommand(
            Command::new("open")
                .about("Open a member's http(s) link: pr_link, link or another *_link field")
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
            Command::new("copy")
                .about("Copy a member's summary to the clipboard")
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
    // Descend through subcommand names (`hotkeys install`); any other word
    // before the cursor is a value, which falls back to the shell.
    let mut command = &root;
    for word in before {
        match command.find_subcommand(word) {
            Some(sub) => command = sub,
            None => return Vec::new(),
        }
    }
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
                "add", "annotate", "back", "board", "copy", "hotkeys", "init", "jump", "lead",
                "open", "playbook", "remove", "replies", "reply", "set", "skill", "status", "talk"
            ]
        );
        assert_eq!(complete(&words("-- status --")), ["--json", "--squad"]);
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
            ["--config", "--json", "--print", "--yes"]
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
