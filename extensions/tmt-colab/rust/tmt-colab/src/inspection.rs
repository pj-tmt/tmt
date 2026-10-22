//! Root-local CLI views. Read snapshots verify owner policy; only the isolated
//! decoder materializes title-cache misses and detail. List misses may fill the rebuildable
//! title projection; no route, source mutation or authority signing lives here.
use crate::{
    Result,
    decoder::{Decoder, layout_of_meta},
    fold,
    keyring::{Keyring, Layout},
    page_expiry::SpaceExpiry,
    store::{Store, owner::OwnerFault},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};
use tmt_colab_model::values;

/// The local catalog for a caller that needs the pages, their policy and the membership head. Its
/// expiry fields assume the default page expiry; [`catalog_for`] applies the space's setting.
pub fn catalog(store: &Store, key: &Keyring) -> Result<Value> {
    catalog_for(store, key, SpaceExpiry::default())
}

/// [`catalog`] with each page's archive time judged by this space's page expiry setting.
/// Every page of the space that is not deleted, archived ones included.
pub fn all_page_ids(store: &Store, key: &Keyring) -> Result<Vec<String>> {
    page_ids(store, key, false)
}
/// The pages that are not archived: the pages a work-in-progress count covers.
pub fn active_page_ids(store: &Store, key: &Keyring) -> Result<Vec<String>> {
    page_ids(store, key, true)
}
fn page_ids(store: &Store, key: &Keyring, active: bool) -> Result<Vec<String>> {
    let catalog = catalog(store, key)?;
    Ok(catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|page| !active || page["archived"] != true)
        .filter_map(|page| page["pageId"].as_str().map(str::to_owned))
        .collect())
}
pub fn catalog_for(store: &Store, key: &Keyring, space: SpaceExpiry) -> Result<Value> {
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let mut catalog = tx.page_list_for_cli(space)?;
        let head = tx.head().map(|h| {
            json!({"revision":h.revision.to_string(),
            "statementHash":values::encode_binary(&h.hash)})
        });
        catalog
            .as_object_mut()
            .ok_or(OwnerFault::Invalid)?
            .remove("ownerKey");
        catalog
            .as_object_mut()
            .ok_or(OwnerFault::Invalid)?
            .remove("revision");
        catalog["membershipHead"] = json!(head);
        Ok(catalog)
    })
}

pub fn detail(store: &Store, key: &Keyring, page: &Value) -> Result<Value> {
    let id = page["pageId"].as_str().ok_or(OwnerFault::Invalid)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let (states, _) = fold::verify_log(&tx.log()?, key, id)?;
        let authority = states.last().ok_or(OwnerFault::Invalid)?;
        if tx.head() != Some(&authority.head) {
            return Err(OwnerFault::Invalid.into());
        }
        let principals = |kind: &str| {
            authority
                .recipients
                .values()
                .map(|issuer| &issuer.recipient)
                .filter(|r| r.kind == kind && r.pages.iter().any(|p| p == id))
                .map(|r| json!({"id":r.id,"role":r.role,"pages":r.pages,"revoked":r.revoked}))
                .collect::<Vec<_>>()
        };
        Ok(json!({"spaceId":key.space_id,"page":page,
            "membershipHead":{"revision":authority.head.revision.to_string(),
            "statementHash":values::encode_binary(&authority.head.hash)},
            "members":principals("member"),"links":principals("link"),
            "discussions":"not-available"}))
    })
}

pub fn title(store: &Store, key: &Keyring, page: &mut Value, program: PathBuf) -> Result<()> {
    if page["archived"] == true {
        page["title"] = Value::Null;
        page["layout"] = Value::Null;
        page["warnings"]
            .as_array_mut()
            .ok_or(OwnerFault::Invalid)?
            .push(json!("title-unavailable"));
    } else {
        let id = page["pageId"].as_str().ok_or(OwnerFault::Invalid)?;
        let snapshot = fold::Snapshot::capture(store, key, id)?;
        let mut decoder = Decoder::new(program)?;
        let view = snapshot.materialize(key, id, &mut decoder)?;
        page["title"] = json!(view.title);
        // A page that was never given a layout is freeform: an absence in its metadata, named here.
        match layout_of_meta(&view.meta) {
            Some(layout) => {
                page["layout"] = json!(layout.kind.as_str());
                if let Some(reason) = layout.reason {
                    page["layoutReason"] = json!(reason);
                }
            }
            None => page["layout"] = json!("freeform"),
        }
    }
    Ok(())
}

/// One list's title reads, separate from the full detail inspection.
pub struct ListTitles<'a> {
    store: &'a Store,
    key: &'a Keyring,
    titles: BTreeMap<String, String>,
    fingerprints: BTreeMap<String, String>,
    writer: crate::page_titles::TitleWriter,
}
impl<'a> ListTitles<'a> {
    pub fn new(layout: &Layout, store: &'a Store, key: &'a Keyring) -> Result<Self> {
        let (titles, fingerprints) =
            store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                Ok((tx.page_titles()?, tx.page_fingerprints()?))
            })?;
        Ok(Self {
            store,
            key,
            titles,
            fingerprints,
            writer: crate::page_titles::TitleWriter::new(layout),
        })
    }
    pub fn attach(&mut self, page: &mut Value, program: PathBuf) -> Result<()> {
        let (store, key) = (self.store, self.key);
        self.attach_with(page, |page| title(store, key, page, program))
    }
    fn attach_with(
        &mut self,
        page: &mut Value,
        fold: impl FnOnce(&mut Value) -> Result<()>,
    ) -> Result<()> {
        let id = page["pageId"]
            .as_str()
            .ok_or(OwnerFault::Invalid)?
            .to_owned();
        if page["archived"] == true {
            page["title"] = Value::Null;
            page["warnings"]
                .as_array_mut()
                .ok_or(OwnerFault::Invalid)?
                .push(json!("title-unavailable"));
            return Ok(());
        }
        if let Some(title) = self.titles.get(&id) {
            page["title"] = json!(title);
            return Ok(());
        }
        let result = fold(page);
        // Layout and content health belong to full detail inspection, never list rows.
        if let Some(fields) = page.as_object_mut() {
            fields.remove("layout");
            fields.remove("layoutReason");
        }
        result?;
        if let Some(display) = page["title"]
            .as_str()
            .and_then(crate::store::owner::display_title)
            .filter(|title| title.len() <= crate::limits::PAGE_TITLE_BYTES)
        {
            page["title"] = json!(display);
        }
        if let Some(expected) = self.fingerprints.get(&id) {
            self.writer
                .record(self.key, &id, expected, page["title"].as_str());
        }
        Ok(())
    }
}

#[cfg(test)]
mod list_title_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        root: PathBuf,
        layout: Layout,
        store: Store,
        key: Keyring,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "tmt-4439-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let layout = Layout::open(&root).unwrap();
            let key = Keyring::open(&layout).unwrap();
            let store = Store::open(&layout).unwrap();
            Self {
                root,
                layout,
                store,
                key,
            }
        }
        fn page(&mut self, n: usize, cached: bool) -> Value {
            let id = format!("10000000-0000-4000-8000-{n:012}");
            self.store.create_page(&id).unwrap();
            if cached {
                self.store
                    .project_transaction(&self.key.space_id, &self.key.owner_public(), |tx| {
                        let fingerprint = tx.page_fingerprint(&id)?.unwrap();
                        tx.record_page_title(&id, &fingerprint, &format!("Page {n}"))?;
                        Ok(())
                    })
                    .unwrap();
            }
            json!({"pageId": id, "archived": false, "warnings": []})
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn a_warm_list_performs_no_per_page_folds() {
        let mut fixture = Fixture::new();
        let mut pages: Vec<_> = (1..=200).map(|n| fixture.page(n, true)).collect();
        let mut titles = ListTitles::new(&fixture.layout, &fixture.store, &fixture.key).unwrap();
        let mut folds = 0;
        for (n, page) in pages.iter_mut().enumerate() {
            titles
                .attach_with(page, |page| {
                    folds += 1;
                    page["title"] = json!(format!("Page {}", n + 1));
                    Ok(())
                })
                .unwrap();
            assert_eq!(page["title"], format!("Page {}", n + 1));
        }
        assert_eq!(folds, 0, "a metadata list folded its warm pages");
    }

    #[test]
    fn only_a_missing_title_is_folded_and_the_next_list_is_warm() {
        let mut fixture = Fixture::new();
        let mut pages = [fixture.page(1, true), fixture.page(2, false)];
        let mut folds = 0;
        for expected in [1, 0] {
            let before = folds;
            let mut titles =
                ListTitles::new(&fixture.layout, &fixture.store, &fixture.key).unwrap();
            for page in &mut pages {
                titles
                    .attach_with(page, |page| {
                        folds += 1;
                        page["title"] = json!("  New\n title\u{202e} ");
                        Ok(())
                    })
                    .unwrap();
            }
            assert_eq!(pages[1]["title"], "New title");
            assert_eq!(
                folds - before,
                expected,
                "folds must equal title-cache misses"
            );
        }
    }
    #[test]
    fn a_changed_page_folds_once_but_an_archived_page_never_folds() {
        let mut fixture = Fixture::new();
        let mut changed = fixture.page(1, true);
        let mut archived = fixture.page(2, true);
        archived["archived"] = json!(true);
        fixture
            .store
            .advance_epoch(changed["pageId"].as_str().unwrap(), 1)
            .unwrap();
        let mut folds = 0;
        for expected in [1, 0] {
            let before = folds;
            let mut titles =
                ListTitles::new(&fixture.layout, &fixture.store, &fixture.key).unwrap();
            titles
                .attach_with(&mut changed, |page| {
                    folds += 1;
                    page["title"] = json!("Changed title");
                    page["layout"] = json!("canvas");
                    page["layoutReason"] = json!("A map");
                    Ok(())
                })
                .unwrap();
            titles
                .attach_with(&mut archived, |_| panic!("archived page folded"))
                .unwrap();
            assert_eq!(folds - before, expected);
            assert_eq!(changed["title"], "Changed title");
            assert!(changed.get("layout").is_none());
            assert!(changed.get("layoutReason").is_none());
            assert!(archived["title"].is_null());
            assert!(
                archived["warnings"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("title-unavailable"))
            );
        }
    }

    #[test]
    fn a_failed_fold_is_not_cached_and_does_not_hide_a_warm_neighbor() {
        let mut fixture = Fixture::new();
        let mut broken = fixture.page(1, false);
        let mut warm = fixture.page(2, true);
        for _ in 0..2 {
            let mut titles =
                ListTitles::new(&fixture.layout, &fixture.store, &fixture.key).unwrap();
            assert!(
                titles
                    .attach_with(&mut broken, |_| Err(OwnerFault::Invalid.into()))
                    .is_err()
            );
            titles
                .attach_with(&mut warm, |_| panic!("warm neighbor folded"))
                .unwrap();
            assert_eq!(warm["title"], "Page 2");
        }
    }
}
