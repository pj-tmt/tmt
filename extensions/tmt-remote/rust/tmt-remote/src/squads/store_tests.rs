use super::*;
use crate::{
    state::{Layout, Serving},
    store::uuid_v4,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
const PEER: &str = "33333333-3333-4333-8333-333333333333";
const AGENT: &str = "66666666-6666-4666-8666-666666666666";

use crate::private_test_case::private_case as private_child;

struct World {
    layout: Layout,
    lease: Serving,
    store: Arc<Mutex<Store>>,
    grant: Grant,
    machine: String,
    now: u64,
}
impl World {
    fn new(root: PathBuf) -> Self {
        let layout = Layout::open(&root).unwrap();
        let admitted_home = PathBuf::from(std::env::var_os("TMT_HOME").unwrap())
            .canonicalize()
            .unwrap();
        assert_eq!(
            layout.directory.parent().unwrap(),
            admitted_home,
            "Store must remain inside the exact private TMT_HOME (including canonical macOS temp aliases)"
        );
        let lease = layout.serve_lock().unwrap();
        let mut store = Store::open(&lease).unwrap();
        let machine = store.machine().unwrap().id;
        let now = now_ms().unwrap();
        let grant = Grant {
            client_id: PEER.into(),
            public_key: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Peer".into(),
            agents: "all".into(),
            scopes: vec!["agents.read".into()],
            mode: "hold".into(),
            issued_at_ms: now,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        store
            .save_cloud_connection(&crate::cloud_pairing::Connection {
                version: 1,
                name: "paired".into(),
                owner: PEER.into(),
                owner_key: grant.public_key,
                config: json!({"projectId":"demo-summary"}),
                peer: machine.clone(),
                uid: "private-uid".into(),
                request: vec![],
                confirmed: true,
                descriptor: vec![],
                reroute_token: None,
                pending_submission: None,
            })
            .unwrap();
        Self {
            layout,
            lease,
            store: Arc::new(Mutex::new(store)),
            grant,
            machine,
            now,
        }
    }
    fn payload(&self, sequence: u64, name: &str) -> Vec<u8> {
        json!({"version":1,"sequence":sequence,"unrecognized":"PRIVATE",
            "summary":{"version":1,"machineName":name,"observedAt":self.now,"status":"available",
                "extra":"PRIVATE","squads":[{"squad":"Team","members":[{"id":AGENT,"name":"worker","state":"review","presence":"active","task":"PRIVATE"}]}]}}).to_string().into_bytes()
    }
    fn receiver(
        &self,
    ) -> (
        crate::cloud_inbox::Receiver,
        Arc<crate::cloud_keys::CloudKeys>,
    ) {
        let (receiver, keys, _) = self.receiver_owner();
        (receiver, keys)
    }
    fn receiver_owner(
        &self,
    ) -> (
        crate::cloud_inbox::Receiver,
        Arc<crate::cloud_keys::CloudKeys>,
        Arc<crate::session::DoorSessions>,
    ) {
        let key = crate::state::MachineKey::open(&self.layout).unwrap();
        let sessions = Arc::new(crate::session::DoorSessions::new(
            self.machine.clone(),
            uuid_v4().unwrap(),
            "http://127.0.0.1:1".into(),
            "/r/summary/x/".into(),
            key.clone(),
            self.store.clone(),
            crate::session::IDLE,
        ));
        let keys = Arc::new(crate::cloud_keys::CloudKeys::new(
            self.layout.clone(),
            self.machine.clone(),
            key,
        ));
        keys.maintain(self.now).unwrap();
        let operations = crate::operations::Operations::new(
            crate::core::CoreClient::at("/must-not-run-core".into()).unwrap(),
            Arc::new(AtomicBool::new(false)),
            crate::limits::CLOUD_ENVELOPE_BYTES,
        );
        let messages = Arc::new(
            crate::message::MessageService::new(
                sessions.clone(),
                crate::limits::CLOUD_ENVELOPE_BYTES,
            )
            .with_operations(Arc::new(operations)),
        );
        (
            crate::cloud_inbox::Receiver::new(sessions.clone(), keys.clone(), messages),
            keys,
            sessions,
        )
    }
    fn document(
        &self,
        keys: &crate::cloud_keys::CloudKeys,
        payload: &[u8],
    ) -> crate::cloud_inbox::document::Document {
        let signer = SigningKey::from_bytes(&[8; 32]);
        let id = uuid_v4().unwrap();
        let window = uuid_v4().unwrap();
        let session = uuid_v4().unwrap();
        let envelope = crate::canonical::Envelope {
            kind: "request",
            id: &id,
            correlation_id: None,
            machine_id: &self.machine,
            window_id: &window,
            client_id: PEER,
            session_id: &session,
            sequence: "1",
            timestamp_ms: self.now,
            origin: "cli",
            operation: "squads.summary",
            payload,
        };
        let signature = crate::canonical::base64url(
            &signer
                .sign(&crate::canonical::envelope(&envelope).unwrap())
                .to_bytes(),
        );
        let exact = json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":self.machine,"windowId":window,"clientId":PEER,"sessionId":session,"sequence":"1","timestampMs":self.now,"origin":"cli","operation":"squads.summary","payload":crate::canonical::base64url(payload),"signature":signature}).to_string();
        let descriptor = keys.advertised(self.now).unwrap();
        let (carrier, _) = crate::cloud_inbox::seal_native(
            exact.as_bytes(),
            &descriptor,
            self.now,
            crate::cloud_hpke::Sender {
                client_id: PEER,
                authentication_key: &self.grant.public_key,
            },
            |bytes| signer.sign(bytes).to_bytes(),
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&carrier).contains("worker"));
        let root = "projects/demo-summary/databases/(default)/documents";
        let stamp = crate::cloud_inbox::stamp(self.now);
        let value = json!({"name":format!("{root}/m/{}/inbox/{id}",self.machine),"createTime":stamp,"fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(carrier)},"createdAt":{"timestampValue":stamp}}});
        crate::cloud_inbox::document::Document::read(&value, root, &self.machine, "inbox").unwrap()
    }
}
#[test]
fn verified_cache_restart_order_unknown_fields_and_revoke() {
    private_child(
        "squads::store::tests::verified_cache_restart_order_unknown_fields_and_revoke",
        |root| {
            let mut world = World::new(root);
            let (receiver, keys) = world.receiver();
            let newest = world.document(&keys, &world.payload(2, "New"));
            let outcome = receiver.receive(&newest, world.now).unwrap().unwrap();
            assert!(!outcome.response_required());
            receiver.receive(&newest, world.now).unwrap();
            let valid = world.document(&keys, &world.payload(3, "Tampered"));
            let mut carrier: serde_json::Value = serde_json::from_slice(valid.bytes()).unwrap();
            let mut ciphertext =
                crate::canonical::base64url_decode(carrier["ciphertext"].as_str().unwrap())
                    .unwrap();
            ciphertext[0] ^= 1;
            carrier["ciphertext"] = json!(crate::canonical::base64url(&ciphertext));
            let root = "projects/demo-summary/databases/(default)/documents";
            let stamp = crate::cloud_inbox::stamp(world.now);
            let row = json!({"name":format!("{root}/m/{}/inbox/{}",world.machine,valid.id()),"createTime":stamp,"fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(serde_json::to_vec(&carrier).unwrap())},"createdAt":{"timestampValue":stamp}}});
            let tampered =
                crate::cloud_inbox::document::Document::read(&row, root, &world.machine, "inbox")
                    .unwrap();
            assert!(receiver.receive(&tampered, world.now).unwrap().is_none());
            let older = world.document(&keys, &world.payload(1, "Old"));
            receiver.receive(&older, world.now).unwrap();
            drop(outcome);
            drop(receiver);
            drop(keys);
            assert_eq!(world.store.lock().unwrap().read().unwrap().len(), 1);
            let bytes = world
                .store
                .lock()
                .unwrap()
                .read()
                .unwrap()
                .remove(0)
                .summary;
            assert_eq!(Summary::read(&bytes).unwrap().machine_name, "New");
            assert!(!String::from_utf8(bytes).unwrap().contains("PRIVATE"));
            drop(world.store);
            world.store = Arc::new(Mutex::new(Store::open(&world.lease).unwrap()));
            assert_eq!(world.store.lock().unwrap().read().unwrap().len(), 1);
            let bad = br#"{"version":2,"sequence":3,"summary":null}"#;
            assert!(
                world
                    .store
                    .lock()
                    .unwrap()
                    .summary_received(&world.grant, AGENT, bad, world.now)
                    .is_err()
            );
            assert_eq!(
                Summary::read(&world.store.lock().unwrap().read().unwrap()[0].summary)
                    .unwrap()
                    .machine_name,
                "New"
            );
            world.store.lock().unwrap().revoke(PEER).unwrap();
            assert!(world.store.lock().unwrap().read().unwrap().is_empty());
            assert!(
                world
                    .store
                    .lock()
                    .unwrap()
                    .summary_received(&world.grant, AGENT, &world.payload(4, "Denied"), world.now)
                    .is_err()
            );
        },
    );
}
#[test]
fn pending_intent_survives_reopen_budget_hold_and_grant_narrowing() {
    private_child(
        "squads::store::tests::pending_intent_survives_reopen_budget_hold_and_grant_narrowing",
        |root| {
            let mut w = World::new(root);
            let summary = Summary::unavailable("Studio", w.now).unwrap();
            let next = Prepared::for_peer(&summary, &w.grant.authority().unwrap()).unwrap();
            let pending = {
                let mut store = w.store.lock().unwrap();
                store.summary_observed(&w.grant, &next, w.now).unwrap();
                store.summary_pending(&w.grant, &next, w.now).unwrap()
            };
            w.store
                .lock()
                .unwrap()
                .summary_defer(PEER, w.now + 60_000)
                .unwrap();
            drop(w.store);
            w.store = Arc::new(Mutex::new(Store::open(&w.lease).unwrap()));
            let mut schedule = w
                .store
                .lock()
                .unwrap()
                .summary_schedule(PEER, w.now + 100, 1000)
                .unwrap()
                .unwrap();
            schedule.observe(next.clone(), 1000);
            assert!(schedule.due(60_899).is_none());
            assert!(schedule.due(60_900).is_some());
            let restored = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&w.grant, &next, w.now + 100)
                .unwrap();
            assert_eq!(restored.id, pending.id);
            assert_eq!(restored.payload, pending.payload);
            // Narrowing before any attempted HTTP discards only the unsent intent;
            // no old broad plaintext can acquire the new grant's publication claim.
            w.store
                .lock()
                .unwrap()
                .connection
                .execute(
                    "UPDATE grants SET revision=revision+1,agents='[]' WHERE client_id=?1",
                    [PEER],
                )
                .unwrap();
            let current = w.store.lock().unwrap().grant(PEER).unwrap().unwrap();
            w.store
                .lock()
                .unwrap()
                .summary_observed(&current, &next, w.now + 100)
                .unwrap();
            let replacement = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&current, &next, w.now + 100)
                .unwrap();
            assert_ne!(pending.id, replacement.id);
            assert_eq!(frame(&replacement.payload).unwrap().0, 2);
            assert!(
                w.store
                    .lock()
                    .unwrap()
                    .summary_claim(&current, &w.machine, &pending, b"old")
                    .is_err()
            );
        },
    );
}

#[test]
fn revoked_grant_cannot_claim_a_late_preparation() {
    private_child(
        "squads::store::tests::revoked_grant_cannot_claim_a_late_preparation",
        |root| {
            let w = World::new(root);
            let next = Prepared::for_peer(
                &Summary::unavailable("Studio", w.now).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            let mut store = w.store.lock().unwrap();
            store.summary_observed(&w.grant, &next, w.now).unwrap();
            let pending = store.summary_pending(&w.grant, &next, w.now).unwrap();
            let carrier = b"unattempted-ciphertext";
            store
                .cloud_freeze_outgoing(&w.machine, &pending.id, &pending.payload, carrier, w.now)
                .unwrap();
            store.revoke(PEER).unwrap();
            assert!(
                store
                    .summary_claim(&w.grant, &w.machine, &pending, carrier)
                    .is_err()
            );
            assert!(
                !store
                    .cloud_one_way_outgoing(&w.machine, &pending.id, &pending.payload)
                    .unwrap()
                    .unwrap()
                    .attempted
            );
        },
    );
}

#[test]
fn uncertain_publication_retains_returned_content_across_restart() {
    private_child(
        "squads::store::tests::uncertain_publication_retains_returned_content_across_restart",
        |root| {
            let mut w = World::new(root);
            let (_, summary) = frame(&w.payload(1, "Studio")).unwrap();
            let a = Prepared::for_peer(&summary, &w.grant.authority().unwrap()).unwrap();
            let b = Prepared::for_peer(
                &Summary::unavailable("Studio", w.now + 1).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            let pending_b = {
                let mut store = w.store.lock().unwrap();
                store.summary_observed(&w.grant, &a, w.now).unwrap();
                let pending_a = store.summary_pending(&w.grant, &a, w.now).unwrap();
                store
                    .summary_published(&w.grant, &pending_a.id, &pending_a.payload, w.now)
                    .unwrap();
                store.summary_observed(&w.grant, &b, w.now + 1).unwrap();
                let pending_b = store.summary_pending(&w.grant, &b, w.now + 1).unwrap();
                store
                    .cloud_freeze_outgoing(
                        &w.machine,
                        &pending_b.id,
                        &pending_b.payload,
                        b"attempted-ciphertext",
                        w.now,
                    )
                    .unwrap();
                assert!(
                    store
                        .summary_claim(&w.grant, &w.machine, &pending_b, b"attempted-ciphertext")
                        .unwrap()
                );
                store.summary_observed(&w.grant, &a, w.now + 2).unwrap();
                pending_b
            };
            drop(w.store);
            w.store = Arc::new(Mutex::new(Store::open(&w.lease).unwrap()));
            let mut restored = w
                .store
                .lock()
                .unwrap()
                .summary_schedule(PEER, w.now + 10_000, 0)
                .unwrap()
                .unwrap();
            restored.observe(a.clone(), 0);
            assert!(
                restored.due(0).is_some(),
                "old confirmed content is no longer certain after claimed B"
            );
            let original = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&w.grant, &a, w.now + 10_000)
                .unwrap();
            assert_eq!(original.id, pending_b.id);
            assert_eq!(original.payload, pending_b.payload);
            let mut store = w.store.lock().unwrap();
            store
                .summary_published(&w.grant, &original.id, &original.payload, w.now + 10_001)
                .unwrap();
            let repair = store.summary_pending(&w.grant, &a, w.now + 15_001).unwrap();
            assert_ne!(repair.id, original.id);
            assert!(frame(&repair.payload).unwrap().0 > frame(&original.payload).unwrap().0);
            assert_eq!(frame(&repair.payload).unwrap().1, summary);
        },
    );
}

#[test]
fn old_attempt_completion_cannot_impersonate_a_new_narrow_projection() {
    private_child(
        "squads::store::tests::old_attempt_completion_cannot_impersonate_a_new_narrow_projection",
        |root| {
            let w = World::new(root);
            let (_, summary) = frame(&w.payload(1, "Studio")).unwrap();
            let broad = Prepared::for_peer(&summary, &w.grant.authority().unwrap()).unwrap();
            let mut store = w.store.lock().unwrap();
            store.summary_observed(&w.grant, &broad, w.now).unwrap();
            let original = store.summary_pending(&w.grant, &broad, w.now).unwrap();
            store
                .cloud_freeze_outgoing(
                    &w.machine,
                    &original.id,
                    &original.payload,
                    b"attempted",
                    w.now,
                )
                .unwrap();
            assert!(
                store
                    .summary_claim(&w.grant, &w.machine, &original, b"attempted")
                    .unwrap()
            );
            store
                .connection
                .execute(
                    "UPDATE grants SET revision=revision+1,agents='[]' WHERE client_id=?1",
                    [PEER],
                )
                .unwrap();
            let current = store.grant(PEER).unwrap().unwrap();
            let narrow = Prepared::for_peer(&summary, &current.authority().unwrap()).unwrap();
            assert_ne!(broad.fingerprint, narrow.fingerprint);
            store
                .summary_observed(&current, &narrow, w.now + 1)
                .unwrap();
            let recovering = store.summary_pending(&current, &narrow, w.now + 1).unwrap();
            assert_eq!(recovering.id, original.id);
            store
                .summary_published(&current, &recovering.id, &recovering.payload, w.now + 2)
                .unwrap();
            let mut restored = store
                .summary_schedule(PEER, w.now + 5002, 0)
                .unwrap()
                .unwrap();
            restored.observe(narrow.clone(), 0);
            assert!(
                restored.due(0).is_some(),
                "observing old broad bytes must leave narrow repair pending"
            );
            let repair = store
                .summary_pending(&current, &narrow, w.now + 5002)
                .unwrap();
            assert!(
                frame(&repair.payload)
                    .unwrap()
                    .1
                    .squads()
                    .unwrap()
                    .is_empty()
            );
            assert!(frame(&repair.payload).unwrap().0 > frame(&original.payload).unwrap().0);
        },
    );
}

#[test]
fn carrier_retention_cannot_turn_an_expired_attempt_into_a_recreate() {
    private_child(
        "squads::store::tests::carrier_retention_cannot_turn_an_expired_attempt_into_a_recreate",
        |root| {
            let w = World::new(root);
            let next = Prepared::for_peer(
                &Summary::unavailable("Studio", w.now).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            let mut store = w.store.lock().unwrap();
            store.summary_observed(&w.grant, &next, w.now).unwrap();
            let old = store.summary_pending(&w.grant, &next, w.now).unwrap();
            store
                .cloud_freeze_outgoing(&w.machine, &old.id, &old.payload, b"expired-attempt", w.now)
                .unwrap();
            assert!(
                store
                    .summary_claim(&w.grant, &w.machine, &old, b"expired-attempt")
                    .unwrap()
            );
            let after = w.now
                + crate::journal::RECOVERY
                + crate::session::CLOCK_SKEW.as_millis() as u64
                + 1;
            store.cloud_prune_outgoing(after).unwrap();
            assert!(
                store
                    .cloud_one_way_outgoing(&w.machine, &old.id, &old.payload)
                    .unwrap()
                    .is_none()
            );
            let fresh = Prepared::for_peer(
                &Summary::unavailable("Studio", after).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            store.summary_observed(&w.grant, &fresh, after).unwrap();
            let pending = store.summary_pending(&w.grant, &fresh, after).unwrap();
            assert_ne!(pending.id, old.id);
            assert!(frame(&pending.payload).unwrap().0 > frame(&old.payload).unwrap().0);
            assert_eq!(frame(&pending.payload).unwrap().1.observed_at, after);
            assert!(
                store
                    .summary_claim(&w.grant, &w.machine, &old, b"expired-attempt")
                    .is_err()
            );
        },
    );
}

#[test]
fn summary_heartbeats_exceed_carrier_capacity_without_accumulating_history() {
    private_child(
        "squads::store::tests::summary_heartbeats_exceed_carrier_capacity_without_accumulating_history",
        |root| {
            let w = World::new(root);
            let mut store = w.store.lock().unwrap();
            let generic_in = uuid_v4().unwrap();
            store
                .cloud_begin(PEER, &generic_in, b"generic incoming", w.now)
                .unwrap();
            store
                .cloud_freeze(PEER, &generic_in, b"generic response")
                .unwrap();
            let generic_out = uuid_v4().unwrap();
            store
                .cloud_freeze_outgoing(
                    &w.machine,
                    &generic_out,
                    b"generic intent",
                    b"generic outgoing",
                    w.now,
                )
                .unwrap();
            assert!(
                store
                    .cloud_claim_outgoing(&w.machine, &generic_out)
                    .unwrap()
            );
            let mut other = w.grant.clone();
            other.client_id = "44444444-4444-4444-8444-444444444444".into();
            other.public_key = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
            other.name = "Other peer".into();
            store.insert_grant(&other).unwrap();
            let initial = Prepared::for_peer(
                &Summary::unavailable("Studio", w.now).unwrap(),
                &other.authority().unwrap(),
            )
            .unwrap();
            store.summary_observed(&other, &initial, w.now).unwrap();
            let active = store.summary_pending(&other, &initial, w.now).unwrap();
            store
                .cloud_freeze_outgoing(
                    &w.machine,
                    &active.id,
                    &active.payload,
                    b"protected active",
                    w.now,
                )
                .unwrap();
            assert!(
                store
                    .summary_claim(&other, &w.machine, &active, b"protected active")
                    .unwrap()
            );
            let count = crate::journal::OPERATIONS as u64 + 1;
            for sequence in 1..=count {
                let time = w.now + sequence * crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS;
                let snapshot = Summary::unavailable("Studio", time).unwrap();
                let payload = json!({"version":1,"sequence":sequence,"summary":snapshot})
                    .to_string()
                    .into_bytes();
                let id = uuid_v4().unwrap();
                store
                    .cloud_begin(PEER, &id, b"verified summary carrier", time)
                    .unwrap();
                store
                    .summary_received(&w.grant, &id, &payload, time)
                    .unwrap();
                store
                    .cloud_freeze(PEER, &id, b"local summary outcome")
                    .unwrap();
                let cleanup = store.cloud_expired(time).unwrap();
                assert!(
                    cleanup
                        .iter()
                        .any(|row| row.peer == PEER && row.id == id && row.summary)
                );
                // The driver calls forget only after the guarded delete succeeds.
                store.cloud_forget(PEER, &id, time).unwrap();
                let prepared =
                    Prepared::for_peer(&snapshot, &w.grant.authority().unwrap()).unwrap();
                store.summary_observed(&w.grant, &prepared, time).unwrap();
                let pending = store.summary_pending(&w.grant, &prepared, time).unwrap();
                store
                    .cloud_freeze_outgoing(
                        &w.machine,
                        &pending.id,
                        &pending.payload,
                        b"summary outgoing",
                        time,
                    )
                    .unwrap();
                assert!(
                    store
                        .summary_claim(&w.grant, &w.machine, &pending, b"summary outgoing")
                        .unwrap()
                );
                store
                    .summary_published(&w.grant, &pending.id, &pending.payload, time)
                    .unwrap();
            }
            let watermark: i64 = store
                .connection
                .query_row(
                    "SELECT sequence FROM squad_cache WHERE client_id=?1",
                    [PEER],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(watermark, count as i64);
            let incoming: i64 = store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM cloud_transport WHERE kind='inbox'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let outgoing: i64 = store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM cloud_transport WHERE kind='outbox'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(incoming, 1, "generic incoming history is retained");
            assert_eq!(
                outgoing, 3,
                "one completion, protected active snapshot and generic outgoing"
            );
            assert!(
                store
                    .cloud_one_way_outgoing(&w.machine, &active.id, &active.payload)
                    .unwrap()
                    .unwrap()
                    .attempted
            );
            assert!(
                store
                    .cloud_one_way_outgoing(&w.machine, &generic_out, b"generic intent")
                    .unwrap()
                    .unwrap()
                    .attempted
            );
            assert!(
                !store
                    .cloud_expired(w.now)
                    .unwrap()
                    .iter()
                    .any(|row| row.id == generic_in)
            );
        },
    );
}

#[test]
fn paired_missing_cache_is_unavailable_without_fabricating_an_observation() {
    private_child(
        "squads::store::tests::paired_missing_cache_is_unavailable_without_fabricating_an_observation",
        |root| {
            let w = World::new(root);
            let store = w.store.lock().unwrap();
            let records = store.read().unwrap();
            assert_eq!(records.len(), 1);
            assert!(records[0].summary.is_empty());
            let list = crate::squads::project_records(&w.machine, records).unwrap();
            assert_eq!(
                list.machines[0].status,
                crate::squads::MachineSummaryStatus::Unavailable
            );
            assert!(list.machines[0].summary.is_none());
            let count: i64 = store
                .connection
                .query_row("SELECT COUNT(*) FROM squad_cache", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        },
    );
}

#[path = "publisher_io_tests.rs"]
mod io_tests;
