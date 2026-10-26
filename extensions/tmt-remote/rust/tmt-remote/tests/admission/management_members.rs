//! Public signed member flows, original outcomes and disclosure through the actual Session owner.
use super::*;

fn other(owner: &OwnerDoor) -> tmt_remote::store::Grant {
    let mut grant = owner.grant.clone();
    grant.client_id = uuid_v4().unwrap();
    grant.public_key = ed25519_dalek::SigningKey::from_bytes(&[17; 32])
        .verifying_key()
        .to_bytes();
    grant.name = "Other machine".into();
    owner.store.lock().unwrap().insert_grant(&grant).unwrap();
    grant
}
fn target(owner: &OwnerDoor, client: &str) -> Value {
    let member = owner
        .store
        .lock()
        .unwrap()
        .member_for_client(client)
        .unwrap()
        .unwrap();
    json!({"operationId":uuid_v4().unwrap(),"remoteId":member.remote_id,"memberId":member.member_id,"memberRevision":member.revision})
}
#[test]
fn member_reads_are_self_only_and_person_rename_has_read_only_original_recovery() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let other = other(&owner);
    let session = owner.open();
    let roster = request(
        &owner,
        app.clone(),
        &session,
        1,
        "remote.members.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(roster["administrator"], false);
    assert_eq!(roster["members"].as_array().unwrap().len(), 1);
    let member = roster["members"][0]["memberId"].as_str().unwrap();
    let machines = request(
        &owner,
        app.clone(),
        &session,
        2,
        "remote.members.machines",
        json!({"memberId":member,"cursor":null,"limit":50}),
    );
    assert_eq!(machines["machines"].as_array().unwrap().len(), 1);
    assert_eq!(machines["machines"][0]["liveSessionCount"], 1);
    assert!(machines["machines"][0]["lastActivityAtMs"].is_number());
    let old = request(
        &owner,
        app.clone(),
        &session,
        3,
        "remote.devices.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(old["devices"].as_array().unwrap().len(), 1);
    let mut change = target(&owner, &owner.grant.client_id);
    change["name"] = json!("Person one");
    let saved = request(
        &owner,
        app.clone(),
        &session,
        4,
        "remote.members.rename",
        change.clone(),
    );
    assert_eq!(saved["state"], "committed");
    assert_eq!(saved["result"]["member"]["displayName"], "Person one");
    assert_eq!(saved["sessionEnded"], false);
    assert_eq!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&owner.grant.client_id)
            .unwrap()
            .unwrap()
            .name,
        owner.grant.name
    );
    let mut next = target(&owner, &owner.grant.client_id);
    next["name"] = json!("Person two");
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            5,
            "remote.members.rename",
            next
        )["state"],
        "committed"
    );
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            6,
            "remote.members.rename",
            change.clone()
        ),
        saved
    );
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            7,
            "remote.management.operation",
            json!({"operationId":change["operationId"]})
        ),
        saved
    );
    assert_eq!(
        owner
            .store
            .lock()
            .unwrap()
            .member_for_client(&owner.grant.client_id)
            .unwrap()
            .unwrap()
            .display_name,
        "Person two"
    );
    let mut denied = target(&owner, &other.client_id);
    denied["name"] = json!("Not mine");
    let id = denied["operationId"].clone();
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            8,
            "remote.members.rename",
            denied
        )["error"]["code"],
        "REMOTE_MEMBER_OWNER_REQUIRED"
    );
    assert_eq!(
        request(
            &owner,
            app,
            &session,
            9,
            "remote.management.operation",
            json!({"operationId":id})
        )["error"]["code"],
        "REMOTE_MANAGEMENT_UNAVAILABLE"
    );
    assert_eq!(core.sends(), 0);
}
#[test]
fn administrator_names_remote_and_any_member_without_giving_members_administration() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let other = other(&owner);
    let session = owner.open();
    let rename =
        json!({"operationId":uuid_v4().unwrap(),"remoteId":owner.machine,"name":"Shared Remote"});
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            1,
            "remote.name.set",
            rename.clone()
        )["error"]["code"],
        "REMOTE_MEMBER_OWNER_REQUIRED"
    );
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
        app.clone(),
        &session,
        2,
        "remote.name.set",
        rename.clone(),
    );
    assert_eq!(saved["state"], "committed");
    assert_eq!(saved["result"]["remote"]["displayName"], "Shared Remote");
    let roster = request(
        &owner,
        app.clone(),
        &session,
        3,
        "remote.members.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(roster["administrator"], true);
    assert_eq!(roster["displayName"], "Shared Remote");
    assert_eq!(roster["members"].as_array().unwrap().len(), 2);
    let mut person = target(&owner, &other.client_id);
    person["name"] = json!("Other person");
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            4,
            "remote.members.rename",
            person
        )["state"],
        "committed"
    );
    assert_eq!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&other.client_id)
            .unwrap()
            .unwrap()
            .name,
        "Other machine"
    );
    owner.store.lock().unwrap().undesignate().unwrap();
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            5,
            "remote.management.operation",
            json!({"operationId":rename["operationId"]})
        ),
        saved
    );
    let private = request(
        &owner,
        app.clone(),
        &session,
        6,
        "remote.members.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(private["administrator"], false);
    assert_eq!(private["members"].as_array().unwrap().len(), 1);
    let id = target(&owner, &other.client_id)["memberId"].clone();
    assert_eq!(
        request(
            &owner,
            app,
            &session,
            7,
            "remote.members.machines",
            json!({"memberId":id,"cursor":null,"limit":50})
        )["error"]["code"],
        "REMOTE_MEMBER_OWNER_REQUIRED"
    );
    assert_eq!(core.sends(), 0);
}

struct Snapshots {
    store: std::sync::Arc<std::sync::Mutex<tmt_remote::store::Store>>,
    scopes: std::sync::Mutex<Vec<Option<String>>>,
}
impl tmt_remote::members::observations::MemberObservationSource for Snapshots {
    fn invitation(&self) -> tmt_remote::members::observations::MemberInvitationView {
        use tmt_remote::members::observations::*;
        assert!(
            self.store.try_lock().is_ok(),
            "coordinator observation must precede Store guards"
        );
        MemberInvitationView::Current {
            expires_at_ms: Some(now_ms().unwrap() + 1000),
            phase: MemberInvitePhase::Unconfirmed,
            local_confirmation_known: true,
            cleanup: MemberInviteCleanup::Unconfirmed,
        }
    }
    fn routes(
        &self,
        _: &tmt_remote::store::Store,
        member: Option<&str>,
    ) -> tmt_remote::members::observations::MemberRouteView {
        self.scopes.lock().unwrap().push(member.map(str::to_owned));
        tmt_remote::members::observations::MemberRouteView::NotRecorded
    }
}
#[test]
fn member_observations_are_scoped_and_taken_without_store_lock_or_provider_calls() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let other = other(&owner);
    let session = owner.open();
    let snapshots = Arc::new(Snapshots {
        store: owner.store.clone(),
        scopes: std::sync::Mutex::new(Vec::new()),
    });
    let app = Arc::new(
        Arc::try_unwrap(app(&owner, &core))
            .ok()
            .unwrap()
            .with_member_observations(snapshots.clone()),
    );
    let own = request(
        &owner,
        app.clone(),
        &session,
        1,
        "remote.members.list",
        json!({"cursor":null,"limit":50}),
    );
    assert!(own["invitation"].is_null());
    assert_eq!(own["routes"]["state"], "not-recorded");
    let id = owner
        .store
        .lock()
        .unwrap()
        .member_for_client(&owner.grant.client_id)
        .unwrap()
        .unwrap()
        .member_id;
    assert_eq!(snapshots.scopes.lock().unwrap().as_slice(), &[Some(id)]);
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
    let full = request(
        &owner,
        app,
        &session,
        2,
        "remote.members.list",
        json!({"cursor":null,"limit":50}),
    );
    assert_eq!(full["invitation"]["state"], "current");
    assert_eq!(full["invitation"]["phase"], "unconfirmed");
    assert_eq!(full["invitation"]["localConfirmationKnown"], true);
    assert_eq!(full["invitation"]["cleanup"], "unconfirmed");
    assert_eq!(snapshots.scopes.lock().unwrap().last(), Some(&None));
    let devices = Devices::new(owner.store.clone(), None);
    let local = tmt_remote::members::reads::local(
        &devices,
        snapshots.as_ref(),
        "remote.members.list",
        &json!({"cursor":null,"limit":50}),
    )
    .unwrap();
    assert_eq!(local["administrator"], true);
    let member = target(&owner, &other.client_id)["memberId"].clone();
    let machines = tmt_remote::members::reads::local(
        &devices,
        snapshots.as_ref(),
        "remote.members.machines",
        &json!({"memberId":member,"cursor":null,"limit":50}),
    )
    .unwrap();
    assert!(machines["machines"][0]["liveSessionCount"].is_null());
    assert!(machines["machines"][0]["lastActivityAtMs"].is_null());
    assert_eq!(core.sends(), 0);
}

#[test]
fn signed_member_sending_and_revoke_keep_policy_and_original_outcomes_in_one_journal() {
    let owner = OwnerDoor::browser();
    let core = core_fixture::Core::new();
    let app = app(&owner, &core);
    let other = other(&owner);
    let session = owner.open();
    let mut sending = target(&owner, &other.client_id);
    sending["enabled"] = json!(true);
    let denied_id = sending["operationId"].as_str().unwrap().to_owned();
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            1,
            "remote.members.sending",
            sending.clone()
        )["error"]["code"],
        "REMOTE_MEMBER_OWNER_REQUIRED"
    );
    assert_eq!(
        request(
            &owner,
            app.clone(),
            &session,
            2,
            "remote.management.operation",
            json!({"operationId":denied_id})
        )["error"]["code"],
        "REMOTE_MANAGEMENT_UNAVAILABLE"
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
    sending["operationId"] = json!(uuid_v4().unwrap());
    let saved = request(
        &owner,
        app.clone(),
        &session,
        3,
        "remote.members.sending",
        sending.clone(),
    );
    assert_eq!(saved["state"], "committed");
    assert_eq!(saved["result"]["member"]["sendingEnabled"], true);
    assert_eq!(saved["sessionEnded"], false);
    assert!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&other.client_id)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    let revoke = target(&owner, &other.client_id);
    let removed = request(
        &owner,
        app.clone(),
        &session,
        4,
        "remote.members.revoke",
        revoke.clone(),
    );
    assert_eq!(removed["state"], "committed");
    assert_eq!(removed["result"]["member"]["sendingEnabled"], false);
    assert!(removed["result"]["member"]["revokedAtMs"].is_number());
    assert!(
        owner
            .store
            .lock()
            .unwrap()
            .grant(&other.client_id)
            .unwrap()
            .unwrap()
            .disabled
    );
    let original = request(
        &owner,
        app.clone(),
        &session,
        5,
        "remote.management.operation",
        json!({"operationId":sending["operationId"]}),
    );
    assert_eq!(
        original, saved,
        "the sending receipt remains historical after the target is revoked"
    );
    assert_eq!(
        request(&owner, app, &session, 6, "remote.members.revoke", revoke),
        removed
    );
    assert_eq!(core.sends(), 0);
}
