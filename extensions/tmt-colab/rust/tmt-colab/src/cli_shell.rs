//! Structured shell commands: conditional actions through the existing page publication owner.
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
};

const ACTIONS: &[(&str, &str, &str)] = &[
    ("item-add", "item.add", "Add a reference to a shell"),
    (
        "item-remove",
        "item.remove",
        "Remove a reference, keeping its source page",
    ),
    ("item-move", "item.move", "Move a reference within a shell"),
    ("item-set", "item.set", "Replace one whole reference"),
    (
        "summary-set",
        "summary.set",
        "Set the shell's two summary lines",
    ),
    (
        "summary-clear",
        "summary.clear",
        "Clear the shell's summary",
    ),
    (
        "reading-suggest",
        "reading.suggest",
        "Add a suggested reading without marking it read",
    ),
];
fn spec(name: &'static str, summary: &'static str, details: &'static str) -> Command {
    let examples: &'static [Example] = match name {
        "create" => &[Example {
            command: "tmt colab shell create --title 'Release notes' --json",
            note: "Run this shell command",
        }],
        "item-add" => &[Example {
            command: "tmt colab shell item-add 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file item-add.json --json",
            note: "Run this shell command",
        }],
        "item-remove" => &[Example {
            command: "tmt colab shell item-remove 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file item-remove.json --json",
            note: "Run this shell command",
        }],
        "item-move" => &[Example {
            command: "tmt colab shell item-move 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file item-move.json --json",
            note: "Run this shell command",
        }],
        "item-set" => &[Example {
            command: "tmt colab shell item-set 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file item-set.json --json",
            note: "Run this shell command",
        }],
        "summary-set" => &[Example {
            command: "tmt colab shell summary-set 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file summary.json --json",
            note: "Run this shell command",
        }],
        "summary-clear" => &[Example {
            command: "tmt colab shell summary-clear 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --json",
            note: "Run this shell command",
        }],
        "reading-suggest" => &[Example {
            command: "tmt colab shell reading-suggest 10000000-0000-4000-8000-000000000001 --expected-revision v1:0000000000000000000000000000000000000000000000000000000000000000 --file reading.json --json",
            note: "Run this shell command",
        }],
        _ => &[Example {
            command: "tmt colab shell read 10000000-0000-4000-8000-000000000001 --json",
            note: "Run this shell command",
        }],
    };
    tmt_cli_style::command(&CommandSpec {
        name,
        summary,
        outputs: OutputModes::HumanAndJson,
        examples,
        details,
    })
}
pub fn command() -> Command {
    let mut shell = spec("shell", "Read and change structured shell pages",
        "Shells contain references and two summary lines, not HTML. A space may have many shells; none is its default or Home. Changes require the revision shell read returned. Nothing opens a browser.")
        .subcommand_required(true)
        .subcommand(spec("create", "Create an empty shell page", "Creates an owner page through ordinary page creation. Never opens a browser.")
            .arg(Arg::new("title").long("title").required(true).value_name("TITLE")))
        .subcommand(spec("read", "Read a shell and its current revision", "Returns the verified structured value, identity, epoch and revision without creating state.")
            .arg(crate::cli_grammar::page()));
    for &(name, _, summary) in ACTIONS {
        let mut action = spec(name, summary,
            "Applies one closed JSON action body to the exact revision you read. Unknown fields, invalid references or limits refuse the whole action. A changed base is COLAB_STALE_BASE; read again. An uncertain publication is inspected by its original operation, never automatically resent.")
            .arg(crate::cli_grammar::page())
            .arg(Arg::new("expected-revision").long("expected-revision").required(true)
                .value_name("REVISION").value_parser(crate::revision_token));
        if name != "summary-clear" {
            action = action.arg(
                Arg::new("file")
                    .long("file")
                    .required(true)
                    .value_name("PATH|-")
                    .value_parser(clap::value_parser!(PathBuf)),
            );
        }
        shell = shell.subcommand(action);
    }
    shell
}
fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required shell command");
    if name == "create" {
        return crate::cli_management::create_shell(root, args);
    }
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let lock = if name == "read" {
        None
    } else {
        crate::write_owner(&layout)?
    };
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    if name == "read" {
        let result = page::read_shell(&store, &key, &page_id, &mut decoder);
        let closed = store.close();
        let view = result?;
        closed?;
        let reach = crate::reach::Reach::gather();
        let path = crate::reach::Reach::path(&key.space_id, &page_id);
        let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
        if args.get_flag("json") {
            let mut value = serde_json::to_value(&view)?;
            reach.annotate(&mut value, &path);
            writeln!(output, "{value}")?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                "SHELL PAGE",
                &[
                    ("page", page_id),
                    ("link", reach.text(&path)),
                    ("title", view.title),
                    ("revision", view.revision),
                    ("epoch", view.epoch),
                    (
                        "shell",
                        String::from_utf8(
                            tmt_colab::closed::canonical(&view.shell)
                                .map_err(|_| Fault::Invalid)?,
                        )?,
                    ),
                ],
            )?;
        }
        return Ok(());
    }
    let action = ACTIONS
        .iter()
        .find(|row| row.0 == name)
        .ok_or(Fault::Invalid)?
        .1;
    let body: Value = if name == "summary-clear" {
        json!({})
    } else {
        let source = crate::page_source(
            args.get_one::<PathBuf>("file")
                .expect("required action file"),
        )?;
        if source.len() > tmt_colab::decoder::UPDATE_BYTES {
            return Err(Fault::Invalid.into());
        }
        serde_json::from_str(&source).map_err(|_| {
            fail(
                "COLAB_INPUT_INVALID",
                "The action file must contain a JSON object.",
            )
        })?
    };
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
            kind: tmt_colab::decoder::StructuredKind::Shell,
            action,
            body: &body,
            display_agent,
            expected_revision: args
                .get_one::<String>("expected-revision")
                .expect("required revision"),
            operation_id: None,
        },
        &mut decoder,
        &now,
    );
    let closed = store.close();
    let prepared = prepared?;
    closed?;
    let receipt = match prepared {
        page::PublicationPreparation::Noop {
            epoch,
            membership_head,
            base_revision,
            memory_limit,
        } => page::Receipt::unchanged(
            &key,
            &page_id,
            "",
            epoch,
            membership_head,
            base_revision,
            memory_limit,
        ),
        page::PublicationPreparation::Write(frozen) => {
            let published = crate::publish_write_owned(
                &layout,
                &key,
                &page_id,
                &frozen,
                &mut decoder,
                now()?,
                lock.as_ref(),
            )?;
            let mut receipt = page::publication_receipt(frozen.job(), &published.record)?;
            if let Some(revision) = published.revision {
                receipt.revision = revision;
            }
            receipt
        }
    };
    crate::print_receipt(receipt, &[], args.get_flag("json"), "SHELL UPDATED")
}
