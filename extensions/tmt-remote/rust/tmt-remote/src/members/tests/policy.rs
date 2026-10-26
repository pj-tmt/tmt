//! The shared writer preserves identity, machine names and narrower grant policy.
use super::*;
use crate::members::policy::{Intent, Target};

fn target(f: &Fixture, grant: &Grant) -> Target {
    let m = f
        .store
        .member_for_client(&grant.client_id)
        .unwrap()
        .unwrap();
    Target {
        remote_id: m.remote_id,
        member_id: m.member_id,
        member_revision: m.revision,
    }
}
fn apply(
    f: &mut Fixture,
    actor: &Grant,
    intent: &Intent,
) -> Result<Change, crate::error::RemoteError> {
    let tx = f
        .store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let change = intent.prepare(&tx, actor, 10)?.apply(10)?;
    tx.commit().unwrap();
    Ok(change)
}
#[test]
fn member_can_rename_only_own_person_and_cannot_grant_sending() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin machine");
    let user = f.browser("User machine");
    let other = f.browser("Other machine");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let intent = Intent::Rename {
        target: target(&f, &user),
        name: "Person".into(),
    };
    let change = apply(&mut f, &user, &intent).unwrap();
    assert_eq!(change.member.display_name, "Person");
    assert!(change.clients.is_empty());
    assert_eq!(
        f.store.grant(&user.client_id).unwrap().unwrap().name,
        "User machine"
    );
    assert_eq!(
        f.store.grant(&user.client_id).unwrap().unwrap().revision,
        user.revision
    );
    let person = f.store.member_for_client(&user.client_id).unwrap().unwrap();
    f.store.rename(&user.client_id, "Renamed machine").unwrap();
    assert_eq!(
        f.store.member_for_client(&user.client_id).unwrap().unwrap(),
        person
    );

    for intent in [
        Intent::Rename {
            target: target(&f, &other),
            name: "Stolen".into(),
        },
        Intent::Sending {
            target: target(&f, &user),
            enabled: true,
        },
        Intent::Revoke {
            target: target(&f, &other),
        },
    ] {
        let before = f.store.members().unwrap();
        assert_eq!(
            apply(&mut f, &user, &intent).unwrap_err().code,
            "REMOTE_MEMBER_OWNER_REQUIRED"
        );
        assert_eq!(f.store.members().unwrap(), before);
    }
}
#[test]
fn admin_mutations_share_m1_writers_and_revision_fences() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let rename = Intent::Rename {
        target: target(&f, &user),
        name: "Updated person".into(),
    };
    apply(&mut f, &admin, &rename).unwrap();
    assert_eq!(
        apply(&mut f, &admin, &rename).unwrap_err().code,
        "REMOTE_MEMBER_CONFLICT"
    );
    let sending = Intent::Sending {
        target: target(&f, &user),
        enabled: true,
    };
    let change = apply(&mut f, &admin, &sending).unwrap();
    assert_eq!(
        change.clients.as_slice(),
        std::slice::from_ref(&user.client_id)
    );
    let g = f.store.grant(&user.client_id).unwrap().unwrap();
    assert!(g.permits_scope("talk"));
    assert_eq!(g.mode, user.mode);
    assert_eq!(g.agents, user.agents);
    let revoke = Intent::Revoke {
        target: target(&f, &user),
    };
    let change = apply(&mut f, &admin, &revoke).unwrap();
    assert!(change.member.revoked_at_ms.is_some());
    assert!(f.store.grant(&user.client_id).unwrap().unwrap().disabled);
    assert!(!change.member.sending_enabled);
}
#[test]
fn strict_member_intent_refuses_cross_remote_and_extra_null_fields() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let mut t = target(&f, &user);
    t.remote_id = uuid_v4().unwrap();
    assert_eq!(
        apply(
            &mut f,
            &admin,
            &Intent::Rename {
                target: t,
                name: "Other remote".into()
            }
        )
        .unwrap_err()
        .code,
        "REMOTE_MEMBER_OWNER_REQUIRED"
    );
    let t = target(&f, &user);
    let value = serde_json::json!({"operationId":uuid_v4().unwrap(),"remoteId":t.remote_id,"memberId":t.member_id,"memberRevision":t.member_revision,"name":null});
    assert!(crate::members::policy::parse("remote.members.revoke", &value).is_err());
}
