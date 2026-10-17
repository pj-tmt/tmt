//! Collection data is owned by the existing owner Store, not page originals or acknowledgments.
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    collections::{Collection, Command, Definition, Fault, Item, Outcome, Search, Sort},
    keyring::{Keyring, Layout},
    store::Store,
};

fn id(n: usize) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}
struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-3541-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap().with_clock(|| Ok(1234));
        Self {
            root,
            layout,
            key,
            store,
        }
    }
    fn oracle(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn apply(&mut self, command: Command, person: bool) -> tmt_colab::Result<Outcome> {
        self.store
            .label_transaction(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.apply_collection(&command, person)
            })
    }
    fn detail(&mut self, id: &str) -> tmt_colab::Result<tmt_colab::collections::Detail> {
        self.store
            .label_transaction(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.collection_detail(id)
            })
    }
    fn directory(&mut self) -> tmt_colab::Result<Vec<Collection>> {
        self.store
            .label_transaction(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.collection_directory()
            })
    }
    fn page(&self, n: usize) {
        self.oracle()
            .execute("INSERT INTO pages(page,epoch) VALUES (?,'1')", [id(n)])
            .unwrap();
        self.oracle()
            .execute(
                "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?,'1',zeroblob(32))",
                [id(n)],
            )
            .unwrap();
    }
    fn manual(&mut self, n: usize) -> Collection {
        let collection = Collection {
            id: id(n),
            name: "Reports".into(),
            maintainer: "PM".into(),
            archived: false,
            definition: Definition::Manual { sort: Sort::Manual },
        };
        assert!(
            self.apply(
                Command::Create {
                    collection: collection.clone()
                },
                false
            )
            .unwrap()
            .changed
        );
        collection
    }
    fn item(n: usize, page: usize) -> Item {
        Item {
            id: id(n),
            page_id: id(page),
            anchor: Some("hosting".into()),
            note: "Before deployment".into(),
            audience: "Ben".into(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn fault(error: Box<dyn std::error::Error + Send + Sync>) -> Option<Fault> {
    error.downcast_ref::<Fault>().copied()
}

#[test]
fn references_and_seen_survive_reopen_without_page_or_owner_records() {
    let mut f = Fixture::new();
    f.page(1);
    let collection = f.manual(10);
    for n in [11, 12] {
        f.apply(
            Command::Add {
                id: collection.id.clone(),
                item: Fixture::item(n, 1),
            },
            false,
        )
        .unwrap();
    }
    let seen = f
        .apply(
            Command::Seen {
                id: collection.id.clone(),
                target: id(11),
            },
            true,
        )
        .unwrap();
    assert_eq!(seen.detail.seen[0].at_ms, 1234);
    f.store = Store::open(&f.layout).unwrap();
    let detail = f.detail(&collection.id).unwrap();
    assert_eq!(
        detail
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        [id(11), id(12)]
    );
    assert_eq!(detail.items[0].page_id, detail.items[1].page_id);
    assert_eq!(detail.seen, seen.detail.seen);
    for table in [
        "membership_log",
        "receipts",
        "checkpoints",
        "owner_operations",
    ] {
        let count: i64 = f
            .oracle()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(f.directory().unwrap(), [collection]);
}

#[test]
fn only_person_actions_mark_seen_and_repeating_them_keeps_the_first_time() {
    let mut f = Fixture::new();
    f.page(1);
    f.manual(10);
    f.apply(
        Command::Add {
            id: id(10),
            item: Fixture::item(11, 1),
        },
        false,
    )
    .unwrap();
    assert_eq!(
        fault(
            f.apply(
                Command::Seen {
                    id: id(10),
                    target: id(11)
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Invalid)
    );
    assert!(f.detail(&id(10)).unwrap().seen.is_empty());
    assert!(
        f.apply(
            Command::Seen {
                id: id(10),
                target: id(11)
            },
            true
        )
        .unwrap()
        .changed
    );
    f.store = Store::open(&f.layout).unwrap().with_clock(|| Ok(5678));
    let repeat = f
        .apply(
            Command::Seen {
                id: id(10),
                target: id(11),
            },
            true,
        )
        .unwrap();
    assert!(!repeat.changed);
    assert_eq!(repeat.detail.seen[0].at_ms, 1234);
    assert_eq!(
        fault(
            f.apply(Command::ClearSeen { id: id(10) }, false)
                .unwrap_err()
        ),
        Some(Fault::Invalid)
    );
    assert!(
        f.apply(Command::ClearSeen { id: id(10) }, true)
            .unwrap()
            .changed
    );
    assert!(
        !f.apply(Command::ClearSeen { id: id(10) }, true)
            .unwrap()
            .changed
    );
}

#[test]
fn frozen_create_and_add_ids_are_idempotent_and_conflicts_leave_everything_unchanged() {
    let mut f = Fixture::new();
    f.page(1);
    let collection = f.manual(10);
    assert!(
        !f.apply(
            Command::Create {
                collection: collection.clone()
            },
            false
        )
        .unwrap()
        .changed
    );
    let item = Fixture::item(11, 1);
    assert!(
        f.apply(
            Command::Add {
                id: id(10),
                item: item.clone()
            },
            false
        )
        .unwrap()
        .changed
    );
    assert!(
        !f.apply(
            Command::Add {
                id: id(10),
                item: item.clone()
            },
            false
        )
        .unwrap()
        .changed
    );
    let mut changed = item;
    changed.note = "Different".into();
    let before = f.detail(&id(10)).unwrap();
    assert_eq!(
        fault(
            f.apply(
                Command::Add {
                    id: id(10),
                    item: changed
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Conflict)
    );
    assert_eq!(f.detail(&id(10)).unwrap(), before);
    assert_eq!(
        fault(
            f.apply(
                Command::Add {
                    id: id(10),
                    item: Fixture::item(12, 99)
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Invalid)
    );
    assert_eq!(f.detail(&id(10)).unwrap(), before);
}

#[test]
fn a_removed_page_keeps_an_unavailable_reference_and_archive_never_changes_pages() {
    let mut f = Fixture::new();
    f.page(1);
    f.manual(10);
    f.apply(
        Command::Add {
            id: id(10),
            item: Fixture::item(11, 1),
        },
        false,
    )
    .unwrap();
    f.oracle()
        .execute("DELETE FROM epoch_secrets WHERE page=?", [id(1)])
        .unwrap();
    let archived = f
        .apply(
            Command::Archive {
                id: id(10),
                archived: true,
            },
            false,
        )
        .unwrap();
    assert!(archived.detail.collection.archived);
    assert_eq!(archived.detail.items.len(), 1);
    let pages: i64 = f
        .oracle()
        .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages, 1);
    assert!(
        f.apply(
            Command::Remove {
                id: id(10),
                item: id(11)
            },
            false
        )
        .unwrap()
        .changed
    );
    assert!(
        !f.apply(
            Command::Remove {
                id: id(10),
                item: id(11)
            },
            false
        )
        .unwrap()
        .changed
    );
}

#[test]
fn smart_query_changes_clear_only_collection_seen_and_manual_changes_are_refused() {
    let mut f = Fixture::new();
    f.page(1);
    let collection = Collection {
        id: id(10),
        name: "Needs review".into(),
        maintainer: "PM".into(),
        archived: false,
        definition: Definition::Smart {
            search: Search::default(),
            needs_you: Some(true),
        },
    };
    f.apply(Command::Create { collection }, false).unwrap();
    f.apply(
        Command::Seen {
            id: id(10),
            target: id(1),
        },
        true,
    )
    .unwrap();
    let query = Search {
        label: vec!["Reports".into()],
        ..Search::default()
    };
    let changed = f
        .apply(
            Command::Query {
                id: id(10),
                search: query.clone(),
                needs_you: Some(true),
            },
            false,
        )
        .unwrap();
    assert!(changed.changed);
    assert!(changed.detail.seen.is_empty());
    assert!(
        !f.apply(
            Command::Query {
                id: id(10),
                search: query,
                needs_you: Some(true)
            },
            false
        )
        .unwrap()
        .changed
    );
    assert_eq!(
        fault(
            f.apply(
                Command::Add {
                    id: id(10),
                    item: Fixture::item(11, 1)
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Invalid)
    );
    assert_eq!(
        fault(
            f.apply(
                Command::Sort {
                    id: id(10),
                    sort: Sort::Manual
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Invalid)
    );
}

#[test]
fn a_refused_capacity_write_does_not_evict_a_reference_or_its_seen_state() {
    let mut f = Fixture::new();
    f.page(1);
    f.manual(10);
    for n in 0..tmt_colab::collections::ITEMS {
        f.apply(
            Command::Add {
                id: id(10),
                item: Fixture::item(100 + n, 1),
            },
            false,
        )
        .unwrap();
    }
    f.apply(
        Command::Seen {
            id: id(10),
            target: id(100),
        },
        true,
    )
    .unwrap();
    let before = f.detail(&id(10)).unwrap();
    assert_eq!(
        fault(
            f.apply(
                Command::Add {
                    id: id(10),
                    item: Fixture::item(999, 1)
                },
                false
            )
            .unwrap_err()
        ),
        Some(Fault::Capacity)
    );
    assert_eq!(f.detail(&id(10)).unwrap(), before);
}

#[test]
fn migration_thirteen_keeps_home_summary_and_page_rows_while_adding_empty_collections() {
    let mut f = Fixture::new();
    f.page(1);
    f.oracle().execute_batch("DROP TABLE collection_seen; DROP TABLE collection_items; DROP TABLE collections; PRAGMA user_version=13; INSERT INTO home_summary VALUES (1,'Ready','Two pages changed','PM',100);").unwrap();
    f.store = Store::open(&f.layout).unwrap();
    assert!(f.directory().unwrap().is_empty());
    let oracle = f.oracle();
    let version: u32 = oracle
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 14);
    let first: String = oracle
        .query_row("SELECT line1 FROM home_summary WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(first, "Ready");
    let pages: i64 = oracle
        .query_row("SELECT COUNT(*) FROM epoch_secrets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages, 1);
}
