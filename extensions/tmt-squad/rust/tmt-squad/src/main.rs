//! `tmt squad` (alias `tmt sq`): an optional extension reached through TMT's
//! external command dispatch. Core owns authoritative state; disposable
//! extension caches hold derived values and observed ages.

mod action;
mod attention;
mod back;
mod board;
mod cache;
mod config;
mod consent;
mod core;
pub(crate) mod cron_clock;
mod cron_command;
pub mod cron_service;
mod effects;
mod filter;
mod hook_protocol;
mod hotkeys;
mod links;
mod look;
mod markup;
mod me;
mod member_actions;
mod membership;
mod observe;
mod playbook;
mod provider;
mod reminder;
mod requests;
mod rows;
mod runner;
mod send;
mod settings;
mod source;
mod specs;
mod split;
mod squad;
mod staleness;
mod status;
mod tab_view;
mod tabs;
mod template;
#[cfg(test)]
mod test_support;
mod theme;
mod view;

use crate::{
    config::Config, consent::Consent, core::Core, core::SquadError, membership::Outcome,
    squad::Squad,
};
use clap::{Arg, ArgAction, ArgMatches, Command, error::ErrorKind};
use serde_json::{Value, json};
use std::{ffi::OsString, io::Write, process::ExitCode};
use tmt_cli_style::{Interaction, Mode, Route};
use tmt_cli_style::{
    Terminal, Token,
    list::Section,
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

/// Who sends, when it should not be the caller's own identity.
fn identity_option() -> Arg {
    Arg::new("identity")
        .long("identity")
        .value_name("NAME")
        .help("Act as this identity [default: this pane's identity, then `tmt squad me`]")
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
                        .help("Also record your saved identity (same as tmt squad me <name>)"),
                ),
        )
        .subcommand(
            build(specs::ME)
                .arg(Arg::new("name").help("Saved identity that is you"))
                .arg(
                    Arg::new("clear")
                        .long("clear")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("name")
                        .help("Stop recording who you are"),
                ),
        )
        .subcommand(
            build(specs::LEAD)
                .arg(Arg::new("name").help("Saved identity to lead").required_unless_present("none"))
                .arg(Arg::new("none").long("none").action(ArgAction::SetTrue)
                    .conflicts_with("name").help("Clear leadership; former leads remain members"))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::ADD)
                .arg(operand("names", "Identities to add").num_args(1..))
                .arg(squad_option()),
        )
        .subcommand(
            build(specs::REMOVE).alias("remove")
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
            build(specs::LS)
                .alias("status").alias("list")
                .arg(squad_option())
                .arg(Arg::new("tab").long("tab").value_name("NAME")
                    .conflicts_with_all(["squad", "refresh-fields"])
                    .help("List the same rows as a built-in or configured board tab"))
                .arg(
                    Arg::new("refresh-fields")
                        .long("refresh-fields")
                        .action(ArgAction::SetTrue)
                        .help("Run field providers that are due before listing"),
                ),
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
                    build(specs::HOTKEYS_REMOVE).alias("remove")
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
                .arg(
                    operand("member", "Member or lead to show")
                        .required(false)
                        .required_unless_present("lead")
                        .conflicts_with("lead"),
                )
                .arg(
                    Arg::new("lead")
                        .long("lead")
                        .action(ArgAction::SetTrue)
                        .help("Show the squad's lead: your own squad's, without --squad"),
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
                .arg(identity_option())
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
        .subcommand(settings::grammar())
        .subcommand(theme::grammar())
        .subcommand(view::grammar())
        .subcommand(playbook::grammar())
        .subcommand(cron_command::grammar())
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

/// `me`: who "you" is and where it comes from, or the one step to set it.
fn me_text(document: &Value, terminal: Terminal) -> String {
    let name = document["me"]["name"].as_str().unwrap_or_default();
    let path = document["path"].as_str().unwrap_or_default();
    match (document["action"].as_str(), document["source"].as_str()) {
        (Some("set"), _) => done(terminal, &format!("You are {name}; recorded in {path}")),
        (Some("clear"), _) if document["changed"] == true => done(
            terminal,
            &format!("Stopped recording who you are in {path}"),
        ),
        (Some("clear"), _) => "No identity was recorded; nothing changed.\n".into(),
        (_, Some("recorded")) => format!("You are {name} (recorded in {path}).\n"),
        (_, Some("pane")) => {
            format!("You are {name}: this pane's saved identity. Nothing is recorded.\n")
        }
        _ => {
            let mut output =
                b"No identity is recorded as you, and this pane has no saved identity.\n".to_vec();
            let _ = message::hint(&mut output, terminal, "tmt squad me <name>");
            String::from_utf8(output).unwrap_or_default()
        }
    }
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
        "ls" | "board" => status::text(document, terminal),
        "hotkeys" => hotkeys_text(document, terminal),
        "playbook" => playbook::text(document, terminal),
        "config" => settings::text(document, terminal),
        "theme" => theme::text(document, terminal),
        "view" => view::text(document, terminal),
        "cron" => cron_command::text(document, terminal),
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
        "annotate" => done(
            terminal,
            &format!(
                "Sent to {} as {} ({})",
                text(&document["to"]),
                text(&document["as"]),
                text(&document["requestId"])
            ),
        ),
        "back" => match document["back"]["focused"]["pane"].as_str() {
            Some(pane) => done(terminal, &format!("Went back to {pane}")),
            None => "Nothing to go back to.\n".into(),
        },
        "open" => done(terminal, &format!("Opened {}", text(&document["opened"]))),
        "copy" => done(terminal, &text(&document["message"])),
        "init" => {
            let name = text(&document["squad"]["name"]);
            let you = document["me"]
                .as_str()
                .map_or(String::new(), |me| format!("; you are {me}"));
            if document["created"] == true {
                done(
                    terminal,
                    &format!("Created squad {name} (room squad-{name}){you}"),
                )
            } else {
                format!("Squad {name} already exists (room squad-{name}){you}.\n")
            }
        }
        "me" => me_text(document, terminal),
        "lead" => {
            let replaced = names(&document["replaced"]);
            let retained = if replaced.is_empty() {
                String::new()
            } else if document["replaced"]
                .as_array()
                .is_some_and(|names| names.len() == 1)
            {
                format!("; {replaced} remains a member")
            } else {
                format!("; {replaced} remain members")
            };
            if document["lead"].is_null() {
                return done(
                    terminal,
                    &format!("Squad {} has no lead{retained}", text(&document["squad"])),
                );
            }
            done(
                terminal,
                &format!(
                    "{} leads squad {}{}",
                    text(&document["lead"]["name"]),
                    text(&document["squad"]),
                    if replaced.is_empty() {
                        String::new()
                    } else {
                        format!(" (replaces {replaced}{retained})")
                    }
                ),
            )
        }
        "add" => document["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|result| result.get("error").is_none())
            .map(|result| {
                let state = match result["stateSet"].as_str() {
                    Some(state) => format!(" (state {state})"),
                    None => String::new(),
                };
                if result["added"] == false {
                    return format!(
                        "{} is already in squad {}{state}.\n",
                        tmt_cli_style::table::escape(&text(&result["name"])),
                        text(&document["squad"])
                    );
                }
                done(
                    terminal,
                    &format!(
                        "Added {} to squad {}{state}",
                        text(&result["name"]),
                        text(&document["squad"])
                    ),
                )
            })
            .collect(),
        "rm" => {
            let cleared = fields(&document["cleared"]);
            done(
                terminal,
                &format!(
                    "Removed {} from squad {}{}",
                    text(&document["removed"]["name"]),
                    text(&document["squad"]),
                    if cleared.is_empty() {
                        String::new()
                    } else {
                        format!("; cleared {cleared}")
                    }
                ),
            )
        }
        "set" => {
            let applied = fields(&document["applied"]);
            if applied.is_empty() {
                String::new()
            } else {
                done(
                    terminal,
                    &format!("Set {applied} on {}", text(&document["member"])),
                )
            }
        }
        _ => unreachable!("tmt squad {command} has no human output"),
    }
}

/// Partial failures of a multi-step command, as `error:` lines for stderr.
fn human_failures(command: &str, document: &Value) -> Vec<(String, Option<String>)> {
    let text = |value: &Value| value["message"].as_str().unwrap_or_default().to_owned();
    match command {
        "add" => document["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|result| result.get("error").is_some())
            .map(|result| {
                (
                    format!(
                        "Could not add {}: {}",
                        result["name"].as_str().unwrap_or_default(),
                        text(&result["error"])
                    ),
                    None,
                )
            })
            .collect(),
        "set" if document.get("failed").is_some() => {
            let skipped = document["notAttempted"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            vec![(
                format!(
                    "Could not set {} on {}: {}",
                    field(document["failed"]["key"].as_str().unwrap_or_default()),
                    document["member"].as_str().unwrap_or_default(),
                    text(&document["failed"]["error"])
                ),
                (!skipped.is_empty()).then(|| {
                    format!(
                        "tmt squad set {} {skipped}",
                        document["member"].as_str().unwrap_or_default()
                    )
                }),
            )]
        }
        _ => Vec::new(),
    }
}

/// A member field's name, from its full `squad.<name>.<field>` key.
fn field(key: &str) -> &str {
    key.rsplit_once('.').map_or(key, |(_, field)| field)
}

/// Field keys as their names, comma separated.
fn fields(keys: &Value) -> String {
    keys.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(field)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Identity names, comma separated.
fn names(values: &Value) -> String {
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `playbook`: `list` and `show` are pure; `install` and `remove` reach core
/// only through the extension-skill door, never the provider directories.
fn playbook_command(matches: &ArgMatches, interaction: Interaction) -> Result<Outcome, SquadError> {
    let (action, flags) = matches.subcommand().expect("subcommand required");
    let flag = |name: &str| flags.try_get_one::<bool>(name).ok().flatten() == Some(&true);
    let name = flags
        .try_get_one::<String>("playbook")
        .ok()
        .flatten()
        .map(String::as_str)
        .unwrap_or_default();
    match action {
        "ls" => Ok(playbook::catalog()),
        "show" => playbook::embedded(name),
        "install" => playbook::install(
            &Core::discover()?,
            name,
            flag("print"),
            Consent::new(flag("yes"), interaction.prompt()),
            flag("force"),
        ),
        _ => playbook::remove(
            &Core::discover()?,
            name,
            Consent::new(flag("yes"), interaction.prompt()),
        ),
    }
    .map(Outcome::from)
}

/// `me`: shows who "you" is and where that comes from, records a saved
/// identity, or clears the record. It never asks.
fn me_command(
    core: &Core,
    config: &mut Config,
    name: Option<&str>,
    clear: bool,
) -> Result<Outcome, SquadError> {
    let path = config.path().display().to_string();
    let shown =
        |me: Option<&me::Me>| me.map_or(Value::Null, |me| json!({"id": me.id, "name": me.name}));
    if let Some(name) = name {
        let recorded = me::set(core, config, name)?;
        return Ok(json!({"action": "set", "me": shown(Some(&recorded)), "source": "recorded", "path": path}).into());
    }
    if clear {
        let changed = config.me()?.is_some();
        if changed {
            config.clear_me()?;
        }
        return Ok(json!({"action": "clear", "changed": changed, "me": Value::Null, "source": Value::Null, "path": path}).into());
    }
    let you = me::resolve_you(core, config)?;
    Ok(json!({
        "action": "show",
        "me": shown(you.as_ref().map(|(me, _)| me)),
        "source": you.map(|(_, source)| source.as_str()),
        "path": path,
    })
    .into())
}

fn run(
    command: &str,
    matches: &ArgMatches,
    interaction: Interaction,
) -> Result<Outcome, SquadError> {
    if command == "playbook" {
        return playbook_command(matches, interaction);
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
    if command == "cron" {
        return cron_command::run(&core, &config, matches).map(|document| Outcome {
            complete: document["complete"] != false,
            document,
        });
    }
    if command == "config" {
        return settings::run(&mut config, matches).map(Outcome::from);
    }
    if command == "view" {
        return view::run(&mut config, matches).map(Outcome::from);
    }
    if command == "theme" {
        return theme::run(&mut config, matches).map(Outcome::from);
    }
    if command == "hotkeys" {
        let (action, flags) = matches.subcommand().expect("subcommand required");
        let flag = |name: &str| flags.try_get_one::<bool>(name).ok().flatten() == Some(&true);
        let explicit = flags
            .try_get_one::<String>("config")
            .ok()
            .flatten()
            .map(std::path::Path::new);
        return match action {
            "install" => hotkeys::install(
                &core,
                &config,
                explicit,
                flag("print"),
                Consent::new(flag("yes"), interaction.prompt()),
            ),
            "rm" => hotkeys::remove(
                &core,
                &config,
                Consent::new(flag("yes"), interaction.prompt()),
            ),
            _ => hotkeys::report(&core, &config),
        }
        .map(Outcome::from);
    }
    if command == "init" {
        return membership::init(
            &core,
            &mut config,
            text("name").unwrap_or_default(),
            text("me"),
        );
    }
    if command == "me" {
        return me_command(&core, &mut config, text("name"), matches.get_flag("clear"));
    }
    if matches!(command, "ls" | "board") {
        let refresh_fields = command == "ls" && matches.get_flag("refresh-fields");
        if command == "ls"
            && let Some(tab) = text("tab")
        {
            return ls_tab_document(&core, &mut config, tab);
        }
        return ls_document(&core, &mut config, text("squad"), refresh_fields);
    }
    if command == "jump" && matches.get_flag("lead") {
        let squad = member_actions::caller_squad(&core, text("squad"))?;
        return member_actions::jump_lead(&core, &squad, &config);
    }
    let squad = Squad::resolve(&core, text("squad"))?;
    match command {
        "lead" => membership::lead(&core, &squad, text("name")),
        "add" => membership::add(&core, &squad, config.layout(&squad.name)?, &many("names")),
        "rm" => membership::remove(&core, &squad, text("name").unwrap_or_default()),
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
        "annotate" => member_actions::annotate(
            &core,
            &squad,
            &mut config,
            text("identity"),
            text("member").unwrap_or_default(),
            text("to") == Some("lead"),
            text("text").unwrap_or_default(),
        ),
        _ => unreachable!("tmt squad {command} is dispatched above"),
    }
}

/// List one aggregate tab through the same owner as the board worker.
fn ls_tab_document(core: &Core, config: &mut Config, name: &str) -> Result<Outcome, SquadError> {
    let key = tab_view::key(config, name)?;
    let squads = Squad::list(core)?;
    let names = squads
        .iter()
        .map(|squad| squad.name.clone())
        .collect::<Vec<_>>();
    let (tabs, _) = tabs::arrange(&names, &config.tabs()?);
    let you = me::resolve_you(core, config)?;
    let mut document = tab_view::load(
        core,
        config,
        &squads,
        &tabs,
        you.as_ref().map(|(me, _)| me),
        &key,
    )?
    .document;
    document["you"] = you.map_or(
        Value::Null,
        |(me, source)| json!({"id": me.id, "name": me.name, "source": source.as_str()}),
    );
    Ok(document.into())
}

/// `ls` (and `board` without a person at a terminal). With `--squad`, that
/// squad's document; without it, always `{squads: [...], you}` in name order,
/// even for one squad or none, so a script's shape never depends on how many
/// squads exist. "You" is resolved once for all of them.
fn ls_document(
    core: &Core,
    config: &mut Config,
    explicit: Option<&str>,
    refresh_fields: bool,
) -> Result<Outcome, SquadError> {
    let squads = match explicit {
        Some(_) => vec![Squad::resolve(core, explicit)?],
        None => {
            let mut squads = Squad::list(core)?;
            squads.sort_by(|a, b| a.name.cmp(&b.name));
            squads
        }
    };
    let you = me::resolve_you(core, config)?;
    let mut documents = Vec::with_capacity(squads.len());
    for squad in &squads {
        let layout = config.layout(&squad.name)?;
        let sections = config.sections(&squad.name)?;
        let states = config.states(&squad.name, layout)?;
        let rows = config.rows(&squad.name)?;
        let providers = config.providers(&squad.name)?;
        let reminders = config.reminders(&squad.name)?;
        let mut observation = observe::observe(
            core,
            config.path(),
            squad,
            reminders,
            &providers,
            observe::Mode::Read(observe::Reads {
                metadata: rows.reads_metadata(),
                notes: false,
            }),
        )?;
        if refresh_fields {
            provider::refresh(
                &squad.name,
                &providers,
                &observation.members,
                status::now_ms(),
            );
            observation.cached = provider::Cache::load(&squad.name);
        }
        let mut document = observation
            .document(
                core,
                squad,
                you.as_ref().map(|(me, _)| me),
                &providers,
                observe::Shape {
                    layout,
                    states: &states,
                    sections: &sections,
                    rows: &rows,
                },
            )?
            .document;
        document["squad"]["attention"] = attention::Attention::of(&document).document();
        let rows = rows.value();
        document["columns"] = rows["columns"].clone();
        document["lines"] = rows["lines"].clone();
        if let Some(hidden) = rows.get("hidden_columns") {
            document["hidden_columns"] = hidden.clone();
        }
        documents.push(document);
    }
    let mut document = match (explicit, <[Value; 1]>::try_from(documents)) {
        (Some(_), Ok([one])) => one,
        (_, Ok([one])) => json!({"squads": [one]}),
        (_, Err(all)) => json!({"squads": all}),
    };
    document["you"] = you.map_or(
        Value::Null,
        |(me, source)| json!({"id": me.id, "name": me.name, "source": source.as_str()}),
    );
    Ok(document.into())
}

/// `tmt squad` with no command (options such as `--json` aside) is `board`,
/// which is the board for a person at a terminal and `ls` anywhere else.
fn bare_is_board(argv: Vec<OsString>) -> Vec<OsString> {
    if argv.iter().skip(1).all(|word| {
        word.to_str().is_some_and(|word| word.starts_with('-'))
            && !matches!(word.to_str(), Some("-h" | "--help" | "-V" | "--version"))
    }) {
        let mut words = argv;
        words.insert(1.min(words.len()), "board".into());
        return words;
    }
    argv
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
    if let Some(failure) = argv
        .get(1)
        .and_then(|word| removed(&word.to_string_lossy()))
    {
        if json {
            return print_document(&failure.to_json(), 2);
        }
        report(&failure);
        return ExitCode::from(2);
    }
    let matches = match request(&bare_is_board(argv)) {
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
    // Decided once: whether a person can see the board or answer a question.
    let interaction = Interaction::detect(json);
    // The board needs a person at a terminal; otherwise it is `ls`.
    if command == "board" && interaction.view() == Mode::Interactive {
        let squad = sub.get_one::<String>("squad").cloned();
        let popup = sub.get_flag("popup");
        return match Core::discover().and_then(|core| board::run(core, squad, popup, interaction)) {
            Ok(signal) => ExitCode::from(board::exit_status(signal)),
            Err(failure) => {
                report(&failure);
                ExitCode::from(1)
            }
        };
    }
    match run(command, sub, interaction) {
        Ok(outcome) => {
            let code = if outcome.complete { 0 } else { 1 };
            if json {
                return print_document(&outcome.document, code);
            }
            let mut stdout = tmt_cli_style::stream::stdout(false);
            let body = human(command, &outcome.document, stdout.terminal());
            let written = stdout
                .write_all(body.as_bytes())
                .and_then(|()| stdout.flush());
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            if command == "cron" {
                for warning in outcome.document["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    let _ = message::warning(
                        &mut stderr,
                        terminal,
                        warning["error"]["message"]
                            .as_str()
                            .unwrap_or("Cron follow-up failed"),
                        None,
                    );
                }
            }
            for (what, hint) in human_failures(command, &outcome.document) {
                let _ = message::error(&mut stderr, terminal, &what, hint.as_deref());
            }
            match written {
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

/// Squad's conversation verbs moved to core (#512). For one release the old
/// names refuse with the replacement; they never forward, and the grammar,
/// help and completion no longer know them.
fn removed(command: &str) -> Option<SquadError> {
    let replacement = match command {
        "talk" => "tmt talk <member> \"…\" --detach",
        "reply" => "tmt answer <member> \"…\"",
        "replies" => "tmt x (and tmt result <request-id>)",
        _ => return None,
    };
    Some(SquadError::hinted(
        "SQUAD_COMMAND_REMOVED",
        &format!("tmt squad {command} was removed"),
        "; use ",
        replacement,
    ))
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
    fn every_command_has_human_output() {
        // `skill show` prints the embedded skill before reaching `human`;
        // hidden commands are protocols, not user commands.
        let grammar = grammar();
        for command in grammar
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(Command::get_name)
        {
            if command == "skill" {
                continue;
            }
            let output = std::panic::catch_unwind(|| {
                human(
                    command,
                    &serde_json::json!({"action": "ls"}),
                    Terminal::PLAIN,
                )
            });
            assert!(output.is_ok(), "tmt squad {command} has no human output");
        }
    }

    #[test]
    fn set_reports_applied_fields_and_the_one_that_failed() {
        let document = serde_json::json!({
            "squad": "product", "member": "coder",
            "applied": ["squad.product.state"],
            "failed": {"key": "squad.product.note",
                       "error": {"code": "STORAGE_UNAVAILABLE", "message": "Disk full."}},
            "notAttempted": ["task=ship", "pending="],
        });
        assert_eq!(
            human("set", &document, Terminal::PLAIN),
            "✓ Set state on coder\n"
        );
        assert_eq!(
            human_failures("set", &document),
            [(
                "Could not set note on coder: Disk full.".to_owned(),
                Some("tmt squad set coder task=ship pending=".to_owned())
            )]
        );
    }

    #[test]
    fn removed_conversation_verbs_refuse_with_their_core_replacement() {
        for (command, replacement) in [
            ("talk", "tmt talk <member> \"…\" --detach"),
            ("reply", "tmt answer <member> \"…\""),
            ("replies", "tmt x (and tmt result <request-id>)"),
        ] {
            let failure = removed(command).expect(command);
            assert_eq!(failure.code, "SQUAD_COMMAND_REMOVED");
            assert_eq!(
                failure.human(),
                (
                    format!("tmt squad {command} was removed").as_str(),
                    Some(replacement)
                )
            );
            assert!(request(&[OsString::from("tmt-squad"), command.into()]).is_err());
        }
        assert!(removed("annotate").is_none());
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
        assert_eq!(complete(&words("-- s")), ["set", "skill"]);
        assert_eq!(complete(&words("-- l")), ["lead", "ls"]);
        assert_eq!(
            complete(&words("-- ")),
            [
                "add", "annotate", "back", "board", "config", "copy", "cron", "help", "hotkeys",
                "init", "jump", "lead", "ls", "me", "open", "playbook", "rm", "set", "skill",
                "theme", "view"
            ]
        );
        assert_eq!(complete(&words("-- view ")), ["ls", "rm", "set"]);
        assert_eq!(
            complete(&words("-- view set --")),
            ["--help", "--json", "--squad"]
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
            ["--help", "--json", "--refresh-fields", "--squad", "--tab"]
        );
        assert_eq!(complete(&words("-- skill s")), ["show"]);
        assert_eq!(
            complete(&words("-- playbook ")),
            ["install", "ls", "rm", "show"]
        );
        assert_eq!(
            complete(&words("-- playbook install --")),
            ["--force", "--help", "--json", "--print", "--yes"]
        );
        assert_eq!(complete(&words("-- hotkeys ")), ["install", "rm", "show"]);
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
            "annotate auth-fix -- -h",
            "annotate auth-fix help",
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
    fn a_bare_invocation_is_the_board_but_help_and_version_are_not() {
        let argv = |words: &[&str]| -> Vec<OsString> {
            std::iter::once("tmt-squad")
                .chain(words.iter().copied())
                .map(Into::into)
                .collect()
        };
        assert_eq!(bare_is_board(argv(&[])), argv(&["board"]));
        assert_eq!(bare_is_board(argv(&["--json"])), argv(&["board", "--json"]));
        for kept in [
            &["-h"][..],
            &["--help"],
            &["-V"],
            &["--version"],
            &["ls"],
            &["help"],
        ] {
            assert_eq!(bare_is_board(argv(kept)), argv(kept), "{kept:?}");
        }
    }

    #[test]
    fn lead_requires_exactly_one_name_or_none() {
        for accepted in ["lead sol", "lead --none", "lead --none --squad product"] {
            assert!(
                matches!(request(&argv(accepted)), Ok(Request::Run(_))),
                "{accepted}"
            );
        }
        for refused in ["lead", "lead sol --none"] {
            assert!(request(&argv(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn help_names_the_command_not_the_executable() {
        let help = grammar().render_help().to_string();
        assert!(
            help.contains("Usage: tmt squad [OPTIONS] [COMMAND]"),
            "{help}"
        );
        assert!(help.contains("alias: tmt sq"));
        assert!(!help.contains("__complete"));
        grammar().debug_assert();
    }
}
