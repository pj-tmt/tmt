//! UUID-keyed reads of core-owned identities and rooms used as Office
//! preflight. Office never reads core tables in its own transactions.
//!
//! These are same-user consistency checks, not an authorization boundary.
//! A reference retired between preflight and commit leaves the same state as
//! the serial order "Office commit, then retirement": identities are never
//! deleted, rooms are retired rather than removed, and core retirement never
//! changes Office rows. The values are plain data so a later implementation
//! over the public `tmt api` can replace [`CoreStore`] without changing callers.

use std::{cell::RefCell, path::Path};
use tmt_adapters::storage::{RoomStoreError, Storage, StorageError, StorageErrorCode};
use tmt_core::{
    identity::{IdentityReader, Lifetime},
    room::RoomRepository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreIdentity {
    pub id: String,
    pub name: String,
    pub lifetime: Lifetime,
    pub retired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRoom {
    pub id: String,
    pub retired: bool,
}

pub trait CoreReferences {
    /// Any identity with this UUID, including a retired one.
    fn identity(&self, id: &str) -> Result<Option<CoreIdentity>, StorageError>;
    /// Every active identity in core's canonical name order.
    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError>;
    /// Any room with this UUID, including a retired one.
    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError>;
}

/// In-process implementation over core's public repository API.
///
/// Retained debt owned by #355: an independently versioned Office binary must
/// eventually reach core only through `tmt api`, never opening or migrating
/// the core database itself.
pub struct CoreStore {
    storage: RefCell<Storage>,
}

impl CoreStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            storage: RefCell::new(Storage::open(path)?),
        })
    }

    pub fn close(&mut self) -> Result<(), StorageError> {
        self.storage.get_mut().close()
    }
}

impl CoreReferences for CoreStore {
    fn identity(&self, id: &str) -> Result<Option<CoreIdentity>, StorageError> {
        let storage = self.storage.borrow();
        let Some(identity) = storage.find_identity_by_id(id)? else {
            return Ok(None);
        };
        let retired = storage.find_active_identity_by_id(id)?.is_none();
        Ok(Some(CoreIdentity {
            id: identity.id,
            name: identity.name,
            lifetime: identity.lifetime,
            retired,
        }))
    }

    fn active_identities(&self) -> Result<Vec<CoreIdentity>, StorageError> {
        Ok(self
            .storage
            .borrow()
            .list_identities()?
            .into_iter()
            .map(|identity| CoreIdentity {
                id: identity.id,
                name: identity.name,
                lifetime: identity.lifetime,
                retired: false,
            })
            .collect())
    }

    fn room(&self, id: &str) -> Result<Option<CoreRoom>, StorageError> {
        let room = match self.storage.borrow_mut().find_historical_meeting_room(id) {
            Ok(room) => room,
            // A non-canonical UUID names no room, as the former direct read did.
            Err(RoomStoreError::Invalid) => None,
            Err(RoomStoreError::Storage(error)) => return Err(error),
            Err(other) => {
                return Err(StorageError::new(
                    StorageErrorCode::Unknown,
                    other.to_string(),
                ));
            }
        };
        Ok(room.map(|room| CoreRoom {
            id: room.id,
            retired: room.retired,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    /// SQL fragments that would read or write core-owned tables.
    const CORE_TABLE_SQL: &[&str] = &[
        "FROM identities",
        "JOIN identities",
        "INTO identities",
        "UPDATE identities",
        "office_meeting_rooms",
        "office_meeting_members",
        "extension_storage_cutovers",
    ];

    /// Only the migration coordinator compares or copies against the core
    /// schema; every other module reaches core through [`super::CoreReferences`].
    /// `migration/switch.rs` alone writes core's cutover receipt, inside its
    /// decision transaction.
    const MIGRATION_MODULES: &[&str] = &[
        "cells.rs",
        "migration.rs",
        "migration/switch.rs",
        "schema.rs",
    ];

    fn visit(root: &Path, directory: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, found);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            if !relative.ends_with(".rs")
                || MIGRATION_MODULES.contains(&relative.as_str())
                || name == "tests.rs"
                || relative.contains("tests/")
                || name.ends_with("_tests.rs")
                || name == "test_support.rs"
            {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
            for pattern in CORE_TABLE_SQL {
                if production.contains(pattern) {
                    found.push(format!("{relative}: {pattern}"));
                }
            }
        }
    }

    #[test]
    fn office_code_outside_migration_never_runs_sql_on_core_tables() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        visit(&root, &root, &mut found);
        assert_eq!(found, Vec::<String>::new());
    }
}
