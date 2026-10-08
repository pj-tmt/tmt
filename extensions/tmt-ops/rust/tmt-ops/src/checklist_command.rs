//! Native checklist grammar and projection; the existing service owns admission and effects.

use crate::{
    checklist::{self, Code, Current, Error, Filter, Service},
    config::Config,
    core::Core,
    membership::Outcome,
    specs,
};
use checklist::model::{
    Action, Completion, Confirmation, Edit, Id, InventoryExpectation, ItemContent, Mutation,
    Request,
};
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{
    Terminal, Token, detail,
    list::Section,
    message,
    table::{Cell, Column, Table},
};

fn option(name: &'static str, value: &'static str, help: &'static str) -> Arg {
    Arg::new(name).long(name).value_name(value).help(help)
}
fn room(command: Command) -> Command {
    command.arg(option("room", "UUID", "Exact Squad room UUID").required(true))
}
fn checklist(command: Command) -> Command {
    room(command).arg(option("checklist", "UUID", "Exact checklist UUID").required(true))
}
fn item(command: Command) -> Command {
    checklist(command).arg(option("item", "UUID", "Exact item UUID").required(true))
}
fn revision(name: &'static str, help: &'static str) -> Arg {
    option(name, "POSITIVE", help)
        .allow_negative_numbers(true)
        .required(true)
}
fn mutation(command: Command) -> Command {
    item(command).arg(revision("expect-revision", "Exact positive item revision"))
}

pub fn grammar() -> Command {
    let build = tmt_cli_style::command;
    build(specs::CHECKLIST)
        .subcommand_required(true)
        .subcommand(
            room(build(specs::CHECKLIST_LS))
                .alias("list")
                .arg(
                    Arg::new("include-archived")
                        .long("include-archived")
                        .action(ArgAction::SetTrue)
                        .help("Include archived items"),
                )
                .arg(
                    option(
                        "completion",
                        "STATE",
                        "Filter completion without changing order",
                    )
                    .value_parser(["open", "complete"]),
                )
                .arg(option("assignee", "UUID", "Filter an exact assignee UUID")),
        )
        .subcommand(item(build(specs::CHECKLIST_SHOW)))
        .subcommand(
            item(build(specs::CHECKLIST_CREATE))
                .arg(
                    Arg::new("title")
                        .required(true)
                        .help("Manually authored title"),
                )
                .arg(
                    option(
                        "expect-inventory",
                        "absent|POSITIVE",
                        "Frozen checklist inventory expectation",
                    )
                    .allow_negative_numbers(true)
                    .required(true),
                )
                .arg(option("body", "TEXT", "Optional authored body"))
                .arg(option(
                    "reference",
                    "HTTP(S)",
                    "Optional inert reference; never fetched or opened",
                ))
                .arg(option(
                    "assignee",
                    "UUID",
                    "Explicit active member UUID; requires manager permission",
                )),
        )
        .subcommand(
            mutation(build(specs::CHECKLIST_EDIT))
                .arg(option("title", "TEXT", "Replace only the title"))
                .arg(option("body", "TEXT", "Replace only the body").conflicts_with("clear-body"))
                .arg(
                    Arg::new("clear-body")
                        .long("clear-body")
                        .action(ArgAction::SetTrue)
                        .help("Explicitly clear the body"),
                )
                .arg(
                    option("reference", "HTTP(S)", "Replace only the inert reference")
                        .conflicts_with("clear-reference"),
                )
                .arg(
                    Arg::new("clear-reference")
                        .long("clear-reference")
                        .action(ArgAction::SetTrue)
                        .help("Explicitly clear the reference"),
                )
                .group(
                    ArgGroup::new("edit")
                        .args([
                            "title",
                            "body",
                            "clear-body",
                            "reference",
                            "clear-reference",
                        ])
                        .multiple(true)
                        .required(true),
                ),
        )
        .subcommand(
            mutation(build(specs::CHECKLIST_ASSIGN))
                .arg(option("assignee", "UUID", "Explicit active member UUID").required(true)),
        )
        .subcommand(mutation(build(specs::CHECKLIST_UNASSIGN)))
        .subcommand(mutation(build(specs::CHECKLIST_COMPLETE)))
        .subcommand(mutation(build(specs::CHECKLIST_REOPEN)))
        .subcommand(mutation(build(specs::CHECKLIST_ARCHIVE)))
        .subcommand(mutation(build(specs::CHECKLIST_RESTORE)))
        .subcommand(
            mutation(build(specs::CHECKLIST_DELETE))
                .arg(revision(
                    "expect-inventory",
                    "Exact positive inventory revision",
                ))
                .arg(option("confirm-item", "UUID", "Confirm this exact item UUID").required(true))
                .arg(revision(
                    "confirm-revision",
                    "Confirm this exact item revision",
                )),
        )
        .subcommand(
            checklist(build(specs::CHECKLIST_REORDER))
                .arg(revision(
                    "expect-inventory",
                    "Exact positive inventory revision",
                ))
                .arg(
                    option(
                        "order",
                        "JSON_UUID_ARRAY",
                        "Full nondeleted order, including archived items",
                    )
                    .required(true),
                ),
        )
}

pub struct ChecklistCommand {
    room_id: Id,
    action: &'static str,
    operation: Operation,
}
enum Operation {
    List(Filter),
    Show { checklist_id: Id, item_id: Id },
    Apply(Request),
}
fn text<'a>(matches: &'a ArgMatches, name: &str) -> Option<&'a str> {
    matches
        .try_get_one::<String>(name)
        .ok()
        .flatten()
        .map(String::as_str)
}
fn required<'a>(matches: &'a ArgMatches, name: &str) -> &'a str {
    text(matches, name).expect("required checklist argument")
}
fn invalid(message: &str) -> Error {
    Error {
        code: Code::InputInvalid,
        message: message.into(),
        current: None,
    }
}
fn positive(matches: &ArgMatches, name: &str) -> Result<u64, Error> {
    let value = required(matches, name);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("Revisions must be positive decimal integers."));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or_else(|| invalid("Revisions must be positive decimal integers within u64."))
}
fn optional_id(matches: &ArgMatches, name: &str) -> Result<Option<Id>, Error> {
    text(matches, name).map(Id::parse).transpose()
}
fn edit_field(matches: &ArgMatches, name: &str, clear: &str) -> Option<Option<String>> {
    if matches.get_flag(clear) {
        Some(None)
    } else {
        text(matches, name).map(|value| Some(value.into()))
    }
}

/// Parse only the admitted grammar. Target UUIDs and revisions stay frozen for this invocation.
pub fn parse(parent: &ArgMatches) -> Result<ChecklistCommand, Error> {
    let (name, flags) = parent.subcommand().expect("checklist action required");
    let room_id = Id::parse(required(flags, "room"))?;
    if name == "ls" {
        return Ok(ChecklistCommand {
            room_id,
            action: "list",
            operation: Operation::List(Filter {
                include_archived: flags.get_flag("include-archived"),
                completion: text(flags, "completion").map(|value| {
                    if value == "open" {
                        Completion::Open
                    } else {
                        Completion::Complete
                    }
                }),
                assignee: optional_id(flags, "assignee")?,
            }),
        });
    }
    let checklist_id = Id::parse(required(flags, "checklist"))?;
    if name == "show" {
        return Ok(ChecklistCommand {
            room_id,
            action: "show",
            operation: Operation::Show {
                checklist_id,
                item_id: Id::parse(required(flags, "item"))?,
            },
        });
    }
    let (action, operation) = match name {
        "create" => (
            "create",
            Action::Create {
                item_id: Id::parse(required(flags, "item"))?,
                inventory: if required(flags, "expect-inventory") == "absent" {
                    InventoryExpectation::Absent
                } else {
                    InventoryExpectation::Revision(positive(flags, "expect-inventory")?)
                },
                content: ItemContent::new(
                    required(flags, "title").into(),
                    text(flags, "body").map(str::to_owned),
                    text(flags, "reference").map(str::to_owned),
                )?,
                assignee: optional_id(flags, "assignee")?,
            },
        ),
        "reorder" => {
            let values: Vec<String> = serde_json::from_str(required(flags, "order"))
                .map_err(|_| invalid("Order must be a JSON array of UUID strings."))?;
            (
                "reorder",
                Action::Reorder {
                    inventory_revision: positive(flags, "expect-inventory")?,
                    order: values
                        .iter()
                        .map(|value| Id::parse(value))
                        .collect::<Result<_, _>>()?,
                },
            )
        }
        _ => {
            let (action, mutation) = match name {
                "edit" => (
                    "edit",
                    Mutation::Edit(Edit {
                        title: text(flags, "title").map(str::to_owned),
                        body: edit_field(flags, "body", "clear-body"),
                        reference: edit_field(flags, "reference", "clear-reference"),
                    }),
                ),
                "assign" => (
                    "assign",
                    Mutation::Assign(Id::parse(required(flags, "assignee"))?),
                ),
                "unassign" => ("unassign", Mutation::Unassign),
                "complete" => ("complete", Mutation::Complete),
                "reopen" => ("reopen", Mutation::Reopen),
                "archive" => ("archive", Mutation::Archive),
                "restore" => ("restore", Mutation::Restore),
                "delete" => (
                    "delete",
                    Mutation::Delete {
                        inventory_revision: positive(flags, "expect-inventory")?,
                        confirmation: Confirmation {
                            item_id: Id::parse(required(flags, "confirm-item"))?,
                            item_revision: positive(flags, "confirm-revision")?,
                        },
                    },
                ),
                _ => unreachable!("registered checklist action"),
            };
            (
                action,
                Action::Item {
                    item_id: Id::parse(required(flags, "item"))?,
                    revision: positive(flags, "expect-revision")?,
                    mutation,
                },
            )
        }
    };
    let request = Request {
        room_id: room_id.clone(),
        checklist_id,
        action: operation,
    };
    request.validate()?;
    Ok(ChecklistCommand {
        room_id,
        action,
        operation: Operation::Apply(request),
    })
}

pub fn run(core: &Core, config: &Config, invocation: ChecklistCommand) -> Outcome {
    finish((|| {
        let service = Service::new(core, config, invocation.room_id)?;
        execute(&service, invocation.action, invocation.operation)
    })())
}
fn execute(service: &Service, action: &str, operation: Operation) -> Result<Value, Error> {
    match operation {
        Operation::List(filter) => {
            let listed = service.list(&filter)?;
            Ok(
                json!({"action":action,"current":current(&listed.current),"totalCount":listed.total_count,"matchedCount":listed.current.items.len()}),
            )
        }
        Operation::Show {
            checklist_id,
            item_id,
        } => {
            Ok(json!({"action":action,"current":current(&service.show(&checklist_id, &item_id)?)}))
        }
        Operation::Apply(request) => {
            let applied = service.apply(&request)?;
            let mut result = json!({"action":action,"current":current(&applied.current),"changed":applied.changed});
            if let Some(id) = applied.item_id {
                result["itemId"] = json!(id.as_str());
            }
            if let Some(revision) = applied.item_revision {
                result["itemRevision"] = json!(revision);
            }
            Ok(result)
        }
    }
}
pub fn finish(result: Result<Value, Error>) -> Outcome {
    match result {
        Ok(document) => document.into(),
        Err(error) => {
            let mut document =
                json!({"error":{"code":error.code.as_str(),"message":error.message}});
            if let Some(view) = error.current {
                document["error"]["current"] = current(&view);
            }
            Outcome {
                document,
                complete: false,
            }
        }
    }
}
fn current(view: &Current) -> Value {
    let items: Vec<_> = view.items.iter().map(|view| {
        let item = &view.item;
        json!({"id":item.id.as_str(),"revision":item.revision,"title":item.content.title,
            "body":item.content.body,"reference":item.content.reference,
            "assignee":item.assignee.as_ref().map(|assignee| json!({"id":assignee.id.as_str(),"label":view.assignee_label,"available":view.assignee_available})),
            "completion":match item.completion { Completion::Open => "open", Completion::Complete => "complete" },"archived":item.archived})
    }).collect();
    json!({"room":{"id":view.room.id.as_str(),"name":view.room.name,"available":view.room.available,"manager":view.room.manager},
        "checklistId":view.checklist_id.as_ref().map(Id::as_str),"inventoryRevision":view.inventory_revision,"items":items,
        "deletion":view.deletion.as_ref().map(|deletion| json!({"itemId":deletion.item_id.as_str(),"deletionRevision":deletion.deletion_revision}))})
}

/// Errors go through main's existing stderr path; typed admitted current is the only context.
pub fn failure(document: &Value) -> Vec<(String, Option<String>)> {
    let Some(error) = document.get("error") else {
        return Vec::new();
    };
    let mut what = error["message"].as_str().unwrap_or_default().to_owned();
    if let Some(view) = error.get("current") {
        what.push_str(&format!(
            " Room {}; checklist {}; inventory revision {}.",
            display(&view["room"]["id"]),
            display(&view["checklistId"]),
            display(&view["inventoryRevision"])
        ));
        for item in view["items"].as_array().into_iter().flatten() {
            what.push_str(&format!(
                " Item {} revision {}.",
                display(&item["id"]),
                display(&item["revision"])
            ));
        }
        if !view["deletion"].is_null() {
            what.push_str(&format!(
                " Deleted item {} at revision {}.",
                display(&view["deletion"]["itemId"]),
                display(&view["deletion"]["deletionRevision"])
            ));
        }
    }
    let hint = match error["code"].as_str() {
        Some("CHECKLIST_OUTCOME_UNKNOWN") => Some("Observe current state; original operation outcome remains unknown. Do not blindly replay.".into()),
        Some("CHECKLIST_CONFLICT") => Some("Review the admitted current revisions and submit a new explicit operation.".into()),
        Some("CHECKLIST_STATE_INVALID") => Some("A manager must explicitly restore the archived item before this operation.".into()),
        _ => None,
    };
    vec![(what, hint)]
}
fn display(value: &Value) -> String {
    if value.is_null() {
        "absent".into()
    } else if let Some(value) = value.as_str() {
        value.into()
    } else {
        value.to_string()
    }
}

pub fn text_output(document: &Value, terminal: Terminal) -> String {
    if document.get("error").is_some() {
        return String::new();
    }
    let view = &document["current"];
    let mut out = Vec::new();
    if !matches!(document["action"].as_str(), Some("list" | "show")) {
        let target = document.get("itemId").unwrap_or(&view["checklistId"]);
        let _ = message::success(
            &mut out,
            terminal,
            &format!(
                "Checklist {}: {}{}; inventory revision {}",
                display(&document["action"]),
                display(target),
                document
                    .get("itemRevision")
                    .map_or(String::new(), |revision| format!(
                        " revision {}",
                        display(revision)
                    )),
                display(&view["inventoryRevision"])
            ),
        );
        if document["changed"] == false {
            out.extend_from_slice(
                b"Nothing changed; exact expectations and permission admitted.\n",
            );
        }
    }
    let title = format!(
        "CHECKLIST {}{}",
        display(&view["room"]["id"]),
        if view["room"]["available"] == false {
            " (room unavailable; read-only)"
        } else {
            ""
        }
    );
    let mut rows = Table::new(&[Column::Fixed, Column::Detail, Column::Fixed, Column::Detail]);
    for item in view["items"].as_array().into_iter().flatten() {
        rows.row([
            Cell::from(format!(
                "{} r{}",
                display(&item["id"]),
                display(&item["revision"])
            )),
            Cell::from(display(&item["title"])),
            Cell::from(format!(
                "{}{}",
                display(&item["completion"]),
                if item["archived"] == true {
                    " archived"
                } else {
                    ""
                }
            )),
            Cell::styled(
                if item["assignee"].is_null() {
                    "unassigned".into()
                } else {
                    format!(
                        "{} ({}){}",
                        display(&item["assignee"]["label"]),
                        display(&item["assignee"]["id"]),
                        if item["assignee"]["available"] == false {
                            " unavailable"
                        } else {
                            ""
                        }
                    )
                },
                Token::Dim,
            ),
        ]);
    }
    let note = if view["checklistId"].is_null() {
        Some("No checklist exists; this read created nothing.")
    } else if document["totalCount"] == 0 {
        Some("The checklist is empty.")
    } else if document["matchedCount"] == 0 {
        Some("No items match these filters; the checklist is not empty.")
    } else {
        None
    };
    let _ = Section {
        title: &title,
        count: view["items"].as_array().map(Vec::len),
        rows,
        note,
        hint: None,
    }
    .write(&mut out, terminal);
    let _ = detail::write(
        &mut out,
        terminal,
        "Identity and revisions",
        &[
            ("checklist", display(&view["checklistId"])),
            ("inventory revision", display(&view["inventoryRevision"])),
        ],
    );
    if document["action"] == "show" {
        for item in view["items"].as_array().into_iter().flatten() {
            let _ = detail::write(
                &mut out,
                terminal,
                &display(&item["id"]),
                &[
                    ("body", display(&item["body"])),
                    ("reference", display(&item["reference"])),
                ],
            );
        }
    }
    String::from_utf8(out).expect("checklist output is UTF-8")
}

#[cfg(test)]
mod tests;
