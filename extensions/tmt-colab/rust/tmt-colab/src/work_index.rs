//! What every page's work says, read once for the whole space: its manual state and its direct
//! sub-pages, for the owner list's stage and progress and the sub-page link rules
//! (`work_links.rs`). Reading a page's work folds the page, so the answers are kept in one small
//! derived file (`page_index.rs` owns how: keyed by the page's own progress, refolded only when it
//! moves, never authority).
use crate::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    page_index::{self, Kept},
    store::Store,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// 5: the work of a page is its sub-pages, and a task-era value not yet migrated is marked
/// (#5825); older entries are refolded.
const VERSION: u64 = 5;

/// The work of the pages of the space: a page missing from it is unavailable (not asked, deleted,
/// unreadable), and `None` is a verified shell.
pub type Found = BTreeMap<String, Option<PageWork>>;

/// One page's work, as the list and the link rules need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageWork {
    /// The verified manual page state.
    pub state: String,
    /// The direct sub-pages, in the page's order.
    pub children: Vec<String>,
    /// The stored value is still the task-era version 1: the serve has not migrated it yet.
    pub legacy: bool,
}

impl Kept for Option<PageWork> {
    const FILE: &'static str = "work-index.json";
    const LOCK: &'static str = "work-index.lock";
    const VERSION: u64 = VERSION;
    fn to_json(&self) -> Value {
        match self {
            Some(work) if work.legacy => {
                json!({"state":work.state,"children":work.children,"legacy":true})
            }
            Some(work) => json!({"state":work.state,"children":work.children}),
            None => json!({}),
        }
    }
    fn from_json(_page: &str, entry: &Value) -> Option<Self> {
        let Some(state) = entry.get("state") else {
            return Some(None);
        };
        if !crate::decoder::valid_page_state(state) {
            return None;
        }
        let children =
            crate::work::children(&json!({"version":2,"order":entry.get("children")?})).ok()?;
        Some(Some(PageWork {
            state: state.as_str()?.to_owned(),
            children,
            legacy: entry.get("legacy") == Some(&Value::Bool(true)),
        }))
    }
}

/// Current cached work only. Missing, stale or unreadable entries are omitted without a fold,
/// lock or write; callers needing freshly derived work use [`scan`].
pub fn cached(layout: &Layout, store: &Store, key: &Keyring, pages: &[String]) -> Result<Found> {
    page_index::cached(layout, store, key, pages)
}

/// The work of each given page: `None` for a shell or a page that cannot be read now. Folds only
/// changed pages and retains current live neighbors in the bounded cache.
pub fn scan(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    pages: &[String],
) -> Result<Found> {
    let live = crate::inspection::all_page_ids(store, key).ok();
    scan_with_live(layout, store, key, decoder, pages, live.as_deref())
}

/// Scan with a live-page catalog the caller already read; `None` leaves the cache untouched.
pub fn scan_with_live(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    pages: &[String],
    live: Option<&[String]>,
) -> Result<Found> {
    page_index::scan(layout, store, key, pages, live, |id| {
        fold(store, key, id, decoder)
    })
}

/// `Some(None)`: the page is a shell. `None`: it could not be read.
fn fold(store: &Store, key: &Keyring, id: &str, decoder: &mut Decoder) -> Option<Option<PageWork>> {
    match page::read_work_or_shell(store, key, id, decoder) {
        Ok(Some(read)) => Some(Some(PageWork {
            state: read.state,
            children: read.children,
            legacy: read.legacy.is_some(),
        })),
        Ok(None) => Some(None),
        Err(_) => None,
    }
}

/// The work a page's `meta` holds, from a fold that already happened: `None` for a shell or a
/// value that does not validate.
pub fn page_work(meta: &Value) -> Option<PageWork> {
    if meta["pageType"].as_str() == Some("shell") {
        return None;
    }
    let empty = crate::work::empty();
    let work = meta.get("work").unwrap_or(&empty);
    let children = crate::work::children(work).ok()?;
    let state = match meta.get("pageState") {
        None => "todo",
        Some(value) if crate::decoder::valid_page_state(value) => value.as_str()?,
        Some(_) => return None,
    };
    Some(PageWork {
        state: state.to_owned(),
        children,
        legacy: work["version"] == 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: &str = "20000000-0000-4000-8000-000000000001";

    fn work(children: &[&str]) -> Option<PageWork> {
        Some(PageWork {
            state: "todo".into(),
            children: children.iter().map(|id| (*id).to_owned()).collect(),
            legacy: false,
        })
    }

    #[test]
    fn cached_reads_only_current_entries_and_never_creates_or_rewrites_the_index() {
        let root =
            std::env::temp_dir().join(format!("tmt-work-index-cache-{}", std::process::id()));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        let pages: Vec<_> = (1..=3)
            .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
            .collect();
        for page in &pages {
            store.create_page(page).unwrap();
        }
        assert!(cached(&layout, &store, &key, &pages).unwrap().is_empty());
        assert!(!layout.directory.join(<Option<PageWork>>::FILE).exists());
        let fingerprints = store
            .owner_read(&key.space_id, &key.owner_public(), |tx| tx.page_progress())
            .unwrap();
        let work = work(&[CHILD]);
        page_index::save(
            &layout,
            &BTreeMap::from([
                (
                    pages[0].clone(),
                    (fingerprints[&pages[0]].clone(), work.clone()),
                ),
                (pages[1].clone(), (fingerprints[&pages[1]].clone(), None)),
            ]),
        )
        .unwrap();
        store.advance_epoch(&pages[1], 1).unwrap();
        let before = std::fs::read(layout.directory.join(<Option<PageWork>>::FILE)).unwrap();
        // These pages deliberately have no decryptable content. The current projection remains
        // usable; a stale/missing one is omitted rather than causing any materialization.
        assert_eq!(
            cached(&layout, &store, &key, &pages).unwrap(),
            BTreeMap::from([(pages[0].clone(), work)])
        );
        assert_eq!(
            std::fs::read(layout.directory.join(<Option<PageWork>>::FILE)).unwrap(),
            before
        );
        assert!(!layout.directory.join(<Option<PageWork>>::LOCK).exists());
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_entry_survives_the_file_and_a_damaged_one_is_dropped() {
        let work = work(&[CHILD]);
        let json = page_index::entry_json(&work, "f1");
        assert_eq!(json["fingerprint"], "f1");
        assert_eq!(<Option<PageWork>>::from_json("p", &json), Some(work));
        assert_eq!(
            <Option<PageWork>>::from_json("p", &page_index::entry_json(&None, "f2")),
            Some(None)
        );
        for damaged in [
            json!({"fingerprint":"f","state":"blocked","children":[]}),
            json!({"fingerprint":"f","state":"todo"}),
            json!({"fingerprint":"f","state":"todo","children":[CHILD, CHILD]}),
            json!({"fingerprint":"f","state":"todo","children":["T-1"]}),
        ] {
            assert_eq!(
                <Option<PageWork>>::from_json("p", &damaged),
                None,
                "{damaged}"
            );
        }
    }

    #[test]
    fn a_page_reads_its_state_and_sub_pages_and_a_shell_or_bad_value_reads_none() {
        assert_eq!(
            page_work(&json!({"pageState":"done"})).unwrap().state,
            "done"
        );
        assert!(page_work(&json!({})).unwrap().children.is_empty());
        assert_eq!(
            page_work(&json!({"work":{"version":2,"order":[CHILD]}})),
            work(&[CHILD])
        );
        assert!(page_work(&json!({"pageState":null})).is_none());
        assert!(page_work(&json!({"work":{"version":2,"order":[1]}})).is_none());
        // A task-era value reads its linked tasks as sub-pages until it is migrated.
        let v1 = json!({"work":{"version":1,"next":3,"order":["T-1","T-2"],"tasks":{
            "T-1":{"title":"plain","state":"todo"},
            "T-2":{"title":"linked","state":"todo","child":CHILD}}}});
        let read = page_work(&v1).unwrap();
        assert_eq!(read.children, [CHILD]);
        assert!(read.legacy);
        assert!(!work(&[CHILD]).unwrap().legacy);
        // The mark survives the derived file and an entry without it reads as migrated.
        let entry = Some(read.clone()).to_json();
        assert_eq!(entry["legacy"], true);
        assert_eq!(<Option<PageWork>>::from_json("p", &entry), Some(Some(read)));
        assert_eq!(
            <Option<PageWork>>::from_json("p", &json!({"state":"todo","children":[CHILD]})),
            Some(work(&[CHILD]))
        );
        assert!(page_work(&json!({"pageType":"shell"})).is_none());
    }
}
