//! Root-local CLI views. Read snapshots verify owner policy; only the isolated
//! decoder materializes titles. No route, mutation or authority signing lives here.
use crate::{
    Result,
    decoder::{Decoder, layout_of_meta},
    fold,
    keyring::Keyring,
    page_expiry::SpaceExpiry,
    store::{Store, owner::OwnerFault},
};
use serde_json::{Value, json};
use std::path::PathBuf;
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
