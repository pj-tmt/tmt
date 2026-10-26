use super::*;
use crate::{board::conversation_checklist::Subject, checklist::events::Logged};

fn key(room: &str) -> Key {
    Key {
        room: room.into(),
        subject: Subject::Conversation("member".into()),
    }
}
fn facts() -> Snapshot {
    let mut facts = Snapshot {
        user: Some("user".into()),
        ..Default::default()
    };
    facts.names.insert("agent".into(), "worker".into());
    facts.titles.insert(
        "item".into(),
        "Current **title** /tmp/a_b [link](https://x)".into(),
    );
    for (i, action) in [
        "add", "edit", "assign", "unassign", "complete", "reopen", "archive", "restore", "delete",
        "reorder",
    ]
    .into_iter()
    .enumerate()
    {
        facts.events.push(Logged {
            id: i as u64 + 1,
            room_id: "room".into(),
            member_id: Some("member".into()),
            checklist_id: "internal list".into(),
            item_id: Some("item".into()),
            actor_id: "agent".into(),
            at_ms: 10000 - i as u64,
            action: action.into(),
            revision: 99,
        });
    }
    facts
}

#[test]
fn all_ten_actions_use_current_titles_and_names_never_internal_uuids() {
    let p = Presentation::new(
        key("room"),
        Arc::new(facts()),
        &Inventory::default(),
        130000,
    );
    assert_eq!(p.rows.len(), 10);
    assert!(
        p.rows[0]
            .text
            .contains("≡ worker · checklist · added Current **title**")
    );
    assert!(p.rows[4].text.contains("marked done Current **title**"));
    assert!(p.rows[9].text.contains("reordered the checklist · 2m ago"));
    assert!(!p.rows[9].text.contains("Current"));
    assert!(
        p.rows
            .iter()
            .all(|row| !row.text.contains("internal list") && !row.text.contains("99"))
    );
}

#[test]
fn user_uuid_is_the_only_filter_and_unknown_user_grants_no_attribution() {
    let mut f = facts();
    f.events[0].actor_id = "user".into();
    f.names.insert("user".into(), "same-name".into());
    f.names.insert("agent".into(), "same-name".into());
    let p = Presentation::new(key("a"), Arc::new(f.clone()), &Inventory::default(), 130000);
    assert_eq!(p.rows.len(), 9);
    assert_eq!(p.rows[0].id, 2);
    f.user = None;
    let p = Presentation::new(key("a"), Arc::new(f), &Inventory::default(), 130000);
    assert!(p.rows.is_empty());
    assert_eq!(
        p.captions,
        ["○ checklist activity unavailable · user identity unresolved"]
    );
}

#[test]
fn missing_labels_are_human_and_control_text_is_inert() {
    let mut f = facts();
    f.titles.clear();
    f.names.insert(
        "agent".into(),
        "\x1b]8;;https://evil\x07\u{202e}\n\x1b[31m".into(),
    );
    let p = Presentation::new(key("a"), Arc::new(f), &Inventory::default(), 130000);
    assert!(
        p.rows[0]
            .text
            .contains("an agent · checklist · added an item")
    );
    assert!(!p.rows[0].text.contains('\x1b'));
}

#[test]
fn event_order_survives_clock_rollback_and_room_anchors_cannot_alias() {
    let facts = Arc::new(facts());
    let a = Presentation::new(key("a"), Arc::clone(&facts), &Inventory::default(), 130000);
    let b = Presentation::new(key("b"), facts, &Inventory::default(), 130000);
    assert_ne!(a.anchor(0), b.anchor(0));
    let order = merge(&[9999, 10000], &[], Some(&a));
    let ids = order
        .iter()
        .filter_map(|source| match source {
            Source::Checklist(i) => Some(a.rows[*i].id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, (1..=10).collect::<Vec<_>>());
    assert!(matches!(order[0], Source::Exchange(0)));
    assert!(matches!(order[1], Source::Exchange(1))); // Exchanges first on a literal tie.
}

#[test]
fn wrapped_activity_is_literal_copyable_and_has_no_source_or_acknowledgment_spans() {
    let activity = Arc::new(Presentation::new(
        key("a"),
        Arc::new(facts()),
        &Inventory::default(),
        130000,
    ));
    let session = Session {
        transcript_context: super::super::transcript::Context {
            activity: Some(activity),
            exchanges: super::super::transcript::Exchanges::Observed,
            ..Default::default()
        },
        ..Default::default()
    };
    let look = Look::default();
    for width in [160, 100, 80, 60, 20] {
        let inventory = Inventory::default();
        let rendered = session.timeline_transcript(width, look, &inventory);
        if width == 160 {
            assert_eq!(
                rendered.mapped.lines.len(),
                10,
                "consecutive activity is compact"
            );
        }
        assert!(
            rendered
                .timeline_spans
                .iter()
                .all(|span| span.source.is_none() && span.legacy.is_none())
        );
        assert!(
            rendered.mapped.mentions.iter().all(Vec::is_empty),
            "file, URL and member-like text is inert"
        );
        assert!(
            rendered
                .mapped
                .lines
                .iter()
                .all(|line| line.width() <= width)
        );
        for (index, row) in rendered.copy.rows.iter().enumerate() {
            assert!(
                row.at.is_none(),
                "literal displayed copy has no authored source"
            );
            assert_eq!(
                rendered.note_sources.at(index),
                super::super::note_source::Source::Unavailable
            );
            assert_eq!(row.shown, rendered.mapped.lines[index].to_string());
        }
        assert!(rendered.anchor_text.contains("checklist:a:member:1"));
        assert!(
            rendered
                .mapped
                .lines
                .iter()
                .any(|line| line.to_string().contains("**title**"))
        );
    }
}

#[test]
fn removing_the_selected_lists_facts_invalidates_the_existing_member_session_cache() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = crate::board::conversation_checklist::tests::activity_scenes::capture_case(
        "checklist-activity",
        Look::default(),
    );
    let draw = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| super::super::render(frame, app, frame.area()))
            .unwrap();
    };
    draw(&app);
    {
        let state = app.conversation.borrow();
        let text = &state.sessions[state.active.as_ref().unwrap()]
            .rendered
            .as_ref()
            .unwrap()
            .anchor_text;
        assert!((0..text.line_count()).any(|line| text.line(line).unwrap().contains("checklist:")));
    }
    app.notes = Default::default();
    draw(&app);
    let state = app.conversation.borrow();
    let next = state.sessions[state.active.as_ref().unwrap()]
        .rendered
        .as_ref()
        .unwrap();
    assert!(
        !(0..next.anchor_text.line_count()).any(|line| next
            .anchor_text
            .line(line)
            .unwrap()
            .contains("checklist:"))
    );
}
