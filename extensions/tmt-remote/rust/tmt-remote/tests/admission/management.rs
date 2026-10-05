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
