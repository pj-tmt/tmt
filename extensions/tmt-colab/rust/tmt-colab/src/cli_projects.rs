//! Projects from the command line: display-only names that organize the owner's page list
//! (contract: Projects). A write goes through the same local management path as every other
//! change, but it creates no owner-log statement and carries no authority. A page's projects are
//! replaced as a whole set (last writer wins); a project change is one request.
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result, inspection,
    keyring::{Keyring, Layout},
    limits::{PAGE_PROJECTS, PROJECT_POINTS, SPACE_PROJECT_PINS},
    page::Fault,
    projects,
    store::Store,
};
use tmt_colab_model::{auth::SPACE_SCOPE_PAGE, values};

use crate::cli_management::{fail, fresh_id, mutate, resolve_page};

macro_rules! command_spec {
    ($name:literal,$summary:literal,$example:literal,$details:literal) => {
        tmt_cli_style::command(&CommandSpec {
            name: $name,
            summary: $summary,
            examples: &[Example {
                command: $example,
                note: $summary,
            }],
            outputs: OutputModes::HumanAndJson,
            details: $details,
        })
    };
}
pub fn command() -> Command {
    let names = |help: &'static str| {
        Arg::new("name")
            .required(true)
            .index(2)
            .num_args(1..)
            .action(ArgAction::Append)
            .help(help)
    };
    command_spec!("project","Organize pages in projects","tmt colab project add 10000000-0000-4000-8000-000000000001 launch","Projects are display-only names that organize your page list; a page can be in several. They carry no authority and are not signed, so they never change who can read or write a page. A page is in at most 8 projects; a name is 1 to 32 characters (no control characters, no leading, trailing or doubled spaces). Anyone who can run these commands, and any paired owner browser, can change them.")
        .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
        .subcommand(command_spec!("ls","List every project with its page count","tmt colab project ls --json","Reads only; archived pages count, and a project with no page is listed.").alias("list"))
        .subcommand(command_spec!("create","Create a project with no page yet","tmt colab project create launch","Creating a project that exists changes nothing. Add pages with `project add`.").arg(Arg::new("name").required(true).index(1).help("The new project")))
        .subcommand(command_spec!("add","Put a page in projects","tmt colab project add 10000000-0000-4000-8000-000000000001 launch 'q4 plan'","A name no project has yet creates it. Adding a project the page is already in changes nothing. Last writer wins: the command replaces the page's whole project set.").arg(crate::cli_grammar::page()).arg(names("Projects to add")))
        .subcommand(command_spec!("remove","Take a page out of projects","tmt colab project remove 10000000-0000-4000-8000-000000000001 launch","Removing a project the page is not in changes nothing. The project stays, even with no page left.").alias("rm").arg(crate::cli_grammar::page()).arg(names("Projects to remove")))
        .subcommand(command_spec!("rename","Rename a project","tmt colab project rename launch release","Archived pages are included. Renaming onto an existing project merges the two: a page in both keeps one entry. No page is deleted.").arg(Arg::new("name").required(true).index(1).help("The project to rename")).arg(Arg::new("to").required(true).index(2).help("Its new name")))
        .subcommand(command_spec!("delete","Delete a project","tmt colab project delete launch","Only the grouping goes: no page is deleted. Archived pages are included.").arg(Arg::new("name").required(true).index(1).help("The project to delete")))
        .subcommand(command_spec!("pin","Pin a project to your project list","tmt colab project pin launch --position 1","Your pinned projects, in your order. Until you pin one the list is the five you used most recently; the first pin makes the list yours. A browser turns your pins into shortcuts once. At most 20. Pinning a pinned project moves it.").arg(Arg::new("name").required(true).index(1).help("The project to pin")).arg(Arg::new("position").long("position").value_parser(clap::value_parser!(usize)).help("Its place in the list, from 1 (default: last)")))
        .subcommand(command_spec!("unpin","Take a project out of your pinned list","tmt colab project unpin launch","The project and its pages stay. Unpinning a project that is not pinned changes nothing.").arg(Arg::new("name").required(true).index(1).help("The project to unpin")))
}
/// The strings of a JSON array, in order; anything else reads as none.
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect()
}
/// The pinned projects the catalog reports, in order, and whether they are the automatic default.
fn pins(catalog: &Value) -> (Vec<String>, bool) {
    (
        strings(&catalog["projectPins"]["names"]),
        catalog["projectPins"]["automatic"] == true,
    )
}
fn invalid_name() -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_INPUT_INVALID",
        &format!(
            "A project is 1 to {PROJECT_POINTS} characters with no control characters and no leading, trailing or doubled spaces."
        ),
    )
}
fn require_valid(name: &str) -> Result<()> {
    if projects::valid(name) {
        Ok(())
    } else {
        Err(invalid_name())
    }
}
fn capacity() -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_CAPACITY",
        &format!(
            "A page is in at most {PAGE_PROJECTS} projects, and the space has a limit on all projects together. Remove one first."
        ),
    )
}
pub fn not_found(name: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_PROJECT_NOT_FOUND",
        &format!("No project is named {name}. Run `tmt colab project ls` to see the projects."),
    )
}
/// Each page's projects and every project of the space, with its page count.
fn current(catalog: &Value) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, usize>) {
    let names = strings;
    let by_page: BTreeMap<String, Vec<String>> = catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|page| {
            Some((
                page["pageId"].as_str()?.to_owned(),
                names(&page["projects"]),
            ))
        })
        .collect();
    let mut counts: BTreeMap<String, usize> = names(&catalog["projects"])
        .into_iter()
        .map(|name| (name, 0))
        .collect();
    for name in by_page.values().flatten() {
        *counts.entry(name.clone()).or_default() += 1;
    }
    (by_page, counts)
}
/// One local display-only management request; the acknowledgment must name this operation.
pub(crate) fn send(
    layout: &Layout,
    key: &Keyring,
    page: &str,
    operation: &str,
    payload: &Value,
) -> Result<()> {
    let operation_id = fresh_id()?;
    let body = serde_json::to_vec(&json!({"space":key.space_id,"page":page,
        "expectedRevision":"","operationId":operation_id,"operation":operation,
        "payload":values::encode_binary(&serde_json::to_vec(payload)?)}))?;
    let reply: Value =
        serde_json::from_slice(&mutate(layout, &key.space_id, &body)?).map_err(|_| {
            fail(
                "COLAB_OUTCOME_UNKNOWN",
                "Invalid acknowledgment; inspect the page list before retrying.",
            )
        })?;
    if reply["operationId"] != operation_id.as_str() {
        return Err(fail(
            "COLAB_OUTCOME_UNKNOWN",
            "Mismatched acknowledgment; inspect the page list before retrying.",
        ));
    }
    Ok(())
}
fn manage(layout: &Layout, key: &Keyring, payload: &Value) -> Result<()> {
    send(layout, key, SPACE_SCOPE_PAGE, "project.manage", payload)
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (mode, args) = args.subcommand().expect("required subcommand");
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let catalog = inspection::catalog(&store, &key)?;
    let page = match mode {
        "add" | "remove" => Some(resolve_page(
            &store,
            &key,
            args.get_one::<String>("page").unwrap(),
        )?),
        _ => None,
    };
    store.close()?;
    let (all, counts) = current(&catalog);
    let names: Vec<String> = args
        .try_get_many::<String>("name")
        .ok()
        .flatten()
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let mut rows: Vec<(&str, String)> = Vec::new();
    let value = match mode {
        "ls" => {
            let (pinned, automatic) = pins(&catalog);
            let place = |name: &String| pinned.iter().position(|held| held == name).map(|n| n + 1);
            for (name, pages) in &counts {
                rows.push((
                    "project",
                    format!(
                        "{name} · {pages} page{}{}",
                        if *pages == 1 { "" } else { "s" },
                        place(name).map_or(String::new(), |n| format!(" · pinned {n}")),
                    ),
                ));
            }
            if counts.is_empty() {
                rows.push(("projects", "none".into()));
            } else if automatic {
                rows.push((
                    "pinned",
                    "automatic: your most recently used projects, until you pin one".into(),
                ));
            }
            json!({"spaceId":key.space_id,"pinsAutomatic":automatic,"projects":counts.iter().map(|(name, pages)| json!({"name":name,"pages":pages,"pinned":place(name)})).collect::<Vec<_>>()})
        }
        "pin" | "unpin" => {
            let name = &names[0];
            counts.get(name).ok_or_else(|| not_found(name))?;
            let (shown, automatic) = pins(&catalog);
            let mut next: Vec<String> =
                shown.iter().filter(|held| *held != name).cloned().collect();
            if mode == "pin" {
                let last = next.len() + 1;
                let place = match args.get_one::<usize>("position") {
                    Some(&place) if (1..=last).contains(&place) => place,
                    Some(_) => {
                        return Err(fail(
                            "COLAB_INPUT_INVALID",
                            &format!("--position is a place from 1 to {last}."),
                        ));
                    }
                    None => last,
                };
                next.insert(place - 1, name.clone());
                if next.len() > SPACE_PROJECT_PINS {
                    return Err(fail(
                        "COLAB_CAPACITY",
                        &format!(
                            "At most {SPACE_PROJECT_PINS} projects can be pinned. Unpin one first."
                        ),
                    ));
                }
            }
            // The first pin makes the list the shown automatic one, so it is written even when the
            // list reads the same.
            let changed = next != shown || (mode == "pin" && automatic);
            if changed {
                manage(&layout, &key, &json!({"action":"pins","names":next}))?;
            }
            let position = next.iter().position(|held| held == name).map(|n| n + 1);
            rows.push(("project", name.clone()));
            rows.push((
                "pinned",
                position.map_or("no".into(), |place| format!("place {place}")),
            ));
            rows.push((
                "status",
                match (changed, mode) {
                    (false, _) => "unchanged",
                    (true, "pin") => "pinned",
                    (true, _) => "unpinned",
                }
                .into(),
            ));
            json!({"spaceId":key.space_id,"name":name,"pinned":position.is_some(),"position":position,"changed":changed})
        }
        "create" => {
            let name = &names[0];
            require_valid(name)?;
            let changed = !counts.contains_key(name);
            if changed {
                manage(&layout, &key, &json!({"action":"create","name":name}))?;
            }
            rows.push(("project", name.clone()));
            rows.push((
                "status",
                if changed { "created" } else { "unchanged" }.into(),
            ));
            json!({"spaceId":key.space_id,"name":name,"changed":changed})
        }
        "add" | "remove" => {
            let id = page.expect("page operand");
            if mode == "add" {
                names.iter().try_for_each(|name| require_valid(name))?;
            }
            let before = all.get(&id).cloned().unwrap_or_default();
            let mut next: Vec<String> = if mode == "add" {
                before.iter().chain(names.iter()).cloned().collect()
            } else {
                before
                    .iter()
                    .filter(|name| !names.contains(name))
                    .cloned()
                    .collect()
            };
            next.sort();
            next.dedup();
            if next.len() > PAGE_PROJECTS {
                return Err(capacity());
            }
            let changed = next != before;
            if changed {
                send(
                    &layout,
                    &key,
                    &id,
                    "page.projects",
                    &json!({"pages":[{"pageId":id,"projects":next}]}),
                )?;
            }
            rows.push(("page", id.clone()));
            rows.push((
                "projects",
                if next.is_empty() {
                    "none".into()
                } else {
                    next.join(", ")
                },
            ));
            rows.push((
                "status",
                if changed {
                    if mode == "add" { "added" } else { "removed" }
                } else {
                    "unchanged"
                }
                .into(),
            ));
            json!({"spaceId":key.space_id,"pageId":id,"projects":next,"changed":changed})
        }
        "rename" | "delete" => {
            let name = &names[0];
            let to = args.try_get_one::<String>("to").ok().flatten();
            if let Some(to) = to {
                require_valid(to)?;
                if to == name {
                    return Err(fail("COLAB_INPUT_INVALID", "Choose a different name."));
                }
            }
            let pages = *counts.get(name).ok_or_else(|| not_found(name))?;
            let payload = match to {
                Some(to) => json!({"action":"rename","name":name,"to":to}),
                None => json!({"action":"delete","name":name}),
            };
            manage(&layout, &key, &payload)?;
            rows.push(("project", name.clone()));
            if let Some(to) = to {
                rows.push(("renamed to", to.clone()));
            }
            rows.push(("pages", pages.to_string()));
            let mut value = json!({"spaceId":key.space_id,"name":name,"pages":pages});
            if let Some(to) = to {
                value["to"] = json!(to);
            }
            value
        }
        _ => unreachable!(),
    };
    let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
    if args.get_flag("json") {
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(&mut output, terminal, "PROJECTS", &rows)?;
    }
    Ok(())
}
