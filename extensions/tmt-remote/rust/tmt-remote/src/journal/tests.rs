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
            public_key: [7; 32],
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
    fn adopt(&mut self, message: &SignedMessage, now: u64) -> Result<Owned, RemoteError> {
        self.store.adopt(
            &self.grant,
            message,
            Some(&message.payload),
            &[],
            b"{}",
            now,
        )
    }
}
fn count(store: &Store, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn recovery_capacity_refuses_new_work_without_evicting_uncertain_work() {
    let mut f = Fixture::new();
    let message = f.message();
    f.adopt(&message, f.now).unwrap();
    f.store.connection.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<999) INSERT INTO operations(id,client_id,operation,digest,phase,receipt,updated_ms) SELECT printf('00000000-0000-4000-8000-%012x',x),?1,'dispatch.create',zeroblob(32),'uncertain','{}',?2 FROM n",params![f.grant.client_id,f.now as i64]).unwrap();
    let next = f.message();
    assert_eq!(
        f.adopt(&next, f.now + RECOVERY + 1).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(count(&f.store, "operations"), 1000);
    assert_eq!(count(&f.store, "entries"), 1);
}
#[test]
fn opaque_cursor_cannot_ack_an_unobserved_position_or_another_client() {
    let mut f = Fixture::new();
    let message = f.message();
    f.adopt(&message, f.now).unwrap();
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
    assert_eq!(count(&f.store, "entries"), 0);
    assert_eq!(count(&f.store, "operations"), 1);
}

#[test]
fn expired_completed_id_is_not_adopted_again_after_metadata_eviction() {
    let mut f = Fixture::new();
    let message = f.message();
    f.adopt(&message, f.now).unwrap();
    f.store
        .connection
        .execute("UPDATE operations SET phase='accepted',frozen=NULL", [])
        .unwrap();
    f.store.page(&f.grant, None, 50, f.now + RECOVERY).unwrap();
    assert_eq!(count(&f.store, "entries"), 0);
    assert_eq!(
        f.adopt(&message, f.now + RECOVERY).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(count(&f.store, "operations"), 1);
    assert_eq!(count(&f.store, "audit"), 1);
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
        f.adopt(&message, f.now).unwrap();
        f.adopt(&message, f.now).unwrap();
    }
    let next = f.message();
    drop(f.store);
    let mut store = Store::open(&f._serving).unwrap();
    assert_eq!(
        store
            .adopt(&f.grant, &next, Some(b"frozen"), &[], b"{}", f.now)
            .unwrap_err()
            .code,
        "REMOTE_RATE_LIMITED"
    );
    assert_eq!(count(&store, "operations"), budgets::SENDS);
}

#[test]
fn revoke_commit_wins_while_adoption_waits_for_the_immediate_transaction() {
    use std::{cell::RefCell, sync::mpsc, time::Duration};
    struct Fence {
        ready: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    thread_local! {static FENCE:RefCell<Option<Fence>>=const {RefCell::new(None)};}
    fn busy(_: i32) -> bool {
        FENCE.with(|slot| {
            let fence = slot.borrow_mut().take().unwrap();
            fence.ready.send(()).unwrap();
            fence.release.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        true
    }
    let f = Fixture::new();
    let message = f.message();
    let mut revoker = Store::open(&f._serving).unwrap();
    let tx = revoker
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "UPDATE grants SET disabled=1,revision=revision+1 WHERE client_id=?1",
        [&f.grant.client_id],
    )
    .unwrap();
    let (ready, blocked) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut store = f.store;
    let worker = std::thread::spawn(move || {
        FENCE.with(|slot| {
            *slot.borrow_mut() = Some(Fence {
                ready,
                release: resume,
            })
        });
        store.connection.busy_handler(Some(busy)).unwrap();
        let result = store.adopt(&f.grant, &message, Some(b"frozen"), &[], b"{}", f.now);
        assert_eq!(count(&store, "operations"), 0);
        result
    });
    // Busy-handler readiness proves BEGIN IMMEDIATE was attempted before revoke committed.
    blocked.recv_timeout(Duration::from_secs(2)).unwrap();
    tx.commit().unwrap();
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "REMOTE_CLOSED");
}

#[test]
fn audit_age_and_count_retention_roll_back_with_failed_adoption() {
    let mut f = Fixture::new();
    f.store.connection.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100001) INSERT INTO audit(position,at_ms,client_id,envelope_id,operation,resources_json,digest,grant_revision,decision,code) SELECT x,CASE WHEN x=1 THEN ?1 ELSE ?2 END,?3,?3,'capabilities','[]',zeroblob(32),1,'refused','REMOTE_CLOSED' FROM n",params![(f.now-RECOVERY) as i64,f.now as i64,f.grant.client_id]).unwrap();
    f.store.connection.execute_batch("CREATE TEMP TRIGGER retention_fault BEFORE INSERT ON audit BEGIN SELECT RAISE(ABORT,'fixture fault'); END;").unwrap();
    let message = f.message();
    assert_eq!(
        f.adopt(&message, f.now).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(count(&f.store, "audit"), 100001);
    for table in ["operations", "entries", "budgets", "streams"] {
        assert_eq!(count(&f.store, table), 0);
    }
    f.store
        .connection
        .execute_batch("DROP TRIGGER retention_fault;")
        .unwrap();
    f.adopt(&message, f.now).unwrap();
    assert_eq!(count(&f.store, "audit"), audit::RECORDS);
    let bounds: (i64, i64) = f
        .store
        .connection
        .query_row("SELECT MIN(position),MIN(at_ms) FROM audit", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(bounds, (3, f.now as i64));
    let metadata:String=f.store.connection.query_row("SELECT client_id||envelope_id||operation||COALESCE(operation_id,'')||resources_json||decision||code FROM audit ORDER BY position DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    assert!(!metadata.contains("private prompt"));
}

#[test]
fn a_finished_record_keeps_its_recovery_horizon_from_its_last_state_change() {
    let mut f = Fixture::new();
    let message = f.message();
    let first = f.adopt(&message, f.now).unwrap();
    // Settled 29 days after adoption: that is its last state change.
    let accepted = json!({"state":"accepted","operationId":first.id,"requestId":"req_11111111-1111-4111-8111-111111111111"});
    f.store
        .settle(
            &f.grant,
            &first.id,
            &accepted,
            Some(b"{}"),
            f.now + 29 * DAY,
        )
        .unwrap();
    // An unrelated adoption just past 30 days after the first adoption must not drop it.
    let at = f.now + RECOVERY + 1;
    let next = f.message();
    f.adopt(&next, at).unwrap();
    f.store.owned(&f.grant, &first.id, at).unwrap();
    // Past 30 days after the settlement it is finished work beyond the horizon and goes.
    let at = f.now + 29 * DAY + RECOVERY + 1;
    let last = f.message();
    f.adopt(&last, at).unwrap();
    assert_eq!(
        f.store.owned(&f.grant, &first.id, at).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
}
#[test]
fn eviction_moves_the_floor_so_older_cursors_expire_and_the_floor_cursor_stays_valid() {
    let mut f = Fixture::new();
    f.store.connection.execute("INSERT INTO streams(client_id,incarnation,key,tip,last_ms) VALUES (?1,?2,zeroblob(32),1000,?3)", params![f.grant.client_id, uuid_v4().unwrap(), f.now as i64]).unwrap();
    f.store.connection.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO entries(client_id,position,envelope,at_ms) SELECT ?1,x,CAST('{}' AS BLOB),?2 FROM n", params![f.grant.client_id, f.now as i64]).unwrap();
    let message = f.message();
    f.adopt(&message, f.now).unwrap();
    assert_eq!(count(&f.store, "entries"), 1000);
    let tx = f.store.connection.transaction().unwrap();
    let stream = stream(&tx, &f.grant.client_id).unwrap();
    assert_eq!(stream.floor, 1);
    let below = cursor(&tx, &f.grant.client_id, &stream, 0, f.now).unwrap();
    let at_floor = cursor(&tx, &f.grant.client_id, &stream, 1, f.now).unwrap();
    drop(tx);
    assert_eq!(
        f.store
            .page(&f.grant, Some(&below), 50, f.now)
            .unwrap_err()
            .code,
        "REMOTE_CURSOR_EXPIRED"
    );
    let page = f.store.page(&f.grant, Some(&at_floor), 50, f.now).unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 50);
}
