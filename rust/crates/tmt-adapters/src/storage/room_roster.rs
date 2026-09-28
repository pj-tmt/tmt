//! One read snapshot for a room roster: selection, members, metadata and status.

use super::{
    Storage, StorageError, StorageErrorCode,
    errors::classify,
    identities::active_identity_by_id,
    room::{RoomStoreError, TransactionRooms},
};
use std::collections::BTreeMap;
use tmt_core::{
    identity::Identity,
    identity_metadata::{IdentityMetadataRepository, MetadataCollection},
    identity_status::IdentityStatus,
    room::{MeetingRoom, ResolveError, resolve_room},
};

#[derive(Debug)]
pub struct RosterMember {
    pub identity: Identity,
    pub metadata: BTreeMap<String, String>,
    pub status: Option<IdentityStatus>,
}

#[derive(Debug)]
pub struct RoomRoster {
    pub room: MeetingRoom,
    /// Effective (non-retired) members in the room's member order.
    pub members: Vec<RosterMember>,
}

#[derive(Debug)]
pub enum RosterError {
    NotFound,
    Ambiguous,
    Storage(StorageError),
}

impl From<StorageError> for RosterError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

fn inconsistent() -> RosterError {
    RosterError::Storage(StorageError::new(
        StorageErrorCode::Corrupt,
        "Room membership references an unreadable identity",
    ))
}

impl Storage {
    /// No writes, migration, acknowledgment or retention effects. The metadata
    /// prefix matches literally; callers validate it with the key grammar.
    pub fn room_roster(
        &self,
        selector: &str,
        metadata_prefix: Option<&str>,
    ) -> Result<RoomRoster, RosterError> {
        // A deferred transaction pins one WAL snapshot at its first read, so
        // selection and every later projection observe the same state.
        let transaction = self
            .connection()?
            .unchecked_transaction()
            .map_err(|error| classify(error, "Read room roster"))?;
        let room = resolve_room(&mut TransactionRooms(&transaction), selector).map_err(
            |error| match error {
                ResolveError::NotFound | ResolveError::Repository(RoomStoreError::NotFound) => {
                    RosterError::NotFound
                }
                ResolveError::Ambiguous(_) => RosterError::Ambiguous,
                ResolveError::Repository(RoomStoreError::Storage(error)) => {
                    RosterError::Storage(error)
                }
                ResolveError::Repository(_) => inconsistent(),
            },
        )?;
        let mut statuses = self.list_active_identity_statuses()?;
        let mut members = Vec::with_capacity(room.member_ids.len());
        for id in &room.member_ids {
            let identity = active_identity_by_id(&transaction, id)?.ok_or_else(inconsistent)?;
            let MetadataCollection::Found(metadata) = self.list_metadata(id)? else {
                return Err(inconsistent());
            };
            members.push(RosterMember {
                identity,
                metadata: metadata
                    .into_iter()
                    .filter(|(key, _)| metadata_prefix.is_none_or(|prefix| key.starts_with(prefix)))
                    .collect(),
                status: statuses.remove(id),
            });
        }
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish room roster read"))?;
        Ok(RoomRoster { room, members })
    }
}

#[cfg(test)]
mod tests;
