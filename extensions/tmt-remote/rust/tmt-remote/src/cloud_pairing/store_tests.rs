use super::*;
use crate::{
    state::Layout,
    store::{Store, uuid_v4},
};
use ed25519_dalek::Signer;

#[test]
fn reciprocal_join_persists_default_off_grant_and_never_resets_later_policy() {
    let root = std::env::temp_dir().join(format!("tmt-reciprocal-{}", uuid_v4().unwrap()));
    let layout = Layout::open(&root).unwrap();
    let serving = layout.serve_lock().unwrap();
    let mut store = Store::open(&serving).unwrap();
    let machine = store.machine().unwrap();
    let (challenge, _, _) = tests::fixture();
    let connection = Connection {
        version: 1,
        name: "paired".into(),
        owner: challenge.owner.clone(),
        owner_key: challenge.owner_key,
        config: serde_json::json!({"projectId":challenge.project}),
        peer: machine.id,
        uid: "local-uid".into(),
        request: vec![],
        confirmed: true,
        descriptor: vec![],
        reroute_token: None,
    };
    let grant = crate::store::Grant {
        client_id: connection.owner.clone(),
        public_key: connection.owner_key,
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Owner".into(),
        agents: "all".into(),
        scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: 1,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    let local_key = [7; 32];
    let confirmed = store
        .confirm_reciprocal(
            &connection,
            "owner-uid",
            &grant,
            &local_key,
            false,
            &[11; 32],
        )
        .unwrap();
    assert!(!confirmed.permits_scope("talk"));
    let route = store.cloud_route_changes().unwrap().pop().unwrap();
    assert_eq!(route.peer, connection.owner);
    assert_eq!(route.uid, "owner-uid");
    assert!(!route.revoked);
    assert_eq!(
        store.cloud_owner_configs().unwrap(),
        vec![connection.config.clone()]
    );
    store.talk(&connection.owner, true).unwrap();
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    let current = store
        .confirm_reciprocal(
            &connection,
            "owner-uid",
            &grant,
            &local_key,
            false,
            &[11; 32],
        )
        .unwrap();
    assert!(current.permits_scope("talk"));
    store.revoke(&connection.owner).unwrap();
    let revoked = store
        .confirm_reciprocal(
            &connection,
            "owner-uid",
            &grant,
            &local_key,
            true,
            &[11; 32],
        )
        .unwrap();
    assert!(
        revoked.disabled,
        "an identical join observation never restores revoked permission"
    );
    assert!(
        store
            .cloud_route_changes()
            .unwrap()
            .iter()
            .all(|route| route.revoked)
    );
    let mut substituted = grant;
    substituted.public_key = [0; 32];
    assert!(
        store
            .confirm_reciprocal(
                &connection,
                "owner-uid",
                &substituted,
                &local_key,
                false,
                &[11; 32]
            )
            .is_err()
    );
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

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
fn owner_confirmation_commits_reverse_capability_with_the_grant_and_keeps_it_after_receipt_expiry()
{
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let record = Connection {
        version: 1,
        name: "reverse".into(),
        owner: candidate.candidate.peer.clone(),
        owner_key: candidate.candidate.public_key,
        config: serde_json::json!({"projectId":candidate.challenge.project}),
        peer: candidate.challenge.owner.clone(),
        uid: "owner-uid".into(),
        request: vec![],
        confirmed: true,
        descriptor: vec![],
        reroute_token: None,
    };
    store.connection.execute_batch("CREATE TRIGGER refuse_connection BEFORE INSERT ON cloud_remotes BEGIN SELECT RAISE(FAIL,'fixture refuses connection'); END;").unwrap();
    assert!(
        store
            .confirm_cloud_peer(&candidate, &[8; 32], 1, true, Some(record.clone()))
            .is_err()
    );
    assert!(store.grants().unwrap().is_empty());
    assert!(store.cloud_route_changes().unwrap().is_empty());
    let receipts: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM cloud_pair_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(receipts, 0);
    store
        .connection
        .execute_batch("DROP TRIGGER refuse_connection;")
        .unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, true, Some(record.clone()))
        .unwrap();
    let expected = super::wire::reverse_reroute_token(
        &[8; 32],
        candidate.challenge(),
        &candidate.candidate.peer,
    )
    .unwrap();
    let saved = store.cloud_connection(&record.owner).unwrap().unwrap();
    assert_eq!(saved.reroute_token, Some(expected));
    assert!(saved.request.is_empty());
    store
        .prune_cloud_receipts(candidate.challenge.expires_at_ms)
        .unwrap();
    drop(store);
    let store = Store::open(&serving).unwrap();
    assert_eq!(
        store
            .cloud_connection(&record.owner)
            .unwrap()
            .unwrap()
            .reroute_token,
        Some(expected)
    );
    assert!(
        store
            .grant(&record.owner)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_private_connection_round_trips_without_adding_a_reroute_token() {
    let (root, serving, candidate) = world();
    let store = Store::open(&serving).unwrap();
    // Explicit pre-token private shape, not a historical release fixture.
    let legacy = serde_json::json!({
        "version":1,"name":"legacy","owner":candidate.challenge.owner,
        "owner_key":candidate.challenge.owner_key,"config":{},"peer":candidate.candidate.peer,
        "uid":"old-uid","request":[],"confirmed":true,"descriptor":[]
    })
    .to_string();
    store
        .connection
        .execute(
            "INSERT INTO cloud_remotes VALUES(?1,?2,?3)",
            rusqlite::params!["legacy", candidate.challenge.owner, legacy],
        )
        .unwrap();
    drop(store);
    let store = Store::open(&serving).unwrap();
    let loaded = store.cloud_connection("legacy").unwrap().unwrap();
    assert!(loaded.reroute_token.is_none());
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::from_str::<serde_json::Value>(&legacy).unwrap()
    );
    let retained: String = store
        .connection
        .query_row(
            "SELECT connection FROM cloud_remotes WHERE name='legacy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        retained, legacy,
        "reading does not rewrite or invent authority"
    );
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_twelve_peer_and_original_receipt_survive_the_token_migration_without_new_authority() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    let receipt = store
        .confirm_cloud_peer(&candidate, &key, 1, false, None)
        .unwrap();
    let mut legacy: serde_json::Value = serde_json::from_slice(&receipt.bytes).unwrap();
    legacy.as_object_mut().unwrap().remove("rerouteToken");
    let bytes = serde_json::to_vec(&legacy).unwrap();
    let proof = crypto::server_proof(&key, &bytes).unwrap();
    store
        .connection
        .execute(
            "UPDATE cloud_pair_receipts SET receipt=?1,proof=?2",
            rusqlite::params![bytes, proof.as_slice()],
        )
        .unwrap();
    let grant = store.grant(&candidate.candidate.peer).unwrap().unwrap();
    let machine = store.machine().unwrap();
    // Reconstruct the real schema-twelve boundary, including an enrolled peer.
    store.connection.execute_batch("ALTER TABLE cloud_peers DROP COLUMN reroute_hash; DELETE FROM _migrations WHERE version=13;").unwrap();
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    assert_eq!(store.machine().unwrap(), machine);
    assert_eq!(
        store.grant(&candidate.candidate.peer).unwrap().unwrap(),
        grant
    );
    let routes = store.cloud_route_changes().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].peer, candidate.candidate.peer);
    assert!(
        routes[0].reroute_hash.is_none(),
        "migration cannot mint a capability for an old peer"
    );
    let original = store
        .confirm_cloud_peer(&candidate, &key, 2, false, None)
        .unwrap();
    assert_eq!(original.bytes, bytes);
    assert_eq!(original.proof, proof);
    crypto::verify_server_proof(&key, &original.bytes, &original.proof).unwrap();
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn confirmed_receipt_carries_a_private_reroute_capability_bound_by_its_proof() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    let receipt = store
        .confirm_cloud_peer(&candidate, &key, 1, false, None)
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&receipt.bytes).unwrap();
    let token = crate::canonical::base64url_bytes(
        value["rerouteToken"]
            .as_str()
            .expect("the confirmed receipt must supply the private reroute capability"),
        32,
    )
    .unwrap();
    assert_ne!(token.as_slice(), key);
    crypto::verify_server_proof(&key, &receipt.bytes, &receipt.proof).unwrap();
    let mut changed = value;
    changed["rerouteToken"] = serde_json::json!(crate::canonical::base64url(&[0; 32]));
    assert!(
        crypto::verify_server_proof(&key, &serde_json::to_vec(&changed).unwrap(), &receipt.proof)
            .is_err()
    );
    drop(store);
    let mut store = Store::open(&serving).unwrap();
    assert_eq!(
        store
            .confirm_cloud_peer(&candidate, &key, 2, false, None)
            .unwrap(),
        receipt
    );
    drop(store);
    drop(serving);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn confirmed_receipt_and_read_only_peer_reopen_and_lost_join_keeps_original_receipt() {
    let (root, serving, candidate) = world();
    let mut store = Store::open(&serving).unwrap();
    let key = response_key(&[4; 16], &candidate.challenge, &candidate.candidate).unwrap();
    let receipt = store
        .confirm_cloud_peer(&candidate, &key, 1, false, None)
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
            .confirm_cloud_peer(&candidate, &key, 2, false, None)
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
            .confirm_cloud_peer(
                &candidate,
                &key,
                candidate.challenge.expires_at_ms,
                false,
                None
            )
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
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
            .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
            .is_err()
    );
    assert!(store.grants().unwrap().is_empty());
    assert!(store.cloud_route_changes().unwrap().is_empty());
    store
        .connection
        .execute_batch("DROP TRIGGER refuse_route")
        .unwrap();
    store
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
        .confirm_cloud_peer(&candidate, &[8; 32], 1, false, None)
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
    let receipt = store
        .confirm_cloud_peer(&candidate, &key, 1, true, None)
        .unwrap();
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
        .confirm_cloud_peer(&candidate, &key, 1, false, None)
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
