use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use crate::test_support::TestDirectory;
use rusqlite::params;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    identity_metadata::{
        IdentityMetadataRepository, MetadataCollection, MetadataFilter, MetadataKey,
        MetadataLookup, MetadataMutation, MetadataValue,
    },
};

use super::super::*;

struct Fixture {
    _directory: TestDirectory,
    database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        Self {
            database: directory.path.join("state").join("tmux-team.db"),
            _directory: directory,
        }
    }

    fn open(&self) -> Storage {
        Storage::open(&self.database).unwrap()
    }
}

fn key(value: &str) -> MetadataKey {
    MetadataKey::parse(value).unwrap()
}

fn value(value: &str) -> MetadataValue {
    MetadataValue::parse(value).unwrap()
}

#[test]
fn set_get_list_remove_and_exact_filters_preserve_string_values() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary)
        .unwrap()
        .identity;
    let bob = create_or_resolve(&mut storage, "Bob", Lifetime::Saved)
        .unwrap()
        .identity;

    assert_eq!(
        storage
            .set_metadata(&alice.id, &key("project"), &value("tmt=alpha"))
            .unwrap(),
        MetadataMutation::Set { changed: true }
    );
    assert_eq!(
        storage
            .find_active_identity_by_id(&alice.id)
            .unwrap()
            .unwrap()
            .lifetime,
        Lifetime::Temporary,
        "metadata must not promote a temporary identity"
    );
    assert_eq!(
        storage
            .set_metadata(&alice.id, &key("project"), &value("tmt=alpha"))
            .unwrap(),
        MetadataMutation::Set { changed: false }
    );
    assert_eq!(
        storage
            .set_metadata(&alice.id, &key("department"), &value("engineering"))
            .unwrap(),
        MetadataMutation::Set { changed: true }
    );
    storage
        .set_metadata(&bob.id, &key("project"), &value("other"))
        .unwrap();

    assert_eq!(
        storage.get_metadata(&alice.id, &key("project")).unwrap(),
        MetadataLookup::Found("tmt=alpha".into())
    );
    assert_eq!(
        storage.list_metadata(&alice.id).unwrap(),
        MetadataCollection::Found(
            [
                ("department".into(), "engineering".into()),
                ("project".into(), "tmt=alpha".into()),
            ]
            .into_iter()
            .collect()
        )
    );
    let matches = storage
        .list_identities_matching(&[
            MetadataFilter::equals("project", "tmt=alpha").unwrap(),
            MetadataFilter::has("department").unwrap(),
        ])
        .unwrap();
    assert_eq!(matches, vec![alice.clone()]);
    assert_eq!(
        storage
            .list_identities_matching(&[MetadataFilter::has("project").unwrap()])
            .unwrap(),
        [alice.clone(), bob]
    );

    assert_eq!(
        storage.remove_metadata(&alice.id, &key("project")).unwrap(),
        MetadataMutation::Removed { removed: true }
    );
    assert_eq!(
        storage.remove_metadata(&alice.id, &key("project")).unwrap(),
        MetadataMutation::Removed { removed: false }
    );
    assert_eq!(
        storage.get_metadata(&alice.id, &key("project")).unwrap(),
        MetadataLookup::KeyNotFound
    );
}

#[test]
fn retired_uuid_is_hidden_and_same_name_replacement_inherits_nothing() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let original = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary)
        .unwrap()
        .identity;
    storage
        .set_metadata(&original.id, &key("project"), &value("tmt"))
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = 1 WHERE id = ?",
            [&original.id],
        )
        .unwrap();

    assert_eq!(
        storage.get_metadata(&original.id, &key("project")).unwrap(),
        MetadataLookup::IdentityNotFound
    );
    assert_eq!(
        storage
            .set_metadata(&original.id, &key("other"), &value("value"))
            .unwrap(),
        MetadataMutation::IdentityNotFound
    );
    assert_eq!(
        storage
            .remove_metadata(&original.id, &key("project"))
            .unwrap(),
        MetadataMutation::IdentityNotFound
    );
    assert_eq!(
        storage.list_metadata(&original.id).unwrap(),
        MetadataCollection::IdentityNotFound
    );
    assert!(
        storage
            .list_identities_matching(&[MetadataFilter::has("project").unwrap()])
            .unwrap()
            .is_empty()
    );
    let replacement = create_or_resolve(&mut storage, "alice", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_ne!(replacement.id, original.id);
    assert_eq!(
        storage.list_metadata(&replacement.id).unwrap(),
        MetadataCollection::Found(Default::default())
    );
}

#[test]
fn entry_limit_is_atomic_and_existing_values_remain_replaceable() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    for index in 0..64 {
        assert_eq!(
            storage
                .set_metadata(&identity.id, &key(&format!("k{index}")), &value("v"))
                .unwrap(),
            MetadataMutation::Set { changed: true }
        );
    }
    assert_eq!(
        storage
            .set_metadata(&identity.id, &key("overflow"), &value("v"))
            .unwrap(),
        MetadataMutation::EntryLimit
    );
    assert_eq!(
        storage
            .set_metadata(&identity.id, &key("k0"), &value("replaced"))
            .unwrap(),
        MetadataMutation::Set { changed: true }
    );
    let count: i64 = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM identity_metadata WHERE identity_id = ?",
            [&identity.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 64);
}

#[test]
fn concurrent_new_keys_never_exceed_the_limit_and_survive_reopen() {
    let fixture = Fixture::new();
    let mut initial = fixture.open();
    let identity = create_or_resolve(&mut initial, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    initial.close().unwrap();
    let identity_id = Arc::new(identity.id);
    let barrier = Arc::new(Barrier::new(5));
    let storages = (0..4)
        .map(|_| Storage::open(&fixture.database).unwrap())
        .collect::<Vec<_>>();
    let threads = storages
        .into_iter()
        .enumerate()
        .map(|(worker, mut storage)| {
            let identity_id = Arc::clone(&identity_id);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut set = 0;
                let mut limited = 0;
                for index in 0..20 {
                    match storage
                        .set_metadata(
                            identity_id.as_str(),
                            &key(&format!("w{worker}.{index}")),
                            &value("v"),
                        )
                        .unwrap()
                    {
                        MetadataMutation::Set { changed: true } => set += 1,
                        MetadataMutation::EntryLimit => limited += 1,
                        other => panic!("unexpected mutation: {other:?}"),
                    }
                }
                storage.close().unwrap();
                (set, limited)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let (set, limited) = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .fold((0, 0), |(set, limited), next| {
            (set + next.0, limited + next.1)
        });
    assert_eq!((set, limited), (64, 16));

    let reopened = fixture.open();
    let MetadataCollection::Found(metadata) = reopened.list_metadata(identity_id.as_str()).unwrap()
    else {
        panic!("identity must remain active")
    };
    assert_eq!(metadata.len(), 64);
}

#[test]
fn exact_search_uses_the_search_index_and_parameterizes_input() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    storage
        .set_metadata(&identity.id, &key("project"), &value("x' OR 1=1 --"))
        .unwrap();
    let matches = storage
        .list_identities_matching(&[MetadataFilter::equals("project", "x' OR 1=1 --").unwrap()])
        .unwrap();
    assert_eq!(matches, [identity]);

    let detail = storage
        .connection()
        .unwrap()
        .prepare(
            "EXPLAIN QUERY PLAN SELECT 1 FROM identity_metadata INDEXED BY identity_metadata_search
             WHERE key = ? AND value = ? AND identity_id = ?",
        )
        .unwrap()
        .query_map(params!["project", "tmt", "id"], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join(" ");
    assert!(detail.contains("identity_metadata_search"), "{detail}");
}

fn changes(body: &str) -> tmt_core::identity_metadata::MetadataChanges {
    crate::identity_metadata::decode_changes(body).unwrap()
}

#[test]
fn conditional_apply_matches_all_or_preserves_all_rows_and_cursor() {
    use tmt_core::identity_metadata::MetadataApply;
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary)
        .unwrap()
        .identity;
    storage
        .set_metadata(&identity.id, &key("state"), &value("blocked"))
        .unwrap();
    storage
        .set_metadata(&identity.id, &key("pending"), &value("review"))
        .unwrap();
    let before = storage.list_metadata(&identity.id).unwrap();
    let cursor = storage.change_cursor().unwrap();
    let conflict = changes(
        r#"[
        {"key":"pending","expect":{"value":"review"},"then":"remove"},
        {"key":"state","expect":{"value":"working"},"then":{"set":"done"}},
        {"key":"missing","expect":{"value":"old"},"then":{"set":"new"}},
        {"key":"created","expect":"any","then":{"set":"new"}}
    ]"#,
    );
    assert_eq!(
        storage.apply_metadata(&identity.id, &conflict).unwrap(),
        MetadataApply::Conflict(
            [
                ("missing".into(), None),
                ("state".into(), Some("blocked".into()))
            ]
            .into_iter()
            .collect()
        )
    );
    assert_eq!(storage.list_metadata(&identity.id).unwrap(), before);
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    let matched = changes(
        r#"[
        {"key":"pending","expect":{"value":"review"},"then":"remove"},
        {"key":"state","expect":{"value":"blocked"},"then":{"set":"working"}},
        {"key":"created","expect":"absent","then":{"set":"new"}},
        {"key":"missing","expect":"any","then":"remove"}
    ]"#,
    );
    assert_eq!(
        storage.apply_metadata(&identity.id, &matched).unwrap(),
        MetadataApply::Applied { changed: true }
    );
    let after = MetadataCollection::Found(
        [
            ("state".into(), "working".into()),
            ("created".into(), "new".into()),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(storage.list_metadata(&identity.id).unwrap(), after);
    assert!(storage.change_cursor().unwrap() > cursor);
    let unchanged = changes(
        r#"[
        {"key":"state","expect":"any","then":{"set":"working"}},
        {"key":"missing","expect":"absent","then":"remove"}
    ]"#,
    );
    let cursor = storage.change_cursor().unwrap();
    assert_eq!(
        storage.apply_metadata(&identity.id, &unchanged).unwrap(),
        MetadataApply::Applied { changed: false }
    );
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        storage
            .find_active_identity_by_id(&identity.id)
            .unwrap()
            .unwrap()
            .lifetime,
        Lifetime::Temporary
    );
    storage.close().unwrap();
    let reopened = fixture.open();
    assert_eq!(reopened.list_metadata(&identity.id).unwrap(), after);
}

#[test]
fn conditional_apply_checks_final_capacity_and_exact_active_uuid() {
    use tmt_core::identity_metadata::MetadataApply;
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    for index in 0..64 {
        storage
            .set_metadata(&identity.id, &key(&format!("k{index}")), &value("v"))
            .unwrap();
    }
    let cursor = storage.change_cursor().unwrap();
    let overflow = changes(
        r#"[{"key":"new","expect":"absent","then":{"set":"v"}},{"key":"k0","expect":"any","then":{"set":"replaced"}}]"#,
    );
    assert_eq!(
        storage.apply_metadata(&identity.id, &overflow).unwrap(),
        MetadataApply::EntryLimit
    );
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        storage.get_metadata(&identity.id, &key("k0")).unwrap(),
        MetadataLookup::Found("v".into())
    );
    // Addition before removal is allowed: capacity belongs to the final set.
    let replace = changes(
        r#"[{"key":"new","expect":"absent","then":{"set":"v"}},{"key":"k0","expect":{"value":"v"},"then":"remove"}]"#,
    );
    assert_eq!(
        storage.apply_metadata(&identity.id, &replace).unwrap(),
        MetadataApply::Applied { changed: true }
    );
    let MetadataCollection::Found(metadata) = storage.list_metadata(&identity.id).unwrap() else {
        panic!("active identity")
    };
    assert_eq!(metadata.len(), 64);
    assert!(metadata.contains_key("new"));
    assert!(!metadata.contains_key("k0"));
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=1 WHERE id=?",
            [&identity.id],
        )
        .unwrap();
    let replacement = create_or_resolve(&mut storage, &identity.id, Lifetime::Saved)
        .unwrap()
        .identity;
    let cursor = storage.change_cursor().unwrap();
    assert_eq!(
        storage.apply_metadata(&identity.id, &replace).unwrap(),
        MetadataApply::IdentityNotFound
    );
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        storage.list_metadata(&replacement.id).unwrap(),
        MetadataCollection::Found(Default::default())
    );
}

#[test]
fn conditional_apply_rolls_back_prior_effects_when_a_later_write_fails() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    storage
        .set_metadata(&identity.id, &key("first"), &value("old"))
        .unwrap();
    storage.connection().unwrap().execute_batch("CREATE TRIGGER refuse_metadata BEFORE INSERT ON identity_metadata WHEN NEW.key = 'second' BEGIN SELECT RAISE(ABORT, 'fixture refusal'); END;").unwrap();
    let cursor = storage.change_cursor().unwrap();
    let input = changes(
        r#"[{"key":"first","expect":{"value":"old"},"then":{"set":"new"}},{"key":"second","expect":"absent","then":{"set":"new"}}]"#,
    );
    assert!(storage.apply_metadata(&identity.id, &input).is_err());
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    assert_eq!(
        storage.list_metadata(&identity.id).unwrap(),
        MetadataCollection::Found([("first".into(), "old".into())].into_iter().collect())
    );
}

#[test]
fn two_conditional_writers_have_one_winner_and_one_conflict() {
    use tmt_core::identity_metadata::MetadataApply;
    let fixture = Fixture::new();
    let mut initial = fixture.open();
    let identity = create_or_resolve(&mut initial, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    initial
        .set_metadata(&identity.id, &key("state"), &value("old"))
        .unwrap();
    initial.close().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let threads = (0..2).map(|index| {
        let mut storage = fixture.open();
        let barrier = Arc::clone(&barrier);
        let id = identity.id.clone();
        thread::spawn(move || {
            let next = format!("writer{index}");
            let input = changes(&format!(r#"[{{"key":"state","expect":{{"value":"old"}},"then":{{"set":"{next}"}}}},{{"key":"owner","expect":"absent","then":{{"set":"{next}"}}}}]"#));
            barrier.wait();
            let result = storage.apply_metadata(&id, &input).unwrap();
            storage.close().unwrap();
            (next, result)
        })
    }).collect::<Vec<_>>();
    barrier.wait();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    let winners = results
        .iter()
        .filter(|(_, result)| matches!(result, MetadataApply::Applied { changed: true }))
        .collect::<Vec<_>>();
    assert_eq!(winners.len(), 1);
    let winner = &winners[0].0;
    let conflicts = results
        .iter()
        .filter_map(|(_, result)| {
            if let MetadataApply::Conflict(current) = result {
                Some(current)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        conflicts,
        vec![
            &[
                ("state".into(), Some(winner.clone())),
                ("owner".into(), Some(winner.clone()))
            ]
            .into_iter()
            .collect()
        ]
    );
    let storage = fixture.open();
    assert_eq!(
        storage.list_metadata(&identity.id).unwrap(),
        MetadataCollection::Found(
            [
                ("state".into(), winner.clone()),
                ("owner".into(), winner.clone())
            ]
            .into_iter()
            .collect()
        )
    );
}
