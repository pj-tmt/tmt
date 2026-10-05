use super::*;
use crate::state::{Layout, Serving};
use crate::store::{DEFAULT_SCOPES, uuid_v4};
use std::path::PathBuf;
struct Fixture {
    store: Store,
    grant: Grant,
    root: PathBuf,
    serving: Option<Serving>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("t1769-{}", uuid_v4().unwrap()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        store.machine().unwrap();
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: [7; 32],
            kind: "browser".into(),
            origin: "http://127.0.0.1:32100".into(),
            name: "Owner".into(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
            mode: "hold".into(),
            issued_at_ms: now_ms().unwrap(),
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        Self {
            store,
            grant,
            root,
            serving: Some(serving),
        }
    }
    fn designate(&mut self) {
        self.store
            .designate(&self.grant.client_id, &self.grant.origin, now_ms().unwrap())
            .unwrap();
    }
    fn adopt(&mut self, id: &str, op: &str, digest: &[u8]) -> Result<Option<Value>> {
        self.store
            .management_adopt(&self.grant, id, op, digest, now_ms().unwrap())
    }
    fn count(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Connection is closed before fixture deletion (replacement owns no files).
        self.store.connection = Connection::open_in_memory().unwrap();
        self.serving.take();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn cap_decimal_preserves_usize_and_rejects_rounding_or_noncanonical_values() {
    assert_eq!(
        cap(&json!(usize::MAX.to_string())).unwrap(),
        Some(usize::MAX)
    );
    assert_eq!(cap(&Value::Null).unwrap(), None);
    for value in [
        json!(8),
        json!("0"),
        json!("08"),
        json!("+8"),
        json!(" 8"),
        json!((usize::MAX as u128 + 1).to_string()),
    ] {
        assert!(cap(&value).is_err(), "{value}");
    }
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.settings.set", &[1; 32]).unwrap();
    let (outcome, _) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.settings.set",
            &[1; 32],
            Mutation::Setting {
                key: "sessionsPerDevice",
                value: json!(usize::MAX),
            },
        )
        .unwrap();
    assert_eq!(
        outcome["result"]["settings"]["sessionsPerDevice"],
        usize::MAX.to_string()
    );
    assert_eq!(
        settings::read(&f.root).unwrap().sessions_per_device(),
        Some(usize::MAX)
    );
    assert_eq!(outcome["sessionEnded"], false);
}
#[test]
fn no_designation_and_removal_refuse_before_settings_or_management_adoption() {
    let mut f = Fixture::new();
    let id = uuid_v4().unwrap();
    assert_eq!(
        f.adopt(&id, "remote.settings.set", &[1; 32])
            .unwrap_err()
            .code,
        READ_ONLY
    );
    assert_eq!(f.count("management_receipts"), 0);
    assert!(!f.root.join("remote/settings.json").exists());
    f.designate();
    f.adopt(&id, "remote.settings.set", &[1; 32]).unwrap();
    f.store.undesignate().unwrap();
    assert_eq!(
        f.store
            .management_effect(
                &f.grant,
                &id,
                "remote.settings.set",
                &[1; 32],
                Mutation::Setting {
                    key: "open",
                    value: json!(false)
                }
            )
            .unwrap_err()
            .code,
        READ_ONLY
    );
    assert!(!f.root.join("remote/settings.json").exists());
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap()["state"],
        "unknown"
    );
    assert_eq!(f.count("operations"), 0); // Agent hold mode does not own this work.
}
#[test]
fn pending_intent_never_reapplies_and_immutable_deadline_is_not_renewed() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    let now = now_ms().unwrap();
    assert!(
        f.store
            .management_adopt(&f.grant, &id, "remote.settings.set", &[1; 32], now)
            .unwrap()
            .is_none()
    );
    let original: (i64, i64) = f
        .store
        .connection
        .query_row(
            "SELECT adopted_ms,deadline_ms FROM management_receipts WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        f.adopt(&id, "remote.settings.set", &[1; 32])
            .unwrap()
            .unwrap()["state"],
        "unknown"
    );
    assert_eq!(
        f.adopt(&id, "remote.settings.set", &[2; 32])
            .unwrap_err()
            .code,
        "REMOTE_INTENT_CONFLICT"
    );
    let later: (i64, i64) = f
        .store
        .connection
        .query_row(
            "SELECT adopted_ms,deadline_ms FROM management_receipts WHERE id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(original, later);
    assert_eq!(
        f.store
            .management_adopt(
                &f.grant,
                &id,
                "remote.settings.set",
                &[1; 32],
                now + journal::RECOVERY
            )
            .unwrap_err()
            .code,
        "REMOTE_MANAGEMENT_UNAVAILABLE"
    );
    assert!(!f.root.join("remote/settings.json").exists());
}
#[test]
fn device_receipt_failure_rolls_back_the_grant_and_designation_together() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.revoke", &[1; 32]).unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER deny_settlement BEFORE UPDATE OF outcome ON management_receipts BEGIN SELECT RAISE(ABORT,'test settlement failure'); END;").unwrap();
    assert!(
        f.store
            .management_effect(
                &f.grant,
                &id,
                "remote.devices.revoke",
                &[1; 32],
                Mutation::Revoke {
                    client: f.grant.client_id.clone()
                }
            )
            .is_err()
    );
    assert_eq!(f.store.grant(&f.grant.client_id).unwrap().unwrap(), f.grant);
    assert!(writable(&f.store.connection, &f.grant).unwrap());
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap()["state"],
        "unknown"
    );
}
#[test]
fn successful_self_change_keeps_receipt_and_cli_exact_repeat_semantics() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.rename", &[1; 32]).unwrap();
    let (no_op, changed) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.rename",
            &[1; 32],
            Mutation::Rename {
                client: f.grant.client_id.clone(),
                name: f.grant.name.clone(),
            },
        )
        .unwrap();
    assert_eq!(no_op["sessionEnded"], false);
    assert!(changed.is_none());
    assert_eq!(
        f.store.grant(&f.grant.client_id).unwrap().unwrap().revision,
        1
    );
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.revoke", &[2; 32]).unwrap();
    let (revoked, changed) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.revoke",
            &[2; 32],
            Mutation::Revoke {
                client: f.grant.client_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(revoked["state"], "committed");
    assert_eq!(revoked["sessionEnded"], true);
    assert!(changed.is_some());
    let target = f.store.grant(&f.grant.client_id).unwrap().unwrap();
    assert!(target.disabled);
    assert_eq!(target.revision, 2);
    assert!(!writable(&f.store.connection, &f.grant).unwrap());
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap(),
        revoked
    );
    assert_eq!(
        f.store
            .revoke(&f.grant.client_id)
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert!(f.store.authorized(&f.grant, now_ms().unwrap()).is_err()); // No revoked-key reader.
}
#[test]
fn settings_write_survives_receipt_failure_without_false_commit_or_readback_replay() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.settings.set", &[1; 32]).unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER deny_settlement BEFORE UPDATE OF outcome ON management_receipts BEGIN SELECT RAISE(ABORT,'test settlement failure'); END;").unwrap();
    assert!(
        f.store
            .management_effect(
                &f.grant,
                &id,
                "remote.settings.set",
                &[1; 32],
                Mutation::Setting {
                    key: "open",
                    value: json!(false)
                }
            )
            .is_err()
    );
    assert!(!settings::read(&f.root).unwrap().open());
    assert_eq!(
        f.adopt(&id, "remote.settings.set", &[1; 32])
            .unwrap()
            .unwrap()["state"],
        "unknown"
    );
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap()["state"],
        "unknown"
    );
}
#[test]
fn sql_page_is_caller_bound_bounded_and_readback_has_no_private_grant_fields() {
    let mut f = Fixture::new();
    for key in 8..15 {
        let mut g = f.grant.clone();
        g.client_id = uuid_v4().unwrap();
        g.public_key = [key; 32];
        f.store.insert_grant(&g).unwrap();
    }
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let (page, next) = f
            .store
            .management_page(&f.grant.client_id, PageInput { cursor, limit: 2 })
            .unwrap();
        assert!(page.len() <= 2);
        for grant in page {
            let view = summary(&grant);
            assert!(view.get("publicKey").is_none());
            assert!(view.get("scopes").is_none());
            ids.push(grant.client_id);
        }
        if next.is_none() {
            break;
        }
        cursor = next;
    }
    assert_eq!(ids.len(), 8);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    let (_, cursor) = f
        .store
        .management_page(
            &f.grant.client_id,
            PageInput {
                cursor: None,
                limit: 1,
            },
        )
        .unwrap();
    assert!(
        f.store
            .management_page(&uuid_v4().unwrap(), PageInput { cursor, limit: 1 })
            .is_err()
    );
    for limit in [0, 51, usize::MAX] {
        assert!(
            f.store
                .management_page(
                    &f.grant.client_id,
                    PageInput {
                        cursor: None,
                        limit
                    }
                )
                .is_err()
        );
    }
}
#[test]
fn designation_survives_restart_but_new_same_key_pairing_cannot_inherit_it() {
    let mut f = Fixture::new();
    f.designate();
    f.store.connection = Connection::open_in_memory().unwrap();
    f.store = Store::open(f.serving.as_ref().unwrap()).unwrap();
    assert!(writable(&f.store.connection, &f.grant).unwrap());
    f.store.revoke(&f.grant.client_id).unwrap();
    let mut new = f.grant.clone();
    new.client_id = uuid_v4().unwrap();
    f.store.insert_grant(&new).unwrap();
    assert!(!writable(&f.store.connection, &new).unwrap());
}

#[test]
fn malformed_settings_projection_cannot_claim_a_healthy_file_or_custom_effective_cap() {
    let mut f = Fixture::new();
    Layout::existing(&f.root)
        .unwrap()
        .unwrap()
        .file("settings.json")
        .unwrap();
    std::fs::write(
        f.root.join("remote/settings.json"),
        br#"{"open":false,"sessionsPerDevice":"damaged"}"#,
    )
    .unwrap();
    let loaded = settings::read_or_default(&f.root);
    let value = settings_json(&loaded);
    assert_eq!(value["open"], true);
    assert_eq!(value["source"], "default");
    assert_eq!(value["sessionsPerDevice"], "8");
    assert_eq!(value["warning"], settings::UNREADABLE);
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.settings.set", &[1; 32]).unwrap();
    let layout = Layout::existing(&f.root).unwrap().unwrap();
    let lock = layout.lock("settings.lock").unwrap();
    let (outcome, _) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.settings.set",
            &[1; 32],
            Mutation::Setting {
                key: "open",
                value: json!(true),
            },
        )
        .unwrap();
    assert_eq!(outcome["state"], "refused");
    assert_eq!(
        std::fs::read(f.root.join("remote/settings.json")).unwrap(),
        br#"{"open":false,"sessionsPerDevice":"damaged"}"#
    );
    drop(lock);
}
