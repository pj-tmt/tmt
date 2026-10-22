//! What every page's work says, read once for the whole space: the owner list's stage, progress and
//! assignees, and the tasks each agent holds for the work-in-progress limit (`wip.rs`). Reading a
//! page's work folds the page, so the answers are kept in one small derived file keyed by the page's
//! own progress (`Store::page_progress`: the epoch and the content counts, not the membership head,
//! so creating or sharing another page does not refold every page): a page whose key is unchanged is
//! not folded again. The file holds nothing the pages do not; deleting it, or any entry going
//! stale, only costs a fold. It is never signed or synced. A revocation alone can leave an entry
//! stale until the page's next write: the list and the limit are display and a working agreement,
//! never authority.
use crate::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    page,
    store::Store,
    wip::Held,
    work::Line,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write};

const FILE: &str = "work-index.json";
const LOCK: &str = "work-index.lock";
/// A larger file is not ours; it reads as empty.
const LIMIT: u64 = 1024 * 1024;
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

fn entry_json(work: &Option<PageWork>, fingerprint: &str) -> Value {
    let mut entry = json!({"fingerprint": fingerprint});
    if let Some(work) = work {
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
fn entry_from(page: &str, entry: &Value) -> Option<(String, Option<PageWork>)> {
    let fingerprint = entry["fingerprint"].as_str()?.to_owned();
    if entry.get("derived").is_none() {
        return Some((fingerprint, None));
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
    Some((
        fingerprint,
        Some(PageWork {
            derived: entry["derived"].clone(),
            held,
            lines,
        }),
    ))
}

fn load(layout: &Layout) -> BTreeMap<String, (String, Option<PageWork>)> {
    let Ok(bytes) = layout.read(FILE, LIMIT) else {
        return BTreeMap::new();
    };
    let Ok(file) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    if file["version"] != VERSION {
        return BTreeMap::new();
    }
    file["pages"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(page, entry)| Some((page.clone(), entry_from(page, entry)?)))
        .collect()
}
fn save(layout: &Layout, pages: &BTreeMap<String, (String, Option<PageWork>)>) -> Result<()> {
    let table: serde_json::Map<String, Value> = pages
        .iter()
        .map(|(page, (fingerprint, work))| (page.clone(), entry_json(work, fingerprint)))
        .collect();
    let bytes = json!({"version": VERSION, "pages": table}).to_string();
    if bytes.len() as u64 > LIMIT {
        return Ok(());
    }
    let mut file = layout.file(FILE)?;
    file.set_len(0)?;
    std::io::Seek::rewind(&mut file)?;
    file.write_all(bytes.as_bytes())?;
    Ok(())
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
    let fingerprints =
        store.owner_read(&key.space_id, &key.owner_public(), |tx| tx.page_progress())?;
    let _lock = crate::settings::locked(layout, LOCK)?;
    let known = load(layout);
    let mut kept = BTreeMap::new();
    let mut found = BTreeMap::new();
    let mut changed = known.len() != pages.len();
    for id in pages {
        let Some(fingerprint) = fingerprints.get(id) else {
            continue;
        };
        let work = match known.get(id) {
            Some((was, work)) if was == fingerprint => work.clone(),
            _ => {
                changed = true;
                match fold(store, key, id, decoder) {
                    Some(work) => work,
                    // Unreadable now: say nothing and ask again next time.
                    None => continue,
                }
            }
        };
        kept.insert(id.clone(), (fingerprint.clone(), work.clone()));
        found.insert(id.clone(), work);
    }
    if changed {
        save(layout, &kept)?;
    }
    Ok(found)
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
        let json = entry_json(&work, "f1");
        assert_eq!(entry_from("p", &json), Some(("f1".into(), work)));
        assert_eq!(
            entry_from("p", &entry_json(&None, "f2")),
            Some(("f2".into(), None))
        );
        assert_eq!(
            entry_from("p", &json!({"fingerprint":"f","derived":{}})),
            None
        );
        assert_eq!(entry_from("p", &json!({"derived":{},"held":[]})), None);
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
