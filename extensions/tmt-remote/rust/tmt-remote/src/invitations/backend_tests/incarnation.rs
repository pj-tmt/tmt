//! Operation ownership follows the confirmed member, not mutable sending policy.
use super::{
    durable_fixture::DurableFixture,
    reinvite::{confirm, remove},
    *,
};
use crate::{pairing::now_ms, wire::SignedMessage};

fn operation(f: &DurableFixture, client: &str) -> (String, SignedMessage, Vec<u8>) {
    let mut store = f.store.lock().unwrap();
    let grant = store.talk(client, true).unwrap().unwrap();
    let id = crate::store::uuid_v4().unwrap();
    let recipient = crate::store::uuid_v4().unwrap();
    let now = now_ms().unwrap();
    let frozen = serde_json::json!({"version":1,"operation":"dispatch.create","originator":"anonymous","input":{"operationId":id,"recipientIds":[recipient],"message":"old uncertain intent","kind":"request"}}).to_string().into_bytes();
    // The Store receives already-admitted syntax. Cryptographic admission has its
    // own signed fixture; this scenario invokes no Core or provider operation.
    let wire = serde_json::json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":f.context.owner,"windowId":crate::store::uuid_v4().unwrap(),"clientId":client,"sessionId":crate::store::uuid_v4().unwrap(),"sequence":"1","timestampMs":now,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(&frozen),"signature":canonical::base64url(&[0;64])});
    let message = SignedMessage::decode(wire.to_string().as_bytes(), 65536).unwrap();
    store
        .adopt(&grant, &message, Some(&frozen), &[recipient], b"{}", now)
        .unwrap();
    store
        .settle(
            &grant,
            &id,
            &serde_json::json!({"state":"uncertain","operationId":id}),
            None,
            now,
        )
        .unwrap();
    (id, message, frozen)
}

#[test]
fn a_new_member_cannot_release_an_uncertain_operation_from_the_removed_incarnation() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    let client = &old.snapshot.candidate.machine;
    let (id, message, frozen) = operation(&f, client);
    remove(&f, &old);
    f.context.offer = crate::store::uuid_v4().unwrap();
    confirm(&f, &io);
    f.reopen();
    let mut store = f.store.lock().unwrap();
    let grant = store.talk(client, true).unwrap().unwrap();
    assert!(store.owned(&grant, &id, now_ms().unwrap()).is_err());
    assert_eq!(
        store
            .adopt(
                &grant,
                &message,
                Some(&frozen),
                &[],
                b"{}",
                now_ms().unwrap()
            )
            .unwrap_err()
            .code,
        "REMOTE_CLOSED"
    );
    let mut effects = 0;
    let result = store.effect(&grant, &id, now_ms, |_| effects += 1);
    let before: (String, Vec<u8>, String) = store
        .connection
        .query_row(
            "SELECT phase,frozen,member_id FROM operations WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    drop(store);
    assert!(
        result.is_err(),
        "new member released an old incarnation's operation"
    );
    assert_eq!(effects, 0);
    assert_eq!(before, ("uncertain".into(), frozen, old.member));
}

#[test]
fn same_member_operation_recovery_survives_sending_off_then_on() {
    let f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, original) = confirm(&f, &io);
    let client = &original.snapshot.candidate.machine;
    let (id, message, frozen) = operation(&f, client);
    f.reopen();
    let mut store = f.store.lock().unwrap();
    store.talk(client, false).unwrap();
    let grant = store.talk(client, true).unwrap().unwrap();
    assert!(grant.revision > original.grant_revision);
    assert_eq!(
        store.member_for_client(client).unwrap().unwrap().member_id,
        original.member
    );
    assert_eq!(
        store.owned(&grant, &id, now_ms().unwrap()).unwrap().phase,
        "uncertain"
    );
    assert_eq!(
        store
            .adopt(
                &grant,
                &message,
                Some(&frozen),
                &[],
                b"{}",
                now_ms().unwrap()
            )
            .unwrap()
            .id,
        id
    );
    let mut effects = 0;
    store.effect(&grant, &id, now_ms, |_| effects += 1).unwrap();
    assert_eq!(effects, 1);
}

#[test]
fn an_unmapped_operation_never_inherits_a_later_member_mapping() {
    let f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, original) = confirm(&f, &io);
    let client = &original.snapshot.candidate.machine;
    let (id, _, _) = operation(&f, client);
    let mut store = f.store.lock().unwrap();
    store
        .connection
        .execute("UPDATE operations SET member_id=NULL WHERE id=?1", [&id])
        .unwrap();
    let grant = store.grant(client).unwrap().unwrap();
    assert!(store.owned(&grant, &id, now_ms().unwrap()).is_err());
    let mut effects = 0;
    let result = store.effect(&grant, &id, now_ms, |_| effects += 1);
    drop(store);
    assert_eq!(result.unwrap_err().code, "REMOTE_CLOSED");
    assert_eq!(effects, 0);
}

#[test]
fn re_pairing_does_not_export_an_old_members_accepted_core_final() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    let client = &old.snapshot.candidate.machine;
    let (id, _, _) = operation(&f, client);
    {
        let mut store = f.store.lock().unwrap();
        let grant = store.grant(client).unwrap().unwrap();
        store
            .cloud_begin(client, &id, b"synthetic owned carrier", now_ms().unwrap())
            .unwrap();
        store.settle(&grant, &id, &serde_json::json!({"state":"accepted","operationId":id,"requestId":format!("req_{}",crate::store::uuid_v4().unwrap())}), None, now_ms().unwrap()).unwrap();
        assert_eq!(
            store.cloud_accepted_dispatches(client).unwrap().as_slice(),
            std::slice::from_ref(&id)
        );
        let tx = store.authorized(&grant, now_ms().unwrap()).unwrap();
        crate::cloud_inbox::finals::mapped_request(&tx, &grant, &id, now_ms().unwrap()).unwrap();
    }
    remove(&f, &old);
    f.context.offer = crate::store::uuid_v4().unwrap();
    confirm(&f, &io);
    f.reopen();
    let mut store = f.store.lock().unwrap();
    assert!(store.cloud_accepted_dispatches(client).unwrap().is_empty());
    let grant = store.grant(client).unwrap().unwrap();
    let tx = store.authorized(&grant, now_ms().unwrap()).unwrap();
    assert!(
        crate::cloud_inbox::finals::mapped_request(&tx, &grant, &id, now_ms().unwrap()).is_err()
    );
    let phase: String = tx
        .query_row("SELECT phase FROM operations WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(phase, "accepted");
}
