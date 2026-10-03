//! Signed journal exchanges use the same private owner fixture as admission.
use super::{AdmissionRoot, OwnerDoor};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::sync::Arc;
use tmt_remote::{
    journal::{DAY, RECOVERY},
    pairing::now_ms,
    state::Layout,
    store::{Store, uuid_v4},
    transport::{LoopbackTransport, Transport},
};

fn control(
    owner: &OwnerDoor,
    session: &str,
    sequence: usize,
    operation: &str,
    input: Value,
) -> Value {
    let wire = owner.wire(
        session,
        &sequence.to_string(),
        operation,
        input.to_string().as_bytes(),
    );
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 1024);
    let bytes = serde_json::to_vec(&wire).unwrap();
    let response = if operation == "subscribe" {
        transport.subscribe(None, &bytes)
    } else {
        transport.ack(None, &bytes)
    }
    .unwrap();
    owner.verify_reply(&serde_json::from_slice(&response).unwrap(), &wire)
}
fn adopt_wire(
    owner: &OwnerDoor,
    wire: &Value,
    frozen: Option<&[u8]>,
) -> Result<tmt_remote::journal::Owned, tmt_remote::error::RemoteError> {
    owner.admit(wire).ok().unwrap().adopt(frozen)
}
fn adopted(
    owner: &OwnerDoor,
    session: &str,
    sequence: usize,
    operation: &str,
    frozen: Option<&[u8]>,
) -> (Value, tmt_remote::journal::Owned) {
    let wire = owner.wire(session, &sequence.to_string(), operation, b"{}");
    let owned = adopt_wire(owner, &wire, frozen).unwrap();
    (wire, owned)
}
#[test]
fn signed_catchup_retry_checkpoint_and_compaction_keep_separate_ownership() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let beginning = control(
        &owner,
        &session,
        1,
        "subscribe",
        json!({"cursor":null,"limit":50,"waitMs":1}),
    );
    assert_eq!(beginning["reason"], "timeout");
    let (wire, owned) = adopted(&owner, &session, 2, "capabilities", None);
    let mut retry = wire.clone();
    retry["sequence"] = json!("3");
    owner.resign(&mut retry);
    assert_eq!(
        adopt_wire(&owner, &retry, None).unwrap().receipt,
        owned.receipt
    );
    let page = control(
        &owner,
        &session,
        4,
        "subscribe",
        json!({"cursor":beginning["nextCursor"],"limit":1,"waitMs":0}),
    );
    let entries = page["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        owner.verify_reply(&entries[0]["envelope"], &wire),
        owned.receipt
    );
    assert_eq!(page["hasMore"], false);
    let cursor = page["nextCursor"].clone();
    for sequence in [5, 6] {
        assert_eq!(
            control(&owner, &session, sequence, "ack", json!({"cursor":cursor}))["cursor"],
            cursor
        );
    }
    let empty = control(
        &owner,
        &session,
        7,
        "subscribe",
        json!({"cursor":cursor,"limit":50,"waitMs":0}),
    );
    assert_eq!(empty["entries"], json!([]));
    assert_eq!(empty["nextCursor"], cursor);
    assert_eq!(
        owner
            .store
            .lock()
            .unwrap()
            .owned(&owner.grant, &owned.id, now_ms().unwrap())
            .unwrap()
            .receipt,
        owned.receipt
    );
}
#[test]
fn exact_changed_intent_and_another_client_refuse_without_an_entry() {
    let mut owner = OwnerDoor::new();
    let session = owner.open();
    let (wire, _) = adopted(&owner, &session, 1, "capabilities", None);
    let mut changed = owner.wire(&session, "2", "capabilities", b"{ }");
    changed["id"] = wire["id"].clone();
    owner.resign(&mut changed);
    assert_eq!(
        adopt_wire(&owner, &changed, None).unwrap_err().code,
        "REMOTE_INTENT_CONFLICT"
    );
    owner.key = SigningKey::from_bytes(&[48; 32]);
    owner.grant.client_id = uuid_v4().unwrap();
    owner.grant.public_key = owner.key.verifying_key().to_bytes();
    owner
        .store
        .lock()
        .unwrap()
        .insert_grant(&owner.grant)
        .unwrap();
    let other_session = owner.open();
    let mut collision = owner.wire(&other_session, "1", "capabilities", b"{}");
    collision["id"] = wire["id"].clone();
    owner.resign(&mut collision);
    assert_eq!(
        adopt_wire(&owner, &collision, None).unwrap_err().code,
        "REMOTE_INTENT_CONFLICT"
    );
    let page = control(
        &owner,
        &other_session,
        2,
        "subscribe",
        json!({"cursor":null,"limit":50,"waitMs":0}),
    );
    assert_eq!(page["entries"], json!([]));
}
#[test]
fn strict_controls_cross_machine_cursors_and_expiry_refuse() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    for (sequence, input) in [
        json!({"cursor":null,"limit":51,"waitMs":0}),
        json!({"cursor":null,"limit":1,"waitMs":25001}),
        json!({"cursor":null,"limit":1,"waitMs":0,"extra":true}),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            control(&owner, &session, sequence + 1, "subscribe", input)["error"]["code"],
            "REMOTE_INPUT_INVALID"
        );
    }
    let (wire, owned) = adopted(&owner, &session, 4, "capabilities", None);
    let page = control(
        &owner,
        &session,
        5,
        "subscribe",
        json!({"cursor":null,"limit":50,"waitMs":0}),
    );
    let cursor = page["nextCursor"].as_str().unwrap();
    let other = OwnerDoor::new();
    let other_session = other.open();
    assert_eq!(
        control(&other, &other_session, 1, "ack", json!({"cursor":cursor}))["error"]["code"],
        "REMOTE_CURSOR_EXPIRED"
    );
    let now = now_ms().unwrap();
    let mut store = owner.store.lock().unwrap();
    assert_eq!(
        store
            .page(&owner.grant, Some(cursor), 50, now + DAY)
            .unwrap_err()
            .code,
        "REMOTE_CURSOR_EXPIRED"
    );
    assert!(
        store.page(&owner.grant, None, 50, now + DAY).unwrap()["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .owned(&owner.grant, wire["id"].as_str().unwrap(), now + DAY)
            .unwrap()
            .receipt,
        owned.receipt
    );
    assert_eq!(
        store
            .owned(&owner.grant, &owned.id, now + RECOVERY)
            .unwrap_err()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
}
#[test]
fn persisted_calls_approvals_and_rollback_do_not_reset_budgets() {
    let owner = OwnerDoor::new();
    let root = AdmissionRoot::new();
    let serving = Layout::open(&root.0).unwrap().serve_lock().unwrap();
    let mut store = Store::open(&serving).unwrap();
    store.insert_grant(&owner.grant).unwrap();
    let now = now_ms().unwrap();
    let recipient = uuid_v4().unwrap();
    for _ in 0..120 {
        store.charge_call(&owner.grant.client_id, now).unwrap();
    }
    for _ in 0..60 {
        store.charge_approval(&recipient, now).unwrap();
    }
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    for time in [now, now - 60000] {
        assert_eq!(
            store
                .charge_call(&owner.grant.client_id, time)
                .unwrap_err()
                .code,
            "REMOTE_RATE_LIMITED"
        );
    }
    assert_eq!(
        store.charge_approval(&recipient, now).unwrap_err().code,
        "REMOTE_RATE_LIMITED"
    );
    store
        .charge_call(&owner.grant.client_id, now + 60000)
        .unwrap();
    store.charge_approval(&recipient, now + 60000).unwrap();
}
#[test]
fn held_adoption_refuses_at_outstanding_capacity() {
    let owner = OwnerDoor::with_policy(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|scope| (*scope).into())
            .collect(),
        "hold",
    );
    let session = owner.open();
    for sequence in 1..=16 {
        assert_eq!(
            adopted(
                &owner,
                &session,
                sequence,
                "dispatch.create",
                Some(b"frozen private intent")
            )
            .1
            .phase,
            "held"
        );
    }
    let excess = owner.wire(&session, "17", "dispatch.create", b"{}");
    assert_eq!(
        adopt_wire(&owner, &excess, Some(b"frozen private intent"))
            .unwrap_err()
            .code,
        "REMOTE_RATE_LIMITED"
    );
}

#[test]
fn authenticated_rate_refusal_is_signed_and_does_not_deadlock() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 1024);
    for sequence in 1..=120 {
        let wire = owner.wire(&session, &sequence.to_string(), "capabilities", b"{}");
        let reply: Value = serde_json::from_slice(
            &transport
                .append(None, &serde_json::to_vec(&wire).unwrap())
                .unwrap(),
        )
        .unwrap();
        let code = owner.verify_reply(&reply, &wire)["error"]["code"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            code,
            if sequence < 120 {
                "REMOTE_CLOSED"
            } else {
                "REMOTE_RATE_LIMITED"
            }
        );
    }
}
