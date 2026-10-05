//! Thin native CLI projection and local status publication; no agent dispatch.
use clap::{Arg, ArgMatches, Command};
use serde_json::json;
use std::{
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result, core,
    decoder::Decoder,
    discussion::{self, StatusEdit},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
};
fn id(name: &'static str) -> Arg {
    Arg::new(name).required(true).value_parser(|value: &str| {
        tmt_colab_model::values::generated_id(value)
            .map(|_| value.to_owned())
            .map_err(|e| e.to_string())
    })
}
macro_rules! cmd {
    ($name:literal, $summary:literal, $example:literal, $details:literal) => {
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
    cmd!("threads", "Read page discussions or change a thread's status", "tmt colab threads 10000000-0000-4000-8000-000000000001 --json",
        "Reads authenticated discussions for the current epoch without creating or migrating state. Labels and times are asserted by writers. Resolve and Reopen publish one immutable local-device status action; creation, deletion and comment edits remain writer-owned. Agent CLI actions never notify other agents.")
        .mut_arg("json", |arg| arg.global(true))
        .args_conflicts_with_subcommands(true).subcommand_negates_reqs(true)
        .arg(id("page"))
        .subcommand(cmd!("resolve", "Resolve a live annotation thread", "tmt colab threads resolve 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001",
            "Publishes only status, through the running owner service or its offline lifecycle lock. The caller name is a display label, never authority. Already resolved is a no-op; ambiguous, deleted and Chat threads are refused. No notifications or automatic retry.").arg(id("page")).arg(id("thread")))
        .subcommand(cmd!("reopen", "Reopen a resolved annotation thread", "tmt colab threads reopen 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001",
            "Uses the same immutable status stream and publication fences as Resolve. Already open is a no-op. No notifications or automatic retry.").arg(id("page")).arg(id("thread")))
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (mode, args) = args.subcommand().unwrap_or(("list", args));
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let page_id = args.get_one::<String>("page").expect("required page");
    let catalog = tmt_colab::inspection::catalog(&store, &key)?;
    let ids = catalog["pageIds"]
        .as_array()
        .ok_or(Fault::Invalid)?
        .iter()
        .map(|row| {
            row["pageId"]
                .as_str()
                .map(str::to_owned)
                .ok_or(Fault::Invalid)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let reach = crate::reach::Reach::gather().with_pages(&ids);
    let path = crate::reach::Reach::path(&key.space_id, page_id);

    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if mode == "list" {
        let view = discussion::read(&store, &key, page_id, &mut decoder);
        let closed = store.close();
        let view = view?;
        closed?;
        if json_output {
            let mut value = serde_json::to_value(&view)?;
            reach.annotate(&mut value, &path);
            writeln!(output, "{value}")?;
        } else {
            let mut rows = vec![
                ("page", page_id.clone()),
                ("link", reach.text(&path)),
                ("title", view.conversations.title),
                ("epoch", view.conversations.epoch),
            ];
            for thread in view.conversations.threads {
                let state = if thread.deleted {
                    "deleted"
                } else if thread.resolved {
                    "resolved"
                } else {
                    "open"
                };
                let actor = thread
                    .status
                    .as_ref()
                    .map(|status| {
                        status
                            .action
                            .agent_name
                            .as_deref()
                            .unwrap_or(&status.action.device_name)
                    })
                    .unwrap_or(&thread.device_name);
                let quote = thread
                    .anchor
                    .as_ref()
                    .map(|a| a.exact.as_str())
                    .unwrap_or("Page comment");
                rows.push((
                    "thread",
                    format!("{} · {state} · {actor} · {quote}", thread.id),
                ));
            }
            let terminal = output.terminal();
            tmt_cli_style::detail::write(&mut output, terminal, "PAGE THREADS", &rows)?;
        }
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .try_into()?;
        let agent_name = core::publisher_agent();
        let thread = args.get_one::<String>("thread").expect("required thread");
        let resolved = mode == "resolve";
        let prepared = discussion::prepare_status(
            &store,
            &key,
            page_id,
            StatusEdit {
                thread,
                resolved,
                agent_name: agent_name.as_deref(),
            },
            &mut decoder,
            now,
        );
        let closed = store.close();
        let prepared = prepared?;
        closed?;
        let changed = prepared.is_some();
        let action = if let Some((publication, action)) = prepared {
            page::publish(&layout, &key, &publication, now, &mut decoder)?;
            Some(action)
        } else {
            None
        };
        if json_output {
            let mut value = json!({"spaceId":key.space_id,"pageId":page_id,"threadId":thread,"resolved":resolved,"changed":changed,"action":action});
            reach.annotate(&mut value, &path);
            writeln!(output, "{value}")?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                if resolved {
                    "THREAD RESOLVED"
                } else {
                    "THREAD REOPENED"
                },
                &[
                    ("page", page_id.clone()),
                    ("link", reach.text(&path)),
                    ("thread", thread.clone()),
                    ("changed", changed.to_string()),
                    ("notifications", "none (agent CLI status only)".into()),
                ],
            )?;
        }
    }
    Ok(())
}
