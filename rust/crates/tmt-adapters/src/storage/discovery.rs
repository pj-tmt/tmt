//! Bounded existing binding projection. No migration, reconciliation or writes.

use super::{
    Storage, StorageError,
    bindings::BindingRows,
    errors::{classify, classify_io},
    migrations,
};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{BindingEntry, BindingRecords},
    discovery::MAX_CANDIDATES,
};

impl Storage {
    pub fn discovery_entries(
        path: &Path,
        socket: &str,
        deadline: Instant,
    ) -> Result<(Vec<BindingEntry>, bool), StorageError> {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), false));
            }
            Err(error) => return Err(classify_io(error, "Inspect existing discovery storage")),
            Ok(_) => {}
        }
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Open discovery storage read-only"))?;
        connection
            .busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable discovery storage wait"))?;
        let result = (|| {
            let transaction = connection
                .transaction()
                .map_err(|error| classify(error, "Observe discovery storage snapshot"))?;
            migrations::require_current(&transaction)?;
            let mut statement = transaction.prepare("SELECT b.identity_id FROM bindings b JOIN identities i ON b.identity_id=i.id WHERE b.transport=? AND b.socket_path=? AND i.retired_at_ms IS NULL ORDER BY b.pane_id LIMIT ?").map_err(|e|classify(e,"Prepare bounded discovery bindings"))?;
            let ids = statement
                .query_map(
                    params![
                        tmt_core::host::HostKind::Tmux.as_str(),
                        socket,
                        (MAX_CANDIDATES + 1) as i64
                    ],
                    |r| r.get::<_, String>(0),
                )
                .and_then(|r| r.collect::<Result<Vec<_>, _>>())
                .map_err(|e| classify(e, "Read discovery binding IDs"))?;
            let mut truncated = ids.len() > MAX_CANDIDATES;
            let mut entries = Vec::new();
            for id in ids.iter().take(MAX_CANDIDATES) {
                if Instant::now() >= deadline {
                    truncated = true;
                    break;
                }
                if let Some(entry) = BindingRows(&transaction).entry_by_id(id)? {
                    entries.push(entry);
                }
            }
            drop(statement);
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish discovery storage snapshot"))?;
            Ok((entries, truncated))
        })();
        let closed = connection
            .close()
            .map_err(|(_, e)| classify(e, "Close discovery storage"));
        match result {
            Ok(value) => closed.map(|()| value),
            Err(error) => Err(error),
        }
    }
}
