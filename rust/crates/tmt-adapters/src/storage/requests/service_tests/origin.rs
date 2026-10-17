use super::support::{Fixture, NOW_MS, endpoint, prepare_input, service};
use tmt_core::request::{
    Originator, RequestOrigin, RequestRoute,
    history::{HistoryQuery, HistoryScope},
};

/// Queues one inbox request for the fixture identity with the given origin and sender.
fn queue(
    fixture: &mut Fixture,
    id: &str,
    origin: RequestOrigin,
    originator: Originator,
    message: &str,
) {
    let mut input = prepare_input(
        fixture,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        originator,
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.origin = origin;
    input.message = message.into();
    service(fixture)
        .enqueue(input, format!("attempt-{id}"), 1)
        .unwrap();
}

fn origins(fixture: &mut Fixture) -> Vec<(String, RequestOrigin)> {
    let query = HistoryQuery {
        scope: HistoryScope::Recipient {
            identity_id: fixture.identity_id.clone(),
            room_id: None,
        },
        before: None,
        limit: 20,
    };
    let page = service(fixture).request_history(query).unwrap();
    let mut found: Vec<_> = page
        .items
        .iter()
        .map(|row| (row.item.request_id.clone(), row.item.origin))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn stored(fixture: &Fixture, id: &str) -> Option<String> {
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            "SELECT origin FROM request_attempts WHERE request_id = ?",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn history_projects_the_origin_each_request_was_created_with() {
    let mut fixture = Fixture::new();
    let notice = "▚ ⏱ product c1 · every 30m · added by Ben";
    // A person's message that looks exactly like a cron notice is still a person's.
    queue(
        &mut fixture,
        "typed",
        RequestOrigin::Person,
        Originator::Unknown,
        notice,
    );
    queue(
        &mut fixture,
        "scheduled",
        RequestOrigin::Automation,
        Originator::Unknown,
        notice,
    );
    // A row from before origins were recorded has no stored value.
    queue(
        &mut fixture,
        "legacy",
        RequestOrigin::Unknown,
        Originator::Unknown,
        "older",
    );
    assert_eq!(
        origins(&mut fixture),
        [
            ("legacy".into(), RequestOrigin::Unknown),
            ("scheduled".into(), RequestOrigin::Automation),
            ("typed".into(), RequestOrigin::Person),
        ]
    );
    assert_eq!(stored(&fixture, "typed").as_deref(), Some("person"));
    assert_eq!(stored(&fixture, "scheduled").as_deref(), Some("automation"));
    assert_eq!(stored(&fixture, "legacy"), None);
    assert_eq!(
        service(&mut fixture)
            .request_detail("scheduled")
            .unwrap()
            .item
            .origin,
        RequestOrigin::Automation
    );
}

#[test]
fn the_origin_survives_renaming_the_sender() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    queue(
        &mut fixture,
        "scheduled",
        RequestOrigin::Automation,
        Originator::Explicit(sender.clone()),
        "tick",
    );
    queue(
        &mut fixture,
        "legacy",
        RequestOrigin::Unknown,
        Originator::Explicit(sender.clone()),
        "older",
    );
    rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .execute(
            "UPDATE identities SET name = 'Renamed', canonical_name = 'renamed' WHERE id = ?",
            [&sender],
        )
        .unwrap();
    assert_eq!(
        origins(&mut fixture),
        [
            ("legacy".into(), RequestOrigin::Unknown),
            ("scheduled".into(), RequestOrigin::Automation),
        ]
    );
    assert_eq!(stored(&fixture, "scheduled").as_deref(), Some("automation"));
    assert_eq!(stored(&fixture, "legacy"), None);
}
