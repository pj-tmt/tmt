//! New-executable composition for the updater's one-time database compaction
//! (#5036): the switch of the Core database to incremental auto-vacuum.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    storage::{Compaction, compact_database},
};

/// The bounded document the updater parses: `status`, plus `reason` for a
/// refusal and the sizes for a compaction.
pub(crate) fn document(outcome: &Compaction) -> Value {
    match outcome {
        Compaction::Absent => json!({"status": "absent"}),
        Compaction::Current => json!({"status": "current"}),
        Compaction::Compacted {
            bytes_before,
            bytes_after,
        } => json!({
            "status": "compacted",
            "bytesBefore": bytes_before,
            "bytesAfter": bytes_after,
        }),
        Compaction::InUse => json!({"status": "in-use"}),
        Compaction::Refused(reason) => json!({"status": "refused", "reason": reason}),
    }
}

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let result = ConfigPaths::discover_with_move_report(|_, _| {})
        .map_err(io::Error::other)
        .and_then(|paths| compact_database(&paths.database));
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            return Failure::new("DATABASE_COMPACTION_FAILED", error.to_string(), 1)
                .caused_by(error)
                .publish(mode);
        }
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(output, "{}", document(&outcome))?;
    } else {
        writeln!(output, "{}", summary(&outcome))?;
    }
    Ok(0)
}

fn summary(outcome: &Compaction) -> String {
    match outcome {
        Compaction::Absent => "No Core database to compact.".into(),
        Compaction::Current => "The Core database already reclaims free space.".into(),
        Compaction::Compacted {
            bytes_before,
            bytes_after,
        } => format!(
            "Compacted the Core database from {} to {}.",
            tmt_cli_style::progress::bytes(*bytes_before),
            tmt_cli_style::progress::bytes(*bytes_after)
        ),
        Compaction::InUse => {
            "The Core database was not compacted: tmt.db is in use by another writer; nothing changed."
                .into()
        }
        Compaction::Refused(reason) => {
            format!("The Core database was not compacted: {reason}; nothing changed.")
        }
    }
}
