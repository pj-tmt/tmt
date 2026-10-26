//! A new owner-confirmed invitation never revives a removed member's authority.
use super::{durable_fixture::DurableFixture, *};
use crate::{members::Owner, pairing::now_ms};

pub(super) fn confirm(f: &DurableFixture, io: &IoBudget<'_>) -> (Snapshot, Committed) {
    let backend = Mailbox::new(f.context.binding.clone());
    let (coordinator, _, snapshot) = f.pending(backend, f.sink.clone(), io);
    coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            now_ms().unwrap(),
            io,
        )
        .unwrap();
    let original = f.sink.original(&snapshot).unwrap().unwrap();
    (snapshot, original)
}

pub(super) fn remove(f: &DurableFixture, original: &Committed) {
    let mut store = f.store.lock().unwrap();
    let member = store
        .member_for_client(&original.snapshot.candidate.machine)
        .unwrap()
        .unwrap();
    store
        .revoke_member(
            Owner::LocalOwner,
            &member.member_id,
            member.revision,
            now_ms().unwrap(),
        )
        .unwrap();
}

#[test]
fn removed_machine_reinvitation_gets_new_member_receipt_and_revision_without_reviving_history() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (old_snapshot, old) = confirm(&f, &io);
    let client = &old_snapshot.candidate.machine;
    let session = crate::store::uuid_v4().unwrap();
    let window = crate::store::uuid_v4().unwrap();
    let before = {
        let mut store = f.store.lock().unwrap();
        let grant = store.grant(client).unwrap().unwrap();
        store
            .start_session(&grant, &session, &window, now_ms().unwrap())
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO cloud_peer_routes VALUES('demo-old','old-uid',?1,1,0,0)",
                [client],
            )
            .unwrap();
        store.connection.execute("INSERT INTO operations(id,client_id,operation,digest,frozen,phase,receipt,updated_ms,references_json,session_id,grant_revision,member_id) VALUES('old-operation',?1,'dispatch.create',?2,NULL,'refused','{}',1,'[]',?3,?4,?5)", rusqlite::params![client,[9_u8;32].as_slice(),session,grant.revision as i64,old.member]).unwrap();
        grant
    };
    remove(&f, &old);
    let tombstone = f
        .store
        .lock()
        .unwrap()
        .member_for_client(client)
        .unwrap()
        .unwrap();
    assert!(tombstone.revoked_at_ms.is_some());
    let disabled = f.store.lock().unwrap().grant(client).unwrap().unwrap();
    assert!(disabled.disabled);
    f.context.offer = crate::store::uuid_v4().unwrap();
    let (_, new) = confirm(&f, &io);
    assert_ne!(new.member, old.member);
    assert!(new.grant_revision > disabled.revision);
    assert_eq!(f.counts(), (1, 2, 1, 2));
    assert!(f.sink.original(&old_snapshot).unwrap().as_ref() == Some(&old));
    assert!(matches!(
        f.sink.claim(&old, Part::Receipt).unwrap(),
        Claim::Stale
    ));
    assert!(!f.sink.acknowledge(&old, Part::Receipt).unwrap());
    f.reopen();
    let mut store = f.store.lock().unwrap();
    let grant = store.grant(client).unwrap().unwrap();
    assert_eq!(grant.revision, new.grant_revision);
    assert!(!grant.disabled);
    assert!(!grant.permits_scope("talk"));
    assert_eq!(grant.mode, "direct");
    assert_eq!(
        store.member_for_client(client).unwrap().unwrap().member_id,
        new.member
    );
    assert_eq!(
        store
            .members()
            .unwrap()
            .into_iter()
            .find(|m| m.member_id == old.member)
            .unwrap(),
        tombstone
    );
    assert_eq!(
        new.receipt,
        crate::pairing::receipt(&grant, &f.context.owner, &f.context.owner_key).unwrap()
    );
    assert_eq!(
        store
            .consume_sequence(client, &session, &window, 1, now_ms().unwrap())
            .unwrap_err()
            .code,
        "REMOTE_CLOSED"
    );
    assert_eq!(
        store
            .start_session(
                &before,
                &crate::store::uuid_v4().unwrap(),
                &window,
                now_ms().unwrap()
            )
            .unwrap_err()
            .code,
        "REMOTE_CLOSED"
    );
    let route: (bool, bool) = store
        .connection
        .query_row(
            "SELECT revoked,pending FROM cloud_peer_routes WHERE client_id=?1",
            [client],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(route, (true, true));
    let operation: (i64, String, String) = store
        .connection
        .query_row(
            "SELECT grant_revision,phase,receipt FROM operations WHERE id='old-operation'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        operation,
        (before.revision as i64, "refused".into(), "{}".into())
    );
    store
        .start_session(
            &grant,
            &crate::store::uuid_v4().unwrap(),
            &window,
            now_ms().unwrap(),
        )
        .unwrap();
}

#[test]
fn active_machine_reinvitation_refuses_without_changing_member_or_receipt() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    f.context.offer = crate::store::uuid_v4().unwrap();
    let (coordinator, _, snapshot) =
        f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
    let error = coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            now_ms().unwrap(),
            &io,
        )
        .err()
        .expect("active grant must refuse");
    assert_eq!(error.code, "REMOTE_ALREADY_PAIRED");
    assert_eq!(f.counts(), (1, 1, 1, 1));
    f.reopen();
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .member_for_client(&snapshot.candidate.machine)
            .unwrap()
            .unwrap()
            .member_id,
        old.member
    );
}

#[test]
fn reinvitation_receipt_failure_rolls_back_the_replacement_and_association() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    remove(&f, &old);
    let (grant, member) = {
        let store = f.store.lock().unwrap();
        (
            store
                .grant(&old.snapshot.candidate.machine)
                .unwrap()
                .unwrap(),
            store
                .member_for_client(&old.snapshot.candidate.machine)
                .unwrap()
                .unwrap(),
        )
    };
    f.context.offer = crate::store::uuid_v4().unwrap();
    f.store.lock().unwrap().connection.execute_batch("CREATE TRIGGER fail_reinvite_receipt BEFORE INSERT ON management_receipts BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END").unwrap();
    let (coordinator, _, snapshot) =
        f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
    assert!(
        coordinator
            .confirm(
                &OwnerActor::local_interactive(),
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io
            )
            .is_err()
    );
    assert_eq!(f.counts(), (1, 1, 1, 1));
    f.reopen();
    {
        let store = f.store.lock().unwrap();
        assert_eq!(store.grant(&grant.client_id).unwrap(), Some(grant));
        assert_eq!(
            store
                .member_for_client(&snapshot.candidate.machine)
                .unwrap(),
            Some(member)
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_reinvite_receipt")
            .unwrap();
    }
    f.context.offer = crate::store::uuid_v4().unwrap();
    confirm(&f, &io);
    assert_eq!(f.counts(), (1, 2, 1, 2));
}

#[test]
fn concurrent_machine_revoke_and_reinvite_keep_one_serial_outcome() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    remove(&f, &old);
    f.context.offer = crate::store::uuid_v4().unwrap();
    let (coordinator, _, snapshot) =
        f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
    let start = Barrier::new(3);
    std::thread::scope(|scope| {
        let confirmation = scope.spawn(|| {
            start.wait();
            coordinator.confirm(
                &OwnerActor::local_interactive(),
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io,
            )
        });
        let revoke = scope.spawn(|| {
            start.wait();
            f.store
                .lock()
                .unwrap()
                .revoke(&snapshot.candidate.machine)
                .unwrap()
        });
        start.wait();
        confirmation.join().unwrap().unwrap();
        revoke.join().unwrap();
    });
    assert_eq!(f.counts(), (1, 2, 1, 2));
    f.reopen();
    let store = f.store.lock().unwrap();
    let grant = store.grant(&snapshot.candidate.machine).unwrap().unwrap();
    let member = store
        .member_for_client(&snapshot.candidate.machine)
        .unwrap()
        .unwrap();
    assert_ne!(member.member_id, old.member);
    assert_eq!(grant.disabled, member.revoked_at_ms.is_some());
    assert_eq!(grant.revision, if grant.disabled { 4 } else { 3 });
    drop(store);
    assert!(matches!(
        f.sink.claim(&old, Part::Receipt).unwrap(),
        Claim::Stale
    ));
}

#[test]
fn new_confirmation_uses_the_newly_verified_key_not_the_revoked_key() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    remove(&f, &old);
    f.context.offer = crate::store::uuid_v4().unwrap();
    let mut candidate = old.snapshot.candidate.clone();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[45; 32]);
    candidate.public_key = signing.verifying_key().to_bytes();
    candidate.name = "Reinvited machine".into();
    let (coordinator, _, snapshot) = f.pending_candidate(
        Mailbox::new(f.context.binding.clone()),
        f.sink.clone(),
        candidate.clone(),
        signing,
        &io,
    );
    coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            now_ms().unwrap(),
            &io,
        )
        .unwrap();
    f.reopen();
    let store = f.store.lock().unwrap();
    let grant = store.grant(&candidate.machine).unwrap().unwrap();
    assert_eq!(grant.public_key, candidate.public_key);
    assert_ne!(grant.public_key, old.snapshot.candidate.public_key);
    assert_eq!(grant.name, candidate.name);
    assert_eq!(grant.revision, 3);
    assert_eq!(grant.mode, "direct");
    assert_eq!(
        grant.scopes,
        crate::store::DEFAULT_SCOPES.map(str::to_owned)
    );
}

#[test]
fn reinvitation_reserves_the_last_revision_for_revocation() {
    for previous in [9_007_199_254_740_989_u64, 9_007_199_254_740_990] {
        let mut f = DurableFixture::new();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (_, old) = confirm(&f, &io);
        remove(&f, &old);
        f.store
            .lock()
            .unwrap()
            .connection
            .execute("UPDATE grants SET revision=?1", [previous as i64])
            .unwrap();
        f.context.offer = crate::store::uuid_v4().unwrap();
        let (coordinator, _, snapshot) =
            f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
        let result = coordinator.confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            now_ms().unwrap(),
            &io,
        );
        if previous == 9_007_199_254_740_989 {
            result.unwrap();
            remove(&f, &f.sink.original(&snapshot).unwrap().unwrap());
            f.reopen();
            let grant = f
                .store
                .lock()
                .unwrap()
                .grant(&snapshot.candidate.machine)
                .unwrap()
                .unwrap();
            assert!(grant.disabled);
            assert_eq!(grant.revision, 9_007_199_254_740_991);
        } else {
            assert_eq!(
                result.err().expect("revision exhaustion must refuse").code,
                "REMOTE_STATE_UNAVAILABLE"
            );
            assert_eq!(f.counts(), (1, 1, 1, 1));
            f.reopen();
            let grant = f
                .store
                .lock()
                .unwrap()
                .grant(&snapshot.candidate.machine)
                .unwrap()
                .unwrap();
            assert!(grant.disabled);
            assert_eq!(grant.revision, previous);
        }
    }
}

#[test]
fn reinviting_one_removed_machine_does_not_change_its_former_members_other_machine() {
    let mut f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, old) = confirm(&f, &io);
    let client = &old.snapshot.candidate.machine;
    let (other, grouped) = {
        let mut store = f.store.lock().unwrap();
        let mut other = store.grant(client).unwrap().unwrap();
        other.client_id = crate::store::uuid_v4().unwrap();
        other.public_key = ed25519_dalek::SigningKey::from_bytes(&[46; 32])
            .verifying_key()
            .to_bytes();
        store.insert_grant(&other).unwrap();
        let group = store.member_for_client(&other.client_id).unwrap().unwrap();
        let request = crate::members::Association {
            operation_id: crate::store::uuid_v4().unwrap(),
            remote_id: group.remote_id,
            member_id: group.member_id.clone(),
            member_revision: group.revision,
            client_id: client.clone(),
            grant_revision: old.grant_revision,
        };
        store
            .associate_member(Owner::LocalOwner, &request, now_ms().unwrap())
            .unwrap();
        store.revoke(client).unwrap();
        (other, store.member_for_client(client).unwrap().unwrap())
    };
    assert!(grouped.revoked_at_ms.is_none());
    f.context.offer = crate::store::uuid_v4().unwrap();
    let (_, new) = confirm(&f, &io);
    assert_ne!(new.member, grouped.member_id);
    f.reopen();
    let store = f.store.lock().unwrap();
    assert_eq!(store.grant(&other.client_id).unwrap(), Some(other.clone()));
    assert_eq!(
        store.member_for_client(&other.client_id).unwrap(),
        Some(grouped.clone())
    );
    assert_eq!(
        store.member_clients(&grouped.member_id).unwrap(),
        [other.client_id]
    );
    assert_eq!(
        store.member_for_client(client).unwrap().unwrap().member_id,
        new.member
    );
}
