use super::*;
use crate::checklist::{
    Current, ItemView, MemberView, RoomView,
    model::{Completion, Item, ItemContent},
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const MEMBER: &str = "22222222-2222-4222-8222-222222222222";
const ME: &str = "55555555-5555-4555-8555-555555555555";
const ALICE: &str = "66666666-6666-4666-8666-666666666666";
const STRANGER: &str = "77777777-7777-4777-8777-777777777777";
const NOW: u64 = 1_000_000_000;

fn id(text: &str) -> Id {
    Id::parse(text).unwrap()
}

fn stamp(by: &str, ago_ms: u64) -> Stamp {
    Stamp {
        by: id(by),
        at_ms: NOW - ago_ms,
    }
}

fn item(
    n: u8,
    reference: Option<&str>,
    completion: Completion,
    added: Option<Stamp>,
    changed: Option<Stamp>,
) -> ItemView {
    ItemView {
        item: Item {
            id: id(&format!("33333333-3333-4333-8333-3333333333{n:02}")),
            revision: 1,
            content: ItemContent::new(format!("item {n}"), None, reference.map(str::to_owned))
                .unwrap(),
            assignee: None,
            completion,
            archived: false,
            added,
            last_changed: changed,
        },
        assignee_label: None,
        assignee_available: true,
    }
}

fn preview(items: Vec<ItemView>) -> Preview {
    Preview {
        actor_id: id(ME),
        current: Current {
            room: RoomView {
                id: id(ROOM),
                name: Some("product".into()),
                available: true,
                manager: true,
            },
            member: Some(MemberView {
                id: id(MEMBER),
                name: Some("worker".into()),
                retired: false,
            }),
            checklist_id: None,
            inventory_revision: Some(1),
            items,
            deletion: None,
        },
        members: vec![(id(ME), "lead".into()), (id(ALICE), "alice".into())],
    }
}

const HOUR: u64 = 3_600_000;
const MINUTE: u64 = 60_000;

#[test]
fn who_added_an_item_is_you_a_roster_name_a_short_id_or_nobody() {
    let rows = rows(
        &preview(vec![
            item(1, None, Completion::Open, Some(stamp(ME, 3 * MINUTE)), None),
            item(
                2,
                None,
                Completion::Open,
                Some(stamp(ALICE, 2 * HOUR)),
                None,
            ),
            item(
                3,
                None,
                Completion::Open,
                Some(stamp(STRANGER, 26 * HOUR)),
                None,
            ),
            item(4, None, Completion::Open, None, None),
        ]),
        NOW,
    );
    let metas: Vec<_> = rows.iter().map(|row| row.meta(usize::MAX)).collect();
    assert_eq!(metas, ["you · 3m", "alice · 2h", "77777777 · 1d", ""]);
}

#[test]
fn a_link_shows_its_host_and_a_done_todo_when_it_was_done() {
    let rows = rows(
        &preview(vec![
            item(
                1,
                Some("https://github.com/pj-tmt/tmt/pull/4554?x=1"),
                Completion::Open,
                Some(stamp(ALICE, HOUR)),
                None,
            ),
            item(
                2,
                None,
                Completion::Complete,
                Some(stamp(ME, 5 * HOUR)),
                Some(stamp(ALICE, 2 * MINUTE)),
            ),
            item(
                3,
                None,
                Completion::Complete,
                Some(stamp(ME, 5 * HOUR)),
                None,
            ),
            item(4, None, Completion::Complete, None, Some(stamp(ME, HOUR))),
            item(5, None, Completion::Complete, None, None),
        ]),
        NOW,
    );
    let kinds: Vec<_> = rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        [Kind::Link, Kind::Done, Kind::Done, Kind::Done, Kind::Done]
    );
    let metas: Vec<_> = rows.iter().map(|row| row.meta(usize::MAX)).collect();
    assert_eq!(
        metas,
        [
            "github.com  alice · 1h",
            "you · done 2m",
            "you · done",
            "done 1h",
            "done"
        ]
    );
}

#[test]
fn archived_items_are_not_listed() {
    let mut archived = item(1, None, Completion::Open, None, None);
    archived.item.archived = true;
    assert!(rows(&preview(vec![archived]), NOW).is_empty());
}

#[test]
fn a_host_stops_at_the_path_and_a_bare_reference_is_its_own_host() {
    assert_eq!(host("https://example.com/a/b"), "example.com");
    assert_eq!(host("http://example.com?q=1"), "example.com");
    assert_eq!(host("example.com/x"), "example.com");
    assert_eq!(host("mailto:a@b.c"), "mailto:a@b.c");
}

#[test]
fn every_row_mark_is_a_registered_board_mark() {
    for kind in [Kind::Link, Kind::Todo, Kind::Done] {
        let row = format!("{} title", kind.mark());
        assert_eq!(
            crate::board::glyph_guard::glyph_error(&row),
            None,
            "{kind:?}"
        );
    }
}
