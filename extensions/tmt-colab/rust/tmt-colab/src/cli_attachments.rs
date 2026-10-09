//! `tmt colab attachment read`: one verified attachment from the running serve, written to a
//! private output directory. The CLI holds no object channel; the serve admits and discloses.
use clap::{Arg, ArgMatches, Command};
use serde_json::json;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    export::{Bundle, Fault},
    keyring::{Keyring, Layout},
    store::Store,
};
use tmt_colab_model::{attachment::AttachmentSelector, crypto};

const DISCLOSURE: &str =
    "This writes an unencrypted copy of the file. Anyone with these files can read it.";
const FILE: &str = "attachment.bin";

pub fn command() -> Command {
    tmt_cli_style::command(&CommandSpec {
        name: "attachment",
        summary: "Read page attachments",
        examples: &[Example {
            command: "tmt colab attachment read 10000000-0000-4000-8000-000000000001 --reference reference.json --json",
            note: "Write one attachment into a private output directory",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Root-local access through the running serve; it never opens storage itself.",
    })
    .subcommand_required(true)
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "read",
            summary: "Write one verified attachment into a private directory",
            examples: &[Example {
                command: "tmt colab attachment read 10000000-0000-4000-8000-000000000001 --reference reference.json --output . --json",
                note: "Create a UUID-named directory holding attachment.bin and manifest.json",
            }],
            outputs: OutputModes::HumanAndJson,
            details: "This writes an unencrypted copy of the file. Anyone with these files can read it.\nThe reference file is the exact `reference` of an attachments entry in an export manifest, so a changed page or ended access reads as unavailable rather than another version. The running serve checks the reference against the current page, access and epoch, then verifies the bytes before they are written. Output is a new private UUID-named subdirectory of --output (default: current directory) with attachment.bin and manifest.json, created the way export does. The result lists the size and SHA-256, never the bytes. Requires tmt colab serve; nothing is read without it.",
        })
        .arg(crate::cli_grammar::page())
        .arg(
            Arg::new("reference")
                .long("reference")
                .required(true)
                .value_name("file")
                .value_parser(clap::value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .value_name("directory")
                .value_parser(clap::value_parser!(PathBuf)),
        ),
    )
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (_, args) = args.subcommand().expect("required attachment command");
    let layout = Layout::existing(root)?.ok_or(Fault::MissingState)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page = crate::cli_management::resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    );
    let closed = store.close();
    let page = page?;
    closed?;
    let reference = reference(args.get_one::<PathBuf>("reference").expect("required"))?;
    let json_output = args.get_flag("json");
    if !json_output {
        let mut warning = tmt_cli_style::stream::stderr();
        let terminal = warning.terminal();
        tmt_cli_style::detail::write(
            &mut warning,
            terminal,
            "PLAINTEXT FILE",
            &[("disclosure", DISCLOSURE.into())],
        )?;
    }
    let bytes = tmt_colab::attachments::ipc::read(&layout, &page, &reference)?;
    let sha256: String = crypto::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let manifest = serde_json::to_vec(&json!({
        "format": "tmt-colab-attachment-read",
        "version": 1,
        "pageId": page,
        "reference": reference,
        "file": {"name": FILE, "sizeBytes": bytes.len(), "sha256": sha256},
    }))?;
    let size = bytes.len();
    let bundle = Bundle::from_files(
        DISCLOSURE,
        vec![(FILE.into(), bytes), ("manifest.json".into(), manifest)],
    )?;
    let parent = args
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or(std::env::current_dir()?);
    let published = bundle.publish(&parent)?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = serde_json::to_value(&published)?;
        value["pageId"] = json!(page);
        value["reference"] = serde_json::to_value(&reference)?;
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "ATTACHMENT READ",
            &[
                ("page", page),
                ("directory", published.directory.display().to_string()),
                ("bytes", size.to_string()),
                ("sha256", sha256),
            ],
        )?;
    }
    Ok(())
}
/// The exact selector the file holds, bounded before it is parsed.
fn reference(path: &Path) -> Result<AttachmentSelector> {
    let mut raw = Vec::new();
    std::fs::File::open(path)?
        .take(tmt_colab_model::attachment::REFERENCE_BYTES as u64 + 1)
        .read_to_end(&mut raw)?;
    Ok(AttachmentSelector::from_json(&raw)?)
}
