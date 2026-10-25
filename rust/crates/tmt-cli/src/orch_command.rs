//! Read-only CLI composition. Rules and JSON stay with their shared owners.
use crate::{
    invocation::{OrchOperation, OutputMode},
    output::{Failure, after_cleanup},
};
use serde_json::Value;
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    orchestration::{self, OrchReadQuery, OrchSubjectDocument},
    storage::Storage,
};
use tmt_core::orchestration::OrchError;

fn policy_failure(e: OrchError) -> Failure {
    Failure::new(
        orchestration::orch_error_code(e),
        orchestration::orch_error_message(e),
        if e == OrchError::NotFound { 3 } else { 1 },
    )
}
fn run(operation: OrchOperation) -> Result<Vec<u8>, Failure> {
    let (query, at) = match operation {
        OrchOperation::Overview {
            range_days,
            at,
            plan,
            member_query,
        } => (
            OrchReadQuery::Overview {
                range_days: Some(range_days),
                at_ms: None,
                plan,
                member_query,
            },
            at,
        ),
        OrchOperation::Plans { plan } => (OrchReadQuery::Plans { plan }, None),
        OrchOperation::Preview {
            file,
            range_days,
            at,
            plan,
            member_query,
        } => (
            OrchReadQuery::Preview {
                candidate: Box::new(
                    orchestration::orch_candidate_file(&file).map_err(policy_failure)?,
                ),
                range_days: Some(range_days),
                at_ms: None,
                plan,
                member_query,
            },
            at,
        ),
        OrchOperation::Why {
            subject,
            before,
            limit,
        } => {
            let (kind, id) = if subject.starts_with("req_") {
                ("request", subject)
            } else if let Some(id) = subject.strip_prefix("job:") {
                ("job", id.to_owned())
            } else {
                ("identity", subject)
            };
            (
                OrchReadQuery::Why {
                    subject: OrchSubjectDocument {
                        kind: kind.into(),
                        id,
                    },
                    before,
                    limit: Some(limit),
                },
                None,
            )
        }
    };
    let paths = ConfigPaths::discover().map_err(|e| {
        Failure::new("CONFIG_ERROR", "Could not resolve configuration paths.", 1).caused_by(e)
    })?;
    let mut storage = Storage::open(&paths.database).map_err(|e| {
        Failure::storage_access(
            e,
            &paths.global_dir,
            "No orchestration operation was performed.",
            "ORCH_STORAGE_UNAVAILABLE",
            "Could not open orchestration storage.",
        )
    })?;
    let result = orchestration::orchestration_read_with_time(
        &storage,
        query,
        tmt_adapters::request_runtime::wall_time_ms(),
        at.as_deref(),
    )
    .map_err(|e| Failure::new(e.code(), e.message(), e.exit()).caused_by(e));
    after_cleanup(result, || storage.close())
}
fn text(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
fn stamp(v: &Value) -> String {
    v.as_u64()
        .and_then(|ms| {
            time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()
        })
        .map(|d| d.to_string())
        .unwrap_or_else(|| "-".into())
}
fn source(v: &Value, plans: &[Value]) -> String {
    match v["kind"].as_str() {
        Some("base") => "Base".into(),
        Some("focus") => "personal focus".into(),
        _ => plans
            .iter()
            .find(|p| p["id"] == v["planId"])
            .map(|p| text(&p["name"]))
            .unwrap_or_else(|| text(&v["planId"])),
    }
}
fn write_overview(
    out: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    v: &Value,
) -> io::Result<()> {
    use tmt_cli_style::table::{Cell, Column, Table};
    let zone = orchestration::OrchZone::load(v["timeZone"].as_str())
        .map_err(|e| io::Error::other(orchestration::orch_error_message(e)))?;
    let local = |v: &Value| {
        zone.format_ms(v.as_u64().expect("shared timestamp"))
            .map_err(|e| io::Error::other(orchestration::orch_error_message(e)))
    };
    tmt_cli_style::detail::write(
        out,
        terminal,
        "Orchestration",
        &[
            ("at", local(&v["atMs"])?),
            ("zone", text(&v["timeZone"])),
            (
                "range",
                format!(
                    "{} → {}",
                    local(&v["range"]["startMs"])?,
                    local(&v["range"]["endMs"])?
                ),
            ),
        ],
    )?;
    if let Some(empty) = v["emptyState"].as_str() {
        tmt_cli_style::message::hint(out, terminal, empty)?;
    }
    let plans = v["plans"].as_array().expect("shared overview plans");
    let mut timeline = Table::new(&[Column::Fixed, Column::Name, Column::Fixed]);
    timeline.row([Cell::from("slot"), Cell::from("plans"), Cell::from("count")]);
    for interval in v["timeline"].as_array().expect("shared timeline") {
        let ids = interval["planIds"].as_array().expect("timeline IDs");
        let names: Vec<_> = ids
            .iter()
            .map(|id| {
                plans
                    .iter()
                    .find(|p| p["id"] == *id)
                    .map(|p| text(&p["name"]))
                    .unwrap_or_else(|| text(id))
            })
            .collect();
        timeline.row([
            Cell::from(format!(
                "{} → {}",
                local(&interval["startMs"])?,
                local(&interval["endMs"])?
            )),
            Cell::from(if names.is_empty() {
                "Base".into()
            } else {
                names.join(", ")
            }),
            Cell::from(ids.len().to_string()),
        ]);
    }
    timeline.write(out, terminal)?;
    let mut table = Table::new(&[
        Column::Name,
        Column::Fixed,
        Column::Fixed,
        Column::Fixed,
        Column::Fixed,
        Column::Name,
    ]);
    table.row([
        Cell::from("member"),
        Cell::from("state"),
        Cell::from("receiving"),
        Cell::from("sending"),
        Cell::from("routine"),
        Cell::from("set by"),
    ]);
    for member in v["members"].as_array().expect("shared members") {
        let e = &member["effective"];
        let receive = &e["receiving"]["value"];
        let receiving = if receive["mode"] == "batch" {
            format!(
                "batch {}m",
                receive["intervalMs"].as_u64().unwrap_or(0) / 60_000
            )
        } else {
            text(&receive["mode"])
        };
        let sources: Vec<_> = ["state", "receiving", "sending", "routine"]
            .iter()
            .map(|f| format!("{f}: {}", source(&e[*f]["source"], plans)))
            .collect();
        table.row([
            Cell::from(text(&member["name"])),
            Cell::from(text(&e["state"]["value"])),
            Cell::from(receiving),
            Cell::from(text(&e["sending"]["value"])),
            Cell::from(text(&e["routine"]["value"])),
            Cell::from(sources.join("; ")),
        ]);
    }
    table.write(out, terminal)
}
fn write_human(
    out: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    v: &Value,
) -> io::Result<()> {
    use tmt_cli_style::table::{Cell, Column, Table};
    if v.get("members").is_some() {
        return write_overview(out, terminal, v);
    }
    if v.get("after").is_some() {
        let changes = v["changes"].as_array().expect("preview changes");
        tmt_cli_style::detail::write(
            out,
            terminal,
            "Plan preview",
            &[
                ("changed members", changes.len().to_string()),
                ("saved", "false".into()),
            ],
        )?;
        for c in changes {
            writeln!(
                out,
                "{}: {}",
                text(&c["identityId"]),
                c["fields"]
                    .as_array()
                    .expect("change fields")
                    .iter()
                    .map(text)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        return write_overview(out, terminal, &v["after"]);
    }
    if let Some(plans) = v["plans"].as_array() {
        if plans.is_empty() {
            return tmt_cli_style::message::hint(
                out,
                terminal,
                "Base only. Plan creation/editing and personal focus controls arrive in orchestration slice 2.",
            );
        }
        let mut table = Table::new(&[Column::Name, Column::Fixed, Column::Fixed, Column::Fixed]);
        for p in plans {
            table.row([
                Cell::from(text(&p["name"])),
                Cell::from(text(&p["priority"])),
                Cell::from(text(&p["enabled"])),
                Cell::from(text(&p["id"])),
            ]);
        }
        return table.write(out, terminal);
    }
    let items = v["items"].as_array().expect("journal items");
    if items.is_empty() {
        return tmt_cli_style::message::hint(
            out,
            terminal,
            "No recorded decision. Slice 1 provides the journal; live producers arrive in slices 2–4.",
        );
    }
    let mut table = Table::new(&[Column::Fixed, Column::Fixed, Column::Fixed, Column::Name]);
    for event in items {
        table.row([
            Cell::from(stamp(&event["recordedAtMs"])),
            Cell::from(text(&event["action"])),
            Cell::from(text(&event["outcome"])),
            Cell::from(text(&event["reason"])),
        ]);
    }
    table.write(out, terminal)
}
pub fn execute(operation: OrchOperation, mode: OutputMode) -> io::Result<u8> {
    let bytes = match run(operation) {
        Ok(v) => v,
        Err(e) => return e.publish(mode),
    };
    let mut out = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        out.write_all(&bytes)?;
        writeln!(out)?;
    } else {
        let value: Value = serde_json::from_slice(&bytes).expect("shared orchestration JSON");
        let terminal = out.terminal();
        write_human(&mut out, terminal, &value)?;
    }
    Ok(0)
}
