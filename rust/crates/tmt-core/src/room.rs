//! Transport-neutral room membership, independent of personal spaces and presence.

use crate::{
    dispatch::{MAX_RECIPIENTS, canonical_id},
    limits::MAX_JS_SAFE_INTEGER,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRoom {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub retired: bool,
    /// Current active identities only; offline identities remain members.
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomWrite {
    pub expected_revision: u64,
    pub name: String,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipChange {
    Join,
    Leave,
}

impl RoomWrite {
    pub fn normalize(mut self) -> Option<Self> {
        if self.expected_revision >= MAX_JS_SAFE_INTEGER
            || self.name.trim().is_empty()
            || self.name.len() > 80
            || self.name.chars().any(char::is_control)
            || self.member_ids.len() > MAX_RECIPIENTS
            || self.member_ids.iter().any(|id| !canonical_id(id))
        {
            return None;
        }
        self.member_ids.sort();
        self.member_ids.dedup();
        Some(self)
    }
}

/// Atomic roster effects shared by CLI and HTTP adapters. Implementations must
/// validate active identity UUIDs and commit each mutation in one transaction.
pub trait RoomRepository {
    type Error;

    fn list_meeting_rooms(&mut self) -> Result<Vec<MeetingRoom>, Self::Error>;
    fn find_meeting_room(&mut self, id: &str) -> Result<Option<MeetingRoom>, Self::Error>;
    /// Historical UUID lookup includes retired rooms; it does not authorize new work.
    fn find_historical_meeting_room(
        &mut self,
        id: &str,
    ) -> Result<Option<MeetingRoom>, Self::Error>;
    fn retire_meeting_room(
        &mut self,
        id: &str,
        expected_revision: u64,
    ) -> Result<MeetingRoom, Self::Error>;
    fn save_meeting_room(&mut self, id: &str, input: RoomWrite)
    -> Result<MeetingRoom, Self::Error>;
    fn change_meeting_membership(
        &mut self,
        id: &str,
        identity_id: &str,
        change: MembershipChange,
    ) -> Result<MeetingRoom, Self::Error>;
}

#[derive(Debug)]
pub enum ResolveError<E> {
    NotFound,
    Ambiguous(Vec<MeetingRoom>),
    Repository(E),
}

/// Active-room reads needed by selection. Every repository provides them; a
/// storage adapter may also provide them inside a caller-owned read transaction.
pub trait ActiveRoomReader {
    type Error;

    fn active_room(&mut self, id: &str) -> Result<Option<MeetingRoom>, Self::Error>;
    fn active_rooms(&mut self) -> Result<Vec<MeetingRoom>, Self::Error>;
}

impl<R: RoomRepository> ActiveRoomReader for R {
    type Error = R::Error;

    fn active_room(&mut self, id: &str) -> Result<Option<MeetingRoom>, Self::Error> {
        self.find_meeting_room(id)
    }

    fn active_rooms(&mut self) -> Result<Vec<MeetingRoom>, Self::Error> {
        self.list_meeting_rooms()
    }
}

/// UUIDs are authoritative; human labels resolve only when the exact name is unique.
pub fn resolve_room<R: ActiveRoomReader>(
    repository: &mut R,
    selector: &str,
) -> Result<MeetingRoom, ResolveError<R::Error>> {
    if canonical_id(selector) {
        return repository
            .active_room(selector)
            .map_err(ResolveError::Repository)?
            .ok_or(ResolveError::NotFound);
    }
    let mut matches = repository
        .active_rooms()
        .map_err(ResolveError::Repository)?
        .into_iter()
        .filter(|room| room.name == selector)
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Err(ResolveError::NotFound),
        1 => Ok(matches.remove(0)),
        _ => Err(ResolveError::Ambiguous(matches)),
    }
}

/// Retired rooms remain addressable by UUID for history, never by a recycled name.
pub fn resolve_historical_room<R: RoomRepository>(
    repository: &mut R,
    selector: &str,
) -> Result<MeetingRoom, ResolveError<R::Error>> {
    if canonical_id(selector) {
        repository
            .find_historical_meeting_room(selector)
            .map_err(ResolveError::Repository)?
            .ok_or(ResolveError::NotFound)
    } else {
        resolve_room(repository, selector)
    }
}

/// Creation is explicit and does not enroll the caller or materialize Office facilities.
pub fn create<R: RoomRepository>(
    repository: &mut R,
    name: String,
) -> Result<MeetingRoom, R::Error> {
    repository.save_meeting_room(
        &uuid::Uuid::new_v4().to_string(),
        RoomWrite {
            expected_revision: 0,
            name,
            member_ids: Vec::new(),
        },
    )
}
