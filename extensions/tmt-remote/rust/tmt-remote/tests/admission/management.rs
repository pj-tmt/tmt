//! Signed browser management with real storage and Session ownership, no core calls.
use super::OwnerDoor;
use super::core_fixture;
use serde_json::{Value, json};
use std::sync::Arc;
use tmt_remote::{
    devices::Devices,
    operations::Operations,
    pairing::now_ms,
    store::uuid_v4,
    transport::{LoopbackTransport, Transport},
};
fn app(owner: &OwnerDoor, core: &core_fixture::Core) -> Arc<Operations> {
    let devices = Arc::new(Devices::new(
        Arc::clone(&owner.store),
        Some(Arc::clone(&owner.sessions)),
    ));
    Arc::new(
        Arc::try_unwrap(core.operations())
            .ok()
            .unwrap()
            .with_management(devices),
    )
}
fn request(
    owner: &OwnerDoor,
    app: Arc<Operations>,
    session: &str,
    sequence: usize,
    operation: &str,
    input: Value,
) -> Value {
    let mut wire = owner.wire(
        session,
        &sequence.to_string(),
        operation,
        input.to_string().as_bytes(),
    );
    if let Some(id) = input["operationId"]
        .as_str()
        .filter(|_| operation != "remote.management.operation")
    {
        wire["id"] = json!(id);
        owner.resign(&mut wire);
    }
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 65536).with_operations(app);
    let bytes = transport
        .append(owner.request_origin(), &serde_json::to_vec(&wire).unwrap())
        .unwrap();
    owner.verify_reply(&serde_json::from_slice(&bytes).unwrap(), &wire)
}
#[test]
fn paired_browser_is_read_only_until_local_designation_and_agent_hold_never_applies() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let session = owner.open();
    let view = request(
        &owner,
        Arc::clone(&app),
        &session,
        1,
        "remote.settings.show",
        json!({}),
    );
    assert_eq!(view["capabilities"]["settingsWrite"], false);
    assert_eq!(view["settings"]["sessionsPerDevice"], "8");
    let id = uuid_v4().unwrap();
    let denied = request(
        &owner,
        Arc::clone(&app),
        &session,
        2,
        "remote.settings.set",
        json!({"operationId":id,"setting":"open","value":false}),
    );
    assert_eq!(denied["error"]["code"], "REMOTE_MANAGEMENT_READ_ONLY");
    assert!(!owner._root.0.join("remote/settings.json").exists());
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let saved = request(
        &owner,
        Arc::clone(&app),
        &session,
        3,
        "remote.settings.set",
        json!({"operationId":id,"setting":"open","value":false}),
    );
    assert_eq!(saved["state"], "committed");
    assert_eq!(saved["sessionEnded"], false);
    owner.store.lock().unwrap().undesignate().unwrap();
    let read = request(
        &owner,
        app,
        &session,
        4,
        "remote.management.operation",
        json!({"operationId":id}),
    );
    assert_eq!(read, saved);
    assert_eq!(core.sends(), 0);
    assert!(core.calls().is_empty());
}
#[test]
fn self_rename_ends_old_session_then_fresh_live_session_reads_original_receipt() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let committed = request(
        &owner,
        Arc::clone(&app),
        &session,
        1,
        "remote.devices.rename",
        json!({"operationId":id,"clientId":owner.grant.client_id,"name":"New name"}),
    );
    assert_eq!(committed["state"], "committed");
    assert_eq!(committed["sessionEnded"], true);
    let old = owner.wire(&session, "2", "remote.settings.show", b"{}");
    assert!(owner.admit(&old).is_err());
    let fresh = owner.open();
    assert_ne!(fresh, session);
    let read = request(
        &owner,
        Arc::clone(&app),
        &fresh,
        1,
        "remote.management.operation",
        json!({"operationId":id}),
    );
    assert_eq!(read, committed);
    let other = request(
        &owner,
        app,
        &fresh,
        2,
        "remote.management.operation",
        json!({"operationId":uuid_v4().unwrap()}),
    );
    assert_eq!(other["error"]["code"], "REMOTE_MANAGEMENT_UNAVAILABLE");
    assert!(core.calls().is_empty());
}
#[test]
fn self_revoke_commits_but_cannot_reopen_or_use_old_session_to_read_receipt() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let committed = request(
        &owner,
        app,
        &session,
        1,
        "remote.devices.revoke",
        json!({"operationId":id,"clientId":owner.grant.client_id}),
    );
    assert_eq!(committed["state"], "committed");
    assert_eq!(committed["sessionEnded"], true);
    let old = owner.wire(
        &session,
        "2",
        "remote.management.operation",
        json!({"operationId":id}).to_string().as_bytes(),
    );
    assert!(owner.admit(&old).is_err());
    let open = owner.wire(
        "new",
        "0",
        "session.open",
        json!({"clientNonce":uuid_v4().unwrap().replace('-',"")})
            .to_string()
            .as_bytes(),
    );
    assert!(
        owner
            .sessions
            .open(owner.request_origin(), &serde_json::to_vec(&open).unwrap())
            .is_none()
    );
    assert!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&owner.grant.client_id)
            .unwrap()
            .unwrap()
            .disabled
    );
    assert!(core.calls().is_empty());
}
#[test]
fn nonbrowser_agent_scopes_cannot_read_remote_management() {
    let owner = OwnerDoor::new();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let session = owner.open();
    let denied = request(&owner, app, &session, 1, "remote.settings.show", json!({}));
    assert_eq!(denied["error"]["code"], "REMOTE_INPUT_INVALID");
    assert!(core.calls().is_empty());
}

#[test]
fn another_key_on_the_same_origin_cannot_impersonate_the_designated_browser() {
    use ed25519_dalek::{Signer, SigningKey};
    use tmt_remote::canonical;
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let payload = json!({"operationId":id,"setting":"open","value":false}).to_string();
    let mut wire = owner.wire(&session, "1", "remote.settings.set", payload.as_bytes());
    wire["id"] = json!(id);
    let impostor = SigningKey::from_bytes(&[48; 32]);
    let signature = impostor
        .sign(&canonical::envelope(&super::test_envelope(&wire, payload.as_bytes())).unwrap());
    wire["signature"] = json!(canonical::base64url(&signature.to_bytes()));
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 65536)
        .with_operations(Arc::clone(&app));
    assert!(
        transport
            .append(owner.request_origin(), &serde_json::to_vec(&wire).unwrap())
            .is_err()
    );
    assert!(!owner._root.0.join("remote/settings.json").exists());
    owner.resign(&mut wire);
    let positive = transport
        .append(owner.request_origin(), &serde_json::to_vec(&wire).unwrap())
        .unwrap();
    let outcome = owner.verify_reply(&serde_json::from_slice(&positive).unwrap(), &wire);
    assert_eq!(outcome["state"], "committed");
    assert!(core.calls().is_empty());
}

#[test]
fn two_valid_paired_browsers_share_origin_but_not_management_designation() {
    let mut owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let first = owner.grant.clone();
    let first_key = owner.key.clone();
    let first_session = owner.open();
    owner
        .store
        .lock()
        .unwrap()
        .designate(&first.client_id, &first.origin, now_ms().unwrap())
        .unwrap();
    let mut second = first.clone();
    second.client_id = uuid_v4().unwrap();
    owner.key = super::SigningKey::from_bytes(&[48; 32]);
    second.public_key = owner.key.verifying_key().to_bytes();
    second.scopes = super::DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect();
    second.mode = "direct".into();
    owner.store.lock().unwrap().insert_grant(&second).unwrap();
    owner.grant = second.clone();
    let second_session = owner.open();
    let view = request(
        &owner,
        Arc::clone(&app),
        &second_session,
        1,
        "remote.settings.show",
        json!({}),
    );
    assert_eq!(view["capabilities"]["settingsWrite"], false);
    let page = request(
        &owner,
        Arc::clone(&app),
        &second_session,
        2,
        "remote.devices.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(page["devices"].as_array().unwrap().len(), 2);
    for (sequence, operation, input) in [
        (
            3,
            "remote.settings.set",
            json!({"operationId":uuid_v4().unwrap(),"setting":"open","value":false}),
        ),
        (
            4,
            "remote.devices.rename",
            json!({"operationId":uuid_v4().unwrap(),"clientId":first.client_id,"name":"Changed"}),
        ),
        (
            5,
            "remote.devices.revoke",
            json!({"operationId":uuid_v4().unwrap(),"clientId":first.client_id}),
        ),
    ] {
        assert_eq!(
            request(
                &owner,
                Arc::clone(&app),
                &second_session,
                sequence,
                operation,
                input
            )["error"]["code"],
            "REMOTE_MANAGEMENT_READ_ONLY"
        );
    }
    owner
        .store
        .lock()
        .unwrap()
        .designate(&second.client_id, &second.origin, now_ms().unwrap())
        .unwrap();
    let admitted = request(
        &owner,
        Arc::clone(&app),
        &second_session,
        6,
        "remote.settings.show",
        json!({}),
    );
    assert_eq!(admitted["capabilities"]["settingsWrite"], true);
    owner.grant = first.clone();
    owner.key = first_key;
    assert_eq!(
        request(
            &owner,
            Arc::clone(&app),
            &first_session,
            1,
            "remote.settings.set",
            json!({"operationId":uuid_v4().unwrap(),"setting":"open","value":false})
        )["error"]["code"],
        "REMOTE_MANAGEMENT_READ_ONLY"
    );
    owner.store.lock().unwrap().undesignate().unwrap();
    owner.grant = second.clone();
    owner.key = super::SigningKey::from_bytes(&[48; 32]);
    assert_eq!(
        request(
            &owner,
            app,
            &second_session,
            7,
            "remote.settings.set",
            json!({"operationId":uuid_v4().unwrap(),"setting":"open","value":false})
        )["error"]["code"],
        "REMOTE_MANAGEMENT_READ_ONLY"
    );
    let oracle =
        rusqlite::Connection::open(owner._serving.layout().directory.join("remote.db")).unwrap();
    let store = owner.store.lock().unwrap();
    for table in ["management_receipts", "operations"] {
        assert_eq!(
            oracle
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(store.grant(&first.client_id).unwrap().unwrap(), first);
    assert_eq!(store.grant(&second.client_id).unwrap().unwrap(), second);
    assert!(!owner._root.0.join("remote/settings.json").exists());
    assert!(core.calls().is_empty());
}

#[test]
fn response_sequence_publication_failure_preserves_committed_settings_receipt() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let session = owner.open();
    let oracle =
        rusqlite::Connection::open(owner._serving.layout().directory.join("remote.db")).unwrap();
    // Fault only publication after the durable committed receipt exists. The
    // fallback also cannot reserve a response; transport loss is expected.
    oracle.execute_batch("CREATE TRIGGER fail_management_publication BEFORE UPDATE OF next_server_sequence ON sessions WHEN EXISTS(SELECT 1 FROM management_receipts WHERE outcome LIKE '%committed%') BEGIN SELECT RAISE(ABORT,'injected response publication failure'); END;").unwrap();
    let id = uuid_v4().unwrap();
    let input = json!({"operationId":id,"setting":"open","value":false});
    let mut wire = owner.wire(
        &session,
        "1",
        "remote.settings.set",
        input.to_string().as_bytes(),
    );
    wire["id"] = json!(id);
    owner.resign(&mut wire);
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 65536)
        .with_operations(Arc::clone(&app));
    assert!(
        transport
            .append(owner.request_origin(), &serde_json::to_vec(&wire).unwrap())
            .is_err()
    );
    assert!(!tmt_remote::settings::read(&owner._root.0).unwrap().open());
    let outcome: String = oracle
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&outcome).unwrap()["state"],
        "committed"
    );
    oracle
        .execute_batch("DROP TRIGGER fail_management_publication")
        .unwrap();
    let read = request(
        &owner,
        app,
        &session,
        2,
        "remote.management.operation",
        json!({"operationId":id}),
    );
    assert_eq!(read["state"], "committed");
    assert_eq!(
        oracle
            .query_row("SELECT COUNT(*) FROM management_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(core.calls().is_empty());
}

#[test]
fn sending_toggle_requires_designation_then_recovers_its_own_original_after_self_change() {
    let owner = OwnerDoor::with_device(vec!["talk".into()], "hold", "all".into(), None, "browser");
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let input = json!({"operationId":id,"clientId":owner.grant.client_id,"enabled":false});
    let denied = request(
        &owner,
        Arc::clone(&app),
        &session,
        1,
        "remote.devices.talk",
        input.clone(),
    );
    assert_eq!(denied["error"]["code"], "REMOTE_MANAGEMENT_READ_ONLY");
    assert!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&owner.grant.client_id)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    owner
        .store
        .lock()
        .unwrap()
        .designate(
            &owner.grant.client_id,
            &owner.grant.origin,
            now_ms().unwrap(),
        )
        .unwrap();
    let result = request(
        &owner,
        Arc::clone(&app),
        &session,
        2,
        "remote.devices.talk",
        input,
    );
    assert_eq!(result["state"], "committed");
    assert_eq!(result["sessionEnded"], true);
    assert_eq!(result["result"]["device"]["talkEnabled"], false);
    let reopened = owner.open();
    let recovered = request(
        &owner,
        Arc::clone(&app),
        &reopened,
        1,
        "remote.management.operation",
        json!({"operationId":id}),
    );
    assert_eq!(recovered, result);
    let fresh = uuid_v4().unwrap();
    let enabled = request(
        &owner,
        app,
        &reopened,
        2,
        "remote.devices.talk",
        json!({"operationId":fresh,"clientId":owner.grant.client_id,"enabled":true}),
    );
    assert_eq!(enabled["result"]["device"]["talkEnabled"], true);
    assert!(
        core.calls().is_empty(),
        "management never dispatches agent work"
    );
}
