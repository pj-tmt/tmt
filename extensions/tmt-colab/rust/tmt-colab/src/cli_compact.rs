//! `tmt colab page compact`: combine this device's own stream of one page into checkpoints, so
//! its history stops counting against the limits a write is refused at. Through the running serve
//! when there is one, otherwise under the serve lifecycle lock; the page's source is unchanged.
use clap::{ArgMatches, Command};
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout, StateFault},
    page::{
        Fault, Limiting, PageUsage,
        compact::{self, Report, Skipped},
        compact_ipc,
    },
    store::{Store, owner::size},
};

use crate::cli_management::resolve_page;

pub fn command() -> Command {
    tmt_cli_style::command(&CommandSpec {
        name: "compact",
        summary: "Combine this device's changes to a page into checkpoints",
        examples: &[Example {
            command: "tmt colab page compact 10000000-0000-4000-8000-000000000001 --json",
            note: "Combine the changes and report the page's usage before and after",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Merges the changes this device wrote to the page, after its last checkpoints, into new checkpoints and drops the merged changes, so they stop counting against the most a page keeps (200 changes, 4 MiB). Through serve when running, or under its lifecycle lock when stopped. The page's source is the same before and after, or nothing is written; running it again finds nothing to combine. It cannot combine other devices' changes: only the device that wrote them can, and `after` shows what remains of them. The merged changes' per-edit attribution is replaced by the checkpoint, which names this device and the range it covers. Archived or deleted pages cannot be compacted.",
    })
    .arg(crate::cli_grammar::page())
}

pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    );
    let closed = store.close();
    let page = page?;
    closed?;
    let report = match layout.serve_lock() {
        Ok(_lock) => {
            let mut store = Store::write_existing(&layout)?;
            let mut decoder = Decoder::new(std::env::current_exe()?)?;
            let report = compact::request(&mut store, &key, &page, &mut decoder, None);
            let closed = store.close();
            let report = report?;
            closed?;
            report
        }
        Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            compact_ipc::request(&layout, &page)?
        }
        Err(error) => return Err(error),
    };
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{}", serde_json::to_string(&report)?)?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(&mut output, terminal, title(&report), &rows(&report))?;
    }
    Ok(())
}

fn title(report: &Report) -> &'static str {
    match report.skipped {
        None => "PAGE COMPACTED",
        Some(Skipped::NothingToCombine) => "NOTHING TO COMBINE",
        Some(Skipped::NotCombined) => "NOT COMBINED",
    }
}

fn rows(report: &Report) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        ("page", report.page_id.clone()),
        ("device", report.by.clone()),
    ];
    if report.compacted {
        rows.push(("covers", format!("changes 1 to {}", report.through)));
        rows.push(("merged", report.merged_objects.to_string()));
    }
    rows.push(("before", describe(&report.before)));
    rows.push(("after", describe(&report.after)));
    rows.push((
        "source",
        format!("sha256 {} (unchanged)", report.source_sha256),
    ));
    match report.skipped {
        Some(Skipped::NotCombined) => rows.push((
            "note",
            "This device's changes could not be combined (the result would be too large for one checkpoint, or it did not finish in time). Nothing was written.".into(),
        )),
        _ if report.after.other_device_updates > 0 => rows.push((
            "note",
            format!(
                "{} changes are on other devices' streams. Only those devices can combine them.",
                report.after.other_device_updates
            ),
        )),
        _ => {}
    }
    rows
}

fn describe(usage: &PageUsage) -> String {
    let state = match usage.gzip_bytes {
        Some(gzip) => format!("{} ({} gzipped)", size(usage.state_bytes), size(gzip)),
        None => size(usage.state_bytes),
    };
    format!(
        "{} of {} changes, {} of {}; state {} of {} ({}%)",
        usage.updates,
        usage.update_limit,
        size(usage.tail_bytes),
        size(usage.tail_bytes_limit),
        state,
        size(usage.gzip_limit),
        usage.percent
    )
}

/// The quiet notice a write prints once a page is at least 80% full: how full, and, when this
/// device has changes of its own to combine, the command that makes room.
pub fn nearly_full(usage: &PageUsage, page: &str) -> String {
    let combinable =
        usage.limiting != Limiting::StoredSize && usage.updates > usage.other_device_updates;
    format!(
        "{}% of what a page keeps ({} of {} changes){}",
        usage.percent,
        usage.updates,
        usage.update_limit,
        if combinable {
            format!("; `tmt colab page compact {page}` combines this device's changes")
        } else {
            String::new()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "10000000-0000-4000-8000-000000000001";

    #[test]
    fn the_nearly_full_notice_names_the_command_only_when_this_device_has_changes_to_combine() {
        let usage = |updates, other| {
            PageUsage::new(
                tmt_colab::page::UsageCounts {
                    updates,
                    other_device_updates: other,
                    ..Default::default()
                },
                0,
                None,
            )
        };
        let own = nearly_full(&usage(170, 0), PAGE);
        assert!(
            own.starts_with("85% of what a page keeps (170 of 200 changes)"),
            "{own}"
        );
        assert!(
            own.contains(&format!("`tmt colab page compact {PAGE}`")),
            "{own}"
        );
        let theirs = nearly_full(&usage(170, 170), PAGE);
        assert!(theirs.starts_with("85%"), "{theirs}");
        assert!(
            !theirs.contains("compact"),
            "only other devices can combine these: {theirs}"
        );
        // A page full by its stored size is not made smaller by combining changes.
        let stored = PageUsage::new(
            tmt_colab::page::UsageCounts {
                updates: 10,
                ..Default::default()
            },
            9_000_000,
            Some(4_500_000),
        );
        let text = nearly_full(&stored, PAGE);
        assert!(text.starts_with("90%"), "{text}");
        assert!(!text.contains("compact"), "{text}");
    }
}
