//! Real Store lifecycle/failure controls for new member-bound immutable outcomes.
use super::*;
use crate::members::policy::{Intent, Target};

fn target(f: &Fixture, grant: &Grant) -> Target {
    let member = f
        .store
        .member_for_client(&grant.client_id)
        .unwrap()
        .unwrap();
    Target {
        remote_id: member.remote_id,
        member_id: member.member_id,
        member_revision: member.revision,
    }
}
fn other(f: &mut Fixture) -> Grant {
    let mut grant = f.grant.clone();
    grant.client_id = uuid_v4().unwrap();
    grant.public_key = [29; 32];
    grant.name = "Other machine".into();
    grant.scopes.retain(|s| s != "talk");
    f.store.insert_grant(&grant).unwrap();
    grant
}
fn run(f: &mut Fixture, id: &str, op: &str, intent: Intent, digest: &[u8]) -> Result<Value> {
    if let Some(original) = f.store.management_adopt(
        &f.grant,
        id,
        op,
        digest,
        now_ms().unwrap(),
        Access::Member(&intent),
    )? {
        return Ok(original);
    }
    f.store
        .management_effect(&f.grant, id, op, digest, Mutation::Member(intent))
        .map(|(outcome, _)| outcome)
}
#[test]
fn member_original_survives_revision_change_and_reopen_but_not_a_new_incarnation() {
    let mut f = Fixture::new();
    let intent = Intent::Rename {
        target: target(&f, &f.grant),
        name: "Person".into(),
    };
    let id = uuid_v4().unwrap();
    let original = run(
        &mut f,
        &id,
        "remote.members.rename",
        intent.clone(),
        &[1; 32],
    )
    .unwrap();
    assert_eq!(original["state"], "committed");
    assert_eq!(original["result"]["member"]["displayName"], "Person");
    assert_eq!(
        run(
            &mut f,
            &id,
            "remote.members.rename",
            intent.clone(),
            &[1; 32]
        )
        .unwrap(),
        original
    );
    assert_eq!(
        run(&mut f, &id, "remote.members.rename", intent, &[2; 32])
            .unwrap_err()
            .code,
        "REMOTE_INTENT_CONFLICT"
    );
    f.store.connection = Connection::open_in_memory().unwrap();
    f.store = Store::open(f.serving.as_ref().unwrap()).unwrap();
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap(),
        original
    );
    let old = f
        .store
        .member_for_client(&f.grant.client_id)
        .unwrap()
        .unwrap();
    f.store.revoke(&f.grant.client_id).unwrap();
    let mut candidate = f.grant.clone();
    candidate.public_key = [31; 32];
    let renewed = f.store.confirm_member_grant(&candidate).unwrap();
    assert_ne!(renewed.member_id, old.member_id);
    assert_eq!(
        receipt(
            &f.store.connection,
            &candidate.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap_err()
        .code,
        "REMOTE_MANAGEMENT_UNAVAILABLE"
    );
    let raw: String = f
        .store
        .connection
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    let stored: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["actorMemberId"], old.member_id);
    assert_eq!(stored["outcome"], original);
}
#[test]
fn refused_member_original_never_retries_when_its_stale_revision_later_matches() {
    let mut f = Fixture::new();
    f.designate();
    let grant = other(&mut f);
    let mut t = target(&f, &grant);
    t.member_revision += 1;
    let intent = Intent::Rename {
        target: t,
        name: "Must not appear".into(),
    };
    let id = uuid_v4().unwrap();
    let original = run(
        &mut f,
        &id,
        "remote.members.rename",
        intent.clone(),
        &[3; 32],
    )
    .unwrap();
    assert_eq!(original["state"], "refused");
    assert_eq!(original["reason"], "REMOTE_MEMBER_CONFLICT");
    let member = f
        .store
        .member_for_client(&grant.client_id)
        .unwrap()
        .unwrap();
    f.store
        .set_member_sending(
            crate::members::Owner::LocalOwner,
            &member.member_id,
            member.revision,
            true,
            now_ms().unwrap(),
        )
        .unwrap();
    assert_eq!(
        run(&mut f, &id, "remote.members.rename", intent, &[3; 32]).unwrap(),
        original
    );
    assert_eq!(
        f.store
            .member_for_client(&grant.client_id)
            .unwrap()
            .unwrap()
            .display_name,
        "Other machine"
    );
}
#[test]
fn losing_designation_after_adoption_cannot_apply_or_change_the_original_unknown() {
    let mut f = Fixture::new();
    f.designate();
    let grant = other(&mut f);
    let intent = Intent::Sending {
        target: target(&f, &grant),
        enabled: true,
    };
    let id = uuid_v4().unwrap();
    f.store
        .management_adopt(
            &f.grant,
            &id,
            "remote.members.sending",
            &[4; 32],
            now_ms().unwrap(),
            Access::Member(&intent),
        )
        .unwrap();
    f.store.undesignate().unwrap();
    let error = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.members.sending",
            &[4; 32],
            Mutation::Member(intent),
        )
        .unwrap_err();
    assert_eq!(error.code, "REMOTE_MEMBER_OWNER_REQUIRED");
    assert!(
        !f.store
            .grant(&grant.client_id)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap()["state"],
        "unknown"
    );
}
#[test]
fn failed_member_settlement_rolls_back_name_sending_and_revoke_in_the_same_transaction() {
    for operation in [
        "remote.members.rename",
        "remote.members.sending",
        "remote.members.revoke",
    ] {
        let mut f = Fixture::new();
        f.designate();
        let grant = other(&mut f);
        let before = f
            .store
            .member_for_client(&grant.client_id)
            .unwrap()
            .unwrap();
        let target = target(&f, &grant);
        let intent = match operation {
            "remote.members.rename" => Intent::Rename {
                target,
                name: "Not committed".into(),
            },
            "remote.members.sending" => Intent::Sending {
                target,
                enabled: true,
            },
            _ => Intent::Revoke { target },
        };
        let id = uuid_v4().unwrap();
        f.store
            .management_adopt(
                &f.grant,
                &id,
                operation,
                &[5; 32],
                now_ms().unwrap(),
                Access::Member(&intent),
            )
            .unwrap();
        f.store.connection.execute_batch("CREATE TRIGGER fail_member_settlement BEFORE UPDATE OF outcome ON management_receipts BEGIN SELECT RAISE(ABORT,'member settlement interrupted'); END;").unwrap();
        assert!(
            f.store
                .management_effect(&f.grant, &id, operation, &[5; 32], Mutation::Member(intent))
                .is_err()
        );
        assert_eq!(
            f.store
                .member_for_client(&grant.client_id)
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(f.store.grant(&grant.client_id).unwrap().unwrap(), grant);
        assert_eq!(
            receipt(
                &f.store.connection,
                &f.grant.client_id,
                &id,
                now_ms().unwrap()
            )
            .unwrap()["state"],
            "unknown"
        );
        assert_eq!(f.count("audit"), 1);
    }
}
#[test]
fn remote_name_before_touch_is_refused_but_after_touch_sql_failure_stays_unknown() {
    let mut f = Fixture::new();
    f.designate();
    let remote = f.store.machine().unwrap().id;
    let id = uuid_v4().unwrap();
    f.store
        .management_adopt(
            &f.grant,
            &id,
            "remote.name.set",
            &[6; 32],
            now_ms().unwrap(),
            Access::RemoteName(&remote),
        )
        .unwrap();
    std::fs::create_dir(f.root.join("remote/settings.json")).unwrap();
    let (refused, _) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.name.set",
            &[6; 32],
            Mutation::RemoteName {
                remote: remote.clone(),
                name: "Not written".into(),
            },
        )
        .unwrap();
    assert_eq!(refused["state"], "refused");
    assert_eq!(refused["reason"], "REMOTE_SETTINGS_UNAVAILABLE");
    std::fs::remove_dir(f.root.join("remote/settings.json")).unwrap();
    let id = uuid_v4().unwrap();
    f.store
        .management_adopt(
            &f.grant,
            &id,
            "remote.name.set",
            &[7; 32],
            now_ms().unwrap(),
            Access::RemoteName(&remote),
        )
        .unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER fail_name_settlement BEFORE UPDATE OF outcome ON management_receipts BEGIN SELECT RAISE(ABORT,'name settlement interrupted'); END;").unwrap();
    assert!(
        f.store
            .management_effect(
                &f.grant,
                &id,
                "remote.name.set",
                &[7; 32],
                Mutation::RemoteName {
                    remote: remote.clone(),
                    name: "Written before SQL failure".into()
                }
            )
            .is_err()
    );
    assert_eq!(
        settings::read(&f.root).unwrap().remote_name(),
        Some("Written before SQL failure")
    );
    let original = f
        .store
        .management_adopt(
            &f.grant,
            &id,
            "remote.name.set",
            &[7; 32],
            now_ms().unwrap(),
            Access::RemoteName(&remote),
        )
        .unwrap()
        .unwrap();
    assert_eq!(original["state"], "unknown");
    std::fs::write(
        f.root.join("remote/settings.json"),
        r#"{"remoteName":"Later external change"}"#,
    )
    .unwrap();
    assert_eq!(
        f.store
            .management_adopt(
                &f.grant,
                &id,
                "remote.name.set",
                &[7; 32],
                now_ms().unwrap(),
                Access::RemoteName(&remote)
            )
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        settings::read(&f.root).unwrap().remote_name(),
        Some("Later external change")
    );
}

#[test]
fn newly_bound_legacy_operation_cannot_be_read_by_a_repaired_member() {
    let mut f = Fixture::new();
    f.designate();
    let id = uuid_v4().unwrap();
    f.adopt(&id, "remote.devices.rename", &[8; 32]).unwrap();
    let (original, _) = f
        .store
        .management_effect(
            &f.grant,
            &id,
            "remote.devices.rename",
            &[8; 32],
            Mutation::Rename {
                client: f.grant.client_id.clone(),
                name: "Machine".into(),
            },
        )
        .unwrap();
    let actor = f
        .store
        .member_for_client(&f.grant.client_id)
        .unwrap()
        .unwrap();
    let stored: String = f
        .store
        .connection
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap()["actorMemberId"],
        actor.member_id
    );
    assert_eq!(
        receipt(
            &f.store.connection,
            &f.grant.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap(),
        original
    );
    f.store.revoke(&f.grant.client_id).unwrap();
    let mut candidate = f.grant.clone();
    candidate.public_key = [91; 32];
    f.store.confirm_member_grant(&candidate).unwrap();
    assert_eq!(
        receipt(
            &f.store.connection,
            &candidate.client_id,
            &id,
            now_ms().unwrap()
        )
        .unwrap_err()
        .code,
        "REMOTE_MANAGEMENT_UNAVAILABLE"
    );
    let after: String = f
        .store
        .connection
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, stored);
}
#[test]
fn historical_unbound_bytes_remain_exact_but_never_cross_a_new_member_or_association() {
    for associate in [false, true] {
        let mut f = Fixture::new();
        f.designate();
        let id = uuid_v4().unwrap();
        f.adopt(&id, "remote.devices.rename", &[9; 32]).unwrap();
        let historical = json!({"operationId":id,"state":"committed","result":{"device":summary(&f.grant)},"sessionEnded":false});
        let bytes = historical.to_string();
        f.store
            .connection
            .execute(
                "UPDATE management_receipts SET outcome=?2 WHERE id=?1",
                params![id, bytes],
            )
            .unwrap();
        let legacy = f
            .store
            .member_for_client(&f.grant.client_id)
            .unwrap()
            .unwrap();
        assert_eq!(legacy.member_id, f.grant.client_id);
        assert_eq!(
            receipt(
                &f.store.connection,
                &f.grant.client_id,
                &id,
                now_ms().unwrap()
            )
            .unwrap(),
            historical
        );
        if associate {
            let target = other(&mut f);
            let member = f
                .store
                .member_for_client(&target.client_id)
                .unwrap()
                .unwrap();
            f.store
                .associate_member(
                    crate::members::Owner::LocalOwner,
                    &crate::members::Association {
                        operation_id: uuid_v4().unwrap(),
                        remote_id: member.remote_id,
                        member_id: member.member_id,
                        member_revision: member.revision,
                        client_id: f.grant.client_id.clone(),
                        grant_revision: f.grant.revision,
                    },
                    now_ms().unwrap(),
                )
                .unwrap();
        } else {
            f.store.revoke(&f.grant.client_id).unwrap();
            let mut candidate = f.grant.clone();
            candidate.public_key = [92; 32];
            f.store.confirm_member_grant(&candidate).unwrap();
        }
        assert_eq!(
            receipt(
                &f.store.connection,
                &f.grant.client_id,
                &id,
                now_ms().unwrap()
            )
            .unwrap_err()
            .code,
            "REMOTE_MANAGEMENT_UNAVAILABLE"
        );
        let after: String = f
            .store
            .connection
            .query_row(
                "SELECT outcome FROM management_receipts WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, bytes);
    }
}
