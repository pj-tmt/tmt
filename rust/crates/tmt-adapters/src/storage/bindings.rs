//! SQLite-backed identity bindings.

use rusqlite::{Connection, OptionalExtension, Row, params};
use tmt_core::{
    binding::{Binding, BindingEntry, BindingRecords, BindingRepository},
    endpoint::{PaneObservation, ProcessIncarnation, ServerEvidence, valid_process_id},
    host::HostKind,
    identity::{Identity, IdentityReader},
    names::ValidatedName,
};

use super::{
    Storage, StorageError,
    errors::classify,
    identities::{identity_row_at, with_immediate_transaction},
};

mod session;

const IDENTITY_COLUMNS: &str =
    "i.id, i.name, i.canonical_name, i.lifetime, i.created_at, i.updated_at";
const BINDING_COLUMNS: &str = "b.id, b.identity_id, b.pane_id, b.server_id, b.socket_path, \
    b.server_pid, b.server_start_time, b.pane_pid, b.runtime_state, b.last_transition, \
    b.runtime_pid, b.runtime_start_identity, b.observed_provider_session_id, \
    b.launch_owner_pid, b.launch_owner_start_identity, b.transport, b.pane_incarnation";

pub(super) struct BindingRows<'a>(pub(super) &'a Connection);

fn process_id_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(offset)?)
        .ok()
        .filter(|value| valid_process_id(*value))
        .ok_or(rusqlite::Error::InvalidQuery)
}

fn stored_process_id(value: u64) -> rusqlite::Result<i64> {
    if !valid_process_id(value) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    i64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn binding_row(row: &Row<'_>, offset: usize) -> rusqlite::Result<Binding> {
    Ok(Binding {
        id: row.get(offset)?,
        identity_id: row.get(offset + 1)?,
        server: ServerEvidence {
            // An unknown stored host is invalid, never a guess.
            host: HostKind::parse(&row.get::<_, String>(offset + 15)?)
                .ok_or(rusqlite::Error::InvalidQuery)?,
            server_id: row.get(offset + 3)?,
            socket_path: row.get(offset + 4)?,
            server_pid: process_id_at(row, offset + 5)?,
            server_start_time: row.get(offset + 6)?,
        },
        pane_id: row.get(offset + 2)?,
        pane_pid: process_id_at(row, offset + 7)?,
        pane_incarnation: pane_incarnation_at(row, offset + 16, offset + 7)?,
        session: session::decode_state(row, offset + 8)?,
    })
}

/// A stored pane incarnation must be one a process observation could have
/// produced for the stored pid.
fn pane_incarnation_at(
    row: &Row<'_>,
    offset: usize,
    pid_offset: usize,
) -> rusqlite::Result<Option<String>> {
    row.get::<_, Option<String>>(offset)?
        .map(|start| {
            ProcessIncarnation::new(process_id_at(row, pid_offset)?, &start)
                .map(|_| start)
                .map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .transpose()
}

fn entry_row(row: &Row<'_>) -> rusqlite::Result<BindingEntry> {
    let identity = identity_row_at(row, 0)?;
    let binding = if row.get_ref(6)?.data_type() == rusqlite::types::Type::Null {
        None
    } else {
        Some(binding_row(row, 6)?)
    };
    Ok(BindingEntry { identity, binding })
}

impl IdentityReader for BindingRows<'_> {
    type Error = StorageError;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        super::identities::IdentityRecords(self.0).find_identity(canonical_name)
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        super::identities::IdentityRecords(self.0).list_identities()
    }
}

impl BindingRecords for BindingRows<'_> {
    fn is_auto_named(&self, identity_id: &str) -> Result<bool, Self::Error> {
        self.0
            .query_row(
                "SELECT auto_named FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                [identity_id],
                |row| row.get(0),
            )
            .map_err(|error| classify(error, "Read automatic name provenance"))
    }

    fn save_identity(&mut self, identity: &Identity) -> Result<Identity, Self::Error> {
        tmt_core::identity::IdentityWriter::save_identity(
            &mut super::identities::IdentityRecords(self.0),
            identity,
        )
    }

    fn session_preferences(
        &self,
        identity_id: &str,
    ) -> Result<tmt_core::binding::session::SessionPreferences, Self::Error> {
        session::preferences(self.0, identity_id)
    }

    fn set_session_preferences(
        &mut self,
        identity_id: &str,
        preferences: &tmt_core::binding::session::SessionPreferences,
    ) -> Result<bool, Self::Error> {
        session::set_preferences(self.0, identity_id, preferences)
    }

    fn set_session_state(
        &mut self,
        binding_id: &str,
        expected: &tmt_core::binding::session::BindingSessionState,
        state: &tmt_core::binding::session::BindingSessionState,
    ) -> Result<bool, Self::Error> {
        session::set_state(self.0, binding_id, expected, state)
    }

    fn entry_by_id(&self, id: &str) -> Result<Option<BindingEntry>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {IDENTITY_COLUMNS}, {BINDING_COLUMNS} \
                     FROM identities AS i LEFT JOIN bindings AS b ON b.identity_id = i.id \
                     WHERE i.id = ? AND i.retired_at_ms IS NULL"
                ),
                [id],
                entry_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find binding"))
    }

    fn entry_by_pane(
        &self,
        host: HostKind,
        pane: &str,
        server: &str,
    ) -> Result<Option<BindingEntry>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {IDENTITY_COLUMNS}, {BINDING_COLUMNS} \
                     FROM bindings AS b JOIN identities AS i ON i.id = b.identity_id \
                     WHERE b.transport = ? AND b.pane_id = ? AND b.server_id = ? \
                       AND i.retired_at_ms IS NULL"
                ),
                [host.as_str(), pane, server],
                entry_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find binding by pane"))
    }

    fn binding_entries(&self) -> Result<Vec<BindingEntry>, Self::Error> {
        let mut statement = self
            .0
            .prepare(&format!(
                "SELECT {IDENTITY_COLUMNS}, {BINDING_COLUMNS} \
                 FROM identities AS i LEFT JOIN bindings AS b ON b.identity_id = i.id \
                 WHERE i.retired_at_ms IS NULL \
                 ORDER BY i.canonical_name COLLATE BINARY"
            ))
            .map_err(|error| classify(error, "Prepare binding list"))?;
        statement
            .query_map([], entry_row)
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "List bindings"))
    }

    fn insert_binding(
        &mut self,
        identity: &Identity,
        server: &ServerEvidence,
        pane: &PaneObservation,
    ) -> Result<Binding, Self::Error> {
        // Reject before INSERT, including callers that handle an error inside
        // their transaction. Conversion is checked at this storage boundary.
        let server_pid = stored_process_id(server.server_pid)
            .map_err(|error| classify(error, "Validate binding server PID"))?;
        let pane_pid = stored_process_id(pane.pane_pid)
            .map_err(|error| classify(error, "Validate binding pane PID"))?;
        let id = uuid::Uuid::new_v4().to_string();
        self.0
            .query_row(
                "INSERT INTO bindings (
                    id, identity_id, transport, pane_id, server_id, socket_path,
                    server_pid, server_start_time, pane_pid, pane_incarnation, bound_at,
                    last_verified_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 RETURNING id, identity_id, pane_id, server_id, socket_path,
                    server_pid, server_start_time, pane_pid, runtime_state, last_transition,
                    runtime_pid, runtime_start_identity, observed_provider_session_id,
                    launch_owner_pid, launch_owner_start_identity, transport, pane_incarnation",
                params![
                    id,
                    identity.id,
                    server.host.as_str(),
                    pane.id,
                    server.server_id,
                    server.socket_path,
                    server_pid,
                    server.server_start_time,
                    pane_pid,
                    pane.pane_incarnation,
                ],
                |row| binding_row(row, 0),
            )
            .map_err(|error| classify(error, "Create binding"))
    }

    fn touch_binding(&mut self, id: &str) -> Result<(), Self::Error> {
        self.0
            .execute(
                "UPDATE bindings SET last_verified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?",
                [id],
            )
            .map_err(|error| classify(error, "Touch binding"))?;
        Ok(())
    }

    fn detach_binding(&mut self, id: &str) -> Result<(), Self::Error> {
        self.0
            .execute("DELETE FROM bindings WHERE id = ?", [id])
            .map_err(|error| classify(error, "Detach binding"))?;
        Ok(())
    }

    fn retire_identity(
        &mut self,
        identity: &Identity,
        remove_content: bool,
    ) -> Result<(), Self::Error> {
        self.0
            .execute("DELETE FROM bindings WHERE identity_id = ?", [&identity.id])
            .map_err(|error| classify(error, "Detach identity binding"))?;
        session::forget(self.0, &identity.id)?;
        if remove_content {
            self.0
                .execute(
                    "DELETE FROM identity_metadata WHERE identity_id = ?",
                    [&identity.id],
                )
                .map_err(|error| classify(error, "Remove identity metadata"))?;
            self.0
                .execute(
                    "DELETE FROM role_profiles WHERE identity_id = ?",
                    [&identity.id],
                )
                .map_err(|error| classify(error, "Remove identity role"))?;
            self.0
                .execute(
                    "DELETE FROM identity_preambles WHERE identity_id = ?",
                    [&identity.id],
                )
                .map_err(|error| classify(error, "Remove identity preamble"))?;
        }
        let changed = self
            .0
            .execute(
                "UPDATE identities
                 SET retired_at_ms = CAST(strftime('%s','now') AS INTEGER) * 1000
                    + CAST(substr(strftime('%f','now'), 4, 3) AS INTEGER)
                 WHERE id = ? AND retired_at_ms IS NULL",
                [&identity.id],
            )
            .map_err(|error| classify(error, "Retire identity"))?;
        if changed == 1 {
            super::identity_hooks::enqueue_retirement(self.0, &identity.id)?;
        }
        Ok(())
    }

    fn rename_identity(
        &mut self,
        identity: &Identity,
        name: &ValidatedName,
    ) -> Result<Identity, Self::Error> {
        // The partial unique index on unretired canonical names is the final
        // arbiter; the service checks first to report which name is taken.
        self.0
            .query_row(
                "UPDATE identities SET name = ?, canonical_name = ?, auto_named = 0,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ? AND retired_at_ms IS NULL
                 RETURNING id, name, canonical_name, lifetime, created_at, updated_at",
                params![name.display_name(), name.canonical_name(), identity.id],
                |row| identity_row_at(row, 0),
            )
            .map_err(|error| classify(error, "Rename identity"))
    }
}

impl BindingRepository for Storage {
    fn with_binding_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn BindingRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        with_immediate_transaction(self, "binding", |transaction| {
            operation(&mut BindingRows(transaction))
        })
    }
}

/// A remembered session dropped because its driver is no longer registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgedSession {
    pub identity_id: String,
    pub name: String,
    pub harness: String,
}

impl Storage {
    /// Read an active identity's remembered preferences without a write
    /// transaction, for read-only projections such as show and list.
    pub fn session_preferences(
        &self,
        identity_id: &str,
    ) -> Result<tmt_core::binding::session::SessionPreferences, StorageError> {
        session::preferences(self.connection()?, identity_id)
    }

    /// Purge remembered sessions, with their driver state, whose driver is not
    /// registered, so no state outlives its driver. Returns what was purged,
    /// for the one report the purging command makes.
    pub fn purge_unregistered_sessions(
        &mut self,
        registered: &[&str],
    ) -> Result<Vec<PurgedSession>, StorageError> {
        with_immediate_transaction(self, "session purge", |transaction| {
            session::purge_unregistered(transaction, registered)
        })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod service_tests;

#[cfg(test)]
mod test_support;
