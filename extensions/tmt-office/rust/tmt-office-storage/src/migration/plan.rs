//! The consented user path: a side-effect-free plan and a migration that runs
//! only for the plan the user saw.

use super::{
    Backup, MigrationError, Quiesce, Result, State, Switched, backups, exists, office_manifest,
    open_source, source_error, source_identity, switch,
};
use crate::{StorageLayout, schema};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Singleton tables core seeds on every install, so a fresh install has rows
/// here without any user data.
const SEEDED: &[&str] = &[
    "office_board_state",
    "office_prop_catalog",
    "office_avatar_catalog",
];

/// What a migration would do now. Computing it takes no lock and creates or
/// changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub state: PlanState,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub backups_directory: PathBuf,
    pub office_rows: u64,
    /// Rows beyond the singleton catalogs core seeds on every install; zero
    /// means there is no user Office data to move yet.
    pub user_rows: u64,
    pub office_bytes: u64,
    /// Space the backup needs: the whole core database.
    pub backup_bytes: u64,
    pub service_running: Option<bool>,
    /// When core recorded the switch, if it did.
    pub switched_at_ms: Option<i64>,
    pub backups: Vec<Backup>,
    pub digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanState {
    /// Office data is in the shared database and can move.
    Pending,
    /// A recorded switch still needs activation or recovery.
    Switching,
    Switched,
    /// The core database lacks the cutover fence; update tmt first.
    Unavailable,
}

impl PlanState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Switching => "switching",
            Self::Switched => "switched",
            Self::Unavailable => "unavailable",
        }
    }
}

pub fn plan(layout: &StorageLayout, service_running: Option<bool>) -> Result<Plan> {
    let backups = backups(layout)?;
    let receipt = super::switch::read_receipt(&layout.source)?;
    let published = exists(&layout.database)?;
    let source = open_source(&layout.source)?;
    let snapshot = source
        .unchecked_transaction()
        .map_err(source_error("Open source snapshot"))?;
    let (manifest, _) = office_manifest(&snapshot)?;
    let office_rows = manifest.rows.iter().map(|(_, count)| count).sum();
    let user_rows = manifest
        .rows
        .iter()
        .filter(|(table, _)| !SEEDED.contains(table))
        .map(|(_, count)| count)
        .sum();
    let mut office_bytes = 0;
    for table in schema::OFFICE_TABLES {
        office_bytes += table_bytes(&snapshot, table)?;
    }
    let (pages, page_size): (i64, i64) = snapshot
        .query_row(
            "SELECT page_count, page_size FROM pragma_page_count, pragma_page_size",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(source_error("Measure the core database"))?;
    let state = match (&receipt, published) {
        (Some(_), true) if super::activated(&layout.database).unwrap_or(false) => {
            PlanState::Switched
        }
        (Some(_), _) => PlanState::Switching,
        (None, _) if super::switch::require_fence(&snapshot).is_err() => PlanState::Unavailable,
        (None, _) => PlanState::Pending,
    };
    drop(snapshot);
    let identity = source_identity(&layout.source)?;
    let mut hasher = Sha256::new();
    for part in [
        state.as_str().to_owned(),
        layout.source.display().to_string(),
        layout.database.display().to_string(),
        identity.device.to_string(),
        identity.inode.to_string(),
        manifest.digest.clone(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    Ok(Plan {
        state,
        source: layout.source.clone(),
        destination: layout.database.clone(),
        backups_directory: layout.backups.clone(),
        office_rows,
        user_rows,
        office_bytes,
        backup_bytes: (pages * page_size).max(0) as u64,
        service_running,
        switched_at_ms: receipt.as_ref().map(|receipt| receipt.switched_at_ms),
        backups,
        digest: hex(&hasher.finalize()),
    })
}

/// Migrates only when the current plan is the one the user consented to.
pub fn migrate(
    layout: &StorageLayout,
    consent: &str,
    service_running: Option<bool>,
    service: &dyn Quiesce,
) -> Result<Switched> {
    let current = plan(layout, service_running)?;
    if current.digest != consent {
        return Err(MigrationError::PlanChanged);
    }
    match current.state {
        PlanState::Pending => {}
        PlanState::Switched | PlanState::Switching => {
            return Err(MigrationError::State(
                "Office storage is already migrated.".to_owned(),
            ));
        }
        PlanState::Unavailable => {
            return Err(MigrationError::Source(
                "The tmt database does not support this migration yet; update tmt first."
                    .to_owned(),
            ));
        }
    }
    let status = super::status(layout)?;
    if status.state != State::Verified {
        super::prepare(layout)?;
        super::copy(layout)?;
        super::verify(layout)?;
    }
    switch(layout, service)
}

fn table_bytes(connection: &rusqlite::Connection, table: &str) -> Result<u64> {
    let columns: Vec<String> = connection
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
        ))
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        })
        .map_err(source_error("Read source table"))?;
    let sum = columns
        .iter()
        .map(|column| format!("coalesce(length(CAST(\"{column}\" AS BLOB)), 0)"))
        .collect::<Vec<_>>()
        .join(" + ");
    connection
        .query_row(
            &format!("SELECT coalesce(sum({sum}), 0) FROM \"{table}\""),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|bytes| bytes.max(0) as u64)
        .map_err(source_error("Measure Office data"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Plan {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state.as_str(),
            "source": self.source,
            "destination": self.destination,
            "backupsDirectory": self.backups_directory,
            "officeRows": self.office_rows,
            "userRows": self.user_rows,
            "officeBytes": self.office_bytes,
            "backupBytes": self.backup_bytes,
            "serviceRunning": self.service_running,
            "switchedAtMs": self.switched_at_ms,
            "backups": self.backups.iter().map(Backup::json).collect::<Vec<_>>(),
            "planDigest": self.digest,
        })
    }
}
