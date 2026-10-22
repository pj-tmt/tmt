//! The owner's local plaintext page titles (#4218): a display-only projection a summary worker
//! writes from the verified fold it already runs, so Home, cards and search need no decrypt step.
//! It lives in the owner store (`page_titles`), is never a log statement and is never synced,
//! published, exported or shared; deleting or archiving a page removes its row, and losing the rows
//! loses nothing because the next fold writes them again (contract: Schema 16).
use crate::{
    keyring::{Keyring, Layout},
    store::Store,
};
use std::collections::BTreeMap;

/// Adds each page's local title to the owner catalog's page entries, most recently updated page
/// first (then by page ID), while the whole serialized reply stays within
/// [`crate::limits::DISCOVERY_BYTES`]. A title that no longer fits is left out, never the catalog:
/// the browser then keeps its own hint, as for a page with no title yet. Archived pages and pages
/// without a title are skipped.
pub(crate) fn attach_to_catalog(
    catalog: &mut serde_json::Value,
    titles: &BTreeMap<String, String>,
) {
    let Ok(encoded) = serde_json::to_vec(catalog) else {
        return;
    };
    let mut used = encoded.len();
    let Some(pages) = catalog["pages"].as_array_mut() else {
        return;
    };
    let mut order: Vec<(u64, usize)> = pages
        .iter()
        .enumerate()
        .filter(|(_, page)| {
            page["archived"] != true
                && page["pageId"]
                    .as_str()
                    .is_some_and(|id| titles.contains_key(id))
        })
        .map(|(index, page)| (page["lastUpdateAtMs"].as_u64().unwrap_or(0), index))
        .collect();
    // The page array is sorted by ID, so the index breaks a tie in ID order.
    order.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, index) in order {
        let Some(title) = pages[index]["pageId"]
            .as_str()
            .and_then(|id| titles.get(id))
        else {
            continue;
        };
        let Ok(text) = serde_json::to_string(title) else {
            continue;
        };
        // `,"title":` and the quoted text.
        let cost = 9 + text.len();
        if used + cost > crate::limits::DISCOVERY_BYTES {
            continue;
        }
        used += cost;
        let title = title.clone();
        pages[index]["title"] = serde_json::Value::String(title);
    }
}

/// One worker's best-effort writer: its own writable store handle, opened on first use. A write that
/// cannot happen (no handle, a busy or stale store) leaves the page without a row, never fails the
/// summary, and is tried again at the page's next fold. The handle never creates or migrates the
/// store: only the serve's own open does, and a worker that raced it would upgrade the store behind
/// the caller that expects it to be behind (#4300).
pub(crate) struct TitleWriter {
    layout: Layout,
    store: Option<Store>,
}

impl TitleWriter {
    pub(crate) fn new(layout: &Layout) -> Self {
        Self {
            layout: layout.clone(),
            store: None,
        }
    }

    /// Records `title` for `page` when it is still at `expected`, the fingerprint its fold started
    /// from. `None` is a page that is not active (archived or deleted): it keeps no title.
    pub(crate) fn record(
        &mut self,
        keyring: &Keyring,
        page: &str,
        expected: &str,
        title: Option<&str>,
    ) {
        if self.store.is_none() {
            self.store = Store::write_existing(&self.layout).ok();
        }
        let Some(store) = self.store.as_mut() else {
            return;
        };
        // Best effort by design (see the type); the error has nowhere useful to go.
        let _ =
            store.project_transaction(
                &keyring.space_id,
                &keyring.owner_public(),
                |tx| match title {
                    Some(title) => tx.record_page_title(page, expected, title).map(|_| ()),
                    None if tx.page_fingerprint(page)?.as_deref() == Some(expected) => {
                        tx.drop_page_title(page)
                    }
                    None => Ok(()),
                },
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(n: usize) -> String {
        format!("10000000-0000-4000-8000-{n:012}")
    }

    /// A catalog of `pages` entries (page `n` updated at `updated(n)`) padded to leave `spare` bytes
    /// of the reply budget.
    fn catalog(pages: usize, updated: impl Fn(usize) -> u64, spare: usize) -> serde_json::Value {
        let entries: Vec<_> = (1..=pages)
            .map(|n| json!({"pageId": id(n), "archived": false, "lastUpdateAtMs": updated(n)}))
            .collect();
        let mut catalog = json!({"pages": entries, "padding": ""});
        let used = serde_json::to_vec(&catalog).unwrap().len();
        catalog["padding"] = "x"
            .repeat(crate::limits::DISCOVERY_BYTES - spare - used)
            .into();
        catalog
    }

    fn titled(catalog: &serde_json::Value) -> Vec<usize> {
        let mut found: Vec<usize> = catalog["pages"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, page)| page.get("title").is_some())
            .map(|(n, _)| n + 1)
            .collect();
        found.sort_unstable();
        found
    }

    #[test]
    fn the_newest_pages_get_their_titles_until_the_reply_is_full() {
        // 1,000 pages, page 1 the oldest: each title costs 9 + 2 quotes + 100 bytes.
        let mut value = catalog(1000, |n| n as u64, 111 * 40 + 50);
        let titles: BTreeMap<_, _> = (1..=1000).map(|n| (id(n), "t".repeat(100))).collect();
        attach_to_catalog(&mut value, &titles);
        assert_eq!(titled(&value), (961..=1000).collect::<Vec<_>>());
        let bytes = serde_json::to_vec(&value).unwrap().len();
        assert!(bytes <= crate::limits::DISCOVERY_BYTES);
        // What is left is less than one more title.
        assert!(crate::limits::DISCOVERY_BYTES - bytes < 111);
    }

    #[test]
    fn a_title_that_does_not_fit_is_left_out_and_a_shorter_one_after_it_still_can() {
        let mut value = catalog(3, |n| 10 - n as u64, 9 + 2 + 20);
        // Page 1 is the newest but its title is too long for what is left; page 2's fits exactly.
        let titles = BTreeMap::from([
            (id(1), "a".repeat(40)),
            (id(2), "b".repeat(20)),
            (id(3), "c".repeat(20)),
        ]);
        attach_to_catalog(&mut value, &titles);
        assert_eq!(titled(&value), [2]);
        assert_eq!(
            serde_json::to_vec(&value).unwrap().len(),
            crate::limits::DISCOVERY_BYTES
        );
    }

    #[test]
    fn a_title_is_costed_by_its_escaped_size_and_a_tie_in_time_goes_to_the_lower_id() {
        // A quote is two bytes on the wire: `""""` costs 9 + 2 + 8.
        let mut value = catalog(2, |_| 5, 9 + 2 + 8);
        let titles = BTreeMap::from([
            (id(1), "\"\"\"\"".to_owned()),
            (id(2), "\"\"\"\"".to_owned()),
        ]);
        attach_to_catalog(&mut value, &titles);
        assert_eq!(titled(&value), [1]);
        assert_eq!(value["pages"][0]["title"], "\"\"\"\"");
        assert!(serde_json::to_vec(&value).unwrap().len() <= crate::limits::DISCOVERY_BYTES);
    }

    /// The summary worker runs in the background of a live serve. If it finds the store behind
    /// this build, its title write is best effort and leaves the page without a row; it must not
    /// be the thing that upgrades the store, since only the serve's own open does that (#4300).
    #[test]
    fn the_background_writer_never_migrates_a_store_that_is_behind() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-4300-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let layout = Layout::open(&root).unwrap();
        drop(Store::open(&layout).unwrap());
        let keyring = Keyring::open(&layout).unwrap();
        let db = layout.directory.join("space.db");
        // Schema 15: before the title projection.
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("DROP TABLE page_titles; PRAGMA user_version=15")
            .unwrap();
        TitleWriter::new(&layout).record(&keyring, "page", "progress", Some("Title"));
        let version: u32 = rusqlite::Connection::open(&db)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(version, 15, "the title writer upgraded the store");
    }

    #[test]
    fn an_archived_page_and_a_page_without_a_title_get_none() {
        let mut value = catalog(3, |n| n as u64, 10_000);
        value["pages"][2]["archived"] = true.into();
        let titles = BTreeMap::from([(id(1), "One".to_owned()), (id(3), "Three".to_owned())]);
        attach_to_catalog(&mut value, &titles);
        assert_eq!(titled(&value), [1]);
        assert_eq!(value["pages"][0]["title"], "One");
    }
}
