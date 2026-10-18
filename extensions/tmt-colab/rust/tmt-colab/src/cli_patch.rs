//! `tmt colab page patch`: an anchored change to a page's source (contract: Ranged read and
//! anchored patch). The patch carries the text it expects, is evaluated against the exact source
//! the page folds to, and then goes through the same whole-source write as `page write`, which
//! commits only the minimal text diff. A concurrent edit elsewhere on the page does not stand in
//! its way; a change to the addressed text refuses it, and nothing is written.
use clap::{Arg, ArgGroup, ArgMatches, Command};
use std::path::{Path, PathBuf};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page::{
        self, Fault,
        patch::{Patch, PatchFault, parse_diff},
    },
    store::Store,
};
use tmt_colab_model::crypto;

use crate::cli_management::{fail, resolve_page};

/// A patch evaluated against a page that moved underneath it (a commit landed between reading
/// the base and committing) is evaluated again on the new base, this many more times.
const RETRIES: usize = 3;

const PATCH: CommandSpec = CommandSpec {
    name: "patch",
    summary: "Change part of a page, anchored by the text you expect",
    examples: &[
        Example {
            command: "tmt colab page patch 10000000-0000-4000-8000-000000000001 --find 'old text' --replace 'new text' --json",
            note: "Replace one exact passage",
        },
        Example {
            command: "tmt colab page patch 10000000-0000-4000-8000-000000000001 --diff change.diff --json",
            note: "Apply a unified diff",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "Changes only the addressed text and writes the minimal change, through serve when running or under its lifecycle lock when stopped, with the same atomic batch, size limits, markup check (--strict refuses findings in the result), changed:false no-op and lost-reply behaviour as page write. Give exactly one form. --find TEXT --replace TEXT (or --find-file/--replace-file): the find text must occur exactly once, anywhere on the page. --lines A:B --range-sha256 HASH --file PATH: lines A..B (1-based, LF-separated; a CR is content) are replaced by the file's exact bytes, only while they still hash to the rangeSha256 page read reported. --diff PATH: a unified diff; each hunk's context and removed lines must match exactly once (or once at the stated line) with no fuzz, and all hunks apply or none do. A missing, ambiguous or changed anchor fails with COLAB_PATCH_CONFLICT naming it and writes nothing; read the page again and patch what it holds now. A concurrent edit elsewhere is merged: when the page moves between reading and committing, the patch is evaluated again on the new page, up to 3 more times. --expected-revision additionally refuses any change to the page since that revision, anywhere, and disables the retries. The result is page write's, including update:{count,bytes,minimal}.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&PATCH)
        .arg(crate::cli_grammar::page())
        .arg(crate::cli_page_check::strict())
        .arg(
            Arg::new("find")
                .long("find")
                .value_name("TEXT")
                .conflicts_with("find-file")
                .help("Exact text that occurs once on the page"),
        )
        .arg(
            Arg::new("find-file")
                .long("find-file")
                .value_name("path|-")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Read the find text from a file, byte for byte"),
        )
        .arg(
            Arg::new("replace")
                .long("replace")
                .value_name("TEXT")
                .conflicts_with("replace-file")
                .help("Text that takes its place (empty deletes)"),
        )
        .arg(
            Arg::new("replace-file")
                .long("replace-file")
                .value_name("path|-")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Read the replacement from a file, byte for byte"),
        )
        .arg(
            Arg::new("lines")
                .long("lines")
                .value_name("A:B")
                .value_parser(lines)
                .requires_all(["range-sha256", "file"])
                .help("Replace lines A through B (1-based)"),
        )
        .arg(
            Arg::new("range-sha256")
                .long("range-sha256")
                .value_name("hash")
                .value_parser(range_hash)
                .requires("lines")
                .help("rangeSha256 of those lines from page read"),
        )
        .arg(
            Arg::new("file")
                .long("file")
                .value_name("path|-")
                .value_parser(clap::value_parser!(PathBuf))
                .requires("lines")
                .help("The exact bytes that replace the lines"),
        )
        .arg(
            Arg::new("diff")
                .long("diff")
                .value_name("path|-")
                .value_parser(clap::value_parser!(PathBuf))
                .help("A unified diff"),
        )
        .group(
            ArgGroup::new("form")
                .args(["find", "find-file", "lines", "diff"])
                .required(true)
                .multiple(false),
        )
        .arg(
            Arg::new("expected-revision")
                .long("expected-revision")
                .value_name("revision")
                .value_parser(crate::revision_token)
                .help("Also refuse if the page changed anywhere since this revision"),
        )
}
fn lines(value: &str) -> std::result::Result<(usize, usize), &'static str> {
    let bad = "Use A:B with 1 <= A <= B, the line numbers page read reported";
    let (start, end) = value.split_once(':').ok_or(bad)?;
    let (start, end): (usize, usize) = (
        start.parse().map_err(|_| bad)?,
        end.parse().map_err(|_| bad)?,
    );
    if start == 0 || end < start {
        return Err(bad);
    }
    Ok((start, end))
}
fn range_hash(value: &str) -> std::result::Result<[u8; 32], &'static str> {
    let bad = "Use the 64 lowercase hex digits of rangeSha256";
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad);
    }
    let mut out = [0; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| bad)?;
    }
    Ok(out)
}

/// The patch the arguments describe, reading each named file once and exactly.
fn build(args: &ArgMatches) -> Result<Patch> {
    let read = |path: &Path| crate::page_source(path);
    let text = |inline: &str, file: &str| -> Result<Option<String>> {
        match (
            args.get_one::<String>(inline),
            args.get_one::<PathBuf>(file),
        ) {
            (Some(text), _) => Ok(Some(text.clone())),
            (None, Some(path)) => Ok(Some(read(path)?)),
            (None, None) => Ok(None),
        }
    };
    if let Some((start, end)) = args.get_one::<(usize, usize)>("lines") {
        return Ok(Patch::Lines {
            start: *start,
            end: *end,
            sha256: *args
                .get_one::<[u8; 32]>("range-sha256")
                .expect("required with --lines"),
            text: read(
                args.get_one::<PathBuf>("file")
                    .expect("required with --lines"),
            )?,
        });
    }
    if let Some(path) = args.get_one::<PathBuf>("diff") {
        return parse_diff(&read(path)?).map(Patch::Diff).map_err(coded);
    }
    let find = text("find", "find-file")?.expect("one form is required");
    let replace = text("replace", "replace-file")?.ok_or_else(|| {
        fail(
            "COLAB_INPUT_INVALID",
            "--find needs --replace (or --replace-file); an empty --replace deletes the text.",
        )
    })?;
    Ok(Patch::Replace { find, replace })
}
fn coded(fault: PatchFault) -> Box<dyn std::error::Error + Send + Sync> {
    fail(fault.code(), &fault.to_string())
}

fn open(layout: &Layout, key: &Keyring, operand: &str) -> Result<(Store, String)> {
    let store = Store::read(layout)?;
    store.require_current_schema()?;
    let id = resolve_page(&store, key, operand)?;
    Ok((store, id))
}

pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let operand = args.get_one::<String>("page").expect("required page");
    // The page operand resolves before any patch file or stdin byte is read.
    let (mut store, mut id) = open(&layout, &key, operand)?;
    let patch = build(args)?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let expected = args
        .get_one::<String>("expected-revision")
        .map(String::as_str);
    let mut retries = 0;
    loop {
        let base = page::read(&store, &key, &id, &mut decoder)?;
        let edited = match patch.apply(&base.source) {
            Ok(edited) => edited,
            Err(fault) => {
                let _ = store.close();
                return Err(coded(fault));
            }
        };
        // The markup check runs on the result, before the write takes its snapshot.
        let warnings = crate::cli_page_check::gate(
            &edited,
            args,
            crate::cli_page_check::Attachments::Page {
                store: &store,
                key: &key,
                page: &id,
                decoder: &mut decoder,
            },
        )?;
        // The edit was evaluated against `base.source`: the preparation refuses a page whose
        // source is anything else, and the commit refuses a page that moved after it.
        match crate::commit_source(
            &layout,
            &key,
            store,
            &id,
            &edited,
            expected,
            Some(crypto::digest(base.source.as_bytes())),
            &mut decoder,
        ) {
            Ok(receipt) => {
                return crate::print_receipt(
                    receipt,
                    &warnings,
                    args.get_flag("json"),
                    "PAGE PATCHED",
                );
            }
            Err(error)
                if expected.is_none()
                    && retries < RETRIES
                    && crate::error_code(error.as_ref()) == Fault::StaleBase.code() =>
            {
                retries += 1;
                (store, id) = open(&layout, &key, operand)?;
            }
            Err(error) => return Err(error),
        }
    }
}
