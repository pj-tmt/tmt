//! A page's work: `tmt colab task ...` and `tmt colab summary ...`. Each change is one conditional
//! action through the structured publication owner, on the revision the command just read (or the
//! `--expected-revision` it was given). An action that would put an agent over its work-in-progress
//! limit is refused here, before anything is prepared (`wip.rs`).
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::{Decoder, StructuredKind},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
    work,
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
pub(crate) fn task_arg() -> Arg {
    Arg::new("task")
        .index(2)
        .required(true)
        .value_name("TASK")
        .help("A task ID such as T-3 (see task ls)")
}
fn revision_arg() -> Arg {
    Arg::new("expected-revision")
        .long("expected-revision")
        .value_name("REVISION")
        .value_parser(crate::revision_token)
        .help("Refuse unless the page is at this revision; without it the command uses the current one")
}
fn file_arg(help: &'static str) -> Arg {
    Arg::new("file")
        .long("file")
        .value_name("PATH|-")
        .value_parser(clap::value_parser!(PathBuf))
        .help(help)
}
fn text_arg(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).long(name).value_name("TEXT").help(help)
}
pub fn task_command() -> Command {
    let assign_help = "NAME or agent:NAME for an agent, person:NAME for a person";
    spec(
        "task",
        "Add, change and list the tasks of a page",
        "A task has a title, a state (todo, in-progress, blocked, in-review, done), and optionally detail, an assignee, acceptance criteria, a sign-off flag and an external ID. Task IDs (T-1, T-2, ...) are never reused. An agent holds at most its work-in-progress limit of assigned todo and in-progress tasks across all pages (tmt colab settings wip-limit). Every change applies to the page's current revision, or to --expected-revision when given, and never opens a browser. A page that changed since is COLAB_STALE_BASE: read again. An invalid value refuses the whole change. An uncertain publication is inspected by its original operation, never automatically resent.",
        example!("tmt colab task ls 10000000-0000-4000-8000-000000000001 --json"),
    )
    .subcommand_required(true)
    .subcommand(
        spec(
            "ls",
            "List a page's tasks and where it stands",
            "Prints the page's revision, its stage, its progress and each task. Creates nothing.",
            example!("tmt colab task ls 10000000-0000-4000-8000-000000000001 --json"),
        )
        .alias("list")
        .arg(crate::cli_grammar::page()),
    )
    .subcommand(
        spec(
            "add",
            "Add a task to a page",
            "The new task starts as todo and takes the next task ID, printed as taskId. Give --title (with the optional flags) or the whole task as JSON with --file.",
            example!("tmt colab task add 10000000-0000-4000-8000-000000000001 --title 'Write the guide' --assignee ada --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(revision_arg())
        .arg(text_arg("title", "The task's title, at most 120 characters"))
        .arg(text_arg("detail", "The task's spec, at most 4096 bytes"))
        .arg(text_arg("assignee", assign_help))
        .arg(
            Arg::new("signoff")
                .long("signoff")
                .action(ArgAction::SetTrue)
                .help("The task needs a person's sign-off when it is in review"),
        )
        .arg(text_arg("external-id", "An ID in another tracker"))
        .arg(file_arg("The task as JSON: {\"title\": ..., \"detail\": ..., \"assignee\": ..., \"acceptance\": [...]}"))
        .group(
            clap::ArgGroup::new("source")
                .args(["title", "file"])
                .required(true),
        )
        .mut_arg("detail", |arg| arg.conflicts_with("file"))
        .mut_arg("assignee", |arg| arg.conflicts_with("file"))
        .mut_arg("signoff", |arg| arg.conflicts_with("file"))
        .mut_arg("external-id", |arg| arg.conflicts_with("file")),
    )
    .subcommand(
        spec(
            "set",
            "Replace a task's title, detail, acceptance, sign-off flag and external ID",
            "Takes the whole editable part as JSON with --file: a field the file omits is cleared. State and assignee have their own commands.",
            example!("tmt colab task set 10000000-0000-4000-8000-000000000001 T-1 --file task.json --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(revision_arg())
        .arg(file_arg("{\"title\": ..., \"detail\": ..., \"acceptance\": [...], \"needsSignoff\": true, \"externalId\": ...}").required(true)),
    )
    .subcommand(
        spec(
            "state",
            "Move a task to another state",
            "A blocked task needs --reason; no other state takes one.",
            example!("tmt colab task state 10000000-0000-4000-8000-000000000001 T-1 in-progress --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(
            Arg::new("state")
                .index(3)
                .required(true)
                .value_parser(work::STATES),
        )
        .arg(revision_arg())
        .arg(text_arg("reason", "Why the task is blocked, at most 256 bytes")),
    )
    .subcommand(
        spec(
            "assign",
            "Assign a task, or leave it unassigned",
            "Without an assignee the task is unassigned.",
            example!("tmt colab task assign 10000000-0000-4000-8000-000000000001 T-1 ada --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(
            Arg::new("assignee")
                .index(3)
                .value_name("ASSIGNEE")
                .help(assign_help),
        )
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "link",
            "Make a task follow another page",
            "The task's state then follows that page: todo while the page has no task, in-progress while it has, done when all of its tasks are (in-review instead when the task needs sign-off). A page has at most one parent, and sub-pages nest to any depth. A loop or an existing invalid hierarchy is refused (COLAB_LINK_REFUSED with a stable reason). Inspect legacy links with tmt colab show PAGE --json and repair them with task unlink. A task with an external ID cannot follow a page, and the state of a task that follows one cannot be set.",
            example!("tmt colab task link 10000000-0000-4000-8000-000000000001 T-3 20000000-0000-4000-8000-000000000001 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(
            Arg::new("child")
                .index(3)
                .required(true)
                .value_name("CHILD")
                .help("The page this task follows, named as any page is"),
        )
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "unlink",
            "Stop a task following its page",
            "The task shows its own state again. Removing a link can repair an invalid hierarchy, even if restoring the task exceeds the work-in-progress limit. The child page is not changed.",
            example!("tmt colab task unlink 10000000-0000-4000-8000-000000000001 T-3 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(revision_arg()),
    )
    .subcommand(
        spec(
            "convert",
            "Turn a task into a page of its own and follow it",
            "Refuses a child page or an existing invalid hierarchy before creating anything (COLAB_LINK_REFUSED with a stable reason). Otherwise creates a new page titled like the task, with the task's detail as its text, then makes the task follow it. If the link fails the page already exists: the command prints it and the tmt colab task link command that finishes the job. It never opens a browser.",
            example!("tmt colab task convert 10000000-0000-4000-8000-000000000001 T-3 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg()),
    )
    .subcommand(
        spec(
            "rm",
            "Remove a task",
            "The task's ID is never handed out again.",
            example!("tmt colab task rm 10000000-0000-4000-8000-000000000001 T-1 --json"),
        )
        .arg(crate::cli_grammar::page())
        .arg(task_arg())
        .arg(revision_arg()),
    )
    .subcommand(crate::cli_work_github::command())
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
fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}

/// `NAME` and `agent:NAME` are an agent, `person:NAME` a person.
fn assignee(value: &str) -> Value {
    match value.split_once(':') {
        Some(("person", name)) => json!({"kind":"person","name":name}),
        Some(("agent", name)) => json!({"kind":"agent","name":name}),
        _ => json!({"kind":"agent","name":value}),
    }
}
fn body_from_file(args: &ArgMatches) -> Result<Value> {
    let source = crate::page_source(args.get_one::<PathBuf>("file").expect("file checked"))?;
    if source.len() > tmt_colab::decoder::UPDATE_BYTES {
        return Err(Fault::Invalid.into());
    }
    serde_json::from_str(&source).map_err(|_| invalid("The file must contain one JSON object."))
}
/// The closed action and body the command asks for.
fn action(group: &str, name: &str, args: &ArgMatches) -> Result<(&'static str, Value)> {
    let task = || {
        args.get_one::<String>("task")
            .expect("required task")
            .clone()
    };
    Ok(match (group, name) {
        ("summary", "set") => (
            "summary.set",
            json!({"line1":args.get_one::<String>("line1"),"line2":args.get_one::<String>("line2")}),
        ),
        ("summary", _) => ("summary.clear", json!({})),
        (_, "add") => {
            let task = if args.get_one::<PathBuf>("file").is_some() {
                body_from_file(args)?
            } else {
                let mut task = json!({"title":args.get_one::<String>("title")});
                if let Some(detail) = args.get_one::<String>("detail") {
                    task["detail"] = json!(detail);
                }
                if let Some(who) = args.get_one::<String>("assignee") {
                    task["assignee"] = assignee(who);
                }
                if args.get_flag("signoff") {
                    task["needsSignoff"] = json!(true);
                }
                if let Some(id) = args.get_one::<String>("external-id") {
                    task["externalId"] = json!(id);
                }
                task
            };
            ("task.add", json!({"task":task}))
        }
        (_, "set") => {
            let mut body = body_from_file(args)?;
            body.as_object_mut()
                .ok_or_else(|| invalid("The file must contain one JSON object."))?
                .insert("taskId".into(), json!(task()));
            ("task.set", body)
        }
        (_, "state") => {
            let mut body = json!({"taskId":task(),"state":args.get_one::<String>("state")});
            if let Some(reason) = args.get_one::<String>("reason") {
                body["reason"] = json!(reason);
            }
            ("task.state", body)
        }
        (_, "link") => (
            "task.link",
            json!({"taskId":task(),"pageId":args.get_one::<String>("child").expect("required child")}),
        ),
        (_, "unlink") => ("task.unlink", json!({"taskId":task()})),
        (_, "assign") => {
            let mut body = json!({"taskId":task()});
            if let Some(who) = args.get_one::<String>("assignee") {
                body["assignee"] = assignee(who);
            }
            ("task.assign", body)
        }
        _ => ("task.remove", json!({"taskId":task()})),
    })
}

fn short(page: &str) -> &str {
    page.get(..8).unwrap_or(page)
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
fn link_refusal(
    why: &Refusal,
    page: &str,
    task: &str,
    child: &str,
) -> Box<dyn std::error::Error + Send + Sync> {
    fault(work_admission::link_refusal(why, page, task, child))
}

/// A work action that was written: the receipt and the work it was based on.
pub(crate) struct Written {
    pub receipt: page::Receipt,
    pub base: page::WorkPage,
}

/// One conditional work action on a page, through the structured publication owner: on the revision
/// it just read, or on `expected`. This is the whole of a write command's effect; a caller that must
/// reach out first (the GitHub driver) reads, reaches out, then calls this.
pub(crate) fn write_work(
    root: &Path,
    page: &str,
    action: &'static str,
    body: &Value,
    expected: Option<&str>,
) -> Result<Written> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let lock = crate::write_owner(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(&store, &key, page)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let read = page::read_work(&store, &key, &page_id, &mut decoder)?;
    // A page named by anything but its ID is read the way every command reads one; a page that is
    // not there is refused as a link would be.
    let linked;
    let body = if action == "task.link" {
        let child = body["pageId"].as_str().unwrap_or_default();
        let id = resolve_page(&store, &key, child)
            .map_err(|_| link_refusal(&Refusal::Unavailable, &page_id, "", child))?;
        linked = json!({"taskId":body["taskId"],"pageId":id});
        &linked
    } else {
        body
    };
    // Whatever the command read is the base unless the caller named one.
    let expected = expected.unwrap_or(&read.revision).to_owned();
    work_admission::admit(
        &Space {
            root,
            layout: &layout,
            store: &store,
            key: &key,
        },
        &mut decoder,
        &page_id,
        &read.work,
        action,
        body,
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
            body,
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
    let receipt = crate::publish_structured_owned(
        &layout,
        &key,
        &page_id,
        prepared,
        &mut decoder,
        now,
        lock.as_ref(),
    )?;
    Ok(Written {
        receipt,
        base: read,
    })
}

pub fn run(root: &Path, group: &str, args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required work command");
    if group == "task" && name == "github" {
        return crate::cli_work_github::run(root, args);
    }
    if group == "task" && name == "ls" {
        return read_and_list(root, args);
    }
    if group == "task" && name == "convert" {
        return convert(root, args);
    }
    let (action, body) = action(group, name, args)?;
    let written = write_work(
        root,
        args.get_one::<String>("page").expect("required page"),
        action,
        &body,
        args.get_one::<String>("expected-revision")
            .map(String::as_str),
    )?;
    let receipt = written.receipt;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    // A new task's ID is the one the page was about to hand out: this command holds the base.
    let created = (action == "task.add" && receipt.changed).then(|| {
        format!(
            "T-{}",
            written.base.work["next"].as_u64().unwrap_or_default()
        )
    });
    if json_output {
        let mut value = serde_json::to_value(&receipt)?;
        if let Some(id) = &created {
            value["taskId"] = json!(id);
        }
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        let mut rows = vec![
            ("page", receipt.page_id.clone()),
            ("revision", receipt.revision.clone()),
        ];
        if let Some(id) = created {
            rows.insert(1, ("task", id));
        }
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if receipt.changed {
                "WORK UPDATED"
            } else {
                "WORK UNCHANGED"
            },
            &rows,
        )?;
    }
    Ok(())
}

/// A page's work read without a write lock: the page's ID and what it holds now.
pub(crate) fn read_page_work(root: &Path, page: &str) -> Result<(String, page::WorkPage)> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(&store, &key, page)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let current = page::read_work(&store, &key, &page_id, &mut decoder);
    let closed = store.close();
    let read = current?;
    closed?;
    Ok((page_id, read))
}

/// `read` with every task that follows a page showing that page's stage. The space's pages are read
/// only when this page has such a task.
fn follow_pages(root: &Path, page_id: &str, read: &mut page::WorkPage) -> Result<()> {
    let linked = read.work["tasks"]
        .as_object()
        .is_some_and(|tasks| tasks.values().any(|task| task.get("child").is_some()));
    if !linked {
        return Ok(());
    }
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let found = tmt_colab::inspection::all_page_ids(&store, &key).and_then(|pages| {
        work_index::scan_with_live(&layout, &store, &key, &mut decoder, &pages, Some(&pages))
    });
    let closed = store.close();
    let found = found?;
    closed?;
    if let Some(derived) = Graph(&found).derived(page_id) {
        read.derived = derived;
    }
    Ok(())
}

/// `task ls`: a read, which takes no write lock and creates nothing.
fn read_and_list(root: &Path, args: &ArgMatches) -> Result<()> {
    let (page_id, mut read) =
        read_page_work(root, args.get_one::<String>("page").expect("required page"))?;
    follow_pages(root, &page_id, &mut read)?;
    list(&page_id, &read, args.get_flag("json"))
}

/// `task convert`: a new page made from the task, then the link. The page is created first, so a link
/// that fails leaves a page and says how to finish; it never searches for one to adopt.
fn convert(root: &Path, args: &ArgMatches) -> Result<()> {
    let page = args.get_one::<String>("page").expect("required page");
    let task_id = args.get_one::<String>("task").expect("required task");
    let (page_id, read) = read_page_work(root, page)?;
    // Refuse what the link would refuse before a page exists.
    let task = tmt_colab::work_convert::convertible(&read.work, task_id)
        .map_err(|blocked| invalid(&blocked.message(task_id)))?;
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let admitted = work_admission::admit_conversion(
        &Space {
            root,
            layout: &layout,
            store: &store,
            key: &key,
        },
        &mut decoder,
        &page_id,
        task_id,
    );
    let closed = store.close();
    admitted.map_err(fault)?;
    closed?;
    let title = task["title"].as_str().unwrap_or_default();
    let created = crate::cli_management::commit_creation(
        root,
        title,
        tmt_colab::work_convert::child_source(title, task["detail"].as_str().unwrap_or_default()),
        Some(&tmt_colab::decoder::PageLayout {
            kind: tmt_colab::decoder::LayoutKind::Column,
            reason: None,
        }),
        false,
    )?;
    let child = created.page_id;
    let finish = format!(
        "The page {child} was created from {task_id}; finish with: tmt colab task link {page_id} {task_id} {child}"
    );
    // Making a page moves the space, so the revision read before is stale. What matters is the task:
    // the link is written on the revision read now, and only if the task is still the one copied.
    let (_, now) = read_page_work(root, &page_id)?;
    if now.work["tasks"].get(task_id.as_str()) != Some(task) {
        return Err(invalid(&format!(
            "{task_id} changed while its page was being made, so it was not linked. {finish}"
        )));
    }
    let written = write_work(
        root,
        &page_id,
        "task.link",
        &json!({"taskId":task_id,"pageId":child}),
        Some(&now.revision),
    )
    .map_err(|error| {
        Box::new(crate::cli_management::ManagementFault {
            code: crate::error_code(error.as_ref()),
            message: format!("{error} {finish}"),
            correlation: json!({}),
            source: Some(error),
        }) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let receipt = written.receipt;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let reach = crate::reach::Reach::gather();
    let path = crate::reach::Reach::path(&read.space_id, &child);
    if json_output {
        let mut value = serde_json::to_value(&receipt)?;
        value["taskId"] = json!(task_id);
        value["childPage"] = json!(child);
        value["title"] = json!(title);
        reach.annotate(&mut value, &path);
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "TASK CONVERTED",
            &[
                ("page", page_id),
                ("task", task_id.clone()),
                ("child page", child),
                ("link", reach.text(&path)),
                ("revision", receipt.revision.clone()),
            ],
        )?;
    }
    Ok(())
}

fn list(page_id: &str, read: &page::WorkPage, json_output: bool) -> Result<()> {
    let reach = crate::reach::Reach::gather();
    let path = crate::reach::Reach::path(&read.space_id, page_id);
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = serde_json::to_value(read)?;
        reach.annotate(&mut value, &path);
        writeln!(output, "{value}")?;
        return Ok(());
    }
    let terminal = output.terminal();
    let derived = &read.derived;
    let mut rows = vec![
        ("page", page_id.to_owned()),
        ("link", reach.text(&path)),
        ("title", read.title.clone()),
        ("revision", read.revision.clone()),
        (
            "stage",
            format!(
                "{} ({}/{} done)",
                derived["stage"].as_str().unwrap_or_default(),
                derived["done"],
                derived["total"]
            ),
        ),
    ];
    if let Some(summary) = read.work.get("summary") {
        rows.push((
            "summary",
            format!(
                "{} / {}",
                summary["line1"].as_str().unwrap_or_default(),
                summary["line2"].as_str().unwrap_or_default()
            ),
        ));
    }
    for id in read.work["order"].as_array().into_iter().flatten() {
        let id = id.as_str().unwrap_or_default();
        let task = &read.work["tasks"][id];
        let state = derived["states"][id]
            .as_str()
            .or(task["state"].as_str())
            .unwrap_or_default();
        let follows = task["child"]
            .as_str()
            .map_or(String::new(), |child| format!("  follows {}", short(child)));
        rows.push((
            "task",
            format!(
                "{id}  {state}{follows}  {}{}  {}",
                task["assignee"]["name"].as_str().unwrap_or("-"),
                if task["needsSignoff"] == true {
                    "  signoff"
                } else {
                    ""
                },
                task["title"].as_str().unwrap_or_default()
            ),
        ));
    }
    tmt_cli_style::detail::write(&mut output, terminal, "WORK", &rows)?;
    Ok(())
}
