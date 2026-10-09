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

#[test]
fn cumulative_identity_limits_refuse_new_adoption_but_preserve_live_original_reads() {
    for global in [false, true] {
        let mut f = Fixture::new();
        f.designate();
        let original = uuid_v4().unwrap();
        f.adopt(&original, "remote.settings.set", &[1; 32]).unwrap();
        let now = now_ms().unwrap();
        let owners = if global {
            (0..4)
                .map(|i| {
                    let mut grant = f.grant.clone();
                    grant.client_id = uuid_v4().unwrap();
                    grant.public_key = [30 + i; 32];
                    f.store.insert_grant(&grant).unwrap();
                    (grant.client_id, if i == 3 { 999 } else { 1000 })
                })
                .collect::<Vec<_>>()
        } else {
            vec![(f.grant.client_id.clone(), 999)]
        };
        let tx = f.store.connection.transaction().unwrap();
        for (owner, count) in owners {
            for _ in 0..count {
                let id = uuid_v4().unwrap();
                // Expired identities still occupy the accepted cumulative bound.
                tx.execute("INSERT INTO management_receipts VALUES (?1,?2,'remote.settings.set',?3,1,0,1,?4)",params![id,owner,[2u8;32].as_slice(),unknown(&id).to_string()]).unwrap();
            }
        }
        tx.commit().unwrap();
        let count = f.count("management_receipts");
        assert_eq!(count, if global { 4000 } else { 1000 });
        assert_eq!(
            f.adopt(&uuid_v4().unwrap(), "remote.settings.set", &[3; 32])
                .unwrap_err()
                .code,
            "REMOTE_MANAGEMENT_CAPACITY"
        );
        assert_eq!(f.count("management_receipts"), count);
        assert!(!f.root.join("remote/settings.json").exists());
        assert_eq!(f.store.grant(&f.grant.client_id).unwrap().unwrap(), f.grant);
        assert_eq!(
            receipt(&f.store.connection, &f.grant.client_id, &original, now).unwrap()["state"],
            "unknown"
        );
        assert_eq!(
            f.adopt(&original, "remote.settings.set", &[1; 32])
                .unwrap()
                .unwrap()["state"],
            "unknown"
        );
    }
}

/// The same test executable owns the interrupted Store/writer. Its parent owns
/// the root and kills/reaps it only after an actual owner milestone is published.
#[test]
fn process_crashes_preserve_original_management_identity_and_durable_boundaries() {
    use std::io::{BufRead, Read, Write};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::Duration;
    const PREFIX: &str = "TMT_MANAGEMENT_CRASH ";
    const TEST: &str = "management::tests::process_crashes_preserve_original_management_identity_and_durable_boundaries";
    fn milestone(phase: &str, id: &str, original: (i64, i64)) {
        println!(
            "{PREFIX}{}",
            json!({"phase":phase,"operationId":id,"adopted":original.0,"deadline":original.1})
        );
        std::io::stdout().flush().unwrap();
        // Parent-visible event, then an owned blocking pipe: no timing-based
        // inference and no continuation through the effect after publication.
        let mut release = [0];
        std::io::stdin().read_exact(&mut release).unwrap();
        panic!("parent must SIGKILL the milestone child, never release it");
    }
    if let Ok(phase) = std::env::var("TMT_MANAGEMENT_CRASH_PHASE") {
        let root = PathBuf::from(std::env::var("TMT_MANAGEMENT_CRASH_ROOT").unwrap());
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let client = std::env::var("TMT_MANAGEMENT_CRASH_CLIENT").unwrap();
        let grant = store.grant(&client).unwrap().unwrap();
        let id = std::env::var("TMT_MANAGEMENT_CRASH_ID").unwrap();
        let operation = if phase.contains("rename") {
            "remote.devices.rename"
        } else if phase.contains("revoke") {
            "remote.devices.revoke"
        } else {
            "remote.settings.set"
        };
        store
            .management_adopt(&grant, &id, operation, &[9; 32], now_ms().unwrap())
            .unwrap();
        let original = store
            .connection
            .query_row(
                "SELECT adopted_ms,deadline_ms FROM management_receipts WHERE id=?1",
                [&id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap();
        if phase == "adopted" {
            milestone(&phase, &id, original);
        }
        let mutation = if phase.contains("rename") {
            Mutation::Rename {
                client,
                name: "Committed new name".into(),
            }
        } else if phase.contains("revoke") {
            Mutation::Revoke { client }
        } else {
            Mutation::Setting {
                key: "open",
                value: json!(false),
            }
        };
        store
            .management_effect_observed(&grant, &id, operation, &[9; 32], mutation, |stage| {
                let reached = match stage {
                    EffectStage::BeforeTouch => phase.starts_with("first-touch"),
                    EffectStage::Truncated => phase == "truncated",
                    EffectStage::Synced => phase == "synced",
                    EffectStage::BeforeCommit => phase.starts_with("uncommitted"),
                    EffectStage::Committed => phase.starts_with("committed"),
                    _ => false,
                };
                if reached {
                    milestone(&phase, &id, original);
                }
            })
            .unwrap();
        panic!("missing crash milestone for {phase}");
    }
    struct Interrupted {
        child: Child,
        reader: Option<JoinHandle<()>>,
    }
    impl Drop for Interrupted {
        fn drop(&mut self) {
            let _ = self.child.kill();
            self.child.wait().unwrap();
            if let Some(reader) = self.reader.take() {
                reader.join().unwrap();
            }
        }
    }
    for phase in [
        "adopted",
        "first-touch-create",
        "first-touch-truncate",
        "truncated",
        "synced",
        "uncommitted-rename",
        "uncommitted-revoke",
        "committed-rename",
        "committed-revoke",
    ] {
        let mut f = Fixture::new();
        f.designate();
        let file = f.root.join("remote/settings.json");
        if phase != "adopted" && phase != "first-touch-create" {
            settings::set_open(&f.root, true).unwrap();
        }
        let before = std::fs::read(&file).ok();
        let id = uuid_v4().unwrap();
        // Hand ownership to the child; a fresh lease must be acquired after death.
        f.store.connection = Connection::open_in_memory().unwrap();
        f.serving.take();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("TMT_MANAGEMENT_CRASH_PHASE", phase)
            .env("TMT_MANAGEMENT_CRASH_ROOT", &f.root)
            .env("TMT_MANAGEMENT_CRASH_CLIENT", &f.grant.client_id)
            .env("TMT_MANAGEMENT_CRASH_ID", &id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut interrupted = Interrupted {
            child,
            reader: None,
        };
        let output = interrupted.child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        interrupted.reader = Some(std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                let line = line.unwrap();
                if let Some(event) = line.strip_prefix(PREFIX) {
                    sender
                        .send(serde_json::from_str::<Value>(event).unwrap())
                        .unwrap();
                    return;
                }
            }
        }));
        let event = receiver.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(event["phase"], phase);
        assert_eq!(event["operationId"], id);
        interrupted.child.kill().unwrap();
        assert_eq!(interrupted.child.wait().unwrap().signal(), Some(9));
        drop(interrupted); // reader joined, pipe closed, no child remains.
        f.serving = Some(Layout::open(&f.root).unwrap().serve_lock().unwrap());
        f.store = Store::open(f.serving.as_ref().unwrap()).unwrap();
        let row: (String, i64, i64, String) = f
            .store
            .connection
            .query_row(
                "SELECT id,adopted_ms,deadline_ms,outcome FROM management_receipts WHERE id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, id);
        assert_eq!(json!(row.1), event["adopted"]);
        assert_eq!(json!(row.2), event["deadline"]);
        let outcome: Value = serde_json::from_str(&row.3).unwrap();
        let committed = phase.starts_with("committed");
        assert_eq!(
            outcome["state"],
            if committed { "committed" } else { "unknown" }
        );
        assert_eq!(outcome["operationId"], id);
        let target = f.store.grant(&f.grant.client_id).unwrap().unwrap();
        assert_eq!(target.revision, if committed { 2 } else { 1 });
        assert_eq!(target.disabled, phase == "committed-revoke");
        assert_eq!(
            target.name,
            if phase == "committed-rename" {
                "Committed new name"
            } else {
                "Owner"
            }
        );
        assert_eq!(
            writable(&f.store.connection, &target).unwrap(),
            phase != "committed-revoke"
        );
        if phase == "truncated" {
            assert_eq!(std::fs::read(&file).unwrap(), b"");
            assert!(settings::read_or_default(&f.root).malformed);
        } else if phase == "synced" {
            assert!(!settings::read(&f.root).unwrap().open());
        } else {
            assert_eq!(std::fs::read(&file).ok(), before);
        }
        let bytes = std::fs::read(&file).ok();
        let operation = if phase.contains("rename") {
            "remote.devices.rename"
        } else if phase.contains("revoke") {
            "remote.devices.revoke"
        } else {
            "remote.settings.set"
        };
        if target.disabled {
            assert!(
                f.store
                    .management_adopt(&target, &id, operation, &[9; 32], now_ms().unwrap())
                    .is_err()
            );
            assert!(f.store.authorized(&target, now_ms().unwrap()).is_err());
        } else {
            assert_eq!(
                f.store
                    .management_adopt(&target, &id, operation, &[9; 32], now_ms().unwrap())
                    .unwrap()
                    .unwrap(),
                outcome
            );
            assert_eq!(
                f.store
                    .management_adopt(&target, &id, operation, &[8; 32], now_ms().unwrap())
                    .unwrap_err()
                    .code,
                "REMOTE_INTENT_CONFLICT"
            );
        }
        assert_eq!(
            receipt(
                &f.store.connection,
                &target.client_id,
                &id,
                now_ms().unwrap()
            )
            .unwrap(),
            outcome
        );
        assert_eq!(std::fs::read(&file).ok(), bytes); // Read/repeat never applies a pending setter.
        let final_times: (i64, i64) = f
            .store
            .connection
            .query_row(
                "SELECT adopted_ms,deadline_ms FROM management_receipts WHERE id=?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(final_times, (row.1, row.2));
        assert_eq!(f.count("management_receipts"), 1);
        assert_eq!(f.count("operations"), 0);
        println!(
            "verified SIGKILL phase={phase} id={id} adopted={} deadline={} state={} revision={} child/reader joined",
            row.1, row.2, outcome["state"], target.revision
        );
    }
}

#[test]
fn talk_toggle_commits_only_the_scope_and_revision_with_an_immutable_receipt() {
    let mut f = Fixture::new();
    f.designate();
    let original = f.grant.clone();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.talk", &[9; 32]).unwrap();
    let (outcome, changed) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.talk",
            &[9; 32],
            Mutation::Talk {
                client: f.grant.client_id.clone(),
                enabled: false,
            },
        )
        .unwrap();
    assert_eq!(outcome["state"], "committed");
    assert_eq!(outcome["result"]["device"]["talkEnabled"], false);
    assert_eq!(outcome["sessionEnded"], true);
    assert_eq!(changed, Some(f.grant.client_id.clone()));
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap(),
        outcome
    );
    let mut after = f.store.grant(&f.grant.client_id).unwrap().unwrap();
    assert_eq!(after.revision, original.revision + 1);
    assert!(!after.permits_scope("talk"));
    after.scopes.push("talk".into());
    after.revision = original.revision;
    assert_eq!(after, original);
    assert_eq!(f.count("audit"), 2);
}

#[test]
fn talk_toggle_noop_preserves_revision_and_a_revoked_device_cannot_be_enabled() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.talk", &[8; 32]).unwrap();
    let (outcome, changed) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.talk",
            &[8; 32],
            Mutation::Talk {
                client: f.grant.client_id.clone(),
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(outcome["result"]["device"]["revision"], f.grant.revision);
    assert_eq!(outcome["sessionEnded"], false);
    assert_eq!(changed, None);
    let other = Grant {
        client_id: uuid_v4().unwrap(),
        public_key: [8; 32],
        ..f.grant.clone()
    };
    f.store.insert_grant(&other).unwrap();
    f.store.revoke(&other.client_id).unwrap();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.talk", &[7; 32]).unwrap();
    let (outcome, changed) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.talk",
            &[7; 32],
            Mutation::Talk {
                client: other.client_id.clone(),
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(outcome["reason"], "REMOTE_DEVICE_REVOKED");
    assert_eq!(changed, None);
    assert!(f.store.grant(&other.client_id).unwrap().unwrap().disabled);
}

#[test]
fn sending_scope_rolls_back_if_its_atomic_settlement_audit_cannot_commit() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.talk", &[6; 32]).unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER fail_talk_audit BEFORE INSERT ON audit BEGIN SELECT RAISE(ABORT,'test audit failure'); END;").unwrap();
    assert!(
        f.store
            .management_effect(
                &f.grant,
                &id,
                "remote.devices.talk",
                &[6; 32],
                Mutation::Talk {
                    client: f.grant.client_id.clone(),
                    enabled: false
                }
            )
            .is_err()
    );
    assert_eq!(f.store.grant(&f.grant.client_id).unwrap().unwrap(), f.grant);
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
    assert_eq!(f.count("audit"), 1, "only adoption committed");
}
