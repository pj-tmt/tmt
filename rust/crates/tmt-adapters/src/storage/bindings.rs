//! SQLite-backed identity bindings.

use rusqlite::{Connection, OptionalExtension, Row, params};
use tmt_core::{
    binding::{Binding, BindingEntry, BindingRecords, BindingRepository, BindingReservation},
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
pub(super) const BINDING_COLUMNS: &str = "b.id, b.identity_id, b.pane_id, b.server_id, b.socket_path, \
    b.server_pid, b.server_start_time, b.pane_pid, b.runtime_state, b.last_transition, \
    b.runtime_pid, b.runtime_start_identity, b.observed_provider_session_id, \
    b.launch_owner_pid, b.launch_owner_start_identity, b.transport, b.pane_incarnation, b.runtime_start_refusal, \
    b.plain_program";

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

const RESERVATION_COLUMNS: &str = "target_binding_id, identity_id, old_binding_id, transport, server_id, socket_path, server_pid, server_start_time, pane_id, pane_pid, pane_incarnation, expected_binding_id, publisher_pid, publisher_start, created_at_ms, expires_at_ms";

fn reservation_row(row: &Row<'_>) -> rusqlite::Result<BindingReservation> {
    let publisher_pid = process_id_at(row, 12)?;
    let publisher_start: String = row.get(13)?;
    let publisher = ProcessIncarnation::new(publisher_pid, &publisher_start)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let created_at_ms =
        u64::try_from(row.get::<_, i64>(14)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let expires_at_ms =
        u64::try_from(row.get::<_, i64>(15)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let reservation = BindingReservation {
        target: Binding {
            id: row.get(0)?,
            identity_id: row.get(1)?,
            server: ServerEvidence {
                host: HostKind::parse(&row.get::<_, String>(3)?)
                    .ok_or(rusqlite::Error::InvalidQuery)?,
                server_id: row.get(4)?,
                socket_path: row.get(5)?,
                server_pid: process_id_at(row, 6)?,
                server_start_time: row.get(7)?,
            },
            pane_id: row.get(8)?,
            pane_pid: process_id_at(row, 9)?,
            pane_incarnation: pane_incarnation_at(row, 10, 9)?,
            session: Default::default(),
        },
        old_binding_id: row.get(2)?,
        expected_binding_id: row.get(11)?,
        publisher,
        created_at_ms,
        expires_at_ms,
    };
    if !valid_reservation(&reservation) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(reservation)
}

fn valid_reservation(reservation: &BindingReservation) -> bool {
    let target = &reservation.target;
    tmt_core::dispatch::canonical_id(&target.id)
        && tmt_core::dispatch::canonical_id(&target.identity_id)
        && target.session == Default::default()
        && target
            .pane_incarnation
            .as_ref()
            .is_none_or(|start| ProcessIncarnation::new(target.pane_pid, start).is_ok())
        && reservation
            .old_binding_id
            .as_ref()
            .is_none_or(|id| tmt_core::dispatch::canonical_id(id))
        && reservation
            .expected_binding_id
            .as_ref()
            .is_none_or(|id| tmt_core::dispatch::canonical_id(id))
        && reservation.created_at_ms <= reservation.expires_at_ms
        && reservation.expires_at_ms <= tmt_core::limits::MAX_JS_SAFE_INTEGER
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
    fn begin_launch_observation(
        &mut self,
        binding: &Binding,
        owner: &ProcessIncarnation,
        driver: &tmt_core::binding::session::HarnessId,
        now_ms: u64,
    ) -> Result<bool, Self::Error> {
        if self
            .entry_by_id(&binding.identity_id)?
            .and_then(|entry| entry.binding)
            .as_ref()
            != Some(binding)
        {
            return Ok(false);
        }
        super::launch_observations::begin(self.0, &binding.id, owner, driver, now_ms)
    }
    fn launch_preferences_match(
        &self,
        observation: &tmt_core::binding::session::launch::LaunchObservation,
    ) -> Result<bool, Self::Error> {
        super::launch_observations::preferences_match(self.0, observation)
    }
    fn launch_observation(
        &self,
        binding_id: &str,
        owner: &ProcessIncarnation,
    ) -> Result<Option<tmt_core::binding::session::launch::LaunchObservation>, Self::Error> {
        super::launch_observations::read(self.0, binding_id, owner)
    }
    fn set_launch_observation(
        &mut self,
        expected: &tmt_core::binding::session::launch::LaunchObservation,
        next: &tmt_core::binding::session::launch::LaunchObservation,
    ) -> Result<bool, Self::Error> {
        super::launch_observations::replace(self.0, expected, next)
    }
    fn insert_identity(
        &mut self,
        name: &ValidatedName,
        lifetime: tmt_core::identity::Lifetime,
    ) -> Result<Identity, Self::Error> {
        tmt_core::identity::IdentityWriter::insert_identity(
            &mut super::identities::IdentityRecords(self.0),
            name,
            lifetime,
            false,
        )
    }

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
        let changed = session::set_state(self.0, binding_id, expected, state)?;
        if changed
            && (state.state != tmt_core::binding::session::RuntimeState::Running
                || state.key != expected.key)
        {
            self.0.execute("UPDATE capture_batches SET closed_ms=?2 WHERE binding_id=?1 AND closed_ms IS NULL",params![binding_id,crate::request_runtime::wall_time_ms() as i64]).map_err(|e|classify(e,"Fence input batch on runtime transition"))?;
        }
        Ok(changed)
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

    fn entries_for_pane(
        &self,
        host: HostKind,
        pane: &str,
    ) -> Result<Vec<BindingEntry>, Self::Error> {
        let mut statement = self
            .0
            .prepare(&format!(
                "SELECT {IDENTITY_COLUMNS}, {BINDING_COLUMNS} \
                 FROM bindings AS b JOIN identities AS i ON i.id = b.identity_id \
                 WHERE b.transport = ? AND b.pane_id = ? AND i.retired_at_ms IS NULL"
            ))
            .map_err(|error| classify(error, "Prepare pane binding candidates"))?;
        statement
            .query_map([host.as_str(), pane], entry_row)
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "Read pane binding candidates"))
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
                    launch_owner_pid, launch_owner_start_identity, transport, pane_incarnation, runtime_start_refusal, plain_program",
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

    fn binding_exists(&self, binding_id: &str) -> Result<bool, Self::Error> {
        self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM bindings WHERE id = ?) OR EXISTS(SELECT 1 FROM binding_reservations WHERE target_binding_id = ?)",
            [binding_id, binding_id], |row| row.get(0),
        ).map_err(|error| classify(error, "Check marker binding"))
    }

    fn reservation_by_target(
        &self,
        target_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error> {
        self.0.query_row(&format!("SELECT {RESERVATION_COLUMNS} FROM binding_reservations WHERE target_binding_id = ?"), [target_id], reservation_row)
            .optional().map_err(|error| classify(error, "Read target reservation"))
    }

    fn reservation_by_identity(
        &self,
        identity_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {RESERVATION_COLUMNS} FROM binding_reservations WHERE identity_id = ?"
                ),
                [identity_id],
                reservation_row,
            )
            .optional()
            .map_err(|error| classify(error, "Read identity reservation"))
    }

    fn reservation_by_pane(
        &self,
        host: HostKind,
        server_id: &str,
        pane_id: &str,
    ) -> Result<Option<BindingReservation>, Self::Error> {
        self.0.query_row(&format!("SELECT {RESERVATION_COLUMNS} FROM binding_reservations WHERE transport = ? AND server_id = ? AND pane_id = ?"),
            [host.as_str(), server_id, pane_id], reservation_row)
            .optional().map_err(|error| classify(error, "Read pane reservation"))
    }

    fn begin_binding_reservation(
        &mut self,
        reservation: &BindingReservation,
    ) -> Result<(), Self::Error> {
        let target = &reservation.target;
        if !valid_reservation(reservation) {
            return Err(classify(
                rusqlite::Error::InvalidQuery,
                "Validate binding reservation",
            ));
        }
        let server_pid = stored_process_id(target.server.server_pid)
            .map_err(|e| classify(e, "Validate server PID"))?;
        let pane_pid =
            stored_process_id(target.pane_pid).map_err(|e| classify(e, "Validate pane PID"))?;
        let publisher_pid = stored_process_id(reservation.publisher.pid())
            .map_err(|e| classify(e, "Validate publisher PID"))?;
        let created = i64::try_from(reservation.created_at_ms)
            .map_err(|_| classify(rusqlite::Error::InvalidQuery, "Validate reservation time"))?;
        let expires = i64::try_from(reservation.expires_at_ms)
            .map_err(|_| classify(rusqlite::Error::InvalidQuery, "Validate reservation lease"))?;
        self.0.execute(
            "INSERT INTO binding_reservations (target_binding_id, identity_id, old_binding_id, transport, server_id, socket_path, server_pid, server_start_time, pane_id, pane_pid, pane_incarnation, expected_binding_id, publisher_pid, publisher_start, created_at_ms, expires_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![target.id, target.identity_id, reservation.old_binding_id, target.server.host.as_str(), target.server.server_id,
                target.server.socket_path, server_pid, target.server.server_start_time, target.pane_id, pane_pid,
                target.pane_incarnation, reservation.expected_binding_id, publisher_pid, reservation.publisher.start_identity(), created, expires],
        ).map_err(|error| classify(error, "Reserve binding"))?;
        Ok(())
    }

    fn finish_binding_reservation(
        &mut self,
        reservation: &BindingReservation,
    ) -> Result<bool, Self::Error> {
        if self.reservation_by_target(&reservation.target.id)?.as_ref() != Some(reservation) {
            return Ok(false);
        }
        self.0
            .execute(
                "DELETE FROM binding_reservations WHERE target_binding_id = ?",
                [&reservation.target.id],
            )
            .map(|count| count == 1)
            .map_err(|error| classify(error, "Finish binding reservation"))
    }

    fn insert_reserved_binding(
        &mut self,
        identity: &Identity,
        reservation: &BindingReservation,
    ) -> Result<Binding, Self::Error> {
        let target = &reservation.target;
        if target.identity_id != identity.id {
            return Err(classify(
                rusqlite::Error::InvalidQuery,
                "Validate reserved identity",
            ));
        }
        let server_pid = stored_process_id(target.server.server_pid)
            .map_err(|e| classify(e, "Validate server PID"))?;
        let pane_pid =
            stored_process_id(target.pane_pid).map_err(|e| classify(e, "Validate pane PID"))?;
        self.0.query_row(
            "INSERT INTO bindings (id, identity_id, transport, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, pane_incarnation, bound_at, last_verified_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) RETURNING id, identity_id, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, runtime_state, last_transition, runtime_pid, runtime_start_identity, observed_provider_session_id, launch_owner_pid, launch_owner_start_identity, transport, pane_incarnation, runtime_start_refusal, plain_program",
            params![target.id, identity.id, target.server.host.as_str(), target.pane_id, target.server.server_id, target.server.socket_path,
                server_pid, target.server.server_start_time, pane_pid, target.pane_incarnation], |row| binding_row(row, 0),
        ).map_err(|error| classify(error, "Activate reserved binding"))
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
        session::end_continuation_of_binding(self.0, id)?;
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
        session::end_continuation_of_identity(self.0, &identity.id)?;
        self.0
            .execute("DELETE FROM bindings WHERE identity_id = ?", [&identity.id])
            .map_err(|error| classify(error, "Detach identity binding"))?;
        session::forget(self.0, &identity.id)?;
        self.0
            .execute(
                "DELETE FROM capture_entries WHERE identity_id=?",
                [&identity.id],
            )
            .map_err(|e| classify(e, "Remove conversation entries"))?;
        self.0
            .execute(
                "DELETE FROM capture_floor WHERE identity_id=?",
                [&identity.id],
            )
            .map_err(|e| classify(e, "Remove conversation coverage"))?;
        self.0
            .execute(
                "DELETE FROM capture_batches WHERE identity_id=?",
                [&identity.id],
            )
            .map_err(|e| classify(e, "Remove input batches"))?;
        self.0
            .execute(
                "DELETE FROM recent_responses WHERE identity_id=?",
                [&identity.id],
            )
            .map_err(|error| classify(error, "Remove recent responses"))?;
        self.0
            .execute(
                "DELETE FROM recent_response_floor WHERE identity_id=?",
                [&identity.id],
            )
            .map_err(|error| classify(error, "Remove the dropped-response floor"))?;
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
    /// Whether an agent is bound to this identity (a recorded pane or endpoint).
    /// The person's identity never takes over such a name.
    pub fn identity_has_binding(&self, identity_id: &str) -> Result<bool, StorageError> {
        self.connection()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM bindings WHERE identity_id = ?)",
                [identity_id],
                |row| row.get(0),
            )
            .map_err(|error| classify(error, "Check identity binding"))
    }

    /// The plain program an unretired identity's current binding runs, read without a write
    /// transaction for polled projections. Detaching deletes the binding row, so only a binding
    /// still attached to its pane reports one.
    pub fn bound_plain_program(&self, identity_id: &str) -> Result<Option<String>, StorageError> {
        self.connection()?
            .query_row(
                "SELECT b.plain_program FROM bindings AS b JOIN identities AS i ON i.id = b.identity_id \
                 WHERE b.identity_id = ? AND i.retired_at_ms IS NULL",
                [identity_id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(|error| classify(error, "Read plain program"))
    }

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

impl Storage {
    /// One read-only joined projection. Host work never occurs in this snapshot.
    pub fn workspace_identities(
        path: &std::path::Path,
        socket: &str,
    ) -> Result<Vec<tmt_core::workspace::StoredIdentity>, StorageError> {
        workspace_projection(path, "b.socket_path = ?", &[socket])
    }

    /// Unretired identities bound on the built-in host, read only. A missing database is
    /// handled by the caller; this never creates one.
    pub fn builtin_workspace_identities(
        path: &std::path::Path,
    ) -> Result<Vec<tmt_core::workspace::StoredIdentity>, StorageError> {
        workspace_projection(path, "b.transport = ?", &[HostKind::pty().as_str()])
    }

    /// Resolve exact snapshot UUIDs, including saved identities now unbound.
    /// A missing database is handled by the caller; this never creates one.
    pub fn workspace_plan_identities(
        path: &std::path::Path,
        ids: &[&str],
    ) -> Result<Vec<tmt_core::workspace::StoredIdentity>, StorageError> {
        if ids.len() > tmt_core::workspace::MAX_PANES {
            return Err(classify(
                rusqlite::Error::InvalidQuery,
                "Workspace identity limit",
            ));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        workspace_projection(path, &format!("i.id IN ({placeholders})"), ids)
    }
}

fn workspace_projection(
    path: &std::path::Path,
    predicate: &str,
    values: &[&str],
) -> Result<Vec<tmt_core::workspace::StoredIdentity>, StorageError> {
    let mut connection =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| classify(error, "Open workspace projection"))?;
    connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(|error| classify(error, "Bound workspace projection"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| classify(error, "Begin workspace projection"))?;
    super::migrations::require_current(&transaction)?;
    let mut records = {
        let mut statement = transaction.prepare(&format!(
                "SELECT {IDENTITY_COLUMNS}, {BINDING_COLUMNS}, p.preferred_harness, p.remembered_harness, p.runtime_mode, p.provider_session_id, p.driver_state_version, p.driver_state, p.stale_at_ms, p.resume_pending_at_ms, p.channel \
                 FROM identities i LEFT JOIN bindings b ON b.identity_id = i.id \
                 LEFT JOIN identity_session_preferences p ON p.identity_id = i.id \
                 WHERE i.retired_at_ms IS NULL AND {predicate} LIMIT 1025"
            )).map_err(|error| classify(error, "Prepare workspace projection"))?;
        statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok(tmt_core::workspace::StoredIdentity {
                    entry: entry_row(row)?,
                    preferences: session::decode_preferences(row, 25)?,
                })
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|error| classify(error, "Read workspace projection"))?
    };
    for record in &mut records {
        record.preferences = session::preferences(&transaction, &record.entry.identity.id)?;
    }
    if records.len() > tmt_core::workspace::MAX_PANES {
        return Err(classify(
            rusqlite::Error::InvalidQuery,
            "Workspace identity limit",
        ));
    }
    transaction
        .commit()
        .map_err(|error| classify(error, "Finish workspace projection"))?;
    connection
        .close()
        .map_err(|(_, error)| classify(error, "Close workspace projection"))?;
    Ok(records)
}
