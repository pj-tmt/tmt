//! Root-local CLI views. Read snapshots verify owner policy; only the isolated
//! decoder materializes titles. No route, mutation or authority signing lives here.
use crate::{
    Result,
    decoder::Decoder,
    fold,
    keyring::Keyring,
    store::{Store, owner::OwnerFault},
};
use serde_json::{Value, json};
use std::path::PathBuf;
use tmt_colab_model::{payload::Payload, values};

pub fn catalog(store: &Store, key: &Keyring) -> Result<Value> {
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let mut catalog = tx.page_list()?;
        let head = tx.head().map(|h| {
            json!({"revision":h.revision.to_string(),
            "statementHash":values::encode_binary(&h.hash)})
        });
        let mut retained = std::collections::BTreeMap::new();
        let mut verified_head = None;
        for envelope in tx.log()? {
            let verified =
                envelope.verify_next(&key.space_id, &key.owner_public(), verified_head.as_ref())?;
            verified_head = Some(verified.head);
            if let Payload::RetentionSet(p) = verified.payload {
                retained.insert(
                    p.page_id,
                    match p.days {
                        tmt_colab_model::payload::Days::Forever => None,
                        tmt_colab_model::payload::Days::Count(n) => Some(n),
                    },
                );
            }
        }
        if verified_head.as_ref() != tx.head() {
            return Err(OwnerFault::Invalid.into());
        }
        for page in catalog["pages"].as_array_mut().ok_or(OwnerFault::Invalid)? {
            let id = page["pageId"].as_str().ok_or(OwnerFault::Invalid)?;
            let days = retained.get(id).copied().unwrap_or(Some(30));
            page["retentionDays"] = json!(days);
            page["lastUpdateAtMs"] = Value::Null;
            page["expiresAtMs"] = Value::Null;
            page["warnings"] = json!(["expiry-unavailable"]);
        }
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
        page["warnings"]
            .as_array_mut()
            .ok_or(OwnerFault::Invalid)?
            .push(json!("title-unavailable"));
    } else {
        let id = page["pageId"].as_str().ok_or(OwnerFault::Invalid)?;
        let snapshot = fold::Snapshot::capture(store, key, id)?;
        let mut decoder = Decoder::new(program)?;
        page["title"] = json!(snapshot.materialize(key, id, &mut decoder)?.title);
    }
    Ok(())
}
