use super::*;
use crate::board::{app::Effect, app::workspace::tests::MEMBER};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

fn source_reader(source: &str) -> super::super::app::App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    app.perform(&crate::action::Action::parse("view-reply").unwrap());
    app.reader_event(&key(KeyCode::Char('s'), KeyModifiers::NONE));
    app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

#[test]
fn reader_attachment_preserves_effect_notice_quote_and_limit_behavior() {
    for case in ["attached", "changed", "limit"] {
        let source = "exact **raw** 漢字 e\u{301}\n\x1b[31m";
        let mut app = source_reader(source);
        let expected = app
            .row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .quote()
            .unwrap();
        assert_eq!(expected.text, source);
        assert_eq!(expected.range, 0..source.len());
        if case == "changed" {
            app.view.as_mut().unwrap().replies[0]["requestId"] = json!("r2");
        } else if case == "limit" {
            let target = app.conversation_target().unwrap();
            let squad = app.target_squad(&target).unwrap();
            let send = app.send_target(target, true).unwrap();
            app.compose_row(send, crate::action::Verb::Talk, squad);
            // Keep the Reader open while a retained draft owns an existing quote.
            app.input.as_mut().unwrap().response_quotes.push(Quote {
                text: "x".repeat(crate::board::app::MESSAGE_LIMIT),
                ..expected.clone()
            });
            app.remember_draft();
            app.input = None;
        }
        app.notice = Some("existing notice".into());
        let before = app.view.as_ref().unwrap().document.clone();
        assert_eq!(
            app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Effect::None
        );
        assert_eq!(app.view.as_ref().unwrap().document, before);
        match case {
            "changed" => {
                assert!(app.row_details.reader.is_some());
                assert!(app.input.is_none());
                assert_eq!(
                    app.notice.as_deref(),
                    Some("The response source changed; reopen it before annotating.")
                );
            }
            "limit" => {
                assert!(app.row_details.reader.is_none());
                let quotes = &app.input.as_ref().unwrap().response_quotes;
                assert_eq!(quotes.len(), 1);
                assert_eq!(quotes[0].text.len(), crate::board::app::MESSAGE_LIMIT);
                assert_eq!(
                    app.notice.as_deref(),
                    Some("Quote exceeds the 1 MiB attachment limit; not attached.")
                );
            }
            _ => {
                assert!(app.row_details.reader.is_none());
                assert_eq!(app.input.as_ref().unwrap().response_quotes, [expected]);
                assert_eq!(app.notice.as_deref(), Some("existing notice"));
                assert_eq!(
                    app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
                    MEMBER
                );
            }
        }
    }
}

#[test]
fn shared_attachment_matches_reader_and_leaves_surface_dismissal_to_the_caller() {
    for changed in [false, true] {
        let mut reader = source_reader("same raw source 漢字");
        let mut inline = source_reader("same raw source 漢字");
        if changed {
            for app in [&mut reader, &mut inline] {
                app.view.as_mut().unwrap().replies[0]["requestId"] = json!("new-request");
            }
        }
        for app in [&mut reader, &mut inline] {
            app.notice = Some("retained notice".into());
        }
        let source = inline.row_details.reader.as_ref().unwrap();
        let target = source.target.clone();
        let quote = source.selection.as_ref().unwrap().quote().unwrap();
        let effect = inline
            .attach_response_quote(target, quote)
            .finish(&mut inline);
        assert!(
            inline.row_details.reader.is_some(),
            "helper cannot dismiss a Reader"
        );
        assert_eq!(
            reader.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            effect
        );
        assert_eq!(reader.notice, inline.notice);
        assert_eq!(
            reader
                .input
                .as_ref()
                .map(|input| (&input.draft_key, &input.response_quotes)),
            inline
                .input
                .as_ref()
                .map(|input| (&input.draft_key, &input.response_quotes))
        );
        assert_eq!(reader.row_details.reader.is_some(), changed);
    }
}
