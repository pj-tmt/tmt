//! A page's work: `tmt colab subpage ...` and `tmt colab summary ...`. Each change is one
//! conditional action through the structured publication owner, on the revision the command just
//! read (or the `--expected-revision` it was given). An attach the link rules refuse (one parent,
//! no loop, an available page) is refused here, before anything is prepared (`work_admission`).
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::{Decoder, StructuredKind},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
    work_admission::{self, Refused, Space},
    work_index,
    work_links::{Graph, Refusal},
};

macro_rules! example {
    ($command:literal) => {
        &[Example {
            command: $command,
            note: "Run this command",
        }]
    };
}
pub(crate) fn spec(
    name: &'static str,
    summary: &'static str,
    details: &'static str,
    examples: &'static [Example],
) -> Command {
    tmt_cli_style::command(&CommandSpec {
        name,
        summary,
        outputs: OutputModes::HumanAndJson,
        examples,
        details,
    })
}
fn revision_arg() -> Arg {
    Arg::new("expected-revision")
        .long("expected-revision")
        .value_name("REVISION")
        .value_parser(crate::revision_token)
        .help("Refuse unless the page is at this revision; without it the command uses the current one")
}
fn text_arg(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).long(name).value_name("TEXT").help(help)
}
fn child_arg() -> Arg {
    Arg::new("child")
        .index(2)
        .required(true)
        .value_name("SUBPAGE")
        .help("The sub-page: its ID or a unique ID prefix")
}
fn before_arg() -> Arg {
    Arg::new("before")
        .long("before")
        .value_name("SUBPAGE")
        .help("Put it in front of this listed sub-page; without it, it goes last")
}

pub fn subpage_command() -> Command {
    spec(
        "subpage",
        "List and change a page's sub-pages",
        "A page's work is its sub-pages: an ordered list of the pages directly under it. A page has one parent, a link never closes a loop, and sub-pages nest at any depth. Progress counts the direct sub-pages by their own page state.",
        example!("tmt colab subpage ls 10000000-0000-4000-8000-000000000001 --json"),
    )
    .subcommand_required(true)
    .subcommand(
        spec(
            "ls",
            "List a page's sub-pages",
            "Read only: takes no write lock and creates nothing. Shows each direct sub-page with its state, the page's progress and its summary.",
            example!("tmt colab subpage ls 10000000-0000-4000-8000-000000000001 --json"),
        )
        .alias("list")
        .arg(crate::cli_grammar::page()),
    )
    .subcommand(
        spec(
            "attach",
            "Make an existing page a sub-page",
            "Refused when the page already has another parent, when it is this page or above it (a loop), or when it is not a page of this space.",
            example!("tmt colab subpage attach 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000002 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(child_arg())
        .arg(before_arg())
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "detach",
            "Remove a page from the sub-pages",
            "The page itself stays; it only stops being a sub-page here. Always admitted for a listed page, so a stored conflict can be repaired.",
            example!("tmt colab subpage detach 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000002 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(child_arg())
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "move",
            "Reorder a sub-page",
            "Moves a listed sub-page in front of --before, or to the end.",
            example!("tmt colab subpage move 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000002 --before 20000000-0000-4000-8000-000000000003 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(child_arg())
        .arg(before_arg())
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "create",
            "Make a new page and attach it",
            "The new page starts with the title as its heading. The page is made first, so an attach that fails leaves the page and says how to finish; it never searches for a page to adopt.",
            example!("tmt colab subpage create 10000000-0000-4000-8000-000000000001 --title 'Launch checklist' --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(text_arg("title", "The new page's title").required(true)),
    )
}

pub fn summary_command() -> Command {
    spec(
        "summary",
        "Set or clear a page's two summary lines",
        "The summary is two short lines shown with the page in lists: the first up to 24 characters, the second up to 60.",
        example!("tmt colab summary set 10000000-0000-4000-8000-000000000001 --line1 'Release plan' --line2 'Read before the release' --json"),
    )
    .subcommand_required(true)
    .subcommand(
        spec(
            "set",
            "Set the summary lines",
            "Replaces both lines; the writer is recorded as the summary's author.",
            example!("tmt colab summary set 10000000-0000-4000-8000-000000000001 --line1 'Release plan' --line2 'Read before the release' --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(revision_arg())
        .arg(text_arg("line1", "The first line, at most 24 characters").required(true))
        .arg(text_arg("line2", "The second line, at most 60 characters").required(true)),
    )
    .subcommand(
        spec(
            "clear",
            "Remove the summary",
            "Does nothing when the page has none.",
            example!("tmt colab summary clear 10000000-0000-4000-8000-000000000001 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(revision_arg()),
    )
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

/// A refusal the shared admission names, as this command's fault.
fn fault(
    error: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    match error.downcast_ref::<Refused>() {
        Some(refused) => Box::new(crate::cli_management::ManagementFault {
            code: refused.code,
            message: refused.message.clone(),
            correlation: json!({}),
            source: Some(error),
        }),
        None => error,
    }
}

/// One conditional work action on a page, through the structured publication owner: on the
/// revision it just read, or on `expected`. Sub-pages named by a prefix are resolved the way every
/// command reads a page; one that is not there is refused as a link would be.
pub(crate) fn write_work(
    root: &Path,
    page: &str,
    action: &'static str,
    body: &Value,
    expected: Option<&str>,
) -> Result<page::Receipt> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let lock = crate::write_owner(&layout)?;
    write_work_owned(root, &layout, page, action, body, expected, lock.as_ref())
}
/// `write_work` under a write lock the caller already holds (`None`: the running serve writes).
fn write_work_owned(
    root: &Path,
    layout: &Layout,
    page: &str,
    action: &'static str,
    body: &Value,
    expected: Option<&str>,
    lock: Option<&nix::fcntl::Flock<std::fs::File>>,
) -> Result<page::Receipt> {
    let key = Keyring::read(layout)?;
    let store = Store::read(layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(&store, &key, page)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let read = page::read_work(&store, &key, &page_id, &mut decoder)?;
    let mut body = body.clone();
    for field in ["pageId", "before"] {
        if let Some(named) = body[field].as_str().map(str::to_owned) {
            let id = resolve_page(&store, &key, &named).map_err(|_| {
                fault(work_admission::link_refusal(
                    &Refusal::Unavailable,
                    &page_id,
                    &named,
                ))
            })?;
            body[field] = json!(id);
        }
    }
    // Whatever the command read is the base unless the caller named one.
    let expected = expected.unwrap_or(&read.revision).to_owned();
    work_admission::admit(
        &Space {
            root,
            layout,
            store: &store,
            key: &key,
        },
        &mut decoder,
        &page_id,
        &json!({"version":2,"order":read.children}),
        action,
        &body,
    )
    .map_err(fault)?;
    let display_agent = crate::core::caller_snapshot().map(|caller| {
        let mut value = json!({"name":caller.name});
        if let Some(id) = caller.agent_id {
            value["id"] = json!(id);
        }
        value
    });
    let prepared = page::prepare_structured_publication(
        &store,
        &key,
        &page_id,
        page::StructuredAction {
            kind: StructuredKind::Work,
            action,
            body: &body,
            display_agent,
            expected_revision: &expected,
            operation_id: None,
        },
        &mut decoder,
        &now,
    );
    let closed = store.close();
    let prepared = prepared?;
    closed?;
    crate::publish_structured_owned(layout, &key, &page_id, prepared, &mut decoder, now, lock)
}

pub fn run(root: &Path, group: &str, args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required work command");
    let page = args.get_one::<String>("page").expect("required page");
    let text = |name: &str| args.get_one::<String>(name).cloned();
    let (action, body) = match (group, name) {
        ("subpage", "ls") => return list(root, page, args.get_flag("json")),
        ("subpage", "create") => return create(root, page, args),
        ("summary", "set") => (
            "summary.set",
            json!({"line1":text("line1"),"line2":text("line2")}),
        ),
        ("summary", _) => ("summary.clear", json!({})),
        (_, "detach") => ("subpage.detach", json!({"pageId":text("child")})),
        (_, verb) => {
            let mut body = json!({"pageId":text("child")});
            if let Some(before) = text("before") {
                body["before"] = json!(before);
            }
            (
                if verb == "move" {
                    "subpage.move"
                } else {
                    "subpage.attach"
                },
                body,
            )
        }
    };
    let receipt = write_work(
        root,
        page,
        action,
        &body,
        args.get_one::<String>("expected-revision")
            .map(String::as_str),
    )?;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{}", serde_json::to_value(&receipt)?)?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if receipt.changed {
                "WORK UPDATED"
            } else {
                "WORK UNCHANGED"
            },
            &[
                ("page", receipt.page_id.clone()),
                ("revision", receipt.revision.clone()),
            ],
        )?;
    }
    Ok(())
}

/// What `subpage ls` and `subpage create` read, without a write lock.
struct Read {
    page_id: String,
    work: page::WorkPage,
    /// The space's work, for the link rules and each sub-page's state.
    found: work_index::Found,
    /// Each readable sub-page's title.
    titles: std::collections::BTreeMap<String, String>,
}
/// The one-shot rewrite of task-era work values (#5825), run when the serve opens a space: every
/// page still holding version 1 gets one `work.migrate` action, which keeps its linked pages in
/// task order and its summary and drops its plain tasks. A page already at version 2 is left alone,
/// so the run is idempotent; a page that cannot be rewritten now is `report`ed and tried at the
/// next open (its first write migrates it too). The serve holds the write lock (`serve_lock`)
/// and passes it, since its own socket is not up yet to take a write. Returns the pages rewritten.
pub(crate) fn migrate_space(
    root: &Path,
    lock: &nix::fcntl::Flock<std::fs::File>,
    report: &dyn Fn(&str),
) -> Result<Vec<String>> {
    let Some(layout) = Layout::existing(root)? else {
        return Ok(Vec::new());
    };
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let legacy: Result<Vec<String>> =
        tmt_colab::inspection::all_page_ids(&store, &key).map(|pages| {
            pages
                .into_iter()
                .filter(|page| {
                    page::read_work(&store, &key, page, &mut decoder).is_ok_and(|read| read.legacy)
                })
                .collect()
        });
    let closed = store.close();
    let legacy = legacy?;
    closed?;
    let mut rewritten = Vec::new();
    for page in legacy {
        match write_work_owned(
            root,
            &layout,
            &page,
            tmt_colab::work::MIGRATE,
            &json!({}),
            None,
            Some(lock),
        ) {
            Ok(_) => rewritten.push(page),
            Err(error) => report(&format!(
                "Page {page} keeps its task-era work until the next serve: {error}"
            )),
        }
    }
    Ok(rewritten)
}

/// A page's work read without a write lock, with what the space says about its sub-pages.
fn read_with_space(root: &Path, page: &str) -> Result<Read> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(&store, &key, page)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let read = page::read_work(&store, &key, &page_id, &mut decoder).and_then(|work| {
        let pages = tmt_colab::inspection::all_page_ids(&store, &key)?;
        let found =
            work_index::scan_with_live(&layout, &store, &key, &mut decoder, &pages, Some(&pages))?;
        let titles = work
            .children
            .iter()
            .filter_map(|child| {
                let title = page::read_work(&store, &key, child, &mut decoder)
                    .ok()?
                    .title;
                Some((child.clone(), title))
            })
            .collect();
        Ok(Read {
            page_id: page_id.clone(),
            work,
            found,
            titles,
        })
    });
    let closed = store.close();
    let read = read?;
    closed?;
    Ok(read)
}

/// `subpage create`: a new page with the title as its heading, then the attach. The page is made
/// first, so an attach that fails leaves a page and says how to finish; it never searches for one
/// to adopt.
fn create(root: &Path, page: &str, args: &ArgMatches) -> Result<()> {
    let title = tmt_colab::subpage_create::title(&json!(
        args.get_one::<String>("title").expect("required title")
    ))
    .map_err(|_| {
        fail(
            "COLAB_INPUT_INVALID",
            "A title is one line of text, not empty.",
        )
    })?
    .to_owned();
    let Read {
        page_id,
        work: read,
        found,
        ..
    } = read_with_space(root, page)?;
    // Refuse what the attach would refuse before a page exists.
    if let Some(why) = Graph(&found).creation_refusal(&page_id) {
        return Err(fault(work_admission::link_refusal(&why, &page_id, "")));
    }
    let created = crate::cli_management::commit_creation(
        root,
        &title,
        tmt_colab::subpage_create::child_source(&title),
        Some(&tmt_colab::decoder::PageLayout {
            kind: tmt_colab::decoder::LayoutKind::Column,
            reason: None,
        }),
        false,
    )?;
    let child = created.page_id;
    let finish = format!(
        "The page {child} was created; finish with: tmt colab subpage attach {page_id} {child}"
    );
    // Making a page moves the space, so the attach is written on the revision read now.
    let receipt = write_work(
        root,
        &page_id,
        "subpage.attach",
        &json!({"pageId":child}),
        None,
    )
    .map_err(|error| {
        Box::new(crate::cli_management::ManagementFault {
            code: crate::error_code(error.as_ref()),
            message: format!("{error} {finish}"),
            correlation: json!({}),
            source: Some(error),
        }) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let reach = crate::reach::Reach::gather();
    let path = crate::reach::Reach::path(&read.space_id, &child);
    if json_output {
        let mut value = serde_json::to_value(&receipt)?;
        value["childPage"] = json!(child);
        value["title"] = json!(title);
        reach.annotate(&mut value, &path);
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "SUB-PAGE CREATED",
            &[
                ("page", page_id),
                ("sub-page", child),
                ("link", reach.text(&path)),
                ("revision", receipt.revision.clone()),
            ],
        )?;
    }
    Ok(())
}

/// `subpage ls`: a read, which takes no write lock and creates nothing.
fn list(root: &Path, page: &str, json_output: bool) -> Result<()> {
    let Read {
        page_id,
        work: read,
        found,
        titles,
    } = read_with_space(root, page)?;
    let graph = Graph(&found);
    let progress = graph.progress(&page_id).unwrap_or_default();
    let reach = crate::reach::Reach::gather();
    let path = crate::reach::Reach::path(&read.space_id, &page_id);
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let subpages: Vec<Value> = read
        .children
        .iter()
        .map(|child| {
            json!({"pageId":child,"title":titles.get(child),"state":graph.page_state(child)})
        })
        .collect();
    if json_output {
        let mut value = serde_json::to_value(&read)?;
        value["subpages"] = json!(subpages);
        value["progress"] = json!(progress);
        value["stage"] = json!(progress.stage());
        reach.annotate(&mut value, &path);
        writeln!(output, "{value}")?;
        return Ok(());
    }
    let terminal = output.terminal();
    let mut rows = vec![
        ("page", page_id.clone()),
        ("link", reach.text(&path)),
        ("title", read.title.clone()),
        ("revision", read.revision.clone()),
        (
            "stage",
            format!(
                "{} ({}/{} done)",
                progress.stage(),
                progress.done,
                progress.total
            ),
        ),
    ];
    if let Some(summary) = &read.summary {
        rows.push((
            "summary",
            format!(
                "{} / {}",
                summary["line1"].as_str().unwrap_or_default(),
                summary["line2"].as_str().unwrap_or_default()
            ),
        ));
    }
    for row in &subpages {
        rows.push((
            "sub-page",
            format!(
                "{}  {}  {}",
                row["pageId"].as_str().unwrap_or_default(),
                row["state"].as_str().unwrap_or("unknown"),
                row["title"].as_str().unwrap_or("(untitled)")
            ),
        ));
    }
    tmt_cli_style::detail::write(&mut output, terminal, "WORK", &rows)?;
    Ok(())
}
