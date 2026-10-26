use super::*;
use crate::members::terminal_machines::{self, MemberMachineMutation as Mutation};
#[test]
fn exact_machine_removal_and_rename_do_not_change_another_machine_or_person_name() {
    let mut f = Fixture::new();
    let first = f.browser("Person");
    let second = f.browser("Laptop");
    let association = f.request(&first, &second);
    f.store
        .associate_member(Owner::LocalOwner, &association, 10)
        .unwrap();
    let before = f.store.grant(&first.client_id).unwrap().unwrap();
    let target = terminal_machines::resolve(&f.store, &second.client_id).unwrap();
    let (renamed, _) = f
        .store
        .terminal_machine_effect(
            &Mutation::Rename {
                target: target.clone(),
                name: "Travel laptop".into(),
            },
            None,
        )
        .unwrap();
    assert_eq!(renamed["changed"], true);
    assert_eq!(
        f.store
            .member_for_client(&second.client_id)
            .unwrap()
            .unwrap()
            .display_name,
        "Person"
    );
    let (removed, clients) = f
        .store
        .terminal_machine_effect(&Mutation::Remove { target }, None)
        .unwrap();
    assert_eq!(removed["changed"], true);
    assert_eq!(clients, vec![second.client_id.clone()]);
    assert_eq!(f.store.grant(&first.client_id).unwrap().unwrap(), before);
    assert!(
        f.store
            .member_for_client(&first.client_id)
            .unwrap()
            .unwrap()
            .revoked_at_ms
            .is_none()
    );
    let target = terminal_machines::resolve(&f.store, &second.client_id).unwrap();
    let (again, clients) = f
        .store
        .terminal_machine_effect(&Mutation::Remove { target }, None)
        .unwrap();
    assert_eq!(again["changed"], false);
    assert!(clients.is_empty());
}
#[test]
fn machine_target_fences_both_grant_and_member_revisions() {
    let mut f = Fixture::new();
    let g = f.browser("Person");
    let old = terminal_machines::resolve(&f.store, &g.client_id).unwrap();
    f.store
        .set_member_sending(
            Owner::LocalOwner,
            &old.member.member_id,
            old.member.member_revision,
            true,
            10,
        )
        .unwrap();
    assert!(
        f.store
            .terminal_machine_effect(&Mutation::Remove { target: old }, None)
            .is_err()
    );
    let mut target = terminal_machines::resolve(&f.store, &g.client_id).unwrap();
    target.grant_revision -= 1;
    assert!(
        f.store
            .terminal_machine_effect(
                &Mutation::Rename {
                    target,
                    name: "Changed".into()
                },
                None
            )
            .is_err()
    );
    assert!(!f.store.grant(&g.client_id).unwrap().unwrap().disabled);
}
#[test]
fn terminal_designation_uses_the_exact_live_browser_and_fences_removal() {
    let mut f = Fixture::new();
    let g = f.browser("Owner browser");
    let target = terminal_machines::resolve(&f.store, &g.client_id).unwrap();
    assert!(
        f.store
            .terminal_machine_effect(
                &Mutation::Designate {
                    target: target.clone()
                },
                Some("http://127.0.0.1:9999")
            )
            .is_err()
    );
    let (set, _) = f
        .store
        .terminal_machine_effect(
            &Mutation::Designate {
                target: target.clone(),
            },
            Some(&g.origin),
        )
        .unwrap();
    assert_eq!(set["designatedClientId"], g.client_id);
    assert_eq!(set["changed"], true);
    assert_eq!(
        f.store
            .member_for_client(&g.client_id)
            .unwrap()
            .unwrap()
            .role,
        Role::Owner
    );
    assert!(
        f.store
            .terminal_machine_effect(
                &Mutation::Undesignate {
                    expected: Some(target)
                },
                None
            )
            .is_err()
    );
    let current = terminal_machines::resolve(&f.store, &g.client_id).unwrap();
    let (same, _) = f
        .store
        .terminal_machine_effect(
            &Mutation::Designate {
                target: current.clone(),
            },
            Some(&g.origin),
        )
        .unwrap();
    assert_eq!(same["changed"], false);
    let (removed, _) = f
        .store
        .terminal_machine_effect(
            &Mutation::Undesignate {
                expected: Some(current),
            },
            None,
        )
        .unwrap();
    assert_eq!(removed["changed"], true);
    assert!(removed["designatedClientId"].is_null());
    assert_eq!(
        f.store
            .member_for_client(&g.client_id)
            .unwrap()
            .unwrap()
            .role,
        Role::User
    );
    let (same, _) = f
        .store
        .terminal_machine_effect(&Mutation::Undesignate { expected: None }, None)
        .unwrap();
    assert_eq!(same["changed"], false);
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT COUNT(*) FROM management_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn machine_member_revision_is_fenced_independently_of_grant_revision() {
    let mut f = Fixture::new();
    let grant = f.browser("Person");
    let target = terminal_machines::resolve(&f.store, &grant.client_id).unwrap();
    f.store
        .connection
        .execute(
            "UPDATE members SET revision=revision+1 WHERE member_id=?1",
            [&target.member.member_id],
        )
        .unwrap();
    assert_eq!(
        f.store.grant(&grant.client_id).unwrap().unwrap().revision,
        target.grant_revision
    );
    assert!(
        f.store
            .terminal_machine_effect(&Mutation::Remove { target }, None)
            .is_err()
    );
    assert!(!f.store.grant(&grant.client_id).unwrap().unwrap().disabled);
}
