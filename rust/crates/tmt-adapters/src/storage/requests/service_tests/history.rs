use super::support::{DAY_MS, Fixture, NOW_MS, endpoint, prepare_input, service};
use tmt_core::{
    request::{
        Originator, RequestError, RequestKind, RequestPrompt, RequestRoute, ResponseProof,
        SubmitResponse,
        attention::FinalState,
        correlation,
        history::{HistoryCursor, HistoryQuery, HistoryScope},
    },
    room::{MembershipChange, RoomRepository, RoomWrite},
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

fn queue(fixture: &mut Fixture, id: &str, room: Option<&str>, message: &str, announcement: bool) {
    let mut input = prepare_input(
        fixture,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Unknown,
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.room_id = room.map(str::to_owned);
    input.message = message.into();
    input.kind = if announcement {
        RequestKind::Announcement
    } else {
        RequestKind::Request
    };
    service(fixture)
        .enqueue(input, format!("attempt-{id}"), 1)
        .unwrap();
}

fn scope(fixture: &Fixture, room_id: Option<&str>) -> HistoryScope {
    HistoryScope::Recipient {
        identity_id: fixture.identity_id.clone(),
        room_id: room_id.map(str::to_owned),
    }
}

fn final_reply(fixture: &mut Fixture, id: &str, body: &str) {
    let route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    service(fixture)
        .submit_response(SubmitResponse {
            request_id: id.into(),
            proof: ResponseProof::Compact(correlation::response_token(
                id,
                &format!("attempt-{id}"),
                &route,
            )),
            body: body.into(),
        })
        .unwrap();
}

#[test]
fn history_includes_acknowledged_unknown_sender_requests_and_never_acknowledges() {
    let mut fixture = Fixture::new();
    let recipient = fixture.identity_id.clone();
    queue(&mut fixture, "first", None, "First question", false);
    queue(&mut fixture, "second", None, "Second question", false);
    let incoming = service(&mut fixture)
        .list_incoming(&recipient, None, None, None)
        .unwrap();
    let first = &incoming.items[0].exchange;
    service(&mut fixture)
        .acknowledge_incoming_request(&recipient, &first.request_id, first.revision)
        .unwrap();
    final_reply(&mut fixture, "first", "First answer");
    let query = HistoryQuery {
        scope: scope(&fixture, None),
        before: None,
        limit: 20,
    };
    let page = service(&mut fixture).request_history(query).unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|row| row.item.request_id.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    assert_eq!(page.items[0].item.originator, Originator::Unknown);
    assert_eq!(page.items[0].item.recipient_acknowledged, Some(false));
    assert_eq!(page.items[1].item.recipient_acknowledged, Some(true));
    assert!(matches!(
        page.items[1].item.final_state,
        FinalState::Retained { content: (), .. }
    ));
    let detail = service(&mut fixture).request_detail("first").unwrap();
    assert!(
        matches!(detail.item.final_state, FinalState::Retained { content, .. } if content == "First answer")
    );
    let unread = service(&mut fixture)
        .list_incoming(&recipient, None, None, None)
        .unwrap();
    assert_eq!(unread.items.len(), 1);
    assert_eq!(unread.items[0].exchange.request_id, "second");
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    assert_eq!(oracle.query_row("SELECT recipient_attention_acknowledged_revision FROM request_attempts WHERE request_id='second'", [],
        |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn history_filters_before_keyset_pagination_and_keeps_original_room_after_leave() {
    let mut fixture = Fixture::new();
    for id in [ROOM, OTHER] {
        fixture
            .storage
            .save_meeting_room(
                id,
                RoomWrite {
                    expected_revision: 0,
                    name: id.into(),
                    member_ids: vec![fixture.identity_id.clone()],
                },
            )
            .unwrap();
    }
    for (id, room) in [
        ("a", Some(ROOM)),
        ("b", Some(OTHER)),
        ("c", Some(ROOM)),
        ("d", None),
    ] {
        queue(&mut fixture, id, room, id, false);
    }
    fixture
        .storage
        .change_meeting_membership(ROOM, &fixture.identity_id, MembershipChange::Leave)
        .unwrap();
    for selected in [scope(&fixture, Some(ROOM)), HistoryScope::Room(ROOM.into())] {
        let first = service(&mut fixture)
            .request_history(HistoryQuery {
                scope: selected.clone(),
                before: None,
                limit: 1,
            })
            .unwrap();
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].item.request_id, "c");
        assert_eq!(
            first.next_before,
            Some(HistoryCursor::Prepared {
                prepared_at_ms: NOW_MS,
                request_id: "c".into()
            })
        );
        // A newer insert between pages must not repeat, skip, or expand the older page.
        if service(&mut fixture).get_context("new").unwrap().is_none() {
            fixture.set_now(NOW_MS + 1);
            queue(&mut fixture, "new", None, "New unscoped request", false);
        }
        let next = service(&mut fixture)
            .request_history(HistoryQuery {
                scope: selected,
                before: first.next_before,
                limit: 1,
            })
            .unwrap();
        assert_eq!(next.items.len(), 1);
        assert_eq!(next.items[0].item.request_id, "a");
        assert_eq!(next.items[0].item.room_id.as_deref(), Some(ROOM));
        assert_eq!(next.next_before, None);
    }
    final_reply(&mut fixture, "a", "Reply after leaving");
    assert!(
        matches!(service(&mut fixture).request_detail("a").unwrap().item.final_state, FinalState::Retained { content, .. } if content == "Reply after leaving")
    );
}

#[test]
fn history_preview_is_bounded_unicode_while_detail_preserves_exact_text() {
    for message in [
        format!("{}\n  end\t", "🤖".repeat(161)),
        format!("head\0{}tail", "🤖".repeat(170)),
        format!("a{}end", "🤖".repeat(161)),
    ] {
        let mut fixture = Fixture::new();
        // Multibyte and NUL fixtures detect byte truncation and SQLite TEXT substring loss.
        queue(&mut fixture, "unicode", None, &message, false);
        let query = HistoryQuery {
            scope: scope(&fixture, None),
            before: None,
            limit: 1,
        };
        let page = service(&mut fixture).request_history(query).unwrap();
        assert_eq!(
            page.items[0].preview,
            Some(message.chars().take(160).collect())
        );
        assert!(
            matches!(service(&mut fixture).request_detail("unicode").unwrap().prompt, RequestPrompt::Retained(prompt) if prompt.message == message && prompt.message_bytes == message.len() as u64)
        );
    }
}

#[test]
fn history_distinguishes_no_reply_missing_final_expired_prompt_and_unknown_pane_read_state() {
    let mut fixture = Fixture::new();
    queue(&mut fixture, "notice", None, "Announcement", true);
    queue(&mut fixture, "missing", None, "Question", false);
    final_reply(&mut fixture, "missing", "Answer");
    let input = prepare_input(
        &fixture,
        "pane",
        endpoint("%2", 43),
        false,
        NOW_MS + 3_600_000,
        Originator::Unknown,
        false,
    );
    service(&mut fixture)
        .prepare(input, "pane-attempt".into(), 1)
        .unwrap();
    assert_eq!(
        service(&mut fixture)
            .request_detail("pane")
            .unwrap()
            .item
            .recipient_acknowledged,
        None
    );
    assert_eq!(
        service(&mut fixture)
            .request_detail("notice")
            .unwrap()
            .item
            .final_state,
        FinalState::NotRequired
    );
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    oracle
        .execute(
            "DELETE FROM request_responses WHERE request_id='missing'",
            [],
        )
        .unwrap();
    assert!(matches!(
        service(&mut fixture)
            .request_detail("missing")
            .unwrap()
            .item
            .final_state,
        FinalState::Unavailable { .. }
    ));
    // An accepted response outlives an earlier prompt under the same retention policy.
    queue(&mut fixture, "late", None, "Earlier prompt", false);
    fixture.set_now(NOW_MS + 3_000);
    final_reply(&mut fixture, "late", "Later answer");
    fixture.set_now(NOW_MS + DAY_MS);
    let detail = service(&mut fixture).request_detail("late").unwrap();
    assert!(matches!(detail.prompt, RequestPrompt::Expired { .. }));
    assert!(
        matches!(detail.item.final_state, FinalState::Retained { content, .. } if content == "Later answer")
    );
    let query = HistoryQuery {
        scope: scope(&fixture, None),
        before: None,
        limit: 20,
    };
    let page = service(&mut fixture).request_history(query).unwrap();
    assert_eq!(
        page.items
            .iter()
            .find(|row| row.item.request_id == "late")
            .unwrap()
            .preview,
        None
    );
    fixture.set_now(NOW_MS + DAY_MS + 3_000);
    assert!(matches!(
        service(&mut fixture)
            .request_detail("late")
            .unwrap()
            .item
            .final_state,
        FinalState::Expired { .. }
    ));
    // Metadata outlives content to preserve the independent reply acceptance window.
    fixture.set_now(NOW_MS + 8 * DAY_MS);
    assert!(matches!(
        service(&mut fixture).request_detail("late"),
        Err(RequestError::NotFound)
    ));
}

#[test]
fn history_rejects_invalid_queries_before_storage_work() {
    let mut fixture = Fixture::new();
    for limit in [0, 51, u64::MAX] {
        let query = HistoryQuery {
            scope: scope(&fixture, None),
            before: None,
            limit,
        };
        assert!(matches!(
            service(&mut fixture).request_history(query),
            Err(RequestError::Invalid(_))
        ));
    }
    let query = HistoryQuery {
        scope: HistoryScope::Room("not-a-room-id".into()),
        before: None,
        limit: 20,
    };
    assert!(matches!(
        service(&mut fixture).request_history(query),
        Err(RequestError::Invalid(_))
    ));
}
