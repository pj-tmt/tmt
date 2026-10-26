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
