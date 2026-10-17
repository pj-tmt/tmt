//! The home summary in the owner store and in the owner-served list: display-only state, one
//! row, no statement and no head change (contract: Home summary).
mod support;
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tmt_colab::{
    home_summary::HomeSummary,
    inspection,
    keyring::{Keyring, Layout},
    registration::Registration,
    store::Store,
};
use tmt_colab_model::values;

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");

struct Fixture {
    root: PathBuf,
    layout: Layout,
    key: Keyring,
    store: Store,
    clock: Arc<AtomicU64>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-3539-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let clock = Arc::new(AtomicU64::new(1_790_000_000_000));
        let store = Fixture::open(&layout, &clock);
        Self {
            root,
            layout,
            key,
            store,
            clock,
        }
    }
    fn open(layout: &Layout, clock: &Arc<AtomicU64>) -> Store {
        let clock = clock.clone();
        Store::open(layout)
            .unwrap()
            .with_clock(move || Ok(clock.load(Ordering::SeqCst)))
    }
    fn oracle(&self) -> Connection {
        Connection::open(self.layout.directory.join("space.db")).unwrap()
    }
    fn write(
        &mut self,
        lines: Option<&[String; 2]>,
        author: Option<&str>,
    ) -> tmt_colab::Result<(Option<HomeSummary>, bool)> {
        self.store
            .label_transaction(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.set_home_summary(lines, author)
            })
    }
    fn read(&mut self) -> Option<HomeSummary> {
        inspection::home_summary(&self.store, &self.key).unwrap()
    }
    fn rows(&self) -> i64 {
        self.oracle()
            .query_row("SELECT COUNT(*) FROM home_summary", [], |r| r.get(0))
            .unwrap()
    }
    /// The owner-served list, as a paired owner browser reads it.
    fn list(&self) -> Value {
        let clock = self.clock.clone();
        let api = Registration::with_decoder_config(
            Store::read(&self.layout)
                .unwrap()
                .with_clock(move || Ok(clock.load(Ordering::SeqCst))),
            Keyring::read(&self.layout).unwrap(),
            support::decoder_config(BINARY.into()),
        )
        .unwrap();
        let context = json!({"deviceId":"30000000-0000-4000-8000-000000000001", "kind":"browser", "origin":"http://127.0.0.1:1", "name":"Owner", "owner":true, "grantRevision":1,
            "publicKey":values::encode_binary(SigningKey::from_bytes(&[9;32]).verifying_key().as_bytes())}).to_string();
        let list = serde_json::from_slice(&api.pages(Some(&context)).unwrap()).unwrap();
        api.close().unwrap();
        list
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn lines(first: &str, second: &str) -> [String; 2] {
    [first.to_owned(), second.to_owned()]
}

#[test]
fn the_schema_adds_the_home_summary_and_keeps_what_was_there() {
    let mut f = Fixture::new();
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 13, "schema {version}");
    f.write(Some(&lines("a", "b")), None).unwrap();
    // A store from before the summary (schema 12) gains the empty table on the next open.
    f.oracle()
        .execute_batch("DROP TABLE collection_seen; DROP TABLE collection_items; DROP TABLE collections; DROP TABLE home_summary; PRAGMA user_version=12;")
        .unwrap();
    f.store = Fixture::open(&f.layout, &f.clock);
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 13);
    assert_eq!(f.rows(), 0);
    assert_eq!(f.read(), None);
    f.write(Some(&lines("c", "d")), None).unwrap();
    assert_eq!(f.read().unwrap().lines, lines("c", "d"));
}

#[test]
fn a_write_replaces_the_one_row_with_the_owners_clock_and_the_writers_name() {
    let mut f = Fixture::new();
    let (stored, changed) = f.write(Some(&lines("One", "Two")), Some("Ada")).unwrap();
    assert!(changed);
    let first = HomeSummary {
        lines: lines("One", "Two"),
        author: Some("Ada".to_owned()),
        at_ms: 1_790_000_000_000,
    };
    assert_eq!(stored, Some(first.clone()));
    assert_eq!(f.read(), Some(first));
    // The time is the store clock at the write, never anything a request carries.
    f.clock.store(1_790_000_060_000, Ordering::SeqCst);
    f.write(Some(&lines("Three", "Four")), None).unwrap();
    assert_eq!(f.rows(), 1);
    assert_eq!(
        f.read(),
        Some(HomeSummary {
            lines: lines("Three", "Four"),
            author: None,
            at_ms: 1_790_000_060_000,
        })
    );
}

#[test]
fn a_refused_write_changes_nothing_and_a_clear_reports_whether_it_removed_a_row() {
    let mut f = Fixture::new();
    f.write(Some(&lines("Keep", "me")), Some("Ada")).unwrap();
    let before = f.read();
    for (bad, author) in [
        (lines("", "ok"), None),
        (lines(&"x".repeat(25), "ok"), None),
        (lines("ok", &"x".repeat(61)), None),
        (lines("ok", "tab\there"), None),
        (lines("ok", "fine"), Some("bad\u{7}name")),
    ] {
        assert!(f.write(Some(&bad), author).is_err(), "{bad:?}");
        assert_eq!(f.read(), before);
    }
    assert_eq!(f.write(None, None).unwrap(), (None, true));
    assert_eq!(f.rows(), 0);
    assert_eq!(f.write(None, None).unwrap(), (None, false));
}

#[test]
fn the_owner_list_carries_the_summary_when_there_is_one_and_no_key_when_there_is_not() {
    let mut f = Fixture::new();
    assert!(f.list().get("homeSummary").is_none());
    f.write(
        Some(&lines("Two pages changed", "Pricing moved")),
        Some("Ada"),
    )
    .unwrap();
    let list = f.list();
    assert_eq!(
        list["homeSummary"],
        json!({"lines":["Two pages changed","Pricing moved"],"author":"Ada","atMs":1_790_000_000_000u64})
    );
    // Only this one key is new: the rest of the list keeps its shape.
    let mut keys: Vec<&str> = list
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "homeSummary",
            "ownerKey",
            "pageIds",
            "pages",
            "revision",
            "spaceId"
        ]
    );
    f.write(None, None).unwrap();
    assert!(f.list().get("homeSummary").is_none());
}

#[test]
fn the_longest_summary_stays_far_inside_the_discovery_cap() {
    let mut f = Fixture::new();
    let author = "a".repeat(tmt_colab::limits::PUBLISHER_AGENT_BYTES);
    f.write(
        Some(&lines(&"🚀".repeat(24), &"🚀".repeat(60))),
        Some(&author),
    )
    .unwrap();
    let encoded = serde_json::to_vec(&f.list()["homeSummary"]).unwrap();
    // 84 four-byte points and the longest author, escaped to six bytes each at worst.
    assert!(encoded.len() < 2 * 1024, "{} bytes", encoded.len());
}
