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
fn completed_links_and_todos_leave_the_display_without_removing_the_source_items() {
    let mut hidden = item(5, None, Completion::Open, None, None);
    hidden.item.archived = true;
    let source = preview(vec![
        item(
            1,
            Some("https://example.org/open"),
            Completion::Open,
            None,
            None,
        ),
        item(2, None, Completion::Open, None, None),
        item(
            3,
            Some("https://example.org/done"),
            Completion::Complete,
            None,
            None,
        ),
        item(4, None, Completion::Complete, None, None),
        hidden,
    ]);
    let shown = rows(&source, NOW);
    assert_eq!(
        shown
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>(),
        ["item 1", "item 2"]
    );
    assert_eq!(
        source.current.items.len(),
        5,
        "references retain the full source"
    );
    assert!(
        rows(
            &preview(vec![item(6, None, Completion::Complete, None, None)]),
            NOW
        )
        .is_empty()
    );
}

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
    let bylines: Vec<_> = rows.iter().map(|row| row.byline.as_str()).collect();
    assert_eq!(bylines, ["you · 3m", "alice · 2h", "77777777 · 1d", ""]);
}

#[test]
fn a_link_row_shows_its_short_form_and_completed_items_have_no_display_rows() {
    let rows = rows(
        &preview(vec![
            item(
                1,
                Some("https://example.org/pull/4554?x=1"),
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
            item(3, None, Completion::Complete, None, None),
        ]),
        NOW,
    );
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        [Kind::Link]
    );
    assert_eq!(rows[0].link, "example.org/…/4554");
    assert_eq!(rows[0].byline, "alice · 1h");
}

#[test]
fn archived_items_are_not_listed() {
    let mut archived = item(1, None, Completion::Open, None, None);
    archived.item.archived = true;
    assert!(rows(&preview(vec![archived]), NOW).is_empty());
}

#[test]
fn a_github_issue_or_pull_reads_owner_repo_number() {
    for (reference, shown) in [
        (
            "https://github.com/tmt-ninja/tmt/issues/4520",
            "tmt-ninja/tmt#4520",
        ),
        (
            "https://github.com/tmt-ninja/tmt/pull/4520/files?diff=split",
            "tmt-ninja/tmt#4520",
        ),
        ("http://GitHub.com/a/b/pull/7#issuecomment-1", "a/b#7"),
    ] {
        assert_eq!(link(reference), shown, "{reference}");
    }
}

#[test]
fn any_other_address_reads_host_then_its_last_segment() {
    for (reference, shown) in [
        (
            "https://github.com/tmt-ninja/tmt/actions/runs/39890481652",
            "github.com/…/39890481652",
        ),
        (
            "https://colab.tmt.ninja/p/abc/checklist",
            "colab.tmt.ninja/…/checklist",
        ),
        ("https://example.com/a/b/", "example.com/…/b"),
        ("https://example.com/only", "example.com/only"),
        ("http://example.com?q=1", "example.com"),
        ("example.com/x", "example.com/x"),
        ("mailto:a@b.c", "mailto:a@b.c"),
        (
            "https://github.com/tmt-ninja/tmt/issues/new",
            "github.com/…/new",
        ),
        (
            "https://github.com/tmt-ninja/tmt/pull/",
            "github.com/…/pull",
        ),
    ] {
        assert_eq!(link(reference), shown, "{reference}");
    }
}

#[test]
fn a_todo_has_a_dash_for_its_link_and_every_status_names_its_state() {
    let rows = rows(
        &preview(vec![
            item(1, None, Completion::Open, None, None),
            item(
                2,
                Some("https://example.org/x"),
                Completion::Open,
                None,
                None,
            ),
        ]),
        NOW,
    );
    assert_eq!(rows[0].link, "—");
    assert_eq!(rows[0].kind.status(), "○ todo");
    assert_eq!(rows[1].kind.status(), "↗ link");
}

#[test]
fn every_status_mark_is_a_registered_board_mark() {
    for kind in [Kind::Link, Kind::Todo] {
        let row = format!("{} title", kind.status());
        assert_eq!(
            crate::board::glyph_guard::glyph_error(&row),
            None,
            "{kind:?}"
        );
    }
}
