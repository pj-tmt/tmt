use super::*;
use crate::members::selectors::{local_machine, local_member};

#[test]
fn stable_member_id_wins_over_another_persons_display_name() {
    let mut f = Fixture::new();
    let alice = f.browser("Alice");
    let other = f.browser(&alice.client_id);
    let selected = local_member(&f.store, &alice.client_id).unwrap();
    assert_eq!(selected.member_id, alice.client_id);
    assert_eq!(selected.display_name, "Alice");
    assert_eq!(local_member(&f.store, "Alice").unwrap(), selected);
    assert_ne!(selected.member_id, other.client_id);
}

#[test]
fn ambiguous_member_names_refuse_without_changing_authority_or_adopting_intent() {
    let mut f = Fixture::new();
    let first = f.browser("Same");
    let second = f.browser("Same");
    let before = f.store.members().unwrap();
    assert_eq!(
        local_member(&f.store, "Same").unwrap_err().code,
        "REMOTE_MEMBER_AMBIGUOUS"
    );
    assert_eq!(
        local_member(&f.store, "Absent").unwrap_err().code,
        "REMOTE_MEMBER_NOT_FOUND"
    );
    assert_eq!(
        local_member(&f.store, "").unwrap_err().code,
        "REMOTE_INPUT_INVALID"
    );
    assert_eq!(
        local_member(&f.store, &first.client_id).unwrap().member_id,
        first.client_id
    );
    assert_eq!(
        local_member(&f.store, &second.client_id).unwrap().member_id,
        second.client_id
    );
    assert_eq!(f.store.members().unwrap(), before);
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
fn person_names_and_machine_ids_remain_distinct_across_reopen() {
    let mut f = Fixture::new();
    let person = f.browser("Person");
    f.store
        .connection
        .execute(
            "UPDATE grants SET name='Laptop' WHERE client_id=?1",
            [&person.client_id],
        )
        .unwrap();
    let member = local_member(&f.store, "Person").unwrap();
    let (machine_member, machine) = local_machine(&f.store, &person.client_id).unwrap();
    assert_eq!(machine_member, member);
    assert_eq!(machine.name, "Laptop");
    assert_eq!(
        local_member(&f.store, "Laptop").unwrap_err().code,
        "REMOTE_MEMBER_NOT_FOUND"
    );
    assert_eq!(
        local_machine(&f.store, "Person").err().unwrap().code,
        "REMOTE_MACHINE_NOT_FOUND"
    );
    f.reopen();
    assert_eq!(local_member(&f.store, "Person").unwrap(), member);
    assert_eq!(
        local_machine(&f.store, &person.client_id).unwrap().1.name,
        "Laptop"
    );
}
