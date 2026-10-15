//! Page labels in the owner store: display-only projection state, one immediate transaction per
//! write, no statement and no head change (contract: Page labels).
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    keyring::{Keyring, Layout},
    limits::{PAGE_LABELS, SPACE_LABELS},
    store::{Store, owner::OwnerFault},
};

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
            "tmt-2588-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
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
    /// An open page: its identity and an epoch secret, as page creation leaves it.
    fn open_page(&mut self, page: &str) {
        self.store.create_page(page).unwrap();
        self.oracle()
            .execute(
                "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?,'1',zeroblob(32))",
                [page],
            )
            .unwrap();
    }
    fn set(&mut self, changes: &[(&str, &[&str])]) -> tmt_colab::Result<()> {
        let changes: Vec<(String, Vec<String>)> = changes
            .iter()
            .map(|(page, labels)| {
                (
                    (*page).to_owned(),
                    labels.iter().map(|name| (*name).to_owned()).collect(),
                )
            })
            .collect();
        let key = &self.key;
        self.store
            .label_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.set_labels(&changes)
            })
    }
    fn labels(&mut self, page: &str) -> Vec<String> {
        let key = &self.key;
        self.store
            .label_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.page_labels(page)
            })
            .unwrap()
    }
    fn rows(&self) -> usize {
        let rows: i64 = self
            .oracle()
            .query_row("SELECT COUNT(*) FROM page_labels", [], |r| r.get(0))
            .unwrap();
        rows as usize
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn page(n: usize) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}
fn fault(error: Box<dyn std::error::Error + Send + Sync>) -> Option<String> {
    error
        .downcast_ref::<OwnerFault>()
        .map(|fault| format!("{fault:?}"))
}

#[test]
fn the_schema_adds_page_labels_and_keeps_what_was_there() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 7, "schema {version}");
    // A store from before labels (schema 6) gains the table on the next open; nothing else moves.
    f.oracle()
        .execute_batch(
            "DROP TABLE cloud_page_rows; DROP TABLE link_publications; DROP TABLE link_admissions; DROP TABLE cloud_publications; DROP TABLE page_labels; PRAGMA user_version=6;",
        )
        .unwrap();
    f.store = Store::open(&f.layout).unwrap();
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 7);
    assert_eq!(f.rows(), 0);
    let pages: i64 = f
        .oracle()
        .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages, 1);
    f.set(&[(&page(1), &["b", "a"])]).unwrap();
    assert_eq!(f.labels(&page(1)), ["a", "b"]);
}

#[test]
fn a_write_replaces_the_whole_set_of_each_page_it_names() {
    let mut f = Fixture::new();
    let (one, two) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&two);
    f.set(&[(&one, &["a", "b"]), (&two, &["x"])]).unwrap();
    f.set(&[(&one, &["b", "c"])]).unwrap();
    assert_eq!(f.labels(&one), ["b", "c"]);
    assert_eq!(f.labels(&two), ["x"]);
    f.set(&[(&one, &[])]).unwrap();
    assert!(f.labels(&one).is_empty());
    assert_eq!(f.rows(), 1);
    // The same write twice is the same state: last writer wins, nothing accumulates.
    f.set(&[(&two, &["x", "y"])]).unwrap();
    f.set(&[(&two, &["x", "y"])]).unwrap();
    assert_eq!(f.labels(&two), ["x", "y"]);
}

#[test]
fn a_refused_batch_changes_nothing_on_any_page() {
    let mut f = Fixture::new();
    let (one, two) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&two);
    f.set(&[(&one, &["keep"])]).unwrap();
    let over: Vec<String> = (0..=PAGE_LABELS).map(|n| format!("l{n}")).collect();
    let over: Vec<&str> = over.iter().map(String::as_str).collect();
    let error = f.set(&[(&one, &["new"]), (&two, &over)]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    let error = f.set(&[(&one, &["new"]), (&two, &[" bad"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    let error = f
        .set(&[(&one, &["new"]), (&two, &["dup", "dup"])])
        .unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    assert_eq!(f.labels(&one), ["keep"]);
    assert!(f.labels(&two).is_empty());
}

#[test]
fn an_unknown_or_deleted_page_is_refused_and_a_repeated_page_is_one_change() {
    let mut f = Fixture::new();
    let (one, gone) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&gone);
    // A deleted page keeps its identity row but loses its epoch secrets (delete_page_data).
    f.oracle()
        .execute("DELETE FROM epoch_secrets WHERE page=?", [&gone])
        .unwrap();
    let error = f.set(&[(&one, &["a"]), (&gone, &["b"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    let error = f.set(&[(&page(9), &["b"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    assert_eq!(f.rows(), 0);
    let error = f.set(&[(&one, &["a"]), (&one, &["b"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
}

#[test]
fn the_space_limit_counts_every_assignment_and_refuses_atomically() {
    let mut f = Fixture::new();
    let pages: Vec<String> = (0..=SPACE_LABELS / PAGE_LABELS).map(page).collect();
    for p in &pages {
        f.open_page(p);
    }
    let names: Vec<String> = (0..PAGE_LABELS).map(|n| format!("l{n}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let full = SPACE_LABELS / PAGE_LABELS;
    let batch: Vec<(&str, &[&str])> = pages[..full]
        .iter()
        .map(|p| (p.as_str(), names.as_slice()))
        .collect();
    for chunk in batch.chunks(32) {
        f.set(chunk).unwrap();
    }
    assert_eq!(f.rows(), SPACE_LABELS);
    let error = f.set(&[(&pages[full], &["one more"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    assert_eq!(f.rows(), SPACE_LABELS);
    // A write that leaves the space at or under the limit goes through, even one that swaps names.
    f.set(&[(&pages[0], &["swap"]), (&pages[full], &["fits"])])
        .unwrap();
    assert_eq!(f.rows(), SPACE_LABELS - PAGE_LABELS + 2);
}
