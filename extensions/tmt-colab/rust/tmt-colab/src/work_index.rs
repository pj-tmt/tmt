//! What every page's work says, read once for the whole space: the owner list's stage, progress and
//! assignees, and the tasks each agent holds for the work-in-progress limit (`wip.rs`). Reading a
//! page's work folds the page, so the answers are kept in one small derived file (`page_index.rs`
//! owns how: keyed by the page's own progress, refolded only when it moves, never authority).
use crate::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    page_index::{self, Kept},
    store::Store,
    wip::Held,
    work::Line,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// 2: a kept task line carries its title.
const VERSION: u64 = 2;

/// The work of the pages of the space: a page missing from it is unavailable (not asked, deleted,
/// unreadable), and `None` is a page with no work.
pub type Found = BTreeMap<String, Option<PageWork>>;

/// One page's work, as the list and the limit need it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageWork {
    /// `work::Derived::to_value()` of the tasks' own states.
    pub derived: Value,
    pub held: Vec<Held>,
    /// Every task, kept only for a page whose tasks follow other pages (`work_links.rs`); empty
    /// otherwise.
    pub lines: Vec<Line>,
}

fn line_json(line: &Line) -> Value {
    let mut value = json!({"id":line.id,"title":line.title,"state":line.state});
    if line.signoff {
        value["signoff"] = json!(true);
    }
    if let Some(assignee) = &line.assignee {
        value["assignee"] = assignee.clone();
    }
    if let Some(child) = &line.child {
        value["child"] = json!(child);
    }
    value
}
fn line_from(value: &Value) -> Option<Line> {
    Some(Line {
        id: value["id"].as_str()?.to_owned(),
        title: value["title"].as_str()?.to_owned(),
        state: value["state"].as_str()?.to_owned(),
        signoff: value.get("signoff") == Some(&Value::Bool(true)),
        assignee: value.get("assignee").cloned(),
        child: match value.get("child") {
            None => None,
            Some(child) => Some(child.as_str()?.to_owned()),
        },
    })
}
impl Kept for Option<PageWork> {
    const FILE: &'static str = "work-index.json";
    const LOCK: &'static str = "work-index.lock";
    const VERSION: u64 = VERSION;
    fn to_json(&self) -> Value {
        let mut entry = json!({});
        if let Some(work) = self {
            entry["derived"] = work.derived.clone();
            entry["held"] = work
                .held
                .iter()
                .map(|held| json!({"agent":held.agent,"task":held.task}))
                .collect();
            if !work.lines.is_empty() {
                entry["lines"] = work.lines.iter().map(line_json).collect();
            }
        }
        entry
    }
    fn from_json(page: &str, entry: &Value) -> Option<Self> {
        if entry.get("derived").is_none() {
            return Some(None);
        }
        let held = entry["held"]
            .as_array()?
            .iter()
            .map(|held| {
                Some(Held {
                    agent: held["agent"].as_str()?.to_owned(),
                    page: page.to_owned(),
                    task: held["task"].as_str()?.to_owned(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let lines = match entry.get("lines") {
            None => Vec::new(),
            Some(lines) => lines
                .as_array()?
                .iter()
                .map(line_from)
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Some(PageWork {
            derived: entry["derived"].clone(),
            held,
            lines,
        }))
    }
}

/// Current cached work only. Missing, stale or unreadable entries are omitted without a fold,
/// lock or write; callers needing freshly derived work use [`scan`].
pub fn cached(layout: &Layout, store: &Store, key: &Keyring, pages: &[String]) -> Result<Found> {
    page_index::cached(layout, store, key, pages)
}

/// The work of each given page: `None` for a page with no work, a shell or a page that cannot be
/// read now. Folds only the pages whose fingerprint changed since the last scan, and keeps the file
/// to the given pages.
pub fn scan(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    decoder: &mut Decoder,
    pages: &[String],
) -> Result<Found> {
    // Unreadable now: say nothing and ask again next time.
    page_index::scan(layout, store, key, pages, |id| {
        fold(store, key, id, decoder)
    })
}

/// `Some(None)`: the page has no work (or is a shell). `None`: it could not be read.
fn fold(store: &Store, key: &Keyring, id: &str, decoder: &mut Decoder) -> Option<Option<PageWork>> {
    match page::read_work(store, key, id, decoder) {
        Ok(read) => Some(from_work(id, &read.work, read.derived)),
        // A shell holds no work.
        Err(error) if error.downcast_ref::<page::Fault>() == Some(&page::Fault::Invalid) => {
            Some(None)
        }
        Err(_) => None,
    }
}

/// The work a page's `meta` holds, as the list and the limit need it, from a fold that already
/// happened: `None` for a page with no work, a shell or a value that does not validate.
pub fn page_work(id: &str, meta: &Value) -> Option<PageWork> {
    if meta["pageType"].as_str() == Some("shell") {
        return None;
    }
    let work = meta.get("work")?;
    let derived = crate::work::derived(work).ok()?.to_value();
    from_work(id, work, derived)
}

fn from_work(id: &str, work: &Value, derived: Value) -> Option<PageWork> {
    if *work == crate::work::empty() {
        return None;
    }
    let lines = crate::work::lines(work).ok()?;
    Some(PageWork {
        held: crate::wip::held(id, work),
        derived,
        lines: if lines.iter().any(|line| line.child.is_some()) {
            lines
        } else {
            Vec::new()
        },
    })
}

/// What the pages other than `page` hold.
pub fn held_elsewhere(found: &Found, page: &str) -> Vec<Held> {
    found
        .iter()
        .filter(|(id, _)| id.as_str() != page)
        .flat_map(|(_, work)| work.iter().flat_map(|work| work.held.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let work = Some(PageWork {
            derived: json!({"stage":"in-progress","done":0,"total":1,"working":[],"signoff":0}),
            held: vec![],
            lines: vec![],
        });
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
        let work = Some(PageWork {
            derived: json!({"stage":"in-progress","done":0,"total":1,"working":[],"signoff":0}),
            held: vec![Held {
                agent: "ada".into(),
                page: "p".into(),
                task: "T-1".into(),
            }],
            lines: vec![Line {
                id: "T-1".into(),
                title: "Do it".into(),
                state: "todo".into(),
                signoff: true,
                assignee: Some(json!({"kind":"agent","name":"ada"})),
                child: Some("20000000-0000-4000-8000-000000000001".into()),
            }],
        });
        let json = page_index::entry_json(&work, "f1");
        assert_eq!(json["fingerprint"], "f1");
        assert_eq!(<Option<PageWork>>::from_json("p", &json), Some(work));
        assert_eq!(
            <Option<PageWork>>::from_json("p", &page_index::entry_json(&None, "f2")),
            Some(None)
        );
        assert_eq!(
            <Option<PageWork>>::from_json("p", &json!({"fingerprint":"f","derived":{}})),
            None
        );
    }
    #[test]
    fn held_elsewhere_leaves_out_the_page_being_changed() {
        let held = |page: &str| PageWork {
            derived: json!({}),
            held: vec![Held {
                agent: "ada".into(),
                page: page.into(),
                task: "T-1".into(),
            }],
            lines: vec![],
        };
        let found = BTreeMap::from([
            ("a".to_owned(), Some(held("a"))),
            ("b".to_owned(), Some(held("b"))),
            ("c".to_owned(), None),
        ]);
        let tasks: Vec<_> = held_elsewhere(&found, "a")
            .into_iter()
            .map(|h| h.page)
            .collect();
        assert_eq!(tasks, ["b"]);
    }
}
