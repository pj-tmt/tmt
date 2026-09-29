//! TMT's own server ID for each server incarnation of a host that keeps no
//! server-level store (Herdr, #479; schema 39). tmux keeps its ID in a server
//! option instead. A restarted server is a new incarnation with a new ID.

use rusqlite::{OptionalExtension, params};

use super::{
    Storage, StorageError,
    errors::{StorageErrorCode, classify},
    identities::with_immediate_transaction,
};

use tmt_core::host::HostServerIds;
pub use tmt_core::host::HostServerIncarnation;

impl Storage {
    /// The UUIDv4 for this incarnation, created the first time it is seen.
    pub fn host_server_id(
        &mut self,
        server: &HostServerIncarnation<'_>,
        now_ms: u64,
    ) -> Result<String, StorageError> {
        let pid = i64::try_from(server.server_pid)
            .map_err(|_| StorageError::new(StorageErrorCode::Unknown, "Invalid server PID"))?;
        let now = i64::try_from(now_ms)
            .map_err(|_| StorageError::new(StorageErrorCode::Unknown, "Invalid clock value"))?;
        with_immediate_transaction(self, "host server", |transaction| {
            let key = params![
                server.host.as_str(),
                server.socket_path,
                pid,
                server.server_start_time
            ];
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT server_id FROM host_servers WHERE host = ? AND socket_path = ?
                     AND server_pid = ? AND server_start_time = ?",
                    key,
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| classify(error, "Read host server"))?;
            if let Some(id) = existing {
                return Ok(id);
            }
            let id = uuid::Uuid::new_v4().to_string();
            transaction
                .execute(
                    "INSERT INTO host_servers (server_id, host, socket_path, server_pid,
                     server_start_time, first_seen_at_ms) VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        id,
                        server.host.as_str(),
                        server.socket_path,
                        pid,
                        server.server_start_time,
                        now
                    ],
                )
                .map_err(|error| classify(error, "Record host server"))?;
            Ok(id)
        })
    }
}

impl HostServerIds for Storage {
    type Error = StorageError;

    fn server_id(&mut self, server: &HostServerIncarnation<'_>) -> Result<String, StorageError> {
        self.host_server_id(server, crate::request_runtime::wall_time_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    fn herdr<'a>(pid: u64, started: &'a str) -> HostServerIncarnation<'a> {
        HostServerIncarnation {
            host: tmt_core::host::HostKind::Herdr,
            socket_path: "/tmp/herdr.sock",
            server_pid: pid,
            server_start_time: started,
        }
    }

    #[test]
    fn one_incarnation_keeps_its_id_and_a_restart_gets_a_new_one() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
        let first = storage.host_server_id(&herdr(41, "Tue 22:09"), 1).unwrap();
        assert!(tmt_core::endpoint::valid_server_id(&first), "{first}");
        assert_eq!(
            storage.host_server_id(&herdr(41, "Tue 22:09"), 2).unwrap(),
            first
        );
        // A reused PID with another start time is another incarnation.
        let restarted = storage.host_server_id(&herdr(41, "Tue 23:00"), 3).unwrap();
        assert_ne!(restarted, first);
        assert!(tmt_core::endpoint::valid_server_id(&restarted));
        let other_socket = HostServerIncarnation {
            socket_path: "/tmp/other.sock",
            ..herdr(41, "Tue 22:09")
        };
        assert_ne!(storage.host_server_id(&other_socket, 4).unwrap(), first);
        let seen: i64 = storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT first_seen_at_ms FROM host_servers WHERE server_id = ?",
                [&first],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(seen, 1);
        storage.close().unwrap();
    }

    #[test]
    fn only_hosts_without_their_own_store_are_recorded() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
        let tmux = HostServerIncarnation {
            host: tmt_core::host::HostKind::Tmux,
            ..herdr(41, "Tue 22:09")
        };
        assert!(storage.host_server_id(&tmux, 1).is_err());
        assert!(storage.host_server_id(&herdr(0, "Tue 22:09"), 1).is_err());
        assert!(storage.host_server_id(&herdr(41, ""), 1).is_err());
        let count: i64 = storage
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM host_servers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        storage.close().unwrap();
    }
}
