//! Actual Store projections, caller authority and bounded cursor isolation.
use super::*;
use crate::members::service::{self, Caller, Page};

fn page(cursor: Option<String>, limit: usize) -> Page {
    Page { cursor, limit }
}

#[test]
fn roster_administrator_is_designation_not_role_label() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    let other = f.browser("Other");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let full = service::roster(&f.store, Caller::Browser(&admin), page(None, 50), 10).unwrap();
    assert!(full.administrator);
    assert_eq!(full.members.len(), 3);
    let own = service::roster(&f.store, Caller::Browser(&user), page(None, 50), 10).unwrap();
    assert!(!own.administrator);
    assert_eq!(own.members.len(), 1);
    assert_eq!(own.members[0].member_id, user.client_id);
    assert!(
        service::machines(
            &f.store,
            Caller::Browser(&user),
            &other.client_id,
            page(None, 50),
            10
        )
        .is_err()
    );
    // Association with an owner person does not designate another machine.
    let request = f.request(&admin, &user);
    f.store
        .associate_member(Owner::LocalOwner, &request, 10)
        .unwrap();
    let user = f.store.grant(&user.client_id).unwrap().unwrap();
    let own = service::roster(&f.store, Caller::Browser(&user), page(None, 50), 10).unwrap();
    assert_eq!(own.members[0].role, Role::Owner);
    assert!(!own.administrator);
    assert_eq!(own.members.len(), 1);
    assert!(
        service::machines(
            &f.store,
            Caller::Browser(&user),
            &other.client_id,
            page(None, 50),
            10
        )
        .is_err()
    );
    assert_eq!(
        service::roster(&f.store, Caller::LocalOwner, page(None, 50), 10)
            .unwrap()
            .members
            .len(),
        3
    );
}
#[test]
fn roster_and_machine_pages_bound_materialization_and_bind_cursors() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    let other = f.browser("Other");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let one = service::roster(&f.store, Caller::Browser(&admin), page(None, 1), 10).unwrap();
    assert_eq!(one.members.len(), 1);
    let cursor = one.next_cursor.unwrap();
    assert!(
        service::roster(
            &f.store,
            Caller::Browser(&user),
            page(Some(cursor.clone()), 1),
            10
        )
        .is_err()
    );
    assert!(
        service::roster(
            &f.store,
            Caller::LocalOwner,
            page(Some(cursor.clone()), 1),
            10
        )
        .is_err()
    );
    assert!(
        service::machines(
            &f.store,
            Caller::Browser(&admin),
            &user.client_id,
            page(Some(cursor.clone()), 1),
            10
        )
        .is_err()
    );
    assert_eq!(
        service::roster(
            &f.store,
            Caller::Browser(&admin),
            page(Some(cursor), 50),
            10
        )
        .unwrap()
        .members
        .len(),
        2
    );
    for limit in [0, 51, usize::MAX] {
        assert!(service::roster(&f.store, Caller::LocalOwner, page(None, limit), 10).is_err());
    }
    let request = f.request(&user, &other);
    f.store
        .associate_member(Owner::LocalOwner, &request, 10)
        .unwrap();
    let first = service::machines(
        &f.store,
        Caller::Browser(&admin),
        &user.client_id,
        page(None, 1),
        10,
    )
    .unwrap();
    assert_eq!(first.machines.len(), 1);
    let next = first.next_cursor.unwrap();
    assert!(
        service::machines(
            &f.store,
            Caller::Browser(&admin),
            &admin.client_id,
            page(Some(next.clone()), 1),
            10
        )
        .is_err()
    );
    let second = service::machines(
        &f.store,
        Caller::Browser(&admin),
        &user.client_id,
        page(Some(next), 1),
        10,
    )
    .unwrap();
    assert_eq!(second.machines.len(), 1);
    assert_ne!(first.machines[0].client_id, second.machines[0].client_id);
    assert!(second.next_cursor.is_none());
}
#[test]
fn changed_key_revision_designation_and_revoke_do_not_disclose_old_scope() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let mut wrong = admin.clone();
    wrong.public_key = [255; 32];
    assert!(service::roster(&f.store, Caller::Browser(&wrong), page(None, 50), 10).is_err());
    f.store
        .designate(&user.client_id, &user.origin, 10)
        .unwrap();
    let old = service::roster(&f.store, Caller::Browser(&admin), page(None, 50), 10).unwrap();
    assert!(!old.administrator);
    assert_eq!(old.members.len(), 1);
    f.store.revoke(&admin.client_id).unwrap();
    assert!(service::roster(&f.store, Caller::Browser(&admin), page(None, 50), 10).is_err());
}

#[test]
fn disclosure_revalidation_refuses_a_full_roster_after_designation_changes() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let user = f.browser("User");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let view = service::roster(&f.store, Caller::Browser(&admin), page(None, 50), 10).unwrap();
    f.store
        .designate(&user.client_id, &user.origin, 10)
        .unwrap();
    assert!(service::validate_roster(&f.store, Caller::Browser(&admin), &view, 10).is_err());
}

#[test]
fn member_summary_counts_people_and_retains_recorded_use_across_revoke_and_restart() {
    let mut f = Fixture::new();
    let private_root = f.root.canonicalize().unwrap();
    assert_eq!(f.store.data_root.canonicalize().unwrap(), private_root);
    assert!(private_root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let admin = f.browser("Admin");
    let person = f.browser("Person");
    let second = f.browser("Second machine");
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let before = service::summary(&f.store, Caller::Browser(&admin), 10).unwrap();
    assert_eq!(before.member_count, Some(3));
    assert_eq!(before.last_active_at_ms, None);
    let association = f.request(&person, &second);
    f.store
        .associate_member(Owner::LocalOwner, &association, 10)
        .unwrap();
    for (client, time) in [
        (&admin.client_id, 100),
        (&person.client_id, 200),
        (&second.client_id, 300),
    ] {
        f.store
            .connection
            .execute(
                "INSERT INTO device_last_use VALUES (?1,?2)",
                params![client, time],
            )
            .unwrap();
    }
    let view = service::summary(&f.store, Caller::Browser(&admin), 10).unwrap();
    assert_eq!(view.member_count, Some(2));
    assert_eq!(view.last_active_at_ms, Some(300));
    let member = f
        .store
        .member_for_client(&person.client_id)
        .unwrap()
        .unwrap();
    f.store
        .revoke_member(Owner::LocalOwner, &member.member_id, member.revision, 20)
        .unwrap();
    f.reopen();
    let after = service::summary(&f.store, Caller::Browser(&admin), 21).unwrap();
    assert_eq!(after.member_count, Some(1));
    // Accepted historical use is not erased or presented as a live Session observation.
    assert_eq!(after.last_active_at_ms, Some(300));
}

#[test]
fn member_summary_keeps_unauthorized_facts_null_and_rechecks_disclosure() {
    let mut f = Fixture::new();
    let admin = f.browser("Admin");
    let other = f.browser("Other");
    let private = service::summary(&f.store, Caller::Browser(&admin), 10).unwrap();
    assert_eq!(private.member_count, None);
    assert_eq!(private.last_active_at_ms, None);
    f.store
        .designate(&admin.client_id, &admin.origin, 10)
        .unwrap();
    let view = service::summary(&f.store, Caller::Browser(&admin), 10).unwrap();
    service::validate_summary(&f.store, Caller::Browser(&admin), &view, 10).unwrap();
    f.store
        .designate(&other.client_id, &other.origin, 10)
        .unwrap();
    assert!(service::validate_summary(&f.store, Caller::Browser(&admin), &view, 10).is_err());
    let now_private = service::summary(&f.store, Caller::Browser(&admin), 10).unwrap();
    assert_eq!(now_private.member_count, None);
    assert_eq!(now_private.last_active_at_ms, None);
    for changed in ["key", "revision", "origin"] {
        let mut stale = admin.clone();
        match changed {
            "key" => stale.public_key = [255; 32],
            "revision" => stale.revision += 1,
            _ => stale.origin.push_str("/other"),
        }
        assert!(service::summary(&f.store, Caller::Browser(&stale), 10).is_err());
    }
    f.store.revoke(&admin.client_id).unwrap();
    assert!(service::summary(&f.store, Caller::Browser(&admin), 10).is_err());
}
