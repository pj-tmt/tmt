//! `tmt colab attachment read`: one verified attachment from the running serve, written to a
//! private output directory. The CLI holds no object channel; the serve admits and discloses.
use crate::cli_management;
use clap::{Arg, ArgMatches, Command};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    attachments::ipc::Verified,
    export::{self, Bundle},
    keyring::{Keyring, Layout},
    page::Fault,
    store::Store,
};
use tmt_colab_model::{attachment::AttachmentSelector, crypto};

const DISCLOSURE: &str =
    "This writes an unencrypted copy of the file. Anyone with these files can read it.";

/// The extension a verified media type earns the written file, so an agent can open it as what it
/// is. Only this allow-list is ever consulted: the author's file name and anything else about the
/// attachment never reaches the path, and an unlisted type is `bin`.
fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "application/json" => "json",
        "text/csv" => "csv",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

pub fn command() -> Command {
    tmt_cli_style::command(&CommandSpec {
        name: "attachment",
        summary: "Read or add page attachments",
        examples: &[Example {
            command: "tmt colab attachment read 10000000-0000-4000-8000-000000000001 20000000 --json",
            note: "Write one attachment into a private output directory",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Root-local access through the running serve; it never opens storage itself. `read` writes an attachment out; `attach` adds a file to the page.",
    })
    .subcommand_required(true)
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "read",
            summary: "Write one verified attachment into a private directory",
            examples: &[Example {
                command: "tmt colab attachment read 10000000-0000-4000-8000-000000000001 20000000 --output . --json",
                note: "Create a UUID-named directory holding attachment.<ext> and manifest.json",
            }],
            outputs: OutputModes::HumanAndJson,
            details: "This writes an unencrypted copy of the file. Anyone with these files can read it.\nName the attachment by the ID a request or `tmt colab threads` lists (the first 8 or more hex characters are enough when they match one file), or with --reference. An ID that matches nothing live, or more than one file, reads as unavailable or invalid; use the full ID from `threads --json` to disambiguate. The reference file is the exact `reference` of an attachments entry in an export manifest, so a changed page or ended access reads as unavailable rather than another version. The running serve checks the reference against the current page, access and epoch, then verifies the bytes before they are written. Output is a new private UUID-named subdirectory of --output (default: the system temporary directory, never the current directory) with attachment.<ext> and manifest.json, created the way export does. The extension comes from the attachment's verified media type (png, jpg, gif, webp, pdf, txt, md, json, csv, mp4; anything else bin); the author's file name is never used. --json prints the written file as `path`; read that file, and delete its directory when you are done. The result lists the size and SHA-256, never the bytes. Requires tmt colab serve; nothing is read without it.",
        })
        .arg(crate::cli_grammar::page())
        .arg(
            Arg::new("attachment")
                .index(2)
                .value_name("attachment")
                .help("Attachment ID, or its first 8 or more hex characters")
                .required_unless_present("reference")
                .conflicts_with("reference"),
        )
        .arg(
            Arg::new("reference")
                .long("reference")
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
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "attach",
            summary: "Add one file to the page as an attachment",
            examples: &[
                Example {
                    command: "tmt colab attachment attach 10000000-0000-4000-8000-000000000001 ./diagram.png --json",
                    note: "Seal and upload the file, then list it on the page",
                },
                Example {
                    command: "tmt colab attachment attach 10000000-0000-4000-8000-000000000001 --resume SLOT",
                    note: "Finish an attach whose reply was lost, without a second upload",
                },
            ],
            outputs: OutputModes::HumanAndJson,
            details: "Requires a running serve, an editable page and an established object channel.\nPair the browser with tmt remote pair, then open the page once to establish the channel.\nThe serve copies the file into a private slot, seals it with this device's writer key, uploads it and lists it on the page.\nThe type defaults from the file extension and is only a label. Files over 8 MiB are refused.\nIf the reply is lost, rerun with --resume SLOT using the printed slot. It checks the backend before sending and never uploads twice.\nIf the page changed after the file was sealed, the uploaded original and slot are discarded. Nothing is listed; run attach again for a new slot.\n--json prints the exact reference that attachment read takes.",
        })
        .arg(crate::cli_grammar::page())
        .arg(
            Arg::new("file")
                .index(2)
                .value_name("file")
                .value_parser(clap::value_parser!(PathBuf))
                .required_unless_present("resume")
                .conflicts_with("resume"),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .value_name("media-type")
                .help("Label such as image/png; defaults from the file extension")
                .conflicts_with("resume"),
        )
        .arg(
            Arg::new("resume")
                .long("resume")
                .value_name("slot")
                .help("Finish the attach of an earlier slot"),
        ),
    )
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (name, args) = args.subcommand().expect("required attachment command");
    if name == "attach" {
        return attach(root, args);
    }
    let layout = Layout::existing(root)?.ok_or(export::Fault::MissingState)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page = crate::cli_management::resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    );
    // An ID only finds the exact reference in the verified local view; the serve's read below
    // still checks that reference against current access and epoch.
    let found = match (&page, args.get_one::<String>("attachment")) {
        (Ok(page), Some(id)) => {
            let mut decoder = tmt_colab::decoder::Decoder::new(std::env::current_exe()?)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            Some(tmt_colab::attachments::resolve(
                &store,
                &key,
                page,
                id,
                &mut decoder,
                deadline,
            ))
        }
        _ => None,
    };
    let closed = store.close();
    let page = page?;
    closed?;
    let reference = match found {
        Some(selector) => selector?,
        None => reference(args.get_one::<PathBuf>("reference").expect("required"))?,
    };
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
    let Verified { bytes, media_type } =
        tmt_colab::attachments::ipc::read(&layout, &page, &reference)?;
    let file_name = format!("attachment.{}", extension(&media_type));
    let sha256: String = crypto::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let manifest = serde_json::to_vec(&json!({
        "format": "tmt-colab-attachment-read",
        "version": 1,
        "pageId": page,
        "reference": reference,
        "file": {"name": file_name, "mediaType": media_type, "sizeBytes": bytes.len(), "sha256": sha256},
    }))?;
    let size = bytes.len();
    let bundle = Bundle::from_files(
        DISCLOSURE,
        vec![
            (file_name.clone(), bytes),
            ("manifest.json".into(), manifest),
        ],
    )?;
    let parent = args
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(std::env::temp_dir);
    let published = bundle.publish(&parent)?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = serde_json::to_value(&published)?;
        value["pageId"] = json!(page);
        value["path"] = json!(published.directory.join(&file_name));
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
                ("file", file_name),
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
    // A reference that cannot be read or parsed is invalid input, never a serve problem.
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(tmt_colab_model::attachment::REFERENCE_BYTES as u64 + 1)
                .read_to_end(&mut raw)
        })
        .map_err(|_| Fault::Invalid)?;
    Ok(AttachmentSelector::from_json(&raw).map_err(|_| Fault::Invalid)?)
}

/// `tmt colab attachment attach`: stage, then let the serve seal, upload and list. The slot is
/// named before anything is sent, so a lost reply is resumed by explicit rerun.
fn attach(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(export::Fault::MissingState)?;
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
    let json_output = args.get_flag("json");
    let (slot, outcome) = match args.get_one::<String>("resume") {
        Some(slot) => (
            slot.clone(),
            tmt_colab::attachments::attach_ipc::attach(&layout, &page, slot, None)
                .map_err(|error| explain(error, true)),
        ),
        None => {
            let file = args.get_one::<PathBuf>("file").expect("required file");
            let (name, kind) = labels(file, args.get_one::<String>("type"))?;
            // Refuse a file that cannot be attached before any slot exists.
            let source = open_source(file)?;
            let staged = tmt_colab::attachments::attach_ipc::stage(&layout, &page, &name, &kind)
                .map_err(|error| explain(error, false))?;
            // Named before the copy and the upload: every failure after this can be resumed.
            let mut notice = tmt_cli_style::stream::stderr();
            writeln!(
                notice,
                "slot {} (if this is interrupted: tmt colab attachment attach {page} --resume {})",
                staged.slot, staged.slot
            )?;
            let digest = stream_into(source, Path::new(&staged.path))?;
            let outcome = tmt_colab::attachments::attach_ipc::attach(
                &layout,
                &page,
                &staged.slot,
                Some(digest),
            )
            .map_err(|error| explain(error, false));
            (staged.slot, outcome)
        }
    };
    let attached = match outcome {
        Ok(attached) => attached,
        Err(error) => {
            if error
                .downcast_ref::<cli_management::ManagementFault>()
                .is_some_and(|fault| fault.code == Fault::Unavailable.code())
            {
                let mut notice = tmt_cli_style::stream::stderr();
                writeln!(
                    notice,
                    "The outcome is uncertain. Rerun to finish without a second upload: tmt colab attachment attach {page} --resume {slot}"
                )?;
            }
            return Err(error);
        }
    };
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(
            output,
            "{}",
            json!({"pageId": page, "slot": slot, "attachment": attached})
        )?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "ATTACHMENT ATTACHED",
            &[
                ("page", page),
                ("attachment", attached.attachment_id),
                ("file", attached.filename),
                ("bytes", attached.plaintext_bytes.to_string()),
            ],
        )?;
    }
    Ok(())
}
/// The label shown for a file and the type it is sent with. Neither grants a preview.
fn labels(file: &Path, kind: Option<&String>) -> Result<(String, String)> {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            crate::cli_management::fail(
                "COLAB_INPUT_INVALID",
                "The file name is not valid text; rename the file.",
            )
        })?
        .to_owned();
    let kind = match kind {
        Some(kind) => kind.clone(),
        None => match file
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("pdf") => "application/pdf",
            Some("txt" | "log") => "text/plain",
            Some("md") => "text/markdown",
            Some("json") => "application/json",
            Some("csv") => "text/csv",
            Some("mp4") => "video/mp4",
            _ => "application/octet-stream",
        }
        .to_owned(),
    };
    tmt_colab_model::attachment::validate_labels(&name, &kind).map_err(|_| {
        crate::cli_management::fail(
            "COLAB_INPUT_INVALID",
            "The file name (at most 255 bytes, no control characters) or --type (like image/png) is not a valid attachment label.",
        )
    })?;
    Ok((name, kind))
}
/// The file to attach, opened without following a link: only a regular file within
/// the size limit. Checked before any slot exists, and again as the bytes stream.
fn open_source(from: &Path) -> Result<std::fs::File> {
    let source = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
        .open(from)
        .map_err(|_| not_attachable())?;
    let metadata = source.metadata()?;
    if !metadata.is_file() {
        return Err(not_attachable());
    }
    if metadata.len() > tmt_colab_model::attachment::PLAINTEXT_BYTES as u64 {
        return Err(too_large());
    }
    Ok(source)
}
/// Copy the open file into the serve's staging file, never following a link there and bounded
/// while it streams: a file that grows past the limit mid-copy is refused, and a copy that fails
/// leaves nothing behind. Returns the SHA-256 of the copied bytes.
fn stream_into(mut source: std::fs::File, to: &Path) -> Result<[u8; 32]> {
    const LIMIT: u64 = tmt_colab_model::attachment::PLAINTEXT_BYTES as u64;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits())
        .open(to)
        .map_err(|_| Fault::Unavailable)?;
    let copied = (|| -> Result<[u8; 32]> {
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut chunk = [0u8; 64 * 1024];
        loop {
            let read = source.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > LIMIT {
                return Err(too_large());
            }
            hash.update(&chunk[..read]);
            target.write_all(&chunk[..read])?;
        }
        target.sync_all()?;
        Ok(hash.finalize().into())
    })();
    if copied.is_err() {
        let _ = std::fs::remove_file(to);
    }
    copied
}
fn not_attachable() -> Box<dyn std::error::Error + Send + Sync> {
    crate::cli_management::fail(
        "COLAB_INPUT_INVALID",
        "The file must be a regular file you own and can read: not missing, a link or a directory.",
    )
}
fn too_large() -> Box<dyn std::error::Error + Send + Sync> {
    crate::cli_management::fail(
        "COLAB_CAPACITY",
        "The file is larger than an attachment can be: at most 8 MiB.",
    )
}
/// The serve's refusal in the words of an attach. The codes stay the shared ones; only the text
/// says what they mean here, where "the write" and "the page source" would mislead.
fn explain(
    error: Box<dyn std::error::Error + Send + Sync>,
    resuming: bool,
) -> Box<dyn std::error::Error + Send + Sync> {
    let code = |fault: &Fault| fault.code();
    let Some(fault) = error.downcast_ref::<Fault>().copied().or_else(|| {
        error
            .downcast_ref::<tmt_colab::page::ipc::WriteError>()
            .and_then(|failure| {
                [
                    Fault::StaleBase,
                    Fault::Invalid,
                    Fault::Capacity,
                    Fault::Missing,
                    Fault::Inactive,
                    Fault::Denied,
                    Fault::Unavailable,
                ]
                .into_iter()
                .find(|fault| fault.code() == failure.code())
            })
    }) else {
        return error;
    };
    let message = match fault {
        Fault::Missing if resuming => {
            "No such slot for this page: it may belong to another page or have expired."
        }
        Fault::Invalid if resuming => {
            "That slot never received its file; run attach again with the file."
        }
        Fault::Missing => "The page has no state to attach to.",
        Fault::StaleBase => {
            "The page changed after the file was sealed. Nothing is listed; run attach again for a new slot."
        }
        Fault::Capacity => {
            "The object backend refused: it is full, or the file is over its limit. Nothing is listed."
        }
        Fault::Inactive => "Archived or deleted pages cannot take attachments.",
        Fault::Denied => {
            "The local writer cannot attach here: its access was revoked or the page is read-only."
        }
        Fault::Unavailable => {
            "The serve, its object channel or the object backend did not answer. Open the page once in a browser through tmt remote, then retry."
        }
        _ => return error,
    };
    crate::cli_management::fail(code(&fault), message)
}

#[cfg(test)]
mod tests {
    use super::extension;

    #[test]
    fn a_verified_media_type_names_the_extension_and_anything_else_is_bin() {
        for (kind, ext) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("application/pdf", "pdf"),
            ("text/plain", "txt"),
            ("application/octet-stream", "bin"),
            ("image/svg+xml", "bin"),
            ("text/html", "bin"),
            ("application/x-sh", "bin"),
            ("image/png; name=x.exe", "bin"),
            ("", "bin"),
        ] {
            assert_eq!(extension(kind), ext, "{kind}");
        }
    }
}
