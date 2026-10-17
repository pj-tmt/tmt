//! Owner collections from the CLI. Reads and edits use the same admitted Store service as the
//! browser; Seen remains a person-only browser action. Smart results are evaluated by Home.
use crate::cli_management::{fail, fresh_id, resolve_page};
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    collections::{
        self, Collection, Command as Operation, Definition, Fault, Item, Reply, Search, Sort, ipc,
    },
    keyring::{Keyring, Layout, StateFault},
    store::Store,
};

macro_rules! spec {
    ($name:literal, $summary:literal, $example:literal $(,)?) => {
        tmt_cli_style::command(&CommandSpec {
            name: $name, summary: $summary,
            examples: &[Example {command: $example, note: $summary}],
            outputs: OutputModes::HumanAndJson,
            details: "Collections are owner-only display metadata, not access grants. No collection command marks a person's items Seen. Smart definitions use the existing All-pages search; only the browser evaluates their results.",
        })
    };
}
fn id() -> Arg {
    Arg::new("id")
        .required(true)
        .help("Collection UUID or an unambiguous prefix (at least 8 characters)")
}
fn field(name: &'static str) -> Arg {
    Arg::new(name).required(true)
}
fn search(command: Command) -> Command {
    command
        .arg(
            Arg::new("q")
                .long("q")
                .help("Existing All-pages literal search"),
        )
        .arg(Arg::new("label").long("label").action(ArgAction::Append))
        .arg(Arg::new("by").long("by").action(ArgAction::Append))
        .arg(
            Arg::new("sort")
                .long("sort")
                .value_parser(["updated", "title"]),
        )
        .arg(
            Arg::new("archived-pages")
                .long("archived-pages")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("needs-you")
                .long("needs-you")
                .action(ArgAction::SetTrue),
        )
}
pub fn command() -> Command {
    spec!(
        "collection",
        "Organize owner pages in collections",
        "tmt colab collection ls --json",
    )
    .mut_arg("json", |arg| arg.global(true))
    .subcommand_required(true)
    .subcommand(
        spec!(
            "ls",
            "List collection definitions",
            "tmt colab collection ls --json",
        )
        .alias("list"),
    )
    .subcommand(
        spec!(
            "show",
            "Read items and Seen marks",
            "tmt colab collection show COLLECTION --json",
        )
        .arg(id()),
    )
    .subcommand(search(
        spec!(
            "create",
            "Create a manual or smart collection",
            "tmt colab collection create Reports --maintainer PM",
        )
        .arg(field("name"))
        .arg(Arg::new("maintainer").long("maintainer").default_value(""))
        .arg(Arg::new("smart").long("smart").action(ArgAction::SetTrue)),
    ))
    .subcommand(
        spec!(
            "add",
            "Append a page or section reference",
            "tmt colab collection add COLLECTION PAGE#section --note 'Review the figures'",
        )
        .arg(id())
        .arg(field("page"))
        .arg(Arg::new("note").long("note").default_value(""))
        .arg(
            Arg::new("audience")
                .long("for")
                .alias("audience")
                .default_value(""),
        ),
    )
    .subcommand(
        spec!(
            "remove",
            "Remove an item, keeping its page",
            "tmt colab collection remove COLLECTION ITEM",
        )
        .arg(id())
        .arg(field("item")),
    )
    .subcommand(
        spec!(
            "rename",
            "Rename a collection",
            "tmt colab collection rename COLLECTION Reports",
        )
        .arg(id())
        .arg(field("name")),
    )
    .subcommand(
        spec!(
            "maintainer",
            "Change the display maintainer",
            "tmt colab collection maintainer COLLECTION PM",
        )
        .arg(id())
        .arg(field("maintainer")),
    )
    .subcommand(
        spec!(
            "sort",
            "Change collection order",
            "tmt colab collection sort COLLECTION title",
        )
        .arg(id())
        .arg(field("order").value_parser(["manual", "updated", "title"])),
    )
    .subcommand(search(
        spec!(
            "query",
            "Replace a smart collection's search and clear its Seen marks",
            "tmt colab collection query COLLECTION --label urgent --needs-you",
        )
        .arg(id()),
    ))
    .subcommand(
        spec!(
            "archive",
            "Archive a collection without changing its pages",
            "tmt colab collection archive COLLECTION",
        )
        .arg(id())
        .arg(
            Arg::new("restore")
                .long("restore")
                .action(ArgAction::SetTrue),
        ),
    )
}
fn text(args: &ArgMatches, name: &str) -> String {
    args.get_one::<String>(name)
        .expect("declared argument")
        .clone()
}
fn saved_search(args: &ArgMatches) -> Result<(Search, Option<bool>)> {
    let many = |name| {
        args.get_many::<String>(name)
            .map(|v| v.cloned().collect())
            .unwrap_or_default()
    };
    let search = Search {
        q: args.get_one::<String>("q").cloned(),
        label: many("label"),
        by: many("by"),
        sort: args
            .get_one::<String>("sort")
            .filter(|v| v.as_str() == "title")
            .map(|_| Sort::Title),
        archived: args.get_flag("archived-pages").then_some(true),
    };
    if !search.valid() {
        return Err(Fault::Invalid.into());
    }
    Ok((search, args.get_flag("needs-you").then_some(true)))
}
fn operation(
    args: &ArgMatches,
    resolve: impl FnOnce(&str) -> Result<String>,
    resolve_collection: impl FnOnce(&str) -> Result<String>,
) -> Result<Operation> {
    let (mode, args) = args.subcommand().expect("required operation");
    let selected = args
        .try_get_one::<String>("id")
        .ok()
        .flatten()
        .map(|value| resolve_collection(value))
        .transpose()?;
    let collection_id = || selected.clone().expect("collection operand");
    let operation = match mode {
        "ls" => Operation::List {},
        "show" => Operation::Show {
            id: collection_id(),
        },
        "create" => {
            let (search, needs_you) = saved_search(args)?;
            let definition = if args.get_flag("smart") {
                Definition::Smart { search, needs_you }
            } else {
                if search != Search::default() || needs_you.is_some() {
                    return Err(Fault::Invalid.into());
                }
                Definition::Manual { sort: Sort::Manual }
            };
            Operation::Create {
                collection: Collection {
                    id: fresh_id()?,
                    name: text(args, "name"),
                    maintainer: text(args, "maintainer"),
                    archived: false,
                    definition,
                },
            }
        }
        "add" => {
            let reference = text(args, "page");
            let (page, anchor) = match reference.split_once('#') {
                Some((page, anchor)) if collections::section_valid(anchor) => {
                    (page, Some(anchor.to_owned()))
                }
                Some(_) => return Err(Fault::Invalid.into()),
                None => (reference.as_str(), None),
            };
            Operation::Add {
                id: collection_id(),
                item: Item {
                    id: fresh_id()?,
                    page_id: resolve(page)?,
                    anchor,
                    note: text(args, "note"),
                    audience: text(args, "audience"),
                },
            }
        }
        "remove" => Operation::Remove {
            id: collection_id(),
            item: text(args, "item"),
        },
        "rename" => Operation::Rename {
            id: collection_id(),
            name: text(args, "name"),
        },
        "maintainer" => Operation::Maintainer {
            id: collection_id(),
            maintainer: text(args, "maintainer"),
        },
        "sort" => Operation::Sort {
            id: collection_id(),
            sort: match text(args, "order").as_str() {
                "manual" => Sort::Manual,
                "title" => Sort::Title,
                _ => Sort::Updated,
            },
        },
        "query" => {
            let (search, needs_you) = saved_search(args)?;
            Operation::Query {
                id: collection_id(),
                search,
                needs_you,
            }
        }
        "archive" => Operation::Archive {
            id: collection_id(),
            archived: !args.get_flag("restore"),
        },
        _ => unreachable!(),
    };
    if !operation.valid(false) {
        return Err(Fault::Invalid.into());
    }
    Ok(operation)
}
fn resolve_collection_id(operand: &str, ids: &[String]) -> Result<String> {
    match tmt_colab::short_links::matches(operand, ids).as_slice() {
        [] => Err(Fault::NotFound.into()),
        [id] => Ok((*id).to_owned()),
        candidates => Err(fail(
            "COLAB_COLLECTION_AMBIGUOUS",
            &format!(
                "Collection prefix is ambiguous: {}. Use a longer ID; nothing was changed.",
                candidates.join(", ")
            ),
        )),
    }
}
fn execute(layout: &Layout, key: &Keyring, command: &Operation) -> Result<Reply> {
    match layout.serve_lock() {
        Ok(_lock) => {
            let mut store = if ipc::read_only(command) {
                Store::read(layout)?
            } else {
                Store::write_existing(layout)?
            };
            let result = store
                .require_current_schema()
                .map_err(Into::into)
                .and_then(|()| {
                    if ipc::read_only(command) {
                        tmt_colab::inspection::collections(&store, key, command)
                    } else {
                        store.label_transaction(&key.space_id, &key.owner_public(), |tx| {
                            let outcome = tx.apply_collection(command, false)?;
                            Ok(Reply::Detail {
                                detail: outcome.detail,
                                changed: outcome.changed,
                            })
                        })
                    }
                });
            let closed = store.close();
            let reply = result?;
            closed?;
            Ok(reply)
        }
        Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            ipc::request(layout, command.clone())
        }
        Err(error) => Err(error),
    }
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(tmt_colab::page::Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let command = operation(
        args,
        |page| {
            let store = Store::read(&layout)?;
            let result = store
                .require_current_schema()
                .map_err(Into::into)
                .and_then(|()| resolve_page(&store, &key, page));
            let closed = store.close();
            let page = result?;
            closed?;
            Ok(page)
        },
        |operand| {
            if tmt_colab_model::values::generated_id(operand).is_ok() {
                return Ok(operand.into());
            }
            if !tmt_colab::short_links::valid_prefix(operand) {
                return Err(Fault::Invalid.into());
            }
            let Reply::Directory { collections } = execute(&layout, &key, &Operation::List {})?
            else {
                return Err(Fault::Unavailable.into());
            };
            resolve_collection_id(
                operand,
                &collections.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            )
        },
    )?;
    let reply = execute(&layout, &key, &command)?;
    let json = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json);
    if json {
        writeln!(output, "{}", serde_json::to_string(&reply)?)?;
    } else {
        let rows = match &reply {
            Reply::Directory { collections } => collections
                .iter()
                .map(|c| {
                    (
                        "collection",
                        format!(
                            "{} · {}{}",
                            c.id,
                            c.name,
                            if c.archived { " · archived" } else { "" }
                        ),
                    )
                })
                .collect(),
            Reply::Detail { detail, changed } => vec![
                (
                    "collection",
                    format!("{} · {}", detail.collection.id, detail.collection.name),
                ),
                ("changed", changed.to_string()),
                (
                    "definition",
                    serde_json::to_string(&detail.collection.definition)?,
                ),
                ("items", serde_json::to_string(&detail.items)?),
                ("seen", serde_json::to_string(&detail.seen)?),
            ],
        };
        let terminal = output.terminal();
        tmt_cli_style::detail::write(&mut output, terminal, "COLLECTIONS", &rows)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "10000000-0000-4000-8000-000000000001";
    fn parse(args: &[&str]) -> ArgMatches {
        command().try_get_matches_from(args).unwrap()
    }
    #[test]
    fn cli_has_no_person_seen_operations() {
        for op in ["seen", "clear-seen"] {
            assert!(
                command()
                    .try_get_matches_from(["collection", op, ID])
                    .is_err()
            );
        }
    }
    #[test]
    fn section_resolution_is_exact_and_search_uses_the_existing_keys() {
        let args = parse(&["collection", "add", ID, "page#part:2", "--note", "Read it"]);
        let op = operation(
            &args,
            |page| {
                assert_eq!(page, "page");
                Ok(ID.into())
            },
            |id| Ok(id.into()),
        )
        .unwrap();
        let Operation::Add { item, .. } = op else {
            panic!("add")
        };
        assert_eq!(item.anchor.as_deref(), Some("part:2"));
        let args = parse(&[
            "collection",
            "query",
            ID,
            "--q",
            "plan",
            "--label",
            "urgent",
            "--by",
            "PM",
            "--sort",
            "title",
            "--archived-pages",
            "--needs-you",
        ]);
        let Operation::Query {
            search, needs_you, ..
        } = operation(&args, |_| unreachable!(), |id| Ok(id.into())).unwrap()
        else {
            panic!("query")
        };
        assert_eq!(search.q.as_deref(), Some("plan"));
        assert_eq!(search.label, ["urgent"]);
        assert_eq!(search.by, ["PM"]);
        assert_eq!(search.sort, Some(Sort::Title));
        assert_eq!(search.archived, Some(true));
        assert_eq!(needs_you, Some(true));
    }
    #[test]
    fn collection_prefixes_never_choose_between_two_matches() {
        let other = "10000000-0000-4000-8000-000000000002";
        let ids = vec![ID.to_owned(), other.to_owned()];
        let error = resolve_collection_id("10000000", &ids).unwrap_err();
        assert!(error.to_string().contains(ID) && error.to_string().contains(other));
        assert_eq!(resolve_collection_id(ID, &ids).unwrap(), ID);
        assert_eq!(resolve_collection_id("10000000", &ids[..1]).unwrap(), ID);
    }

    #[test]
    fn invalid_reference_never_resolves_and_manual_create_refuses_query_flags() {
        for reference in ["page#", "page#div > p", "page#part#other"] {
            let args = parse(&["collection", "add", ID, reference]);
            assert!(operation(&args, |_| panic!("must not resolve"), |id| Ok(id.into())).is_err());
        }
        let args = parse(&["collection", "create", "Reports", "--needs-you"]);
        assert!(operation(&args, |_| unreachable!(), |id| Ok(id.into())).is_err());
    }
}
