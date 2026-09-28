//! `tmt office storage`: report and run the consented Office storage migration
//! through the verified companion.

use crate::{
    invocation::{OfficeStorageOperation, OutputMode},
    output::Failure,
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::office_companion::invoke_office_storage;
use tmt_office_model::office_protocol::OfficeInvocation;

const PLAN_DEADLINE: Duration = Duration::from_secs(60);
/// The backup copies the whole tmt database, so migration gets far longer than
/// one-shot operations.
const MIGRATE_DEADLINE: Duration = Duration::from_secs(30 * 60);

pub fn run(
    executable: &Path,
    operation: OfficeStorageOperation,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let plan = invoke(
        executable,
        OfficeInvocation::StoragePlan,
        b"{}",
        PLAN_DEADLINE,
    )?;
    let yes = match operation {
        OfficeStorageOperation::Status => {
            return emit(&plan, &status_text(&plan), mode);
        }
        OfficeStorageOperation::Migrate { yes } => yes,
    };
    if text(&plan, "state") != "pending" {
        return emit(&plan, &status_text(&plan), mode);
    }
    if !mode.json {
        writeln!(io::stderr().lock(), "{}", plan_text(&plan)).map_err(io_failure)?;
    }
    if !crate::office_command::consent(yes, mode, "Migrate Office data now")? {
        return Ok(0);
    }
    let input = json!({"consent": plan["planDigest"]})
        .to_string()
        .into_bytes();
    let result = invoke(
        executable,
        OfficeInvocation::StorageMigrate,
        &input,
        MIGRATE_DEADLINE,
    )
    .map_err(|failure| {
        if failure.code == "OFFICE_STORAGE_TIMEOUT" {
            Failure::new(
                "OFFICE_STORAGE_OUTCOME_UNKNOWN",
                "The migration did not finish in time; its outcome is unknown. Run 'tmt office storage status'.",
                1,
            )
        } else {
            failure
        }
    })?;
    emit(&result, &done_text(&result, &plan), mode)
}

/// One stderr line when Office data can move; silent for an Office companion
/// without storage migration, on any failure, in JSON mode or with hints off.
pub fn pending_hint(executable: &Path, mode: OutputMode) {
    if mode.json || std::env::var("TMT_HINTS").is_ok_and(|value| value.eq_ignore_ascii_case("off"))
    {
        return;
    }
    let pending = invoke(
        executable,
        OfficeInvocation::StoragePlan,
        b"{}",
        Duration::from_secs(5),
    )
    .is_ok_and(|plan| text(&plan, "state") == "pending");
    if pending {
        let _ = writeln!(
            io::stderr().lock(),
            "Office storage migration available: run 'tmt office storage migrate' (the current storage keeps working until you do)."
        );
    }
}

fn invoke(
    executable: &Path,
    operation: OfficeInvocation,
    input: &[u8],
    deadline: Duration,
) -> Result<Value, Failure> {
    let value = invoke_office_storage(executable, operation, input, Instant::now() + deadline)
        .map_err(|error| match error.kind() {
            io::ErrorKind::Unsupported => Failure::new(
                "OFFICE_INCOMPATIBLE",
                "The installed Office companion does not support storage migration. Update Office before retrying.",
                1,
            )
            .caused_by(error),
            io::ErrorKind::TimedOut => {
                Failure::new("OFFICE_STORAGE_TIMEOUT", error.to_string(), 1).caused_by(error)
            }
            _ => io_failure(error),
        })?;
    if let Some(error) = value.get("error") {
        return Err(Failure::new(
            text(error, "code").to_owned(),
            text(error, "message").to_owned(),
            1,
        ));
    }
    Ok(value)
}

fn emit(value: &Value, human: &str, mode: OutputMode) -> Result<u8, Failure> {
    let output = if mode.json {
        value.to_string()
    } else {
        human.to_owned()
    };
    writeln!(io::stdout().lock(), "{output}").map_err(io_failure)?;
    Ok(0)
}

fn status_text(plan: &Value) -> String {
    let mut lines = match text(plan, "state") {
        "pending" => vec![
            "Office storage: shared with tmt (migration available)".to_owned(),
            format!("  Source:       {}", text(plan, "source")),
            format!("  Destination:  {}", text(plan, "destination")),
            format!(
                "  Office data:  {} rows, {} in the shared database",
                number(plan, "officeRows"),
                size(number(plan, "officeBytes"))
            ),
            format!("  Service:      {}", service(plan)),
            "Run 'tmt office storage migrate' to move Office data into its own database."
                .to_owned(),
        ],
        "switched" => vec![format!(
            "Office storage: {} (migrated {})",
            text(plan, "destination"),
            plan["switchedAtMs"]
                .as_i64()
                .map_or_else(|| "earlier".to_owned(), utc_time)
        )],
        "switching" => vec![
            "Office storage: the migration was recorded but has not finished.".to_owned(),
            "Run any Office command to finish it; if it reports that recovery is needed, see the Office storage recovery section of the Office docs.".to_owned(),
        ],
        _ => vec![
            "Office storage: shared with tmt".to_owned(),
            "Update tmt to move Office data into its own database.".to_owned(),
        ],
    };
    let backups = plan["backups"].as_array().cloned().unwrap_or_default();
    if !backups.is_empty() {
        lines.push("Retained backups:".to_owned());
        for backup in &backups {
            lines.push(format!(
                "  {}  {}",
                text(backup, "directory"),
                size(number(backup, "bytes"))
            ));
        }
        lines.push("Backups are kept until you remove them.".to_owned());
    }
    lines.join("\n")
}

fn plan_text(plan: &Value) -> String {
    [
        "This moves Office data into its own database.".to_owned(),
        format!("  Source:       {}", text(plan, "source")),
        format!("  Destination:  {}", text(plan, "destination")),
        format!(
            "  Backup:       {}/office-storage-<time of migration>/ (a full copy of the tmt database, plus Office configuration)",
            text(plan, "backupsDirectory")
        ),
        format!("  Backup size:  {} of free space needed", size(number(plan, "backupBytes"))),
        format!(
            "  Office data:  {} rows, {}",
            number(plan, "officeRows"),
            size(number(plan, "officeBytes"))
        ),
        "The Office service stops while the backup and switch run; tmt commands keep working, apart from a short pause while the switch commits.".to_owned(),
        "This is forward-only:".to_owned(),
        "  - Office data will live in office.db, and older tmt or tmt-office versions cannot write it.".to_owned(),
        "  - The shared database keeps a read-only copy of the Office data until a later tmt update removes it from core.".to_owned(),
        "  - Undoing this means restoring the backup, which also discards any tmt activity after the backup.".to_owned(),
    ]
    .join("\n")
}

fn done_text(result: &Value, plan: &Value) -> String {
    let mut lines = vec![
        format!("Office data moved to {}.", text(result, "database")),
        format!(
            "Backup kept at {} ({}).",
            text(&result["backup"], "directory"),
            size(number(&result["backup"], "bytes"))
        ),
    ];
    if result["serviceWasRunning"].as_bool() == Some(true)
        || plan["serviceRunning"].as_bool() == Some(true)
    {
        lines.push(
            "The Office service was stopped for the migration. Start it again with 'tmt office start'."
                .to_owned(),
        );
    }
    lines.join("\n")
}

fn service(plan: &Value) -> &'static str {
    match plan["serviceRunning"].as_bool() {
        Some(true) => "running",
        Some(false) => "stopped",
        None => "unknown",
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn number(value: &Value, key: &str) -> u64 {
    value[key].as_u64().unwrap_or(0)
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `yyyy-mm-dd hh:mm UTC` for a UNIX time in milliseconds.
fn utc_time(milliseconds: i64) -> String {
    let seconds = milliseconds.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time / 3600,
        time % 3600 / 60
    )
}

fn io_failure(error: io::Error) -> Failure {
    Failure::new("OFFICE_IO_ERROR", error.to_string(), 1).caused_by(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(state: &str) -> Value {
        json!({
            "state": state,
            "source": "/root/tmux-team.db",
            "destination": "/root/office/office.db",
            "backupsDirectory": "/root/backups",
            "officeRows": 12,
            "officeBytes": 2048,
            "backupBytes": 3 * 1024 * 1024,
            "serviceRunning": true,
            "switchedAtMs": 1_790_000_000_000_i64,
            "backups": [],
            "planDigest": "0".repeat(64),
        })
    }

    #[test]
    fn status_and_plan_text_follow_the_approved_wording() {
        assert_eq!(
            status_text(&plan("pending")),
            "Office storage: shared with tmt (migration available)\n  Source:       /root/tmux-team.db\n  Destination:  /root/office/office.db\n  Office data:  12 rows, 2.0 KiB in the shared database\n  Service:      running\nRun 'tmt office storage migrate' to move Office data into its own database."
        );
        let mut switched = plan("switched");
        switched["backups"] = json!([{"directory": "/root/backups/office-storage-20260921T141320Z", "bytes": 5_000_000}]);
        assert_eq!(
            status_text(&switched),
            "Office storage: /root/office/office.db (migrated 2026-09-21 14:13 UTC)\nRetained backups:\n  /root/backups/office-storage-20260921T141320Z  4.8 MiB\nBackups are kept until you remove them."
        );
        let text = plan_text(&plan("pending"));
        assert!(text.starts_with("This moves Office data into its own database.\n"));
        assert!(text.contains("  Backup size:  3.0 MiB of free space needed\n"));
        assert!(text.ends_with("which also discards any tmt activity after the backup."));
    }

    #[test]
    fn completion_reminds_to_restart_only_a_stopped_service() {
        let result = json!({"database": "/root/office/office.db", "backup": {"directory": "/b", "bytes": 10}, "serviceWasRunning": false});
        let mut stopped = plan("pending");
        stopped["serviceRunning"] = json!(false);
        assert_eq!(
            done_text(&result, &stopped),
            "Office data moved to /root/office/office.db.\nBackup kept at /b (10 B)."
        );
        assert!(
            done_text(&result, &plan("pending"))
                .ends_with("Start it again with 'tmt office start'.")
        );
    }
}
