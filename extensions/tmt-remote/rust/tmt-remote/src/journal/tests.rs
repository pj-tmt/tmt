//! Independent SQLite failure/capacity preparation; protocol admission is covered separately.
use super::*;
use crate::state::{Layout, Serving};
use crate::store::DEFAULT_SCOPES;
use std::path::PathBuf;
struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Fixture {
    store: Store,
    grant: Grant,
    now: u64,
    _serving: Serving,
    _root: Root,
}
impl Fixture {
    fn new() -> Self {
        let root = Root(std::env::temp_dir().join(format!("t1055-b-{}", uuid_v4().unwrap())));
        let serving = Layout::open(&root.0).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        store.machine().unwrap();
        let now = crate::pairing::now_ms().unwrap();
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Test device".into(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
            mode: "direct".into(),
            issued_at_ms: now,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        Self {
            store,
            grant,
            now,
            _serving: serving,
            _root: root,
        }
    }
    // Store tests receive already-admitted syntax; these bytes are never published.
    fn message(&self) -> SignedMessage {
        let body = json!({"version":1,"profile":"local-v1","kind":"request","id":uuid_v4().unwrap(),"correlationId":null,"machineId":self.store.connection.query_row("SELECT machine_id FROM machine",[],|r|r.get::<_,String>(0)).unwrap(),"windowId":uuid_v4().unwrap(),"clientId":self.grant.client_id,"sessionId":uuid_v4().unwrap(),"sequence":"1","timestampMs":self.now,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(br#"{"message":"private prompt"}"#),"signature":canonical::base64url(&[0;64])});
        SignedMessage::decode(body.to_string().as_bytes(), 1024).unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.store
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}
#[test]
fn audit_failure_rolls_back_intent_entry_and_send_budget_then_recovers() {
    let mut f = Fixture::new();
    let message = f.message();
    f.store.connection.execute_batch("CREATE TEMP TRIGGER audit_fault BEFORE INSERT ON audit BEGIN SELECT RAISE(ABORT,'fixture fault'); END;").unwrap();
    assert_eq!(
        f.store
            .adopt(
                &f.grant,
                &message,
                Some(b"frozen private prompt"),
                b"{}",
                f.now
            )
            .unwrap_err()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    for table in ["operations", "entries", "budgets", "audit", "streams"] {
        assert_eq!(f.count(table), 0);
    }
    f.store
        .connection
        .execute_batch("DROP TRIGGER audit_fault;")
        .unwrap();
    f.store
        .adopt(
            &f.grant,
            &message,
            Some(b"frozen private prompt"),
            b"{}",
            f.now,
        )
        .unwrap();
    assert_eq!(f.count("operations"), 1);
    assert_eq!(f.count("entries"), 1);
    assert_eq!(f.count("audit"), 1);
    let metadata:String=f.store.connection.query_row("SELECT client_id||envelope_id||operation||COALESCE(operation_id,'')||resources_json||decision||code FROM audit",[],|r|r.get(0)).unwrap();
    assert!(!metadata.contains("private prompt"));
}
#[test]
fn recovery_capacity_refuses_new_work_without_evicting_uncertain_work() {
    let mut f = Fixture::new();
    let message = f.message();
    f.store
        .adopt(&f.grant, &message, Some(b"frozen"), b"{}", f.now)
        .unwrap();
    f.store.connection.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<999) INSERT INTO operations(id,client_id,operation,digest,phase,receipt,updated_ms) SELECT printf('00000000-0000-4000-8000-%012x',x),?1,'capabilities',zeroblob(32),'uncertain','{}',?2 FROM n",params![f.grant.client_id,f.now as i64]).unwrap();
    let next = f.message();
    assert_eq!(
        f.store
            .adopt(
                &f.grant,
                &next,
                Some(b"frozen"),
                b"{}",
                f.now + RECOVERY + 1
            )
            .unwrap_err()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(f.count("operations"), 1000);
    assert_eq!(
        f.count("entries"),
        1,
        "failed adoption rolls back retention and metadata changes too"
    );
    assert_eq!(
        f.store
            .owned(&f.grant, message.envelope().id, f.now + RECOVERY + 1)
            .unwrap_err()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
}
#[test]
fn opaque_cursor_cannot_ack_an_unobserved_position_or_another_client() {
    let mut f = Fixture::new();
    let message = f.message();
    f.store
        .adopt(&f.grant, &message, Some(b"frozen"), b"{}", f.now)
        .unwrap();
    let tx = f.store.connection.transaction().unwrap();
    let stream = stream(&tx, &f.grant.client_id).unwrap();
    let unseen = cursor(&tx, &f.grant.client_id, &stream, 1, f.now).unwrap();
    drop(tx);
    assert_eq!(
        f.store.ack(&f.grant, &unseen, f.now).unwrap_err().code,
        "REMOTE_INPUT_INVALID"
    );
    let page = f.store.page(&f.grant, None, 50, f.now).unwrap();
    f.store
        .ack(&f.grant, page["nextCursor"].as_str().unwrap(), f.now)
        .unwrap();
    let mut other = f.grant.clone();
    other.client_id = uuid_v4().unwrap();
    other.public_key = [8; 32];
    f.store.insert_grant(&other).unwrap();
    assert_eq!(
        f.store
            .ack(&other, page["nextCursor"].as_str().unwrap(), f.now)
            .unwrap_err()
            .code,
        "REMOTE_CURSOR_EXPIRED"
    );
    f.store.page(&f.grant, None, 50, f.now).unwrap();
    assert_eq!(f.count("entries"), 0);
    assert_eq!(f.count("operations"), 1);
}

#[test]
fn expired_completed_id_is_not_adopted_again_after_metadata_eviction() {
    let mut f = Fixture::new();
    let message = f.message();
    f.store
        .adopt(&f.grant, &message, Some(b"frozen"), b"{}", f.now)
        .unwrap();
    f.store
        .connection
        .execute("UPDATE operations SET phase='accepted',frozen=NULL", [])
        .unwrap();
    f.store.page(&f.grant, None, 50, f.now + RECOVERY).unwrap();
    assert_eq!(f.count("entries"), 0);
    assert_eq!(
        f.store
            .adopt(&f.grant, &message, Some(b"frozen"), b"{}", f.now + RECOVERY)
            .unwrap_err()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(f.count("operations"), 1);
    assert_eq!(f.count("audit"), 1);
}

#[test]
fn wait_wakes_on_session_end_or_shutdown_even_before_wait_starts() {
    use crate::{session::DoorSessions, state::MachineKey};
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    for stop in [false, true] {
        let mut f = Fixture::new();
        let machine = f.store.machine().unwrap();
        let sessions = Arc::new(DoorSessions::new(
            machine.id,
            uuid_v4().unwrap(),
            "http://127.0.0.1:32100".into(),
            format!("{}/x/", machine.route_prefix),
            MachineKey::open(&Layout::open(&f._root.0).unwrap()).unwrap(),
            Arc::new(Mutex::new(f.store)),
            Duration::from_secs(43200),
        ));
        let (ready, started) = std::sync::mpsc::channel();
        let (send, receive) = std::sync::mpsc::channel();
        let waiter = Arc::clone(&sessions);
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            send.send(waiter.wait(0, Instant::now() + Duration::from_secs(25)))
                .unwrap();
        });
        // The notification may race ahead of wait; the captured generation prevents loss.
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        if stop {
            sessions.shutdown();
        } else {
            sessions.end_device(&f.grant.client_id);
        }
        let result = receive.recv_timeout(Duration::from_secs(2));
        sessions.shutdown();
        worker.join().unwrap();
        let result = result.expect("authority event did not wake wait");
        if stop {
            assert_eq!(result.unwrap_err().code, "REMOTE_CLOSED");
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn send_budget_survives_restart_and_an_explicit_retry_spends_once() {
    let mut f = Fixture::new();
    for _ in 0..budgets::SENDS {
        let message = f.message();
        f.store
            .adopt(&f.grant, &message, Some(b"frozen"), b"{}", f.now)
            .unwrap();
        f.store
            .adopt(
                &f.grant,
                &message,
                Some(b"ignored replacement"),
                b"{}",
                f.now,
            )
            .unwrap();
    }
    let next = f.message();
    drop(f.store);
    let mut store = Store::open(&f._serving).unwrap();
    assert_eq!(
        store
            .adopt(&f.grant, &next, Some(b"frozen"), b"{}", f.now)
            .unwrap_err()
            .code,
        "REMOTE_RATE_LIMITED"
    );
    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, budgets::SENDS);
}
