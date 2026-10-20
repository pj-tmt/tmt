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
        owner_fold::Why,
    },
    store::{Store, owner::size},
};

use crate::cli_management::resolve_page;

pub fn command() -> Command {
    tmt_cli_style::command(&CommandSpec {
        name: "compact",
        summary: "Combine a page's changes into checkpoints",
        examples: &[Example {
            command: "tmt colab page compact 10000000-0000-4000-8000-000000000001 --json",
            note: "Combine the changes and report the page's usage before and after",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Merges the changes this device wrote to the page, after its last checkpoints, into new checkpoints, and does the same for the changes of the other devices you own (a browser paired through Remote, another machine's agent), signed by this device, then drops the merged changes, so they stop counting against the most a page keeps (200 changes, 4 MiB). Through serve when running, or under its lifecycle lock when stopped. The page's source is the same before and after, or nothing is written; running it again finds nothing to combine. A run combines a bounded number of devices; when it names devices it did not reach, run it again. Changes it cannot combine stay as they are and the result names the device and why: a bridge, link or commenter device can only combine its own. The merged changes' per-edit attribution is replaced by the checkpoint, which names the device that signed it and the range it covers. Archived or deleted pages cannot be compacted.",
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
        if report.through > 0 {
            rows.push(("covers", format!("changes 1 to {}", report.through)));
        }
        rows.push(("merged", report.merged_objects.to_string()));
    }
    for folded in &report.folded {
        rows.push((
            "combined",
            format!("device {}: changes 1 to {}", folded.stream, folded.through),
        ));
    }
    rows.push(("before", describe(&report.before)));
    rows.push(("after", describe(&report.after)));
    rows.push((
        "source",
        format!("sha256 {} (unchanged)", report.source_sha256),
    ));
    let own_changes = report.before.updates > report.before.other_device_updates;
    if report.skipped == Some(Skipped::NotCombined) && own_changes {
        rows.push((
            "note",
            "This device's changes could not be combined (the result would be too large for one checkpoint, or it did not finish in time). Nothing was written.".into(),
        ));
    }
    for left in &report.left {
        rows.push(("left", format!("device {}: {}", left.stream, why(left.why))));
    }
    if report.unfolded > 0 {
        rows.push((
            "note",
            format!(
                "{} more {} not reached this time. Run the command again to continue.",
                report.unfolded,
                if report.unfolded == 1 {
                    "device was"
                } else {
                    "devices were"
                }
            ),
        ));
    }
    if report.left.is_empty() && report.unfolded == 0 && report.after.other_device_updates > 0 {
        rows.push((
            "note",
            format!(
                "{} changes from other devices were not combined.",
                report.after.other_device_updates
            ),
        ));
    }
    rows
}

/// Why a device's changes were left as they are, for the person who ran the command.
fn why(why: Why) -> &'static str {
    match why {
        Why::NotOwnerDevice => {
            "only that device can combine its own changes (it is a bridge, link or commenter device)"
        }
        Why::TooLarge => "its combined changes would be too large for one checkpoint",
        Why::Refused => "combining them would have changed the page, so nothing was written",
    }
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

/// The quiet notice a write prints once a page is at least 80% full: how full, and, when changes
/// after the checkpoints could be combined, the command that makes room.
pub fn nearly_full(usage: &PageUsage, page: &str) -> String {
    let combinable = usage.limiting != Limiting::StoredSize && usage.updates > 0;
    format!(
        "{}% of what a page keeps ({} of {} changes){}",
        usage.percent,
        usage.updates,
        usage.update_limit,
        if combinable {
            format!("; `tmt colab page compact {page}` combines changes into checkpoints")
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
    fn the_nearly_full_notice_names_the_command_when_changes_can_be_combined() {
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
        assert!(own.ends_with("combines changes into checkpoints"), "{own}");
        // Other owner devices' changes are folded too, so they are no reason to stay quiet.
        let theirs = nearly_full(&usage(170, 170), PAGE);
        assert!(theirs.starts_with("85%"), "{theirs}");
        assert!(
            theirs.contains(&format!("`tmt colab page compact {PAGE}`")),
            "{theirs}"
        );
        let empty = nearly_full(&usage(0, 0), PAGE);
        assert!(!empty.contains("compact"), "{empty}");
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
