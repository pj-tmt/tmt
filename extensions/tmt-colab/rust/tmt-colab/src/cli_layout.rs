//! A page's layout at the command line (#3926): the `--layout`/`--reason` choice `page create`
//! requires, and `page layout`, which changes it later. The frame the choice selects is the
//! renderer's; here the choice is validated before anything is created or read, then stored as
//! page metadata through the same owner path as every other page write.
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::{Decoder, LayoutKind, PageLayout, valid_layout_reason},
    keyring::{Keyring, Layout},
    page::{self, Fault},
    store::Store,
};
use tmt_colab_model::crypto;

use crate::cli_management::{fail, resolve_page};

/// The skill topics an agent reads before writing a page's markup, in the order they help.
pub const READ_FIRST: [&str; 2] = ["layout", "components"];

const KINDS: [&str; 3] = ["column", "full", "canvas"];

const LAYOUT: CommandSpec = CommandSpec {
    name: "layout",
    summary: "Choose how Colab frames a page",
    examples: &[
        Example {
            command: "tmt colab page layout 10000000-0000-4000-8000-000000000001 column --json",
            note: "Frame an existing page as a text column",
        },
        Example {
            command: "tmt colab page layout 10000000-0000-4000-8000-000000000001 canvas --reason 'full-screen board'",
            note: "Let the page draw its own frame, and say why",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "Sets the page's layout without touching its source, through serve when running or under its lifecycle lock when stopped. column: a text column up to 1280 px wide with Colab's gaps; full: blocks may run edge to edge; canvas: the page draws its own frame, which needs --reason (one line, at most 1024 bytes) because Colab then applies none. A page that was never given a layout is freeform and shows as it was written. Setting the layout the page already has reports changed false. --expected-revision refuses unless the page is still at that revision from page read. The layout rules and what each kind sets are in `tmt colab skill layout`.",
};

/// `--layout` and `--reason` on `page create`. The choice is required, but by `choice`, not by
/// clap, so the refusal can point at what to read.
pub fn create_args(command: Command) -> Command {
    command.arg(kind_arg("layout")).arg(reason_arg())
}

fn kind_arg(name: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .value_name("column|full|canvas")
        .value_parser(KINDS)
        .help("How Colab frames the page (see tmt colab skill layout)")
}

fn reason_arg() -> Arg {
    Arg::new("reason")
        .long("reason")
        .value_name("TEXT")
        .help("Why a canvas page draws its own frame (canvas only; one line, at most 1024 bytes)")
}

pub fn command() -> Command {
    tmt_cli_style::command(&LAYOUT)
        .arg(crate::cli_grammar::page())
        .arg(
            Arg::new("kind")
                .required(true)
                .index(2)
                .value_name("column|full|canvas")
                .value_parser(KINDS)
                .help("The layout to give the page"),
        )
        .arg(reason_arg())
        .arg(
            Arg::new("expected-revision")
                .long("expected-revision")
                .value_name("revision")
                .value_parser(crate::revision_token)
                .help("Refuse unless the page is still at this revision from page read"),
        )
}

fn input(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}

/// The layout the arguments choose: a kind, and for a canvas its reason. Checked before any state
/// is initialized or any source is read, so a refused create leaves nothing behind.
fn chosen(kind: &str, reason: Option<&str>) -> Result<PageLayout> {
    let kind = LayoutKind::parse(kind).expect("clap restricts the kind");
    let reason = reason.map(str::trim);
    match (kind, reason) {
        (LayoutKind::Canvas, None | Some("")) => Err(input(
            "A canvas page draws its own frame, so say why: --layout canvas --reason \"<one line>\".",
        )),
        (LayoutKind::Canvas, Some(reason)) if !valid_layout_reason(reason) => Err(input(
            "The reason is one line of text, at most 1024 bytes, without control characters.",
        )),
        (LayoutKind::Canvas, Some(reason)) => Ok(PageLayout {
            kind,
            reason: Some(reason.to_owned()),
        }),
        (_, Some(_)) => Err(input(
            "--reason belongs to a canvas page; column and full pages need none.",
        )),
        (_, None) => Ok(PageLayout { kind, reason: None }),
    }
}

/// What `page create` must be given, or the refusal that tells the agent where to read.
pub fn choice(args: &ArgMatches) -> Result<PageLayout> {
    match args.get_one::<String>("layout") {
        Some(kind) => chosen(kind, args.get_one::<String>("reason").map(String::as_str)),
        None => Err(input(
            "Choose a layout for this page: --layout column (a text column up to 1280 px, most pages), \
             --layout full (blocks may run edge to edge) or --layout canvas --reason \"<why>\" \
             (the page draws its own frame). Or start from a template that sets both: \
             --template NAME (tmt colab template ls). Read before writing: tmt colab skill layout.",
        )),
    }
}

/// A layout kind in the words of `show` and the create result.
pub fn label(kind: &str) -> String {
    match kind {
        "column" => "column (max 1280 px)".to_owned(),
        other => other.to_owned(),
    }
}

pub fn describe(layout: &PageLayout) -> String {
    label(layout.kind.as_str())
}

/// The JSON fields a layout adds to a result: the kind, and for a canvas its whole reason.
pub fn json_fields(layout: &PageLayout) -> Value {
    match &layout.reason {
        Some(reason) => json!({"layout": layout.kind.as_str(), "layoutReason": reason}),
        None => json!({"layout": layout.kind.as_str()}),
    }
}

pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let wanted = chosen(
        args.get_one::<String>("kind").expect("required kind"),
        args.get_one::<String>("reason").map(String::as_str),
    )?;
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let base = page::read(&store, &key, &id, &mut decoder)?;
    let receipt = crate::commit_source(
        &layout,
        &key,
        store,
        &id,
        &base.source,
        args.get_one::<String>("expected-revision")
            .map(String::as_str),
        Some(crypto::digest(base.source.as_bytes())),
        Some(&wanted),
        &mut decoder,
    )?;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = serde_json::to_value(&receipt)?;
        if let (Some(fields), Some(target)) =
            (json_fields(&wanted).as_object(), value.as_object_mut())
        {
            target.extend(fields.clone());
        }
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        let mut rows = vec![
            ("page", receipt.page_id.clone()),
            ("layout", describe(&wanted)),
        ];
        if let Some(reason) = &wanted.reason {
            rows.push(("reason", tmt_cli_style::table::escape(reason)));
        }
        rows.push(("revision", receipt.revision.clone()));
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if receipt.changed {
                "PAGE LAYOUT SET"
            } else {
                "PAGE UNCHANGED"
            },
            &rows,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canvas_needs_a_trimmed_one_line_reason_and_the_others_take_none() {
        assert_eq!(
            chosen("column", None).unwrap(),
            PageLayout {
                kind: LayoutKind::Column,
                reason: None
            }
        );
        assert!(chosen("full", Some("why")).is_err());
        assert!(chosen("canvas", None).is_err());
        assert!(chosen("canvas", Some("   ")).is_err());
        assert!(chosen("canvas", Some("two\nlines")).is_err());
        assert!(chosen("canvas", Some(&"x".repeat(1025))).is_err());
        assert_eq!(
            chosen("canvas", Some("  a board  "))
                .unwrap()
                .reason
                .as_deref(),
            Some("a board")
        );
        assert!(chosen("canvas", Some(&"x".repeat(1024))).is_ok());
    }
}
