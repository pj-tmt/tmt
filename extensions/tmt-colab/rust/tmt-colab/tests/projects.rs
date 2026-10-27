//! Projects in the owner store: display-only projection state, one immediate transaction per
//! write, no statement and no head change (contract: Projects). Pins are the same kind of state and
//! share this fixture (contract: Pins).
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    keyring::{Keyring, Layout},
    limits::{PAGE_PROJECTS, SPACE_PINS, SPACE_PROJECT_PAGES, SPACE_PROJECT_PINS, SPACE_PROJECTS},
    projects::Change,
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
            .map(|(page, projects)| {
                (
                    (*page).to_owned(),
                    projects.iter().map(|name| (*name).to_owned()).collect(),
                )
            })
            .collect();
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.set_projects(&changes)
            })
    }
    fn manage(&mut self, change: Change) -> tmt_colab::Result<()> {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.manage_project(&change)
            })
    }
    fn projects(&mut self, page: &str) -> Vec<String> {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.page_projects(page)
            })
            .unwrap()
    }
    fn names(&mut self) -> Vec<String> {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| tx.projects())
            .unwrap()
    }
    fn pin(&mut self, page: &str, pinned: bool) -> tmt_colab::Result<bool> {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| {
                tx.set_pinned(page, pinned)
            })
    }
    fn pins(&mut self) -> Vec<String> {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| tx.pins())
            .unwrap()
    }
    /// The pinned projects and whether the owner has written the list.
    fn project_pins(&mut self) -> (bool, Vec<String>) {
        let key = &self.key;
        self.store
            .project_transaction(&key.space_id, &key.owner_public(), |tx| tx.project_pins())
            .unwrap()
    }
    fn pin_projects(&mut self, names: &[&str]) -> tmt_colab::Result<()> {
        self.manage(Change::Pins(
            names.iter().map(|n| (*n).to_owned()).collect(),
        ))
    }
    /// The effective pinned projects as the page list reports them.
    fn reported_pins(&self) -> serde_json::Value {
        tmt_colab::inspection::catalog(&self.store, &self.key).unwrap()["projectPins"].clone()
    }
    fn rows(&self) -> usize {
        let rows: i64 = self
            .oracle()
            .query_row("SELECT COUNT(*) FROM page_projects", [], |r| r.get(0))
            .unwrap();
        rows as usize
    }
    /// The store as schema 12 left it: page labels, no projects table.
    fn as_schema_12(&mut self, labels: &[(&str, &str)]) {
        let db = self.oracle();
        db.execute_batch(
            "ALTER TABLE edit_decisions DROP COLUMN result; DROP TABLE page_auto_cleaned; DROP TABLE project_pin_choice; DROP TABLE project_pins; DROP TABLE page_pins; DROP TABLE page_titles; DROP TABLE page_projects; DROP TABLE projects; ALTER TABLE pages DROP COLUMN last_activity_at_ms; ALTER TABLE checkpoints DROP COLUMN signer;
             CREATE TABLE page_labels(page TEXT NOT NULL, label TEXT NOT NULL,
                PRIMARY KEY(page,label), FOREIGN KEY(page) REFERENCES pages(page)) WITHOUT ROWID;
             PRAGMA user_version=12;",
        )
        .unwrap();
        for (page, label) in labels {
            db.execute(
                "INSERT INTO page_labels(page,label) VALUES (?,?)",
                [page, label],
            )
            .unwrap();
        }
        drop(db);
    }
    fn reopen(&mut self) {
        self.store = Store::open(&self.layout).unwrap();
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
fn the_schema_adds_projects_and_keeps_what_was_there() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 13, "schema {version}");
    // A store from before labels (schema 6) gains the tables on the next open; nothing else moves.
    f.oracle()
        .execute_batch(
            "DROP TABLE edit_decisions; DROP TABLE cloud_page_rows; DROP TABLE link_publications; DROP TABLE link_admissions; DROP TABLE cloud_publications; DROP TABLE page_auto_cleaned; DROP TABLE project_pin_choice; DROP TABLE project_pins; DROP TABLE page_pins; DROP TABLE page_titles; DROP TABLE page_projects; DROP TABLE projects; ALTER TABLE pages DROP COLUMN last_activity_at_ms; ALTER TABLE checkpoints DROP COLUMN signer; PRAGMA user_version=6;",
        )
        .unwrap();
    f.reopen();
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 13);
    assert_eq!(f.rows(), 0);
    assert!(f.names().is_empty());
    let pages: i64 = f
        .oracle()
        .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages, 1);
    f.set(&[(&page(1), &["b", "a"])]).unwrap();
    assert_eq!(f.projects(&page(1)), ["a", "b"]);
}

#[test]
fn every_label_becomes_a_project_of_the_same_name_with_its_pages() {
    let mut f = Fixture::new();
    let (one, two, three, gone) = (page(1), page(2), page(3), page(4));
    for p in [&one, &two, &three, &gone] {
        f.open_page(p);
    }
    // A deleted page keeps its identity row and has no label rows (they leave with its data).
    f.oracle()
        .execute("DELETE FROM epoch_secrets WHERE page=?", [&gone])
        .unwrap();
    f.as_schema_12(&[
        (&one, "urgent"),
        (&one, "q4 plan"),
        (&two, "urgent"),
        (&three, "日本語"),
    ]);
    f.reopen();
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert!(version >= 13);
    assert_eq!(f.names(), ["q4 plan", "urgent", "日本語"]);
    assert_eq!(f.projects(&one), ["q4 plan", "urgent"]);
    assert_eq!(f.projects(&two), ["urgent"]);
    assert_eq!(f.projects(&three), ["日本語"]);
    assert!(f.projects(&gone).is_empty());
    assert_eq!(f.rows(), 4);
    let old: i64 = f
        .oracle()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='page_labels'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 0, "the label table is gone");
}

#[test]
fn a_store_with_no_labels_migrates_to_no_projects() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    f.as_schema_12(&[]);
    f.reopen();
    assert!(f.names().is_empty());
    assert_eq!(f.rows(), 0);
    f.set(&[(&page(1), &["new"])]).unwrap();
    assert_eq!(f.names(), ["new"]);
}

#[test]
fn a_write_replaces_the_whole_set_of_each_page_it_names() {
    let mut f = Fixture::new();
    let (one, two) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&two);
    f.set(&[(&one, &["a", "b"]), (&two, &["x"])]).unwrap();
    f.set(&[(&one, &["b", "c"])]).unwrap();
    assert_eq!(f.projects(&one), ["b", "c"]);
    assert_eq!(f.projects(&two), ["x"]);
    f.set(&[(&one, &[])]).unwrap();
    assert!(f.projects(&one).is_empty());
    assert_eq!(f.rows(), 1);
    // The same write twice is the same state: last writer wins, nothing accumulates.
    f.set(&[(&two, &["x", "y"])]).unwrap();
    f.set(&[(&two, &["x", "y"])]).unwrap();
    assert_eq!(f.projects(&two), ["x", "y"]);
}

#[test]
fn a_page_write_creates_the_projects_it_names_and_a_project_outlives_its_pages() {
    let mut f = Fixture::new();
    let one = page(1);
    f.open_page(&one);
    f.set(&[(&one, &["b", "a"])]).unwrap();
    assert_eq!(f.names(), ["a", "b"]);
    f.set(&[(&one, &[])]).unwrap();
    assert_eq!(f.names(), ["a", "b"], "an empty project stays");
    assert_eq!(f.rows(), 0);
}

#[test]
fn a_refused_batch_changes_nothing_on_any_page() {
    let mut f = Fixture::new();
    let (one, two) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&two);
    f.set(&[(&one, &["keep"])]).unwrap();
    let over: Vec<String> = (0..=PAGE_PROJECTS).map(|n| format!("l{n}")).collect();
    let over: Vec<&str> = over.iter().map(String::as_str).collect();
    let error = f.set(&[(&one, &["new"]), (&two, &over)]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    let error = f.set(&[(&one, &["new"]), (&two, &[" bad"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    let error = f
        .set(&[(&one, &["new"]), (&two, &["dup", "dup"])])
        .unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Invalid"));
    assert_eq!(f.projects(&one), ["keep"]);
    assert!(f.projects(&two).is_empty());
    assert_eq!(f.names(), ["keep"], "a refused batch creates no project");
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
    let pages: Vec<String> = (0..=SPACE_PROJECT_PAGES / PAGE_PROJECTS)
        .map(page)
        .collect();
    for p in &pages {
        f.open_page(p);
    }
    let names: Vec<String> = (0..PAGE_PROJECTS).map(|n| format!("l{n}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let full = SPACE_PROJECT_PAGES / PAGE_PROJECTS;
    let batch: Vec<(&str, &[&str])> = pages[..full]
        .iter()
        .map(|p| (p.as_str(), names.as_slice()))
        .collect();
    for chunk in batch.chunks(32) {
        f.set(chunk).unwrap();
    }
    assert_eq!(f.rows(), SPACE_PROJECT_PAGES);
    let error = f.set(&[(&pages[full], &["one more"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    assert_eq!(f.rows(), SPACE_PROJECT_PAGES);
    assert!(!f.names().contains(&"one more".to_owned()));
    // A write that leaves the space at or under the limit goes through, even one that swaps names.
    f.set(&[(&pages[0], &["swap"]), (&pages[full], &["fits"])])
        .unwrap();
    assert_eq!(f.rows(), SPACE_PROJECT_PAGES - PAGE_PROJECTS + 2);
}

fn create(name: &str) -> Change {
    Change::Create(name.to_owned())
}
fn rename(from: &str, to: &str) -> Change {
    Change::Rename {
        from: from.to_owned(),
        to: to.to_owned(),
    }
}
fn delete(name: &str) -> Change {
    Change::Delete(name.to_owned())
}

#[test]
fn a_project_can_exist_with_no_page_and_creating_it_again_changes_nothing() {
    let mut f = Fixture::new();
    f.manage(create("launch")).unwrap();
    f.manage(create("launch")).unwrap();
    assert_eq!(f.names(), ["launch"]);
    assert_eq!(f.rows(), 0);
    for bad in ["", " lead", "two  spaces", "tab\there"] {
        let error = f.manage(create(bad)).unwrap_err();
        assert_eq!(fault(error).as_deref(), Some("Invalid"), "{bad:?}");
    }
    assert_eq!(f.names(), ["launch"]);
}

#[test]
fn the_space_holds_a_bounded_number_of_projects() {
    let mut f = Fixture::new();
    let one = page(1);
    f.open_page(&one);
    for n in 0..SPACE_PROJECTS {
        f.manage(create(&format!("p{n:03}"))).unwrap();
    }
    let error = f.manage(create("one more")).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    let error = f.set(&[(&one, &["one more"])]).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    assert_eq!(f.names().len(), SPACE_PROJECTS);
    assert_eq!(f.rows(), 0, "the refused write left the page alone");
    // At the limit an existing name still works, and a rename keeps the count.
    f.manage(create("p000")).unwrap();
    f.set(&[(&one, &["p001"])]).unwrap();
    f.manage(rename("p002", "renamed")).unwrap();
    assert_eq!(f.names().len(), SPACE_PROJECTS);
    // Deleting one makes room for a new name.
    f.manage(delete("p003")).unwrap();
    f.manage(create("one more")).unwrap();
}

#[test]
fn a_space_that_migrated_above_the_name_limit_keeps_its_names_and_refuses_only_a_new_one() {
    let mut f = Fixture::new();
    let one = page(1);
    f.open_page(&one);
    {
        let db = f.oracle();
        for n in 0..=SPACE_PROJECTS {
            db.execute(
                "INSERT INTO projects(name) VALUES (?)",
                [format!("p{n:03}")],
            )
            .unwrap();
        }
    }
    f.set(&[(&one, &["p000", "p001"])]).unwrap();
    f.manage(create("p002")).unwrap();
    let error = f.manage(create("new")).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    f.manage(rename("p003", "p004")).unwrap();
}

#[test]
fn renaming_moves_every_page_and_onto_an_existing_name_it_merges() {
    let mut f = Fixture::new();
    let (one, two, three) = (page(1), page(2), page(3));
    for p in [&one, &two, &three] {
        f.open_page(p);
    }
    f.set(&[
        (&one, &["old"]),
        (&two, &["old", "new"]),
        (&three, &["other"]),
    ])
    .unwrap();
    f.manage(rename("old", "fresh")).unwrap();
    assert_eq!(f.projects(&one), ["fresh"]);
    assert_eq!(f.projects(&two), ["fresh", "new"]);
    assert_eq!(f.projects(&three), ["other"]);
    assert_eq!(f.names(), ["fresh", "new", "other"]);
    // Onto an existing name: a page in both keeps one entry, and the old name is gone.
    f.manage(rename("fresh", "new")).unwrap();
    assert_eq!(f.projects(&one), ["new"]);
    assert_eq!(f.projects(&two), ["new"]);
    assert_eq!(f.names(), ["new", "other"]);
    assert_eq!(f.rows(), 3);
    // An empty project can be renamed too.
    f.manage(create("empty")).unwrap();
    f.manage(rename("empty", "vacant")).unwrap();
    assert_eq!(f.names(), ["new", "other", "vacant"]);
}

#[test]
fn deleting_removes_the_grouping_and_never_a_page() {
    let mut f = Fixture::new();
    let (one, two) = (page(1), page(2));
    f.open_page(&one);
    f.open_page(&two);
    f.set(&[(&one, &["a", "b"]), (&two, &["a"])]).unwrap();
    f.manage(delete("a")).unwrap();
    assert_eq!(f.projects(&one), ["b"]);
    assert!(f.projects(&two).is_empty());
    assert_eq!(f.names(), ["b"]);
    let pages: i64 = f
        .oracle()
        .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages, 2);
    f.manage(delete("b")).unwrap();
    assert!(f.names().is_empty());
}

#[test]
fn an_unknown_project_cannot_be_renamed_or_deleted_and_a_rename_needs_a_new_valid_name() {
    let mut f = Fixture::new();
    let one = page(1);
    f.open_page(&one);
    f.set(&[(&one, &["a"])]).unwrap();
    for change in [
        rename("missing", "b"),
        delete("missing"),
        rename("a", "a"),
        rename("a", " bad"),
        rename("a", ""),
    ] {
        let error = f.manage(change).unwrap_err();
        assert_eq!(fault(error).as_deref(), Some("Invalid"));
    }
    assert_eq!(f.names(), ["a"]);
    assert_eq!(f.projects(&one), ["a"]);
}

#[test]
fn pins_keep_pin_order_change_only_when_the_state_changes_and_need_an_open_page() {
    let mut f = Fixture::new();
    for n in 1..=3 {
        f.open_page(&page(n));
    }
    assert!(f.pins().is_empty());
    assert!(f.pin(&page(2), true).unwrap());
    assert!(f.pin(&page(1), true).unwrap());
    assert!(!f.pin(&page(2), true).unwrap(), "pinning a pinned page");
    assert_eq!(f.pins(), [page(2), page(1)], "pin order, not page order");
    assert!(
        !f.pin(&page(3), false).unwrap(),
        "unpinning an unpinned page"
    );
    assert!(f.pin(&page(2), false).unwrap());
    assert!(f.pin(&page(2), true).unwrap());
    assert_eq!(f.pins(), [page(1), page(2)], "a repin goes last");
    // An unknown page and a malformed ID are refused, and nothing changes.
    for refused in [page(9), "not-a-page".to_owned()] {
        let error = f.pin(&refused, true).unwrap_err();
        assert_eq!(fault(error).as_deref(), Some("Invalid"), "{refused}");
    }
    assert_eq!(f.pins(), [page(1), page(2)]);
}

#[test]
fn a_pin_past_the_space_limit_is_refused_and_keeps_the_older_pins() {
    let mut f = Fixture::new();
    for n in 1..=SPACE_PINS + 1 {
        f.open_page(&page(n));
    }
    for n in 1..=SPACE_PINS {
        assert!(f.pin(&page(n), true).unwrap());
    }
    let error = f.pin(&page(SPACE_PINS + 1), true).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    assert_eq!(f.pins().len(), SPACE_PINS);
    assert_eq!(f.pins()[0], page(1));
    // Unpinning frees a place.
    assert!(f.pin(&page(1), false).unwrap());
    assert!(f.pin(&page(SPACE_PINS + 1), true).unwrap());
}

#[test]
fn schema_18_adds_the_pin_tables_to_a_store_from_schema_17_and_the_list_starts_unchosen() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    f.set(&[(&page(1), &["a"])]).unwrap();
    f.pin_projects(&["a"]).unwrap();
    assert_eq!(f.project_pins(), (true, vec!["a".to_owned()]));
    f.oracle()
        .execute_batch(
            "ALTER TABLE edit_decisions DROP COLUMN result; DROP TABLE page_auto_cleaned; DROP TABLE project_pin_choice; DROP TABLE project_pins; PRAGMA user_version=17;",
        )
        .unwrap();
    f.reopen();
    let version: u32 = f
        .oracle()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    // Reopening applies migration 18 and all later migrations. Keep the
    // current version pinned, updating it deliberately when another migration is added.
    assert_eq!(version, 20);
    assert_eq!(f.project_pins(), (false, vec![]));
    assert_eq!(f.names(), ["a"]);
}

#[test]
fn the_pinned_list_is_written_whole_in_the_owners_order_and_an_empty_list_is_a_choice() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    f.set(&[(&page(1), &["a", "b", "c"])]).unwrap();
    assert_eq!(f.project_pins(), (false, vec![]));
    f.pin_projects(&["c", "a"]).unwrap();
    assert_eq!(
        f.project_pins(),
        (true, vec!["c".to_owned(), "a".to_owned()])
    );
    // The same write again changes nothing; a new order replaces the old one.
    f.pin_projects(&["c", "a"]).unwrap();
    f.pin_projects(&["a", "b", "c"]).unwrap();
    assert_eq!(f.project_pins().1, ["a", "b", "c"]);
    // Empty is the owner's choice, not the automatic default.
    f.pin_projects(&[]).unwrap();
    assert_eq!(f.project_pins(), (true, vec![]));
}

#[test]
fn a_refused_pinned_list_changes_nothing_and_the_limit_is_a_capacity_refusal() {
    let mut f = Fixture::new();
    f.open_page(&page(1));
    f.set(&[(&page(1), &["a", "b"])]).unwrap();
    f.pin_projects(&["a", "b"]).unwrap();
    for bad in [
        vec!["a", "missing"],
        vec!["a", "a"],
        vec![" a"],
        vec!["a", ""],
    ] {
        let error = f.pin_projects(&bad).unwrap_err();
        assert_eq!(fault(error).as_deref(), Some("Invalid"), "{bad:?}");
        assert_eq!(f.project_pins().1, ["a", "b"], "{bad:?}");
    }
    let many: Vec<String> = (0..=SPACE_PROJECT_PINS)
        .map(|n| format!("p{n:02}"))
        .collect();
    for name in &many {
        f.manage(create(name)).unwrap();
    }
    let all: Vec<&str> = many.iter().map(String::as_str).collect();
    let error = f.pin_projects(&all).unwrap_err();
    assert_eq!(fault(error).as_deref(), Some("Capacity"));
    assert_eq!(f.project_pins().1, ["a", "b"]);
    // Exactly the limit is accepted, with no older pin dropped.
    f.pin_projects(&all[..SPACE_PROJECT_PINS]).unwrap();
    assert_eq!(f.project_pins().1.len(), SPACE_PROJECT_PINS);
}

#[test]
fn a_pin_moves_with_a_rename_keeps_the_earlier_place_in_a_merge_and_leaves_with_a_delete() {
    let mut f = Fixture::new();
    for name in ["a", "b", "c", "d", "u"] {
        f.manage(create(name)).unwrap();
    }
    f.pin_projects(&["a", "b", "c"]).unwrap();
    // A rename keeps the place; onto an unpinned project the pin moves to the new name.
    f.manage(rename("b", "x")).unwrap();
    assert_eq!(f.project_pins().1, ["a", "x", "c"]);
    f.manage(rename("x", "d")).unwrap();
    assert_eq!(f.project_pins().1, ["a", "d", "c"]);
    // Onto a pinned project the two merge and the earlier place wins.
    f.manage(rename("c", "a")).unwrap();
    assert_eq!(f.project_pins().1, ["a", "d"]);
    f.manage(rename("d", "a")).unwrap();
    assert_eq!(f.project_pins().1, ["a"]);
    // An unpinned project renamed onto a pinned one adds nothing; creating pins nothing.
    f.manage(rename("u", "a")).unwrap();
    f.manage(create("fresh")).unwrap();
    assert_eq!(f.project_pins().1, ["a"]);
    // Delete removes the pin, and an emptied list stays the owner's.
    f.manage(delete("a")).unwrap();
    assert_eq!(f.project_pins(), (true, vec![]));
}

#[test]
fn the_page_list_reports_the_written_list_or_the_five_most_recently_used_projects() {
    let mut f = Fixture::new();
    for n in 1..=7 {
        f.open_page(&page(n));
        f.oracle()
            .execute(
                "UPDATE pages SET last_update_at_ms=? WHERE page=?",
                rusqlite::params![(n * 1000) as i64, page(n)],
            )
            .unwrap();
    }
    // Page n is in project "p<n>"; page 7 also holds "p1", so p1 is as recent as p7. "idle" has no page.
    let names: Vec<String> = (1..=7).map(|n| format!("p{n}")).collect();
    for n in 1..=7 {
        let mut projects = vec![names[n - 1].as_str()];
        if n == 7 {
            projects.push("p1");
        }
        f.set(&[(&page(n), &projects)]).unwrap();
    }
    f.manage(create("idle")).unwrap();
    // p7 and p1 tie at 7000 and sort by code point; the next three are p6, p5, p4.
    assert_eq!(
        f.reported_pins(),
        serde_json::json!({"names":["p1","p7","p6","p5","p4"],"automatic":true})
    );
    // The owner's list replaces it for good, including an empty one.
    f.pin_projects(&["idle", "p2"]).unwrap();
    assert_eq!(
        f.reported_pins(),
        serde_json::json!({"names":["idle","p2"],"automatic":false})
    );
    f.pin_projects(&[]).unwrap();
    assert_eq!(
        f.reported_pins(),
        serde_json::json!({"names":[],"automatic":false})
    );
}
