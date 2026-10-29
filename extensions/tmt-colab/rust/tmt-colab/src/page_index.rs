//! One small derived file per question that every page can answer but only a fold can read (the
//! work index, the template index): the answers are kept keyed by each page's own progress
//! (`Store::page_progress`: the epoch and the content counts, not the membership head, so creating
//! or sharing another page does not refold every page), and a page whose key is unchanged is not
//! folded again. A file holds nothing the pages do not; deleting it, or any entry going stale, only
//! costs a fold on a scan. It is never signed or synced. A revocation alone can leave an entry stale
//! until the page's next write: the answers are display and working agreements, never authority.
use crate::{
    Result,
    keyring::{Keyring, Layout},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

/// A larger file is not ours; it reads as empty.
const LIMIT: u64 = 1024 * 1024;

/// What one index keeps for a page, and the file that keeps it.
pub(crate) trait Kept: Clone {
    const FILE: &'static str;
    const LOCK: &'static str;
    /// A file of another version reads as empty.
    const VERSION: u64;
    /// The entry's own fields; the page's key is added beside them.
    fn to_json(&self) -> Value;
    /// `None` for an entry that does not read: it is dropped and asked again.
    fn from_json(page: &str, entry: &Value) -> Option<Self>;
}

fn entry_from<E: Kept>(page: &str, entry: &Value) -> Option<(String, E)> {
    Some((
        entry["fingerprint"].as_str()?.to_owned(),
        E::from_json(page, entry)?,
    ))
}

pub(crate) fn entry_json<E: Kept>(kept: &E, fingerprint: &str) -> Value {
    let mut entry = kept.to_json();
    entry["fingerprint"] = json!(fingerprint);
    entry
}

pub(crate) fn load<E: Kept>(layout: &Layout) -> BTreeMap<String, (String, E)> {
    let Ok(bytes) = layout.read(E::FILE, LIMIT) else {
        return BTreeMap::new();
    };
    let Ok(file) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    if file["version"] != E::VERSION {
        return BTreeMap::new();
    }
    file["pages"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(page, entry)| Some((page.clone(), entry_from(page, entry)?)))
        .collect()
}

pub(crate) fn save<E: Kept>(layout: &Layout, pages: &BTreeMap<String, (String, E)>) -> Result<()> {
    let table: serde_json::Map<String, Value> = pages
        .iter()
        .map(|(page, (fingerprint, kept))| (page.clone(), entry_json(kept, fingerprint)))
        .collect();
    let bytes = json!({"version": E::VERSION, "pages": table}).to_string();
    if bytes.len() as u64 > LIMIT {
        return Ok(());
    }
    let mut file = layout.file(E::FILE)?;
    file.set_len(0)?;
    std::io::Seek::rewind(&mut file)?;
    file.write_all(bytes.as_bytes())?;
    Ok(())
}

/// Current kept answers only. Missing, stale or unreadable entries are omitted without a fold, lock
/// or write; callers needing freshly derived answers use [`scan`].
pub(crate) fn cached<E: Kept>(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    pages: &[String],
) -> Result<BTreeMap<String, E>> {
    let fingerprints =
        store.owner_read(&key.space_id, &key.owner_public(), |tx| tx.page_progress())?;
    let known = load::<E>(layout);
    Ok(pages
        .iter()
        .filter_map(|id| {
            let (was, kept) = known.get(id)?;
            (fingerprints.get(id) == Some(was)).then(|| (id.clone(), kept.clone()))
        })
        .collect())
}

/// Answer only the requested pages. Retain current entries for verified live neighbors; if the
/// catalog could not be read, leave the file untouched and still return requested answers. When
/// the 1 MiB limit would be exceeded, evict largest unrequested entries first, breaking ties by
/// page ID. If the requested entries alone exceed the limit, the existing bounded save is skipped.
pub(crate) fn scan<E: Kept>(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    pages: &[String],
    live: Option<&[String]>,
    mut fold: impl FnMut(&str) -> Option<E>,
) -> Result<BTreeMap<String, E>> {
    let fingerprints =
        store.owner_read(&key.space_id, &key.owner_public(), |tx| tx.page_progress())?;
    let _lock = crate::settings::locked(layout, E::LOCK)?;
    let known = load::<E>(layout);
    let mut kept = known.clone();
    let mut changed = false;
    let live_ids = live.map(|ids| ids.iter().collect::<BTreeSet<_>>());
    if let Some(live) = &live_ids {
        kept.retain(|id, (fingerprint, _)| {
            live.contains(id) && fingerprints.get(id) == Some(fingerprint)
        });
        changed = kept.len() != known.len();
    }
    let mut found = BTreeMap::new();
    for id in pages {
        let Some(fingerprint) = fingerprints.get(id) else {
            continue;
        };
        let answer = match known.get(id) {
            Some((was, answer)) if was == fingerprint => answer.clone(),
            _ => {
                changed = true;
                match fold(id) {
                    Some(answer) => answer,
                    None => {
                        kept.remove(id);
                        continue;
                    }
                }
            }
        };
        if live_ids.as_ref().is_none_or(|live| live.contains(id)) {
            if !kept.contains_key(id) {
                changed = true;
            }
            kept.insert(id.clone(), (fingerprint.clone(), answer.clone()));
        }
        found.insert(id.clone(), answer);
    }
    if changed && live.is_some() {
        let requested: BTreeSet<_> = pages.iter().collect();
        let mut neighbors: Vec<_> = kept
            .iter()
            .filter(|(id, _)| !requested.contains(id))
            .map(|(id, (fingerprint, value))| {
                let bytes = serde_json::to_vec(&entry_json(value, fingerprint))
                    .expect("serializable entry")
                    .len()
                    + serde_json::to_vec(id).expect("serializable page ID").len()
                    + 2;
                (id.clone(), bytes)
            })
            .collect();
        neighbors.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut size = json!({"version": E::VERSION, "pages": kept.iter().map(|(id, (fingerprint, value))| (id.clone(), entry_json(value, fingerprint))).collect::<serde_json::Map<String, Value>>()}).to_string().len();
        for (id, bytes) in neighbors {
            if size <= LIMIT as usize {
                break;
            }
            kept.remove(&id);
            size = size.saturating_sub(bytes - usize::from(kept.is_empty()));
        }
        save(layout, &kept)?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Entry(String);
    impl Kept for Entry {
        const FILE: &'static str = "work-index.json";
        const LOCK: &'static str = "work-index.lock";
        const VERSION: u64 = 1;
        fn to_json(&self) -> Value {
            json!({"value": self.0})
        }
        fn from_json(_: &str, entry: &Value) -> Option<Self> {
            Some(Self(entry["value"].as_str()?.to_owned()))
        }
    }
    fn fixture(name: &str) -> (std::path::PathBuf, Layout, Keyring, Store, Vec<String>) {
        let root = std::env::temp_dir().join(format!("tmt-index-{name}-{}", std::process::id()));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        let pages: Vec<_> = (1..=3)
            .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
            .collect();
        for id in &pages {
            store.create_page(id).unwrap();
        }
        (root, layout, key, store, pages)
    }
    #[test]
    fn alternating_sets_reuse_unchanged_neighbors_and_return_only_requested() {
        let (root, layout, key, store, pages) = fixture("alternating");
        let mut folds = BTreeMap::<String, usize>::new();
        let mut run = |ids: &[String]| {
            scan(&layout, &store, &key, ids, Some(&pages), |id| {
                *folds.entry(id.to_owned()).or_default() += 1;
                Some(Entry(id.to_owned()))
            })
            .unwrap()
        };
        assert_eq!(run(&pages).len(), 3);
        let before = std::fs::read(layout.directory.join(Entry::FILE)).unwrap();
        assert_eq!(run(&pages[..1]).len(), 1);
        assert_eq!(
            std::fs::read(layout.directory.join(Entry::FILE)).unwrap(),
            before
        );
        assert_eq!(run(&pages).len(), 3);
        assert!(folds.values().all(|n| *n == 1));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn requested_entries_displace_largest_warm_neighbors_when_cache_is_full() {
        let (root, layout, key, store, pages) = fixture("capacity");
        let warm = scan(&layout, &store, &key, &pages[..2], Some(&pages), |id| {
            Some(Entry("x".repeat(if id == pages[0] {
                700_000
            } else {
                200_000
            })))
        })
        .unwrap();
        assert_eq!(warm.len(), 2);
        let requested = scan(&layout, &store, &key, &pages[2..], Some(&pages), |_| {
            Some(Entry("y".repeat(200_000)))
        })
        .unwrap();
        assert_eq!(requested.len(), 1);
        let kept = load::<Entry>(&layout);
        assert!(kept.contains_key(&pages[2]));
        assert!(kept.contains_key(&pages[1]));
        assert!(!kept.contains_key(&pages[0]));
        assert!(
            std::fs::metadata(layout.directory.join(Entry::FILE))
                .unwrap()
                .len()
                <= LIMIT
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn oversized_requested_results_are_returned_without_replacing_the_cache() {
        let (root, layout, key, store, pages) = fixture("oversized");
        scan(&layout, &store, &key, &pages[..1], Some(&pages), |_| {
            Some(Entry("warm".into()))
        })
        .unwrap();
        let before = std::fs::read(layout.directory.join(Entry::FILE)).unwrap();
        let result = scan(&layout, &store, &key, &pages[1..], Some(&pages), |_| {
            Some(Entry("x".repeat(LIMIT as usize)))
        })
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            std::fs::read(layout.directory.join(Entry::FILE)).unwrap(),
            before
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_deleted_and_failed_folds_do_not_return_or_poison_neighbors() {
        let (root, layout, key, mut store, pages) = fixture("lifecycle");
        scan(&layout, &store, &key, &pages, Some(&pages), |id| {
            Some(Entry(id.into()))
        })
        .unwrap();
        store.advance_epoch(&pages[0], 1).unwrap();
        let live = pages[..2].to_vec(); // The verified catalog excludes the third, deleted page.
        let failed =
            scan::<Entry>(&layout, &store, &key, &pages[..1], Some(&live), |_| None).unwrap();
        assert!(failed.is_empty());
        let kept = load::<Entry>(&layout);
        assert!(!kept.contains_key(&pages[0]));
        assert!(kept.contains_key(&pages[1]));
        assert!(!kept.contains_key(&pages[2]));
        let recovered = scan(&layout, &store, &key, &pages[..1], Some(&live), |_| {
            Some(Entry("new".into()))
        })
        .unwrap();
        assert_eq!(recovered[&pages[0]], Entry("new".into()));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_catalog_returns_requested_folds_without_touching_cache() {
        let (root, layout, key, mut store, pages) = fixture("unavailable");
        scan(&layout, &store, &key, &pages, Some(&pages), |id| {
            Some(Entry(id.into()))
        })
        .unwrap();
        let before = std::fs::read(layout.directory.join(Entry::FILE)).unwrap();
        store.advance_epoch(&pages[0], 1).unwrap();
        let result = scan(&layout, &store, &key, &pages[..1], None, |_| {
            Some(Entry("fresh".into()))
        })
        .unwrap();
        assert_eq!(result[&pages[0]], Entry("fresh".into()));
        assert_eq!(
            std::fs::read(layout.directory.join(Entry::FILE)).unwrap(),
            before
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_marks_stay_scoped_to_requested_pages() {
        use crate::template_index::Marks;
        let (root, layout, key, store, pages) = fixture("template");
        let mut folds = 0;
        let first = scan(&layout, &store, &key, &pages, Some(&pages), |_| {
            folds += 1;
            Some(Marks::default())
        })
        .unwrap();
        assert_eq!(first.len(), 3);
        let subset = scan(&layout, &store, &key, &pages[..1], Some(&pages), |_| {
            folds += 1;
            Some(Marks::default())
        })
        .unwrap();
        assert_eq!(subset.len(), 1);
        let again = scan(&layout, &store, &key, &pages, Some(&pages), |_| {
            folds += 1;
            Some(Marks::default())
        })
        .unwrap();
        assert_eq!(again, first);
        assert_eq!(folds, 3);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
