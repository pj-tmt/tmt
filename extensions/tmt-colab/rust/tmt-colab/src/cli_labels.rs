//! Page labels from the command line: display-only names that organize the owner's page list
//! (contract: Page labels). A write goes through the same local management path as every other
//! change, but it creates no owner-log statement and carries no authority. Last writer wins per
//! page: the command reads the current set and replaces the whole set.
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result, inspection,
    keyring::{Keyring, Layout},
    labels,
    limits::{LABEL_POINTS, LABEL_REQUEST_PAGES, PAGE_LABELS},
    page::Fault,
    store::Store,
};
use tmt_colab_model::values;

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
    command_spec!("label","Organize pages with labels","tmt colab label add 10000000-0000-4000-8000-000000000001 urgent","Labels are display-only names that organize your page list. They carry no authority and are not signed, so they never change who can read or write a page. A page holds at most 8 labels of 1 to 32 characters (no control characters, no leading, trailing or doubled spaces). Anyone who can run these commands, and any paired owner browser, can change them.")
        .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
        .subcommand(command_spec!("ls","List every label with its page count","tmt colab label ls --json","Reads only; archived pages count.").alias("list"))
        .subcommand(command_spec!("add","Add labels to a page","tmt colab label add 10000000-0000-4000-8000-000000000001 urgent 'q4 plan'","Adding a label the page already has changes nothing. Last writer wins: the command replaces the page's whole label set.").arg(crate::cli_grammar::page()).arg(names("Labels to add")))
        .subcommand(command_spec!("remove","Remove labels from a page","tmt colab label remove 10000000-0000-4000-8000-000000000001 urgent","Removing a label the page does not have changes nothing.").alias("rm").arg(crate::cli_grammar::page()).arg(names("Labels to remove")))
        .subcommand(command_spec!("rename","Rename a label on every page that has it","tmt colab label rename urgent soon","Runs in one step per 64 pages. A page that already has the new name keeps one label. Archived pages are included; no page is deleted.").arg(Arg::new("name").required(true).index(1).help("The label to rename")).arg(Arg::new("to").required(true).index(2).help("Its new name")))
        .subcommand(command_spec!("delete","Delete a label from every page that has it","tmt colab label delete urgent","Only the label goes: no page is deleted. Archived pages are included.").arg(Arg::new("name").required(true).index(1).help("The label to delete")))
}
fn invalid_name() -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_INPUT_INVALID",
        &format!(
            "A label is 1 to {LABEL_POINTS} characters with no control characters and no leading, trailing or doubled spaces."
        ),
    )
}
fn require_valid(name: &str) -> Result<()> {
    if labels::valid(name) {
        Ok(())
    } else {
        Err(invalid_name())
    }
}
fn capacity() -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_CAPACITY",
        &format!(
            "A page holds at most {PAGE_LABELS} labels, and the space has a limit on all labels together. Remove one first."
        ),
    )
}
fn not_found(name: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_LABEL_NOT_FOUND",
        &format!("No page has the label {name}. Run `tmt colab label ls` to see the labels."),
    )
}
fn current(catalog: &Value) -> BTreeMap<String, Vec<String>> {
    catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|page| {
            Some((
                page["pageId"].as_str()?.to_owned(),
                page["labels"]
                    .as_array()?
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect(),
            ))
        })
        .collect()
}
/// One `page.labels` request per `LABEL_REQUEST_PAGES` pages, each one store transaction. The page
/// IDs go in ascending order and the first one initiates, as every management payload names them.
fn write(layout: &Layout, key: &Keyring, changes: &[(String, Vec<String>)]) -> Result<()> {
    let mut sorted = changes.to_vec();
    sorted.sort();
    for chunk in sorted.chunks(LABEL_REQUEST_PAGES) {
        let payload = json!({"pages":chunk.iter().map(|(page, names)| json!({"pageId":page,"labels":names})).collect::<Vec<_>>()});
        let operation_id = fresh_id()?;
        let body = serde_json::to_vec(&json!({"space":key.space_id,"page":chunk[0].0,
            "expectedRevision":"1","operationId":operation_id,"operation":"page.labels",
            "payload":values::encode_binary(&serde_json::to_vec(&payload)?)}))?;
        let reply: Value =
            serde_json::from_slice(&mutate(layout, &key.space_id, &body)?).map_err(|_| {
                fail(
                    "COLAB_OUTCOME_UNKNOWN",
                    "Invalid label acknowledgment; inspect labels before retrying.",
                )
            })?;
        if reply["operationId"] != operation_id.as_str() {
            return Err(fail(
                "COLAB_OUTCOME_UNKNOWN",
                "Mismatched label acknowledgment; inspect labels before retrying.",
            ));
        }
    }
    Ok(())
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
    let all = current(&catalog);
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
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for name in all.values().flatten() {
                *counts.entry(name).or_default() += 1;
            }
            for (name, pages) in &counts {
                rows.push((
                    "label",
                    format!(
                        "{name} · {pages} page{}",
                        if *pages == 1 { "" } else { "s" }
                    ),
                ));
            }
            if counts.is_empty() {
                rows.push(("labels", "none".into()));
            }
            json!({"spaceId":key.space_id,"labels":counts.iter().map(|(name, pages)| json!({"name":name,"pages":pages})).collect::<Vec<_>>()})
        }
        "add" | "remove" => {
            let id = page.expect("page operand");
            names.iter().try_for_each(|name| {
                if mode == "add" {
                    require_valid(name)
                } else {
                    Ok(())
                }
            })?;
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
            if next.len() > PAGE_LABELS {
                return Err(capacity());
            }
            let changed = next != before;
            if changed {
                write(&layout, &key, &[(id.clone(), next.clone())])?;
            }
            rows.push(("page", id.clone()));
            rows.push((
                "labels",
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
            json!({"spaceId":key.space_id,"pageId":id,"labels":next,"changed":changed})
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
            let changes: Vec<(String, Vec<String>)> = all
                .iter()
                .filter(|(_, held)| held.contains(name))
                .map(|(id, held)| {
                    let mut next: Vec<String> =
                        held.iter().filter(|held| *held != name).cloned().collect();
                    next.extend(to.cloned());
                    next.sort();
                    next.dedup();
                    (id.clone(), next)
                })
                .collect();
            if changes.is_empty() {
                return Err(not_found(name));
            }
            write(&layout, &key, &changes)?;
            rows.push(("label", name.clone()));
            if let Some(to) = to {
                rows.push(("renamed to", to.clone()));
            }
            rows.push(("pages", changes.len().to_string()));
            let mut value = json!({"spaceId":key.space_id,"name":name,"pages":changes.len()});
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
        tmt_cli_style::detail::write(&mut output, terminal, "PAGE LABELS", &rows)?;
    }
    Ok(())
}
