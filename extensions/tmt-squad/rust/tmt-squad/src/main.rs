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
mod hook_protocol;
mod hotkeys;
mod me;
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
use tmt_cli_style::{
    Terminal, Token,
    list::Section,
    mark::Mark,
    message,
    table::{Cell, Column, Table},
    value,
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
    let build = tmt_cli_style::command;
    build(specs::ROOT)
        .bin_name("tmt squad")
        .version(env!("CARGO_PKG_VERSION"))
        // The release proof expects exactly `squad <version>`.
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
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

/// Replies to the user's squad requests, newest first as core lists them.
/// A row previews one line; the whole body stays exact behind `tmt result`.
fn replies_text(document: &Value, terminal: Terminal) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let replies: Vec<&Value> = document["replies"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if replies.is_empty() {
        let mut text = "No replies to your squad requests yet.\n".to_owned();
        if document["olderRequestsNotShown"] == true {
            text.push_str(&terminal.paint(Token::Dim, "older requests not shown"));
            text.push('\n');
        }
        return text;
    }
    let mut table = Table::new(&[
        Column::Fixed,
        Column::Name,
        Column::Fixed,
        Column::Fixed,
        Column::Detail,
    ]);
    for reply in &replies {
        let text = |key: &str| reply[key].as_str().unwrap_or_default();
        let age = reply["submittedAtMs"].as_u64().map_or(String::new(), |at| {
            value::relative_time(now.saturating_sub(at))
        });
        let (mark, preview) = match reply["response"].as_str() {
            Some(response) => (
                Mark::Done,
                response.lines().next().unwrap_or_default().to_owned(),
            ),
            None => (Mark::Idle, format!("waiting: {}", text("prompt"))),
        };
        table.row([
            Cell::styled(mark.symbol(), mark.token()),
            text("to").into(),
            Cell::styled(age, Token::Dim),
            Cell::styled(text("requestId"), Token::Dim),
            preview.into(),
        ]);
    }
    let newest = replies
        .iter()
        .find(|reply| reply["response"].is_string())
        .and_then(|reply| reply["requestId"].as_str())
        .map(|id| format!("tmt result {id}"));
    let section = Section {
        title: "replies",
        count: Some(replies.len()),
        rows: table,
        note: (document["olderRequestsNotShown"] == true).then_some("older requests not shown"),
        hint: newest.as_deref(),
    };
    let mut output = Vec::new();
    let _ = section.write(&mut output, terminal);
    String::from_utf8(output).unwrap_or_default()
}

fn hotkeys_text(document: &Value, terminal: Terminal) -> String {
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
            done(
                terminal,
                &format!(
                    "Installed hotkeys: {} sources {}.{}",
                    path(&document["target"]),
                    path(&document["squadFile"]),
                    document["backup"]
                        .as_str()
                        .map_or(String::new(), |backup| format!(" Backup: {backup}."))
                ),
            )
        } else {
            "Already installed; nothing changed.\n".into()
        };
    }
    if document.get("removed").is_some() {
        return if document["changed"] == true {
            done(
                terminal,
                &format!("Removed squad's hotkeys ({})", document["removed"]),
            )
        } else {
            "No squad hotkeys were installed; nothing changed.\n".into()
        };
    }
    // The report: label/value rows, home-abbreviated paths, one next step.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let shown = |value: &Value| {
        value::home_path(
            std::path::Path::new(value.as_str().unwrap_or_default()),
            home.as_deref(),
        )
    };
    let mut table = Table::new(&[Column::Fixed, Column::Detail]);
    table.row([
        Cell::styled("installed", Token::Dim),
        Cell::from(if document["installed"] == true {
            "yes"
        } else {
            "no"
        }),
    ]);
    table.row([
        Cell::styled("keys", Token::Dim),
        Cell::from(format!(
            "popup {}, pane {}{}",
            path(&document["keys"]["popup"]),
            path(&document["keys"]["pane"]),
            document["keys"]["back"]
                .as_str()
                .map_or(String::new(), |key| format!(", back {key}")),
        )),
    ]);
    table.row([
        Cell::styled("squad file", Token::Dim),
        Cell::from(format!(
            "{}{}",
            shown(&document["squadFile"]),
            if document["current"] == true {
                ""
            } else {
                " (out of date)"
            }
        )),
    ]);
    if document["executableExists"] == false {
        table.row([
            Cell::styled("tmt", Token::Dim),
            Cell::from(format!(
                "{} (no longer exists)",
                shown(&document["executable"])
            )),
        ]);
    }
    let stale = document["current"] != true || document["executableExists"] == false;
    let section = Section {
        title: "hotkeys",
        count: None,
        rows: table,
        note: None,
        hint: stale.then_some("tmt squad hotkeys install"),
    };
    let mut output = Vec::new();
    let _ = section.write(&mut output, terminal);
    String::from_utf8(output).unwrap_or_default()
}

/// One `✓ <past tense> <object>` line, rendered for `terminal`.
fn done(terminal: Terminal, text: &str) -> String {
    let mut line = Vec::new();
    let _ = message::success(&mut line, terminal, text);
    String::from_utf8(line).unwrap_or_default()
}

fn human(command: &str, document: &Value, terminal: Terminal) -> String {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    match command {
        "status" | "board" => status::text(document, terminal),
        "hotkeys" => hotkeys_text(document, terminal),
        "playbook" => playbook::text(document, terminal),
        "jump" => {
            let mut output = done(
                terminal,
                &format!(
                    "Jumped to {} ({})",
                    text(&document["member"]),
                    text(&document["focused"]["pane"])
                ),
            );
            if let Some(warning) = document["warning"].as_str() {
                let mut line = Vec::new();
                let _ = message::warning(&mut line, terminal, warning, None);
                output.push_str(&String::from_utf8(line).unwrap_or_default());
            }
            output
        }
        "talk" | "annotate" => done(
            terminal,
            &format!(
                "Sent to {} ({})",
                text(&document["to"]),
                text(&document["requestId"])
            ),
        ),
        "replies" => replies_text(document, terminal),
        "reply" => done(
            terminal,
            &format!(
                "Replied to {} ({})",
                text(&document["from"]),
                text(&document["requestId"])
            ),
        ),
        "back" => match document["back"]["focused"]["pane"].as_str() {
            Some(pane) => done(terminal, &format!("Went back to {pane}")),
            None => "Nothing to go back to.\n".into(),
        },
        "open" => done(terminal, &format!("Opened {}", text(&document["opened"]))),
        "copy" => done(terminal, &text(&document["message"])),
        "init" if document["created"] == true => done(
            terminal,
            &format!(
                "Created squad {name} (room squad-{name}); you are {}",
                document["me"].as_str().unwrap_or("unset"),
                name = text(&document["squad"]["name"]),
            ),
        ),
        "init" => format!(
            "Squad {name} already exists (room squad-{name}); you are {}.\n",
            document["me"].as_str().unwrap_or("unset"),
            name = text(&document["squad"]["name"]),
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
        let interactive =
            std::io::stdin().is_terminal() && tmt_cli_style::stream::stderr().is_terminal();
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
        "replies" => member_actions::replies(&core, &squad, &mut config),
        "talk" => member_actions::talk(
            &core,
            &squad,
            &mut config,
            text("member").unwrap_or_default(),
            text("text").unwrap_or_default(),
        ),
        "reply" => member_actions::answer(
            &core,
            &squad,
            &mut config,
            text("member").unwrap_or_default(),
            text("request"),
            text("text").unwrap_or_default(),
        ),
        "annotate" => member_actions::annotate(
            &core,
            &squad,
            &mut config,
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
            let me = me::resolve(&core, &mut config)?;
            requests::overlay(&core, &squad, me.as_ref(), &mut document)?;
            Ok(document.into())
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    // Core's hook protocol, before the grammar: never a user command.
    if argv
        .get(1)
        .is_some_and(|argument| argument == hook_protocol::PREFIX)
    {
        return hook_protocol::run(&argv[1..]);
    }
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
            return print_document(&failure.to_json(), 2);
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
            return print_completion(&complete(&words));
        }
        "skill" => return print_embedded(SKILL),
        _ => {}
    }
    // The board needs a terminal; otherwise it is `status`, text or JSON.
    if command == "board" && !json && tmt_cli_style::stream::stdout(false).is_terminal() {
        let squad = sub.get_one::<String>("squad").cloned();
        let popup = sub.get_flag("popup");
        return match Core::discover().and_then(|core| board::run(core, squad, popup)) {
            Ok(signal) => ExitCode::from(board::exit_status(signal)),
            Err(failure) => {
                report(&failure);
                ExitCode::from(1)
            }
        };
    }
    match run(command, sub) {
        Ok(outcome) => {
            let code = if outcome.complete { 0 } else { 1 };
            if json {
                return print_document(&outcome.document, code);
            }
            let mut stdout = tmt_cli_style::stream::stdout(false);
            let body = human(command, &outcome.document, stdout.terminal());
            match stdout
                .write_all(body.as_bytes())
                .and_then(|()| stdout.flush())
            {
                Ok(()) => ExitCode::from(code),
                Err(_) => ExitCode::FAILURE,
            }
        }
        Err(failure) if json => print_document(&failure.to_json(), 1),
        Err(failure) => {
            report(&failure);
            ExitCode::from(1)
        }
    }
}

/// `--json`: one document and a newline, unstyled.
fn print_document(document: &Value, code: u8) -> ExitCode {
    let mut stdout = tmt_cli_style::stream::stdout(true);
    match writeln!(stdout, "{document}").and_then(|()| stdout.flush()) {
        Ok(()) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}

/// `__complete`: one candidate per line, for the shell.
fn print_completion(candidates: &[String]) -> ExitCode {
    let mut stdout = tmt_cli_style::stream::stdout(true);
    let written = candidates
        .iter()
        .try_for_each(|candidate| writeln!(stdout, "{candidate}"))
        .and_then(|()| stdout.flush());
    if written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `skill show`: the embedded skill, byte for byte.
fn print_embedded(text: &str) -> ExitCode {
    let mut stdout = tmt_cli_style::stream::stdout(true);
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// A human failure: `error: <what>`, then `hint: <next>` when there is one.
/// The code is for `--json`; human output does not repeat it.
fn report(failure: &SquadError) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    let (what, hint) = failure.human();
    let _ = message::error(&mut stderr, terminal, what, hint);
}

#[cfg(test)]
mod cli_style_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_preview_one_line_and_point_at_the_whole_body() {
        let document = serde_json::json!({"replies": [
            {"to": "auth-fix", "requestId": "req_1", "prompt": "check retries",
             "response": "Done.\nSecond line stays behind tmt result.", "submittedAtMs": 0},
            {"to": "docs", "requestId": "req_2", "prompt": "draft the guide", "response": null},
        ]});
        let text = replies_text(&document, Terminal::PLAIN);
        assert!(text.starts_with("REPLIES 2\n"), "{text}");
        assert!(text.contains("✓  auth-fix"), "{text}");
        assert!(text.contains("req_1  Done.\n"), "{text}");
        assert!(!text.contains("Second line"), "{text}");
        assert!(text.contains("◌  docs"), "{text}");
        assert!(text.contains("req_2  waiting: draft the guide\n"), "{text}");
        assert!(text.ends_with("hint: tmt result req_1\n"), "{text}");
        assert_eq!(
            replies_text(&serde_json::json!({"replies": []}), Terminal::PLAIN),
            "No replies to your squad requests yet.\n"
        );
    }

    #[test]
    fn the_hotkeys_report_is_a_section_with_the_next_step_only_when_stale() {
        let report = |current: bool| {
            serde_json::json!({"installed": true, "current": current,
                "keys": {"popup": "S", "pane": "B", "back": null},
                "squadFile": "/nowhere/squad.tmux.conf"})
        };
        let text = hotkeys_text(&report(true), Terminal::PLAIN);
        assert!(text.starts_with("HOTKEYS\n  installed   yes\n"), "{text}");
        assert!(text.contains("keys        popup S, pane B\n"), "{text}");
        assert!(!text.contains("hint:"), "{text}");
        let stale = hotkeys_text(&report(false), Terminal::PLAIN);
        assert!(
            stale.contains("/nowhere/squad.tmux.conf (out of date)"),
            "{stale}"
        );
        assert!(
            stale.ends_with("hint: tmt squad hotkeys install\n"),
            "{stale}"
        );
    }

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
    fn version_prints_squad_and_the_package_version() {
        for flag in ["--version", "-V"] {
            let Err(error) = request(&argv(flag)) else {
                panic!("{flag} ran a command");
            };
            assert_eq!(error.kind(), ErrorKind::DisplayVersion, "{flag}");
            assert!(!error.use_stderr(), "{flag}");
            assert_eq!(
                error.to_string(),
                format!("squad {}\n", env!("CARGO_PKG_VERSION")),
                "{flag}"
            );
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
