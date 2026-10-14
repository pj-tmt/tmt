//! New-executable composition for the native updater's database-rename phase (#2292).
//! The updater runs this in the executable it just activated, so the code that
//! understands the new basename is the code that renames.

use crate::{invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    storage::{DatabaseRename, Kept, rename_database},
};

/// The bounded document the updater parses: exactly `status`, plus `reason`
/// when the database kept its name for a reason the user can act on.
pub(crate) fn document(outcome: &DatabaseRename) -> Value {
    match outcome {
        DatabaseRename::Absent => json!({"status": "absent"}),
        DatabaseRename::Current => json!({"status": "current"}),
        DatabaseRename::Renamed => json!({"status": "renamed"}),
        DatabaseRename::Kept(Kept::InUse) => json!({"status": "in-use"}),
        DatabaseRename::Kept(Kept::Refused(reason)) => {
            json!({"status": "refused", "reason": reason})
        }
    }
}

pub fn execute(mode: OutputMode) -> io::Result<u8> {
    let result = ConfigPaths::discover_with_move_report(|_, _| {})
        .map_err(io::Error::other)
        .and_then(|paths| rename_database(&paths.global_dir));
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            return Failure::new("DATABASE_RENAME_FAILED", error.to_string(), 1)
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

fn summary(outcome: &DatabaseRename) -> String {
    match outcome {
        DatabaseRename::Absent => "No Core database to rename.".into(),
        DatabaseRename::Current => "The Core database already has its current name.".into(),
        DatabaseRename::Renamed => "Renamed the Core database to tmt.db.".into(),
        DatabaseRename::Kept(Kept::InUse) => {
            "The Core database keeps its former name while another process holds it.".into()
        }
        DatabaseRename::Kept(Kept::Refused(reason)) => {
            format!("The Core database keeps its former name: {reason}.")
        }
    }
}
