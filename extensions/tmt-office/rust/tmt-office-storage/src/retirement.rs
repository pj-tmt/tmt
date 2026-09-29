//! Office's durable record of identity retirements, consulted by the pairing
//! fence before any write that grants or extends pairing authority.
//!
//! Core's retirement stays authoritative: an identity counts as retired when
//! Office recorded it or core reports it retired. Only `office.db` holds the
//! marker; before the storage switch marking changes nothing, because the
//! shared core database already records the retirement.

use crate::{
    OfficeStore, StorageLayout,
    core_references::{CoreIdentity, CoreReferences, CoreRoom},
};
use rusqlite::OptionalExtension;
use std::time::{SystemTime, UNIX_EPOCH};
use tmt_adapters::{
    config::ConfigPaths,
    office_pairing::RetirementFence,
    storage::{StorageError, classify},
};
use tmt_office_model::office_protocol::OfficeError;

impl OfficeStore {
    pub(crate) fn has_retirement_marker(&self, identity_id: &str) -> Result<bool, StorageError> {
        if !self.is_office_database() {
            return Ok(false);
        }
        self.connection()?
            .query_row(
                "SELECT 1 FROM office_retired_identities WHERE identity_id = ?",
                [identity_id],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
            .map_err(|error| classify(error, "Read Office retirement marker"))
    }

    /// Records that Office observed a retirement. The retirement time is the
    /// observation time; core keeps the authoritative timestamp.
    pub(crate) fn mark_retired(
        &mut self,
        identity_id: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        if !self.is_office_database() {
            return Ok(());
        }
        self.connection()?
            .execute(
                "INSERT INTO office_retired_identities (identity_id, retired_at_ms, recorded_at_ms) VALUES (?, ?, ?) ON CONFLICT (identity_id) DO NOTHING",
                rusqlite::params![identity_id, now_ms, now_ms],
            )
            .map(|_| ())
            .map_err(|error| classify(error, "Record Office retirement marker"))
    }
}

/// The pairing fence over the configured Office store.
pub struct OfficeRetirementFence {
    layout: Option<StorageLayout>,
}

impl OfficeRetirementFence {
    /// Uses the same configuration discovery as the pairing operation itself.
    pub fn discover() -> Self {
        Self {
            layout: ConfigPaths::discover()
                .ok()
                .map(|paths| StorageLayout::new(&paths)),
        }
    }

    pub fn new(layout: StorageLayout) -> Self {
        Self {
            layout: Some(layout),
        }
    }

    fn store(&self) -> Result<OfficeStore, OfficeError> {
        let layout = self
            .layout
            .as_ref()
            .ok_or(OfficeError::CredentialsUnavailable)?;
        OfficeStore::open_configured(layout).map_err(|_| OfficeError::CredentialsUnavailable)
    }
}

impl RetirementFence for OfficeRetirementFence {
    fn is_retired(&self, identity_id: &str) -> Result<bool, OfficeError> {
        let store = self.store()?;
        if store
            .has_retirement_marker(identity_id)
            .map_err(|_| OfficeError::CredentialsUnavailable)?
        {
            return Ok(true);
        }
        // An identity core does not know is fenced too.
        Ok(store
            .references()
            .identity(identity_id)
            .map_err(|_| OfficeError::CredentialsUnavailable)?
            .is_none_or(|identity| identity.retired))
    }

    fn mark_retired(&self, identity_id: &str) -> Result<(), OfficeError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| OfficeError::CredentialsUnavailable)?
            .as_millis() as i64;
        self.store()?
            .mark_retired(identity_id, now_ms)
            .map_err(|_| OfficeError::CredentialsUnavailable)
    }
}

/// Office's recorded retirements, loaded when `office.db` opens.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Markers {
    identities: std::collections::HashSet<String>,
    rooms: std::collections::HashSet<String>,
}

impl Markers {
    pub(crate) fn load(connection: &rusqlite::Connection) -> rusqlite::Result<Self> {
        let ids = |sql: &str| -> rusqlite::Result<std::collections::HashSet<String>> {
            connection
                .prepare(sql)?
                .query_map([], |row| row.get(0))?
                .collect()
        };
        Ok(Self {
            identities: ids("SELECT identity_id FROM office_retired_identities")?,
            rooms: ids("SELECT room_id FROM office_retired_rooms")?,
        })
    }
}

/// Core references with Office's markers applied: a marked identity or room
/// reads as retired, never as missing, so history stays visible and no new
/// authority is granted.
pub(crate) struct MarkedReferences {
    core: Box<dyn CoreReferences + Send>,
    markers: Markers,
}

impl MarkedReferences {
    pub(crate) fn new(core: Box<dyn CoreReferences + Send>, markers: Markers) -> Self {
        Self { core, markers }
    }
}

impl CoreReferences for MarkedReferences {
    fn identity(&self, id: &str) -> Result<Option<CoreIdentity>, StorageError> {
        Ok(self.core.identity(id)?.map(|mut identity| {
            identity.retired |= self.markers.identities.contains(id);
            identity
        }))
    }

    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError> {
        Ok(self
            .core
            .active_identities()?
            .into_iter()
            .filter(|identity| !self.markers.identities.contains(&identity.id))
            .collect())
    }

    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError> {
        Ok(self.core.room(id)?.map(|mut room| {
            room.retired |= self.markers.rooms.contains(id);
            room
        }))
    }
}

/// Placeholder while the store swaps its reference port; never queried.
pub(crate) struct Unset;

impl CoreReferences for Unset {
    fn identity(&self, _: &str) -> Result<Option<CoreIdentity>, StorageError> {
        unreachable!("replaced before use")
    }
    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError> {
        unreachable!("replaced before use")
    }
    fn room(&self, _: &str) -> Result<Option<CoreRoom>, StorageError> {
        unreachable!("replaced before use")
    }
}

/// Which marker a reference write checks inside its Office transaction.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Marker {
    Identity,
    Room,
}

/// Whether Office recorded this reference as retired. The shared core file
/// before the switch has no marker tables, so it reports nothing there.
pub(crate) fn is_marked(
    connection: &rusqlite::Connection,
    marker: Marker,
    id: &str,
) -> Result<bool, StorageError> {
    let (table, column) = match marker {
        Marker::Identity => ("office_retired_identities", "identity_id"),
        Marker::Room => ("office_retired_rooms", "room_id"),
    };
    let present: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Read Office retirement markers"))?;
    if !present {
        return Ok(false);
    }
    connection
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column} = ?)"),
            [id],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Read Office retirement markers"))
}
