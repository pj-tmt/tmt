//! Root-local storage reporting and verified reclaim.
use crate::cli_management::resolve_page;
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout, StateFault},
    page::Fault,
    storage::{self, Reclaim},
    store::{Store, owner::size},
};

fn spec(name: &'static str, summary: &'static str, examples: &'static [Example]) -> Command {
    tmt_cli_style::command(&CommandSpec {
        name,
        summary,
        examples,
        outputs: OutputModes::HumanAndJson,
        details: "Reports retained bytes and estimated reclaimable history. Comments and signed history stay protected. Attachments are not removed.",
    })
}
fn dry_run() -> Arg {
    Arg::new("dry-run")
        .long("dry-run")
        .action(ArgAction::SetTrue)
        .help("Estimate without changing state")
}
pub fn command() -> Command {
    spec(
        "storage",
        "Show local storage for each page",
        &[Example {
            command: "tmt colab storage --json",
            note: "Show storage on this machine",
        }],
    )
    .mut_arg("json", |arg| arg.global(true))
    .subcommand(
        spec(
            "reclaim",
            "Clean up eligible page history",
            &[Example {
                command: "tmt colab storage reclaim --dry-run",
                note: "Preview cleanup",
            }],
        )
        .arg(dry_run()),
    )
}
pub fn page_command() -> Command {
    spec(
        "reclaim",
        "Clean up this page's eligible history",
        &[Example {
            command: "tmt colab page reclaim 10000000-0000-4000-8000-000000000001 --dry-run",
            note: "Preview cleanup for one page",
        }],
    )
    .arg(crate::cli_grammar::page())
    .arg(dry_run())
}
fn human(bytes: u64) -> String {
    size(usize::try_from(bytes).unwrap_or(usize::MAX))
}
fn print_page(report: &Reclaim, json_output: bool) -> Result<()> {
    if let Some(error) = &report.error {
        let message = if let Some(freed) = report.freed.history.filter(|bytes| *bytes > 0) {
            format!(
                "{} Freed {} before the error. The page is unchanged.",
                error.message,
                human(freed)
            )
        } else {
            format!("{} The page is unchanged.", error.message)
        };
        return Err(Box::new(crate::cli_management::ManagementFault {
            code: "COLAB_UNAVAILABLE",
            message,
            correlation: serde_json::to_value(report)?,
            source: None,
        }));
    }
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{}", serde_json::to_string(report)?)?;
    } else if let Some(reason) = &report.skipped {
        writeln!(out, "skipped  {reason}")?;
    } else if report.dry_run {
        writeln!(
            out,
            "would free {}  page unchanged",
            human(report.freeable.history)
        )?;
    } else {
        writeln!(
            out,
            "reclaimed  {}  page unchanged",
            report
                .freed
                .history
                .map(human)
                .unwrap_or_else(|| "unknown".into())
        )?;
    }
    Ok(())
}

pub fn page(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    store.close()?;
    let dry = args.get_flag("dry-run");
    // A preview must not create the serve lock. Its database reads are snapshot-only.
    let lock = if dry {
        if layout.running()? {
            Err(StateFault::AlreadyServing.into())
        } else {
            Ok(None)
        }
    } else {
        layout.serve_lock().map(Some)
    };
    let report = match lock {
        Ok(_lock) => {
            let mut store = if dry {
                Store::read(&layout)?
            } else {
                Store::write_existing(&layout)?
            };
            let mut decoder = Decoder::new(std::env::current_exe()?)?;
            let report = storage::reclaim(&mut store, &key, &id, &mut decoder, dry)?;
            store.close()?;
            report
        }
        Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            tmt_colab::storage_ipc::reclaim(&layout, &id, dry)?
        }
        Err(error) => return Err(error),
    };
    print_page(&report, args.get_flag("json"))
}

pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    if let Some(("reclaim", reclaim)) = args.subcommand() {
        return space_reclaim(root, reclaim, args.get_flag("json"));
    }
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let mut report = storage::read(&store, &key, &layout, &mut decoder)?;
    store.close()?;
    // The active owner service can supply per-page Remote namespace allocation. A missing or
    // slow service leaves it unknown; storage reporting never starts a service.
    if layout.running().unwrap_or(false) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        for row in &mut report.pages {
            let Some(wait) = until.checked_duration_since(std::time::Instant::now()) else {
                break;
            };
            if let Ok(remote) = tmt_colab::storage_ipc::read(&layout, &row.page_id, wait) {
                row.apply_attachment_bytes(remote.attachments_bytes);
            }
        }
    }
    let json_output = args.get_flag("json");
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{}", serde_json::to_string(&report)?)?;
    } else {
        writeln!(
            out,
            "SPACE  {}   space.db {} ({} free inside, returned on next cleanup)",
            tmt_cli_style::table::escape(&report.space),
            human(report.db_bytes),
            human(report.db_free_bytes)
        )?;
        writeln!(
            out,
            "PAGE                      CONTENT  HISTORY  FILES  FREEABLE"
        )?;
        for row in &report.pages {
            let title = row.title.as_deref().unwrap_or(&row.page_id);
            let title = tmt_cli_style::table::escape(title);
            let files = row
                .attachments_bytes
                .map(human)
                .unwrap_or_else(|| "—".into());
            writeln!(
                out,
                "{title:<25} {:>8} {:>8} {:>8} {:>8}",
                human(row.content_bytes),
                human(row.history_bytes),
                files,
                human(row.freeable.history)
            )?;
        }
    }
    Ok(())
}

fn space_failure(
    report: &storage::SpaceReclaim,
) -> Result<Box<dyn std::error::Error + Send + Sync>> {
    let message = format!(
        "Cleanup stopped after {} pages. Freed {} before the error. Pages are unchanged.",
        report.pages.len(),
        report
            .freed
            .history
            .map(human)
            .unwrap_or_else(|| "unknown".into())
    );
    Ok(Box::new(crate::cli_management::ManagementFault {
        code: "COLAB_UNAVAILABLE",
        message,
        correlation: serde_json::to_value(report)?,
        source: None,
    }))
}

fn space_uncertain(
    report: &storage::SpaceReclaim,
    error: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    let mut correlation = serde_json::json!(report);
    correlation["freedBeforeUnknown"] = serde_json::json!(report.freed);
    correlation["freed"]["history"] = serde_json::Value::Null;
    let known = report
        .freed
        .history
        .map(human)
        .unwrap_or_else(|| "unknown".into());
    Box::new(crate::cli_management::ManagementFault {
        code: "COLAB_UNAVAILABLE",
        message: format!(
            "Cleanup stopped after {} completed pages (freed {known}); the current page outcome is unknown: {error}",
            report.pages.len()
        ),
        correlation,
        source: Some(error),
    })
}

fn space_reclaim(root: &Path, args: &ArgMatches, json_output: bool) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let source = Store::read(&layout)?;
    source.require_current_schema()?;
    let pages = tmt_colab::inspection::all_page_ids(&source, &key)?;
    source.close()?;
    let dry = args.get_flag("dry-run");
    let mut report = storage::SpaceReclaim::new(dry);
    let lock = if dry {
        if layout.running()? {
            Err(StateFault::AlreadyServing.into())
        } else {
            Ok(None)
        }
    } else {
        layout.serve_lock().map(Some)
    };
    match lock {
        Ok(_lock) => {
            let mut store = if dry {
                Store::read(&layout)?
            } else {
                Store::write_existing(&layout)?
            };
            let mut decoder = Decoder::new(std::env::current_exe()?)?;
            for id in &pages {
                let entry = storage::reclaim(&mut store, &key, id, &mut decoder, dry)
                    .map_err(|error| space_uncertain(&report, error))?;
                let failed = entry.error.is_some();
                report.add(entry)?;
                if failed {
                    return Err(space_failure(&report)?);
                }
            }
            if !dry {
                store.reclaim_free_pages();
            }
            store.close()?;
        }
        Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            for id in &pages {
                let entry = tmt_colab::storage_ipc::reclaim(&layout, id, dry)
                    .map_err(|error| space_uncertain(&report, error))?;
                let failed = entry.error.is_some();
                report.add(entry)?;
                if failed {
                    return Err(space_failure(&report)?);
                }
            }
        }
        Err(error) => return Err(error),
    }
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{}", serde_json::to_string(&report)?)?;
    } else if dry {
        writeln!(
            out,
            "would free {} across {} pages; nothing visible changes",
            human(report.freeable.history),
            report.pages.len()
        )?;
    } else {
        writeln!(
            out,
            "reclaimed {} across {} pages; pages unchanged",
            report
                .freed
                .history
                .map(human)
                .unwrap_or_else(|| "unknown".into()),
            report.pages.len()
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_batch_preserves_subtotal_without_claiming_a_total() {
        let mut report = storage::SpaceReclaim::new(false);
        report.freed.history = Some(42);
        let error = space_uncertain(&report, std::io::Error::other("lost reply").into());
        let fault = error
            .downcast_ref::<crate::cli_management::ManagementFault>()
            .unwrap();
        assert!(fault.correlation["freed"]["history"].is_null());
        assert_eq!(fault.correlation["freedBeforeUnknown"]["history"], 42);
        assert!(fault.message.contains("outcome is unknown"));
        assert!(fault.message.contains(&human(42)));
    }
}
