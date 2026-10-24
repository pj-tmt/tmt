use super::*;
use crate::{
    state::Layout,
    store::{Store, uuid_v4},
};
use ed25519_dalek::Signer;

fn world() -> (std::path::PathBuf, crate::state::Serving, VerifiedCandidate) {
    let root = std::env::temp_dir().join(format!("tmt-cloud-pair-{}", uuid_v4().unwrap()));
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let candidate = tests::verified();
    let store = Store::open(&serving).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO machine VALUES(1,?1,'/r/abcdefghijklmnop')",
            [&candidate.challenge.owner],
        )
        .unwrap();
    drop(store);
    (root, serving, candidate)
}

#[test]
fn confirmed_receipt_and_read_only_peer_reopen_and_lost_join_keeps_original_receipt() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    let receipt = store
        .confirm_cloud_peer(&candidate, &key, 1, false)
        .unwrap();
    crypto::verify_server_proof(&key, &receipt.bytes, &receipt.proof).unwrap();
    let grant = store.grant(&candidate.candidate.peer).unwrap().unwrap();
    assert!(!grant.permits_scope("talk"));
    assert_eq!(
        grant.scopes,
        crate::store::DEFAULT_SCOPES.map(str::to_owned)
    );
    store.talk(&grant.client_id, true).unwrap();
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    assert_eq!(
        store
            .confirm_cloud_peer(&candidate, &key, 2, false)
            .unwrap(),
        receipt
    );
    assert!(
        store
            .grant(&grant.client_id)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    assert_eq!(store.cloud_route_changes().unwrap().len(), 1);
    assert!(
        store
            .confirm_cloud_peer(&candidate, &key, candidate.challenge.expires_at_ms, false)
            .is_err()
    );
    assert_eq!(store.grants().unwrap().len(), 1);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn signed_uid_change_keeps_permissions_and_revokes_every_old_route_across_restart() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    store.talk(&candidate.candidate.peer, true).unwrap();
    let before = store.grant(&candidate.candidate.peer).unwrap().unwrap();
    let generation = store
        .next_cloud_reroute_generation(&candidate.challenge.owner)
        .unwrap();
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    assert!(
        store
            .next_cloud_reroute_generation(&candidate.challenge.owner)
            .unwrap()
            > generation
    );
    let (request, key) = tests::reroute_fixture(generation, "new-sender-uid");
    let signature = key.sign(&reroute(&request).unwrap()).to_bytes();
    let verified = verify_reroute(
        &request,
        &request.uid,
        &key.verifying_key().to_bytes(),
        &signature,
        request.timestamp_ms,
    )
    .unwrap();
    store
        .apply_cloud_reroute(&candidate.challenge.project, &verified, 3)
        .unwrap();
    assert_eq!(store.grant(&request.peer).unwrap().unwrap(), before);
    assert!(
        store
            .apply_cloud_reroute(&candidate.challenge.project, &verified, 4)
            .is_err()
    );
    let routes = store.cloud_route_changes().unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes.iter().filter(|r| !r.revoked).count(), 1);
    assert!(
        routes
            .iter()
            .find(|r| r.uid == candidate.candidate.uid)
            .unwrap()
            .revoked
    );
    assert_eq!(routes.iter().find(|r| !r.revoked).unwrap().uid, request.uid);
    let (mut request, key) = tests::reroute_fixture(generation + 5, "third-uid");
    let signature = key.sign(&reroute(&request).unwrap()).to_bytes();
    let verified = verify_reroute(
        &request,
        &request.uid,
        &key.verifying_key().to_bytes(),
        &signature,
        request.timestamp_ms,
    )
    .unwrap();
    store
        .apply_cloud_reroute(&candidate.challenge.project, &verified, 4)
        .unwrap();
    request.generation += 1;
    let signature = key.sign(&reroute(&request).unwrap()).to_bytes();
    let verified = verify_reroute(
        &request,
        &request.uid,
        &key.verifying_key().to_bytes(),
        &signature,
        request.timestamp_ms,
    )
    .unwrap();
    store.revoke(&request.peer).unwrap();
    assert!(
        store
            .apply_cloud_reroute(&candidate.challenge.project, &verified, 5)
            .is_err()
    );
    drop(store);
    let store = Store::open(&serving).unwrap();
    assert!(
        store
            .cloud_route_changes()
            .unwrap()
            .iter()
            .all(|r| r.revoked)
    );
    assert!(store.grant(&request.peer).unwrap().unwrap().disabled);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_publication_ack_cannot_hide_a_new_route_or_revocation() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    let old = store.cloud_route_changes().unwrap().remove(0);
    store.revoke(&candidate.candidate.peer).unwrap();
    store.cloud_route_published(&old).unwrap();
    let pending = store.cloud_route_changes().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].revoked);
    store.cloud_route_published(&pending[0]).unwrap();
    assert!(store.cloud_route_changes().unwrap().is_empty());
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn confirmation_failure_rolls_back_the_grant_and_routing_together() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    store.connection.execute_batch("CREATE TRIGGER refuse_route BEFORE INSERT ON cloud_peer_routes BEGIN SELECT RAISE(ABORT,'fixture route failure'); END;").unwrap();
    assert!(
        store
            .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
            .is_err()
    );
    assert!(store.grants().unwrap().is_empty());
    assert!(store.cloud_route_changes().unwrap().is_empty());
    store
        .connection
        .execute_batch("DROP TRIGGER refuse_route")
        .unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    assert_eq!(store.grants().unwrap().len(), 1);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn lost_join_after_code_erasure_reads_only_the_exact_confirmed_receipt() {
    let (root, serving, candidate) = world();
    let (challenge, original, key) = tests::fixture();
    let mac = crypto::enrollment_mac(&[4; 16], &enrollment(&challenge, &original).unwrap());
    let signature = key
        .sign(
            &crate::canonical::possession(&enrollment(&challenge, &original).unwrap(), &mac)
                .unwrap(),
        )
        .to_bytes();
    let mut store = Store::open(&serving).unwrap();
    assert!(
        store
            .cloud_join_receipt(&challenge, &original, &original.uid, &mac, &signature, 1)
            .is_err()
    );
    assert!(store.grants().unwrap().is_empty());
    let receipt = store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    drop(store);
    let store = Store::open(&serving).unwrap();
    assert_eq!(
        store
            .cloud_join_receipt(&challenge, &original, &original.uid, &mac, &signature, 2)
            .unwrap(),
        receipt
    );
    assert!(
        store
            .cloud_join_receipt(&challenge, &original, "wrong-uid", &mac, &signature, 2)
            .is_err()
    );
    assert!(
        store
            .cloud_join_receipt(
                &challenge,
                &original,
                &original.uid,
                &mac,
                &signature,
                challenge.expires_at_ms
            )
            .is_err()
    );
    let mut changed = original;
    changed.name = "Changed name".into();
    let bytes =
        crate::canonical::possession(&enrollment(&challenge, &changed).unwrap(), &mac).unwrap();
    let signature = key.sign(&bytes).to_bytes();
    assert!(
        store
            .cloud_join_receipt(&challenge, &changed, &changed.uid, &mac, &signature, 2)
            .is_err()
    );
    assert_eq!(store.grants().unwrap().len(), 1);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_expiry_owner_pin_and_native_key_are_rechecked_before_route_swap() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    let before = store.cloud_route_changes().unwrap();
    let (request, key) = tests::reroute_fixture(2, "new-uid");
    let mut foreign = request.clone();
    foreign.owner_key = ed25519_dalek::SigningKey::from_bytes(&[10; 32])
        .verifying_key()
        .to_bytes();
    let signature = key.sign(&reroute(&foreign).unwrap()).to_bytes();
    let proof = verify_reroute(
        &foreign,
        &foreign.uid,
        &key.verifying_key().to_bytes(),
        &signature,
        foreign.timestamp_ms,
    )
    .unwrap();
    assert!(
        store
            .apply_cloud_reroute(&candidate.challenge.project, &proof, 2)
            .is_err()
    );
    let wrong_key = ed25519_dalek::SigningKey::from_bytes(&[11; 32]);
    let signature = wrong_key.sign(&reroute(&request).unwrap()).to_bytes();
    let proof = verify_reroute(
        &request,
        &request.uid,
        &wrong_key.verifying_key().to_bytes(),
        &signature,
        request.timestamp_ms,
    )
    .unwrap();
    assert!(
        store
            .apply_cloud_reroute(&candidate.challenge.project, &proof, 2)
            .is_err()
    );
    let signature = key.sign(&reroute(&request).unwrap()).to_bytes();
    let proof = verify_reroute(
        &request,
        &request.uid,
        &key.verifying_key().to_bytes(),
        &signature,
        request.timestamp_ms,
    )
    .unwrap();
    store
        .connection
        .execute(
            "UPDATE grants SET expires_at_ms=2 WHERE client_id=?1",
            [&request.peer],
        )
        .unwrap();
    assert!(
        store
            .apply_cloud_reroute(&candidate.challenge.project, &proof, 2)
            .is_err()
    );
    assert_eq!(store.cloud_route_changes().unwrap(), before);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn additive_migration_preserves_schema_nine_identity_key_and_sending_policy() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false)
        .unwrap();
    let before = store
        .talk(&candidate.candidate.peer, true)
        .unwrap()
        .unwrap();
    let machine = store.machine().unwrap();
    let key = crate::state::MachineKey::open(serving.layout())
        .unwrap()
        .public();
    // Simulated schema-nine boundary, not an assertion about a released build.
    store.connection.execute_batch("DROP TABLE cloud_remotes; DROP TABLE cloud_owner_projects; DROP TABLE cloud_reroute_counters; DROP TABLE cloud_peer_routes; DROP TABLE cloud_pair_receipts; DROP TABLE cloud_peers; DROP TABLE cloud_transport; DELETE FROM _migrations WHERE version>=10;").unwrap();
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    assert_eq!(store.machine().unwrap(), machine);
    assert_eq!(
        store.grant(&candidate.candidate.peer).unwrap().unwrap(),
        before
    );
    assert_eq!(
        crate::state::MachineKey::open(serving.layout())
            .unwrap()
            .public(),
        key
    );
    assert!(store.cloud_route_changes().unwrap().is_empty());
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn one_ceremony_can_enable_talk_and_receipt_cleanup_never_changes_trust() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    let receipt = store.confirm_cloud_peer(&candidate, &key, 1, true).unwrap();
    let grant = store.grant(&candidate.candidate.peer).unwrap().unwrap();
    assert!(grant.permits_scope("talk"));
    let decoded: serde_json::Value = serde_json::from_slice(&receipt.bytes).unwrap();
    assert!(
        decoded["grant"]["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "talk")
    );
    store
        .prune_cloud_receipts(candidate.challenge.expires_at_ms - 1)
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM cloud_pair_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    store
        .prune_cloud_receipts(candidate.challenge.expires_at_ms)
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM cloud_pair_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store.grant(&grant.client_id).unwrap().unwrap().revision,
        grant.revision
    );
    assert_eq!(store.cloud_route_changes().unwrap().len(), 1);
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn person_management_requires_a_unique_name_and_accepts_an_exact_client_id() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    store
        .confirm_cloud_peer(&candidate, &key, 1, false)
        .unwrap();
    let peer = candidate.candidate.peer.clone();
    let name = candidate.candidate.name.clone();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(store));
    let devices = crate::devices::Devices::new(shared.clone(), None);
    assert!(devices.talk(&name, true).unwrap().permits_scope("talk"));
    let mut other = shared.lock().unwrap().grant(&peer).unwrap().unwrap();
    other.client_id = uuid_v4().unwrap();
    other.public_key = ed25519_dalek::SigningKey::from_bytes(&[6; 32])
        .verifying_key()
        .to_bytes();
    shared.lock().unwrap().insert_grant(&other).unwrap();
    assert_eq!(
        devices.talk(&name, false).unwrap_err().code,
        "REMOTE_DEVICE_AMBIGUOUS"
    );
    assert_eq!(
        devices.revoke(&name).unwrap_err().code,
        "REMOTE_DEVICE_AMBIGUOUS"
    );
    assert!(
        shared
            .lock()
            .unwrap()
            .grant(&peer)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    assert!(devices.revoke(&peer).unwrap().disabled);
    assert_eq!(
        devices.talk(&name, false).unwrap().client_id,
        other.client_id
    );
    drop(devices);
    drop(shared);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}
