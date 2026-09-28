//! One durable roster, read inside the same transaction that fences room dispatch.

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction,
};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::{
    dispatch::canonical_id,
    room::{ActiveRoomReader, MeetingRoom, MembershipChange, RoomRepository, RoomWrite},
};

#[derive(Debug)]
pub enum RoomStoreError {
    Invalid,
    NotFound,
    RevisionConflict,
    IdentityInactive,
    Retired,
    Storage(StorageError),
}
impl RoomStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "ROOM_INVALID",
            Self::NotFound => "ROOM_NOT_FOUND",
            Self::RevisionConflict => "ROOM_REVISION_CONFLICT",
            Self::IdentityInactive => "ROOM_IDENTITY_INACTIVE",
            Self::Retired => "ROOM_RETIRED",
            Self::Storage(_) => "STORAGE_UNAVAILABLE",
        }
    }
}
impl std::fmt::Display for RoomStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for RoomStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}
impl From<StorageError> for RoomStoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

pub(super) fn read_room(
    connection: &Connection,
    id: &str,
) -> Result<Option<MeetingRoom>, StorageError> {
    Ok(read_historical_room(connection, id)?.filter(|room| !room.retired))
}

pub(super) fn read_historical_room(
    connection: &Connection,
    id: &str,
) -> Result<Option<MeetingRoom>, StorageError> {
    let Some((name, revision, retired)) = connection
        .query_row(
            "SELECT name,revision,retired FROM office_meeting_rooms WHERE room_id=?",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify(error, "Read meeting room"))?
    else {
        return Ok(None);
    };
    let revision = u64::try_from(revision)
        .ok()
        .filter(|value| *value > 0 && *value <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or_else(|| StorageError::new(StorageErrorCode::Corrupt, "Invalid meeting revision"))?;
    let mut statement = connection.prepare(
        "SELECT m.identity_id FROM office_meeting_members m JOIN identities i ON i.id=m.identity_id WHERE m.room_id=? AND i.retired_at_ms IS NULL ORDER BY m.identity_id"
    ).map_err(|error| classify(error, "Read meeting membership"))?;
    let member_ids = statement
        .query_map([id], |row| row.get(0))
        .map_err(|error| classify(error, "Read meeting membership"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| classify(error, "Read meeting membership"))?;
    Ok(Some(MeetingRoom {
        id: id.into(),
        name,
        revision,
        retired,
        member_ids,
    }))
}

fn read_active_rooms(connection: &Connection) -> Result<Vec<MeetingRoom>, RoomStoreError> {
    let ids = {
        let mut statement = connection
            .prepare(
                "SELECT room_id FROM office_meeting_rooms WHERE retired=0 ORDER BY name,room_id",
            )
            .map_err(|error| classify(error, "List meeting rooms"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| classify(error, "List meeting rooms"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| classify(error, "List meeting rooms"))?
    };
    ids.iter()
        .map(|id| read_room(connection, id)?.ok_or(RoomStoreError::NotFound))
        .collect()
}

/// Active-room selection inside a caller-owned read transaction, so selection
/// and every later read share one snapshot. It never writes.
pub(super) struct TransactionRooms<'a>(pub(super) &'a Connection);

impl ActiveRoomReader for TransactionRooms<'_> {
    type Error = RoomStoreError;

    fn active_room(&mut self, id: &str) -> Result<Option<MeetingRoom>, RoomStoreError> {
        if !canonical_id(id) {
            return Err(RoomStoreError::Invalid);
        }
        Ok(read_room(self.0, id)?)
    }

    fn active_rooms(&mut self) -> Result<Vec<MeetingRoom>, RoomStoreError> {
        read_active_rooms(self.0)
    }
}

impl RoomRepository for Storage {
    type Error = RoomStoreError;

    fn find_meeting_room(&mut self, id: &str) -> Result<Option<MeetingRoom>, RoomStoreError> {
        Ok(self
            .find_historical_meeting_room(id)?
            .filter(|room| !room.retired))
    }

    fn find_historical_meeting_room(
        &mut self,
        id: &str,
    ) -> Result<Option<MeetingRoom>, RoomStoreError> {
        if !canonical_id(id) {
            return Err(RoomStoreError::Invalid);
        }
        let transaction = self
            .connection_mut()?
            .transaction()
            .map_err(|error| classify(error, "Read meeting room"))?;
        let room = read_historical_room(&transaction, id)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish meeting room read"))?;
        Ok(room)
    }

    fn list_meeting_rooms(&mut self) -> Result<Vec<MeetingRoom>, RoomStoreError> {
        // One read transaction keeps definition and effective membership coherent.
        let transaction = self
            .connection_mut()?
            .transaction()
            .map_err(|error| classify(error, "Read meeting rooms"))?;
        let rooms = read_active_rooms(&transaction)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish meeting room read"))?;
        Ok(rooms)
    }

    fn save_meeting_room(
        &mut self,
        id: &str,
        input: RoomWrite,
    ) -> Result<MeetingRoom, RoomStoreError> {
        if !canonical_id(id) {
            return Err(RoomStoreError::Invalid);
        }
        let input = input.normalize().ok_or(RoomStoreError::Invalid)?;
        with_immediate_transaction(self, "Save meeting room", |transaction| {
            write_room(transaction, id, input)
        })
    }

    fn retire_meeting_room(
        &mut self,
        id: &str,
        expected_revision: u64,
    ) -> Result<MeetingRoom, RoomStoreError> {
        if !canonical_id(id)
            || expected_revision == 0
            || expected_revision > tmt_core::limits::MAX_JS_SAFE_INTEGER
        {
            return Err(RoomStoreError::Invalid);
        }
        with_immediate_transaction(self, "Retire meeting room", |transaction| {
            let mut room =
                read_historical_room(transaction, id)?.ok_or(RoomStoreError::NotFound)?;
            if room.retired
                && (room.revision == expected_revision || room.revision == expected_revision + 1)
            {
                return Ok(room);
            }
            if room.retired
                || room.revision != expected_revision
                || expected_revision == tmt_core::limits::MAX_JS_SAFE_INTEGER
            {
                return Err(RoomStoreError::RevisionConflict);
            }
            transaction
                .execute(
                    "UPDATE office_meeting_rooms SET retired=1,revision=revision+1 WHERE room_id=?",
                    [id],
                )
                .map_err(|error| classify(error, "Retire meeting room"))?;
            room.retired = true;
            room.revision += 1;
            Ok(room)
        })
    }

    /// Read-modify-write under the roster lock; concurrent joins cannot lose members.
    fn change_meeting_membership(
        &mut self,
        id: &str,
        identity_id: &str,
        change: MembershipChange,
    ) -> Result<MeetingRoom, RoomStoreError> {
        if !canonical_id(id) || !canonical_id(identity_id) {
            return Err(RoomStoreError::Invalid);
        }
        with_immediate_transaction(self, "Change meeting membership", |transaction| {
            let current = read_room(transaction, id)?.ok_or(RoomStoreError::NotFound)?;
            let mut input = RoomWrite {
                expected_revision: current.revision,
                name: current.name,
                member_ids: current.member_ids,
            };
            match change {
                MembershipChange::Join => {
                    if !input.member_ids.iter().any(|member| member == identity_id) {
                        input.member_ids.push(identity_id.into());
                    }
                }
                MembershipChange::Leave => input.member_ids.retain(|member| member != identity_id),
            }
            let input = input.normalize().ok_or(RoomStoreError::Invalid)?;
            write_room(transaction, id, input)
        })
    }
}

/// Both conditional roster replacement and atomic membership edits use one writer.
fn write_room(
    transaction: &Connection,
    id: &str,
    input: RoomWrite,
) -> Result<MeetingRoom, RoomStoreError> {
    let current = read_historical_room(transaction, id)?;
    if current.as_ref().is_some_and(|room| room.retired) {
        return Err(RoomStoreError::Retired);
    }
    if current.as_ref().map_or(0, |room| room.revision) != input.expected_revision {
        return Err(RoomStoreError::RevisionConflict);
    }
    for identity in &input.member_ids {
        let active = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE id=? AND retired_at_ms IS NULL)",
                [identity],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| classify(error, "Check meeting member"))?;
        if !active {
            return Err(RoomStoreError::IdentityInactive);
        }
    }
    if let Some(room) =
        current.filter(|room| room.name == input.name && room.member_ids == input.member_ids)
    {
        return Ok(room);
    }
    let revision = input.expected_revision + 1;
    transaction.execute(
                "INSERT INTO office_meeting_rooms(room_id,name,revision) VALUES (?,?,?) ON CONFLICT(room_id) DO UPDATE SET name=excluded.name,revision=excluded.revision",
                params![id, input.name, revision as i64]
            ).map_err(|error| classify(error, "Save meeting room"))?;
    transaction
        .execute("DELETE FROM office_meeting_members WHERE room_id=?", [id])
        .map_err(|error| classify(error, "Replace meeting members"))?;
    for identity in &input.member_ids {
        transaction
            .execute(
                "INSERT INTO office_meeting_members(room_id,identity_id) VALUES (?,?)",
                params![id, identity],
            )
            .map_err(|error| classify(error, "Save meeting member"))?;
    }
    Ok(MeetingRoom {
        id: id.into(),
        name: input.name,
        revision,
        retired: false,
        member_ids: input.member_ids,
    })
}

#[cfg(test)]
mod tests;
