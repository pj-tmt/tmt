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
use std::{collections::BTreeMap, io::Write};

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

/// The answer for each given page, folding (through `fold`) only the pages whose key changed since
/// the last scan, and keeping the file to the given pages. `fold` returns `None` for a page that
/// cannot be read now: it is left out and asked again next time.
pub(crate) fn scan<E: Kept>(
    layout: &Layout,
    store: &Store,
    key: &Keyring,
    pages: &[String],
    mut fold: impl FnMut(&str) -> Option<E>,
) -> Result<BTreeMap<String, E>> {
    let fingerprints =
        store.owner_read(&key.space_id, &key.owner_public(), |tx| tx.page_progress())?;
    let _lock = crate::settings::locked(layout, E::LOCK)?;
    let known = load::<E>(layout);
    let mut kept = BTreeMap::new();
    let mut found = BTreeMap::new();
    let mut changed = known.len() != pages.len();
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
                    None => continue,
                }
            }
        };
        kept.insert(id.clone(), (fingerprint.clone(), answer.clone()));
        found.insert(id.clone(), answer);
    }
    if changed {
        save(layout, &kept)?;
    }
    Ok(found)
}
