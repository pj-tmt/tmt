//! Whole-team read over the request owner's content-free projection.
use crate::{
    invocation::OutputMode,
    output::{Failure, after_cleanup},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};
use tmt_adapters::{config::ConfigPaths, request_runtime::wall_time_ms, storage::Storage};
use tmt_core::request::{
    RequestService,
    flow::{self, FlowReport},
};

fn run(window_ms: u64) -> Result<FlowReport, Failure> {
    let paths = ConfigPaths::discover()
        .map_err(|e| Failure::new("X_ERROR", "Could not read exchange flow.", 1).caused_by(e))?;
    let mut storage = Storage::open(&paths.database).map_err(|e| {
        Failure::storage_access(
            e,
            &paths.global_dir,
            "No exchange flow was changed.",
            "X_ERROR",
            "Could not read exchange flow.",
        )
    })?;
    let result = RequestService::new(&mut storage, wall_time_ms)
        .exchange_flow(window_ms)
        .map_err(|e| Failure::new("X_ERROR", "Could not read exchange flow.", 1).caused_by(e));
    after_cleanup(result, || storage.close())
}
fn document(report: &FlowReport) -> Value {
    json!({"observedAtMs": report.observed_at_ms, "sinceMs": report.since_ms, "bucketMs": flow::BUCKET_MS,
        "rows": report.rows.iter().map(|row| json!({
            "senderIdentityId": row.sender_identity_id, "recipientIdentityId": row.recipient_identity_id,
            "kind": row.kind.as_str(), "bucketStartMs": row.bucket_start_ms, "count": row.count, "bytes": row.bytes
        })).collect::<Vec<_>>()})
}
pub fn execute(window_ms: u64, mode: OutputMode) -> io::Result<u8> {
    let report = match run(window_ms) {
        Ok(report) => report,
        Err(e) => return e.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(stdout, "{}", document(&report))?;
    } else {
        let mut totals = BTreeMap::<_, (u64, u64)>::new();
        for row in report.rows {
            let total = totals
                .entry((row.sender_identity_id, row.recipient_identity_id, row.kind))
                .or_default();
            total.0 = total
                .0
                .checked_add(row.count)
                .ok_or_else(|| io::Error::other("Flow count overflow"))?;
            total.1 = total
                .1
                .checked_add(row.bytes)
                .ok_or_else(|| io::Error::other("Flow bytes overflow"))?;
        }
        writeln!(
            stdout,
            "Exchange flow: {}–{} UTC ms",
            report.since_ms, report.observed_at_ms
        )?;
        tmt_command_output::table::write(
            &mut stdout,
            ["SENDER", "RECIPIENT", "KIND", "COUNT", "BYTES"],
            totals
                .into_iter()
                .map(|((sender, recipient, kind), (count, bytes))| {
                    [
                        sender.unwrap_or_else(|| "unknown".into()),
                        recipient.unwrap_or_else(|| "unknown".into()),
                        kind.as_str().into(),
                        count.to_string(),
                        bytes.to_string(),
                    ]
                }),
        )?;
    }
    Ok(0)
}
