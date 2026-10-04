use super::support::{DAY_MS, Fixture, NOW_MS, endpoint, prepare_input, request_snapshot, service};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, RequestError, RequestRoute, ResponseProof, SubmitResponse,
        attention::FinalState,
        correlation,
        history::{HistoryCursor, HistoryQuery, HistoryScope},
    },
    room::{RoomRepository, RoomWrite},
};
const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
fn sender(fixture: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut fixture.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}
fn queue(fixture: &mut Fixture, originator: &str, id: &str, room: Option<&str>) {
    let mut input = prepare_input(
        fixture,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Explicit(originator.into()),
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.room_id = room.map(str::to_owned);
    service(fixture)
        .enqueue(input, format!("attempt-{id}"), 1)
        .unwrap();
}
fn reply(fixture: &mut Fixture, id: &str, body: &str) {
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
fn query(originator: &str, limit: u64, before: Option<HistoryCursor>) -> HistoryQuery {
    HistoryQuery {
        scope: HistoryScope::OriginatorResults(originator.into()),
        limit,
        before,
    }
}
#[test]
fn results_order_by_submission_not_preparation_across_rooms_and_keep_acknowledged_work() {
    let mut fixture = Fixture::new();
    let originator = sender(&mut fixture, "Originator");
    let outsider = sender(&mut fixture, "Outsider");
    for room in [ROOM, OTHER] {
        fixture
            .storage
            .save_meeting_room(
                room,
                RoomWrite {
                    expected_revision: 0,
                    name: room.into(),
                    member_ids: vec![fixture.identity_id.clone()],
                },
            )
            .unwrap();
    }
    queue(&mut fixture, &originator, "old", Some(ROOM));
    fixture.set_now(NOW_MS + 1);
    queue(&mut fixture, &originator, "a", Some(OTHER));
    queue(&mut fixture, &originator, "z", None);
    queue(&mut fixture, &originator, "unanswered", None);
    queue(&mut fixture, &outsider, "foreign", Some(ROOM));
    reply(&mut fixture, "a", "Question? is still just a reply");
    reply(&mut fixture, "z", "Blocked text is still just a reply");
    fixture.set_now(NOW_MS + 2);
    reply(&mut fixture, "foreign", "Hidden from this originator");
    reply(&mut fixture, "old", "Late final wins");
    service(&mut fixture)
        .acknowledge_all_exchanges(&originator)
        .unwrap();
    let before = request_snapshot(&fixture.database);
    let first = service(&mut fixture)
        .request_history(query(&originator, 2, None))
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|row| row.item.request_id.as_str())
            .collect::<Vec<_>>(),
        ["old", "z"]
    );
    assert_eq!(first.items[0].item.room_id.as_deref(), Some(ROOM));
    assert_eq!(first.items[1].item.room_id, None);
    assert_eq!(
        first.next_before,
        Some(HistoryCursor::Submitted {
            submitted_at_ms: NOW_MS + 1,
            request_id: "z".into()
        })
    );
    let second = service(&mut fixture)
        .request_history(query(&originator, 2, first.next_before))
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].item.request_id, "a");
    assert_eq!(second.items[0].item.room_id.as_deref(), Some(OTHER));
    assert_eq!(second.next_before, None);
    assert_eq!(request_snapshot(&fixture.database), before);
    assert!(
        service(&mut fixture)
            .list_exchanges(&originator, None, None)
            .unwrap()
            .items
            .is_empty()
    );
}
#[test]
fn results_keep_expired_and_unavailable_headers_without_cleanup_or_retention_renewal() {
    let mut fixture = Fixture::new();
    let originator = sender(&mut fixture, "Originator");
    for id in ["early", "missing", "late"] {
        queue(&mut fixture, &originator, id, None);
    }
    reply(&mut fixture, "early", "Early body");
    reply(&mut fixture, "missing", "Missing body");
    fixture.set_now(NOW_MS + 3_000);
    reply(&mut fixture, "late", "Retained later body");
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    oracle
        .execute(
            "DELETE FROM request_responses WHERE request_id='missing'",
            [],
        )
        .unwrap();
    let page = service(&mut fixture)
        .request_history(query(&originator, 8, None))
        .unwrap();
    let missing = page
        .items
        .iter()
        .find(|row| row.item.request_id == "missing")
        .unwrap();
    assert!(matches!(
        missing.item.final_state,
        FinalState::Unavailable { .. }
    ));
    assert!(missing.response_preview.is_none());
    fixture.set_now(NOW_MS + DAY_MS);
    let before = request_snapshot(&fixture.database);
    let page = service(&mut fixture)
        .request_history(query(&originator, 8, None))
        .unwrap();
    let early = page
        .items
        .iter()
        .find(|row| row.item.request_id == "early")
        .unwrap();
    assert!(matches!(
        early.item.final_state,
        FinalState::Expired {
            submitted_at_ms: NOW_MS,
            ..
        }
    ));
    assert!(early.response_preview.is_none());
    assert!(page.items[0].preview.is_none());
    assert_eq!(
        page.items[0].response_preview.as_ref().unwrap().text,
        "Retained later body"
    );
    assert_eq!(request_snapshot(&fixture.database), before);
    fixture.set_now(NOW_MS + 8 * DAY_MS);
    let before = request_snapshot(&fixture.database);
    assert!(
        service(&mut fixture)
            .request_history(query(&originator, 8, None))
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(request_snapshot(&fixture.database), before);
}
#[test]
fn results_preview_sanitizes_the_first_line_and_bounds_utf8_without_changing_exact_bodies() {
    for (body, expected, truncated) in [
        ("plain reply".into(), "plain reply".into(), false),
        (
            "head\0tail\t\u{1b}[31m\u{85}x\r\nsecond".into(),
            "head tail  [31m x".into(),
            true,
        ),
        // Direction controls are spaces; script letters, joiners and emoji remain data.
        (
            "a\u{061c}\u{200e}\u{200f}\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\u{2066}\u{2067}\u{2068}\u{2069}z".into(),
            "a            z".into(),
            false,
        ),
        (
            "עברית العربية می\u{200c}روم 👩\u{200d}💻 ✈\u{fe0f}".into(),
            "עברית العربية می\u{200c}روم 👩\u{200d}💻 ✈\u{fe0f}".into(),
            false,
        ),
        ("first\rsecond".into(), "first".into(), true),
        (
            "first\u{2028}second\u{2029}third".into(),
            "first".into(),
            true,
        ),
        ("\nsecond".into(), "".into(), true),
        ("🤖".repeat(160), "🤖".repeat(160), false),
        ("🤖".repeat(161), "🤖".repeat(160), true),
        (
            format!("a{}", "🤖".repeat(161)),
            format!("a{}", "🤖".repeat(159)),
            true,
        ),
        ("a".repeat(1_048_576), "a".repeat(160), true),
    ] {
        let mut fixture = Fixture::new();
        let originator = sender(&mut fixture, "Originator");
        queue(&mut fixture, &originator, "reply", None);
        reply(&mut fixture, "reply", &body);
        // Inspect the actual SQL projection, not just the rendered output cap.
        let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
        let sql = super::super::history::history_query(
            &HistoryScope::OriginatorResults(originator.clone()),
            false,
        );
        let mut statement = oracle.prepare(&sql).unwrap();
        let column = statement.column_count() - 1;
        let prefix: Vec<u8> = statement
            .query_row(
                rusqlite::params![
                    originator,
                    Option::<String>::None,
                    Option::<i64>::None,
                    Option::<String>::None,
                    9,
                    NOW_MS as i64
                ],
                |row| row.get(column),
            )
            .unwrap();
        assert_eq!(prefix, body.as_bytes()[..body.len().min(644)]);
        let before = request_snapshot(&fixture.database);
        let page = service(&mut fixture)
            .request_history(query(&originator, 8, None))
            .unwrap();
        let preview = page.items[0].response_preview.as_ref().unwrap();
        assert_eq!(preview.text, expected);
        assert_eq!(preview.truncated, truncated);
        assert!(preview.text.chars().count() <= 160 && preview.text.len() <= 640);
        assert_eq!(request_snapshot(&fixture.database), before);
        assert!(
            matches!(service(&mut fixture).request_detail("reply").unwrap().item.final_state,
            FinalState::Retained { content, .. } if content == body)
        );
    }
}
#[test]
fn results_enforce_page_caps_and_reject_history_cursors() {
    let mut fixture = Fixture::new();
    let originator = sender(&mut fixture, "Originator");
    for i in 0..51 {
        let id = format!("request-{i:02}");
        queue(&mut fixture, &originator, &id, None);
        reply(&mut fixture, &id, "Reply");
    }
    let first = service(&mut fixture)
        .request_history(query(&originator, 50, None))
        .unwrap();
    assert_eq!(first.items.len(), 50);
    let second = service(&mut fixture)
        .request_history(query(&originator, 50, first.next_before))
        .unwrap();
    assert_eq!(second.items[0].item.request_id, "request-00");
    assert_eq!(second.next_before, None);
    for invalid in [
        query(&originator, 0, None),
        query(&originator, 51, None),
        query("invalid", 8, None),
        query(
            &originator,
            8,
            Some(HistoryCursor::Prepared {
                prepared_at_ms: NOW_MS,
                request_id: "request-01".into(),
            }),
        ),
    ] {
        assert!(matches!(
            service(&mut fixture).request_history(invalid),
            Err(RequestError::Invalid(_))
        ));
    }
}
