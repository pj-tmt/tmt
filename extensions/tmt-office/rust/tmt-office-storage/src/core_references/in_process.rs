//! The in-process reference port over core's repository API, for tests over
//! disposable core files only (the crate's own tests and the companion's, via
//! the `in-process-core` feature). Production reaches core through
//! [`super::ProcessReferences`].

use super::{CoreIdentity, CoreReferences, CoreRoom};
use crate::StorageLayout;
use std::{cell::RefCell, path::Path};
use tmt_adapters::storage::{RoomStoreError, Storage, StorageError, StorageErrorCode};
use tmt_core::{identity::IdentityReader, room::RoomRepository};

/// In-process implementation over core's public repository API, for tests
/// over disposable core files only.
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

/// The test reference port for a layout's core file.
pub(crate) fn references(
    layout: &StorageLayout,
) -> Result<Box<dyn CoreReferences + Send>, StorageError> {
    Ok(Box::new(CoreStore::open(&layout.source)?))
}
