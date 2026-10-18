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
    limits::{PAGE_PROJECTS, PROJECT_POINTS},
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
    let names = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect()
    };
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
/// One local management request; the acknowledgment must name this operation.
fn send(
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
                "Invalid project acknowledgment; inspect projects before retrying.",
            )
        })?;
    if reply["operationId"] != operation_id.as_str() {
        return Err(fail(
            "COLAB_OUTCOME_UNKNOWN",
            "Mismatched project acknowledgment; inspect projects before retrying.",
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
            for (name, pages) in &counts {
                rows.push((
                    "project",
                    format!(
                        "{name} · {pages} page{}",
                        if *pages == 1 { "" } else { "s" }
                    ),
                ));
            }
            if counts.is_empty() {
                rows.push(("projects", "none".into()));
            }
            json!({"spaceId":key.space_id,"projects":counts.iter().map(|(name, pages)| json!({"name":name,"pages":pages})).collect::<Vec<_>>()})
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
