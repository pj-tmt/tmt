use super::*;
use crate::test_support::TestDirectory;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    identity_metadata::set_identity_metadata,
    identity_status::set_identity_status,
    room::{RoomRepository, RoomWrite},
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const TWIN: &str = "33333333-3333-4333-8333-333333333333";

fn identity(storage: &mut Storage, name: &str, lifetime: Lifetime) -> String {
    create_or_resolve(storage, name, lifetime)
        .unwrap()
        .identity
        .id
}

fn room(storage: &mut Storage, id: &str, members: Vec<String>) -> MeetingRoom {
    storage
        .save_meeting_room(
            id,
            RoomWrite {
                expected_revision: 0,
                name: "Design room".into(),
                member_ids: members,
            },
        )
        .unwrap()
}

fn metadata(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).into(), (*value).into()))
        .collect()
}

#[test]
fn one_read_projects_members_prefixed_metadata_and_status_without_writes() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("roster.db")).unwrap();
    let alice = identity(&mut storage, "Alice", Lifetime::Saved);
    let bob = identity(&mut storage, "Bob", Lifetime::Temporary);
    let outsider = identity(&mut storage, "Carol", Lifetime::Saved);
    let saved = room(&mut storage, ROOM, vec![alice.clone(), bob.clone()]);
    for (key, value) in [
        ("squad.a.state", "review"),
        ("squad.ab.state", "other squad"),
        ("team", "core"),
    ] {
        set_identity_metadata(&mut storage, &alice, key, value).unwrap();
    }
    set_identity_metadata(&mut storage, &outsider, "squad.a.state", "hidden").unwrap();
    let status =
        set_identity_status(&mut storage, &alice, "Reviewing".into(), None, 100, 1000).unwrap();
    let changes = storage.connection().unwrap().total_changes();

    let roster = storage
        .room_roster("Design room", Some("squad.a."))
        .unwrap();
    assert_eq!(roster.room, saved);
    let ids: Vec<_> = roster
        .members
        .iter()
        .map(|m| m.identity.id.clone())
        .collect();
    assert_eq!(
        ids, saved.member_ids,
        "room member order, outsiders excluded"
    );
    let by_id = |id: &str| roster.members.iter().find(|m| m.identity.id == id).unwrap();
    assert_eq!(
        by_id(&alice).metadata,
        metadata(&[("squad.a.state", "review")])
    );
    assert_eq!(by_id(&alice).status, Some(status));
    assert_eq!(by_id(&alice).identity.lifetime, Lifetime::Saved);
    assert!(by_id(&bob).metadata.is_empty());
    assert_eq!(by_id(&bob).status, None);

    let unfiltered = storage.room_roster(ROOM, None).unwrap();
    assert_eq!(
        unfiltered
            .members
            .iter()
            .find(|m| m.identity.id == alice)
            .unwrap()
            .metadata,
        metadata(&[
            ("squad.a.state", "review"),
            ("squad.ab.state", "other squad"),
            ("team", "core"),
        ])
    );
    let connection = storage.connection().unwrap();
    assert_eq!(
        connection.total_changes(),
        changes,
        "roster reads never write"
    );
    assert!(
        connection.is_autocommit(),
        "the read transaction is finished"
    );
}

#[test]
fn selection_reuses_active_room_policy_and_excludes_retired_members() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("selection.db")).unwrap();
    let alice = identity(&mut storage, "Alice", Lifetime::Saved);
    let bob = identity(&mut storage, "Bob", Lifetime::Temporary);
    let empty = room(&mut storage, ROOM, vec![]);
    assert!(storage.room_roster(ROOM, None).unwrap().members.is_empty());
    storage
        .save_meeting_room(
            ROOM,
            RoomWrite {
                expected_revision: empty.revision,
                name: empty.name.clone(),
                member_ids: vec![alice.clone(), bob.clone()],
            },
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = 5 WHERE id = ?",
            [&bob],
        )
        .unwrap();
    let roster = storage.room_roster(ROOM, None).unwrap();
    assert_eq!(roster.members.len(), 1);
    assert_eq!(roster.members[0].identity.id, alice);

    assert!(matches!(
        storage.room_roster("Unknown room", None),
        Err(RosterError::NotFound)
    ));
    assert!(matches!(
        storage.room_roster("design room", None),
        Err(RosterError::NotFound)
    ));
    room(&mut storage, TWIN, vec![]);
    assert!(matches!(
        storage.room_roster("Design room", None),
        Err(RosterError::Ambiguous)
    ));
    assert_eq!(storage.room_roster(TWIN, None).unwrap().room.id, TWIN);

    let current = storage.room_roster(ROOM, None).unwrap().room.revision;
    storage.retire_meeting_room(ROOM, current).unwrap();
    assert!(matches!(
        storage.room_roster(ROOM, None),
        Err(RosterError::NotFound)
    ));
    assert_eq!(
        storage.room_roster("Design room", None).unwrap().room.id,
        TWIN
    );
}
