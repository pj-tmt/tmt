//! The home summary from the command line: the two lines under Today's greeting (contract: Home
//! summary). It is display-only owner data, so a write creates no owner-log statement and carries
//! no authority. Like `page compact` it goes through the running serve when there is one and
//! otherwise runs under the serve lifecycle lock; there is no page, so no management operation.
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::json;
use std::{io::Write, path::Path};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    home_summary::{
        self, HomeSummary, LINE_POINTS,
        ipc::{self, Outcome},
        line_valid,
    },
    keyring::{Keyring, Layout, StateFault},
    page::Fault,
    store::Store,
};

use crate::cli_management::fail;

macro_rules! command_spec {
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
    };
}
pub fn command() -> Command {
    command_spec!("home","Manage what Today shows under its greeting","tmt colab home summary show","Today's home page shows a short two-line summary of what changed since the last update, written by the space's maintainer agent. It is display-only and unsigned: it carries no authority and no page can write it.")
        .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
        .subcommand(
            command_spec!("summary","Write, read or clear the two-line summary","tmt colab home summary set --line 'Two pages changed' --line 'Pricing moved to review'","The summary is two lines: the first up to 24 characters, the second up to 60 (counted as characters, not bytes). A longer line is refused with an error, never cut. Today shows both lines with who wrote them and when.")
                .mut_arg("json",|arg|arg.global(true)).subcommand_required(true)
                .subcommand(
                    command_spec!("set","Replace the summary with two lines","tmt colab home summary set --line 'Two pages changed' --line 'Pricing moved to review'","Replaces the whole summary. The writer is recorded as the calling agent's display name (left out when there is no agent identity) and the time is the owner's clock.")
                        .arg(Arg::new("line").long("line").value_name("LINE").action(ArgAction::Append).allow_hyphen_values(true).help("One line; give it twice, first line first")),
                )
                .subcommand(command_spec!("show","Read the current summary","tmt colab home summary show --json","Reads only. Prints `summary: null` when none is set."))
                .subcommand(command_spec!("clear","Remove the summary","tmt colab home summary clear","Today then shows no summary. Clearing when none is set changes nothing.")),
        )
}

fn refuse_lines(lines: &[String]) -> Box<dyn std::error::Error + Send + Sync> {
    let [first, second] = LINE_POINTS;
    if lines.len() != 2 {
        return fail(
            "COLAB_INPUT_INVALID",
            "Give exactly two lines: --line FIRST --line SECOND.",
        );
    }
    for (index, (line, max)) in lines.iter().zip(LINE_POINTS).enumerate() {
        if line.chars().count() > max {
            return fail(
                "COLAB_INPUT_INVALID",
                &format!(
                    "Line {} is {} characters; it holds at most {max} (line 1: {first}, line 2: {second}). Shorten it; nothing was changed.",
                    index + 1,
                    line.chars().count()
                ),
            );
        }
        if !line_valid(line, max) {
            return fail(
                "COLAB_INPUT_INVALID",
                &format!(
                    "Line {} must have text and no control characters (such as a line break). Nothing was changed.",
                    index + 1
                ),
            );
        }
    }
    fail("COLAB_INPUT_INVALID", "The summary lines are not valid.")
}
/// Store the lines (or clear), through the serve when it runs, else under its lifecycle lock.
fn write(
    layout: &Layout,
    key: &Keyring,
    lines: Option<&[String; 2]>,
    author: Option<&str>,
) -> Result<Outcome> {
    match layout.serve_lock() {
        Ok(_lock) => {
            let mut store = Store::write_existing(layout)?;
            let outcome = store
                .require_current_schema()
                .map_err(Into::into)
                .and_then(|()| {
                    store.label_transaction(&key.space_id, &key.owner_public(), |tx| {
                        tx.set_home_summary(lines, author)
                    })
                });
            let closed = store.close();
            let (summary, changed) = outcome?;
            closed?;
            Ok(Outcome { summary, changed })
        }
        Err(error) if error.downcast_ref::<StateFault>() == Some(&StateFault::AlreadyServing) => {
            ipc::request(layout, lines, author)
        }
        Err(error) => Err(error),
    }
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let (_, args) = args.subcommand().expect("required summary subcommand");
    let (mode, args) = args.subcommand().expect("required subcommand");
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let outcome = match mode {
        "show" => {
            let store = Store::read(&layout)?;
            let summary = store
                .require_current_schema()
                .map_err(Into::into)
                .and_then(|()| tmt_colab::inspection::home_summary(&store, &key));
            let closed = store.close();
            let summary = summary?;
            closed?;
            Outcome {
                summary,
                changed: false,
            }
        }
        "set" => {
            let lines: Vec<String> = args
                .get_many::<String>("line")
                .map(|values| values.cloned().collect())
                .unwrap_or_default();
            if !home_summary::lines_valid(&lines) {
                return Err(refuse_lines(&lines));
            }
            let lines: [String; 2] = lines.try_into().expect("two valid lines");
            let author = tmt_colab::core::publisher_agent();
            write(&layout, &key, Some(&lines), author.as_deref())?
        }
        "clear" => write(&layout, &key, None, None)?,
        _ => unreachable!(),
    };
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = json!({"summary": outcome.summary});
        if mode == "clear" {
            value["changed"] = json!(outcome.changed);
        }
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        let rows = rows(mode, &outcome);
        tmt_cli_style::detail::write(&mut output, terminal, "HOME SUMMARY", &rows)?;
    }
    Ok(())
}
fn rows(mode: &str, outcome: &Outcome) -> Vec<(&'static str, String)> {
    let Some(HomeSummary {
        lines,
        author,
        at_ms,
    }) = &outcome.summary
    else {
        return vec![(
            "summary",
            if mode == "clear" && outcome.changed {
                "cleared"
            } else {
                "none"
            }
            .into(),
        )];
    };
    let mut rows = vec![("line 1", lines[0].clone()), ("line 2", lines[1].clone())];
    if let Some(author) = author {
        rows.push(("written by", author.clone()));
    }
    rows.push(("written", utc(*at_ms)));
    rows
}
/// `2026-10-17 09:30 UTC` for an epoch-millisecond time (civil-from-days, proleptic Gregorian).
fn utc(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    let minutes = ms / 60_000 % 1440;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_prints_as_a_utc_minute() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_792_229_400_000), "2026-10-17 09:30 UTC");
        assert_eq!(utc(951_782_400_000 + 59 * 60_000), "2000-02-29 00:59 UTC");
    }
}
