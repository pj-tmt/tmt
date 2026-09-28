//! Office's durable record of identity retirements, consulted by the pairing
//! fence before any write that grants or extends pairing authority.
//!
//! Core's retirement stays authoritative: an identity counts as retired when
//! Office recorded it or core reports it retired. Only `office.db` holds the
//! marker; before the storage switch marking changes nothing, because the
//! shared core database already records the retirement.

use crate::{OfficeStore, StorageLayout};
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
