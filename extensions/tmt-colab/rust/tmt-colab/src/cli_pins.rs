//! Pins from the command line: a pinned page leads the owner's page list (contract: Pins). Like
//! projects, a pin is display-only owner-store state: no owner-log statement, no authority, never
//! synced, published, exported or shared.
use clap::{ArgMatches, Command};
use serde_json::json;
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result, inspection,
    keyring::{Keyring, Layout},
    limits::SPACE_PINS,
    page::Fault,
    store::Store,
};

use crate::cli_management::{fail, resolve_page};
use crate::cli_projects::send;

macro_rules! pin_command {
    ($name:literal,$summary:literal,$example:literal,$details:literal) => {
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
        .mut_arg("json", |arg| arg.global(true))
        .arg(crate::cli_grammar::page())
    };
}
/// `pin` and `unpin`, two top-level commands.
pub fn commands() -> [Command; 2] {
    [
        pin_command!(
            "pin",
            "Pin a page to the top of your page list",
            "tmt colab pin 10000000-0000-4000-8000-000000000001",
            "A pinned page leads Home and the quick switcher, in the order you pinned it. A pin is display-only: it is not signed, never leaves this machine and changes nobody's access. At most 20 pages can be pinned, and an archived page cannot be pinned. Archiving or deleting a page removes its pin; unarchiving does not bring it back."
        ),
        pin_command!(
            "unpin",
            "Put a pinned page back in its normal place",
            "tmt colab unpin 10000000-0000-4000-8000-000000000001",
            "Unpinning a page that is not pinned changes nothing."
        ),
    ]
}
pub fn run(root: &Path, command: &str, args: &ArgMatches) -> Result<()> {
    let pin = command == "pin";
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id = resolve_page(&store, &key, args.get_one::<String>("page").unwrap())?;
    let catalog = inspection::catalog(&store, &key)?;
    store.close()?;
    let row = catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|page| page["pageId"] == id.as_str())
        .ok_or_else(|| fail("COLAB_PAGE_NOT_FOUND", "Local page is not available."))?;
    let held = row["pinned"] == true;
    let pinned_count = catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|page| page["pinned"] == true)
        .count();
    if pin && !held {
        if row["archived"] == true {
            return Err(fail(
                "COLAB_PAGE_ARCHIVED",
                "An archived page cannot be pinned. Unarchive it first.",
            ));
        }
        if pinned_count >= SPACE_PINS {
            return Err(fail(
                "COLAB_CAPACITY",
                &format!("At most {SPACE_PINS} pages can be pinned. Unpin one first."),
            ));
        }
    }
    let changed = pin != held;
    if changed {
        send(
            &layout,
            &key,
            &id,
            "page.pin",
            &json!({"pageId":id,"pinned":pin}),
        )?;
    }
    let mut output = tmt_cli_style::stream::stdout(args.get_flag("json"));
    if args.get_flag("json") {
        writeln!(
            output,
            "{}",
            json!({"spaceId":key.space_id,"pageId":id,"pinned":pin,"changed":changed})
        )?;
    } else {
        let terminal = output.terminal();
        let rows = vec![
            ("page", id.clone()),
            ("pinned", if pin { "yes" } else { "no" }.into()),
            (
                "status",
                if changed {
                    if pin { "pinned" } else { "unpinned" }
                } else {
                    "unchanged"
                }
                .into(),
            ),
        ];
        tmt_cli_style::detail::write(&mut output, terminal, "PIN", &rows)?;
    }
    Ok(())
}
