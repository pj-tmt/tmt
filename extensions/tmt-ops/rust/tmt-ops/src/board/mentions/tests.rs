use super::*;
use crate::board::app::{Request, tests::crew};
use ratatui::crossterm::event::KeyModifiers;
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn app() -> App {
    let mut app = crew(Default::default(), vec![]);
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app.references = crate::board::inventory::Inventory::from_listed(
        &[crate::squad::Squad {
            name: "product".into(),
            room_id: "product-room".into(),
        }],
        &Ok(serde_json::json!({"identities":[{"id":"docs-id","name":"docs"}]})),
    );
    app.ask(
        String::new(),
        Compose::Talk {
            to: "auth-fix".into(),
        },
        "product".into(),
    );
    app
}
#[test]
fn inserting_a_squad_reference_keeps_the_direct_recipient_and_does_not_send() {
    let mut app = app();
    for ch in "@product".chars() {
        assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
    }
    assert!(app.mentions.completion.open());
    assert_eq!(app.mentions.completion.matches[0].kind, "squad");
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.input.as_ref().unwrap().text, "@product ");
    assert!(
        matches!(key(&mut app,KeyCode::Enter),Effect::Act(Request::Talk { to,text,.. }) if to=="auth-fix" && text=="@product ")
    );
}
#[test]
fn no_matches_and_escape_capture_keys_without_closing_the_composer() {
    let mut app = app();
    for ch in "@pxoj1".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    assert!(app.mentions.completion.open());
    assert!(app.mentions.completion.matches.is_empty());
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.input.is_some());
    key(&mut app, KeyCode::Esc);
    assert!(!app.mentions.completion.open());
    assert!(app.input.is_some());
    assert_eq!(app.input.as_ref().unwrap().text, "@pxoj1");
    key(&mut app, KeyCode::Esc);
    assert!(app.input.is_none());
}

#[test]
fn missing_inventory_does_not_invent_squad_ids_from_tab_names() {
    let mut app = app();
    app.references = Default::default();
    for ch in "@product".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    assert!(app.mentions.completion.open());
    assert!(app.mentions.completion.matches.is_empty());
}
#[test]
fn insertion_carries_plain_text_across_renames() {
    let mut app = app();
    for ch in "@docs".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    key(&mut app, KeyCode::Tab);
    app.view.as_mut().unwrap().document["sections"][1]["rows"][0]["name"] =
        serde_json::json!("renamed");
    assert_eq!(app.input.as_ref().unwrap().text, "@docs ");
    assert!(
        matches!(key(&mut app,KeyCode::Enter),Effect::Act(Request::Talk { to,text,.. }) if to=="auth-fix" && text=="@docs ")
    );
}

#[test]
fn global_inventory_includes_common_pool_and_uses_room_and_identity_ids() {
    let inventory = crate::board::inventory::Inventory::from_listed(
        &[crate::squad::Squad {
            name: "team".into(),
            room_id: "room-1".into(),
        }],
        &Ok(serde_json::json!({"identities":[{"id":"agent-1","name":"pool-member"}]})),
    );
    assert!(
        inventory
            .candidates
            .iter()
            .any(|candidate| candidate.id == "squad:room-1")
    );
    assert!(
        inventory
            .candidates
            .iter()
            .any(|candidate| candidate.id == "agent:agent-1")
    );
    let mut app = app();
    app.references = inventory;
    for ch in "@pool".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(app.mentions.completion.matches[0].label, "pool-member");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().text, "@pool-member ");
    assert!(
        matches!(app.input.as_ref().unwrap().compose, Compose::Talk { ref to } if to=="auth-fix")
    );
}

#[test]
fn status_reason_uses_completion_without_applying_status_or_changing_actor() {
    let mut app = app();
    let target = app.row_target(0).unwrap();
    let send = app.send_target(target, true).unwrap();
    app.attach_row(send, "product".into());
    app.input.as_mut().unwrap().compose = Compose::Status;
    app.start_status();
    app.status_draft.as_mut().unwrap().focus = 2;
    for ch in "@product".chars() {
        assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
    }
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    let draft = app.status_draft.as_ref().unwrap();
    assert_eq!(draft.reason.as_str(), "@product ");
    assert!(draft.outcome.is_none());
    assert_eq!(draft.target.as_ref().unwrap().actor, "ben-id");
}

#[test]
fn cron_message_completes_plain_text_but_schedule_is_not_a_message_field() {
    use crate::board::cronboard::{detail_id, test_cron, test_view};
    let mut app = app();
    let job = test_view("auth-fix", "old message", None);
    let id = detail_id(&job);
    app.cron.replace(Ok(test_cron(
        vec![job],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    assert_eq!(app.cron_key('E', Some(&id)), Effect::None);
    assert!(app.message_field());
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Backspace);
    for ch in "@product".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "@product ");
    assert!(app.message_field(), "completion must not advance the form");
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(!app.message_field(), "schedule has its own syntax");
    key(&mut app, KeyCode::Char('@'));
    assert!(!app.mentions.completion.open());
}
