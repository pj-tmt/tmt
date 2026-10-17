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
            app.perform(&crate::action::Action::parse("annotate member").unwrap()),
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
            reader.perform(&crate::action::Action::parse("annotate member").unwrap()),
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

#[test]
fn changed_exact_source_is_refused_at_send_without_losing_the_draft() {
    for change in ["request", "body", "range"] {
        let mut app = source_reader("held exact passage");
        app.perform(&crate::action::Action::parse("annotate member").unwrap());
        app.paste("my unsent comment");
        match change {
            "request" => app.view.as_mut().unwrap().replies[0]["requestId"] = json!("replaced"),
            "body" => app.view.as_mut().unwrap().replies[0]["response"] = json!("different body"),
            _ => app.input.as_mut().unwrap().response_quotes[0].range = 0..usize::MAX,
        }
        let before = app.input.as_ref().unwrap().clone();
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None,
            "{change}"
        );
        let retained = app.input.as_ref().unwrap();
        assert_eq!(retained.text.as_str(), before.text.as_str());
        assert_eq!(retained.response_quotes, before.response_quotes);
        assert_eq!(retained.compose, before.compose);
        assert_eq!(
            app.notice.as_deref(),
            Some("The response source changed; nothing sent. Draft retained.")
        );
    }
}

#[test]
fn final_body_limit_counts_the_row_tag_and_quote_markup_and_retains_the_source() {
    let mut app = source_reader(&"x".repeat(crate::board::app::MESSAGE_LIMIT));
    app.perform(&crate::action::Action::parse("annotate member").unwrap());
    let quotes = app.input.as_ref().unwrap().response_quotes.clone();
    assert_eq!(quotes.len(), 1);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(app.input.as_ref().unwrap().response_quotes, quotes);
    assert_eq!(
        app.notice.as_deref(),
        Some("Message including quotes exceeds 1 MiB; nothing sent. Draft retained.")
    );
}

#[test]
fn pending_questions_quote_only_the_acquired_exact_body_and_revalidate_before_send() {
    use crate::board::{
        app::Request,
        app::workspace::tests::{board, observe_waiting_member},
        shell::OpsTab,
    };
    const RAW: &str = "full question\n\n\"漢字\"";
    for case in ["unacquired", "failed", "acquired", "replaced", "refused"] {
        let mut app = board();
        app.shell.tab = OpsTab::Overview;
        observe_waiting_member(&mut app);
        let target = app.workspace.selected.clone().unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
            .unwrap()["waitingOnYou"] =
            json!([{"requestId":"held-question","preview":"short preview","preparedAtMs":1}]);
        app.perform(&crate::action::Action::parse("view-reply").unwrap());
        let source = app.detail_read().unwrap();
        assert_eq!(source.kind, crate::board::home_leads::Kind::Question);
        if case != "unacquired" {
            app.apply_message(
                &source,
                if case == "failed" {
                    Err("read refused".into())
                } else {
                    Ok(RAW.into())
                },
            );
        }
        app.reader_event(&key(KeyCode::Char('s'), KeyModifiers::NONE));
        if matches!(case, "unacquired" | "failed") {
            assert!(
                app.row_details.reader.as_ref().unwrap().selection.is_none(),
                "{case}"
            );
            continue;
        }
        app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let selected = app
            .row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .expect("an acquired question is exact source")
            .quote()
            .unwrap();
        assert_eq!(selected.text, RAW);
        app.perform(&crate::action::Action::parse("annotate member").unwrap());
        app.paste("my decision");
        app.reconcile_row_details();
        assert!(
            app.response_quote_current(
                &crate::board::row_detail::Target::Row(crate::board::app::RowTarget::Workspace(
                    target.clone()
                )),
                &selected
            ),
            "an acquired question keeps its exact source after composer placement; tab={:?}; notice={:?}",
            app.shell.tab,
            app.notice
        );
        if case == "replaced" {
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .rows
                .get_mut(&target)
                .unwrap()["waitingOnYou"][0]["requestId"] = json!("other-question");
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Effect::None
            );
            assert_eq!(app.input.as_ref().unwrap().text.as_str(), "my decision");
            assert_eq!(app.input.as_ref().unwrap().response_quotes, [selected]);
        } else {
            let sent = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(
                matches!(&sent, Effect::Act(Request::Annotate { text, .. }) if text == "[product · worker]\n> full question\n>\n> \"漢字\"\n\nmy decision"),
                "effect={sent:?}; notice={:?}",
                app.notice
            );
            if case == "refused" {
                let Effect::Act(request) = sent else {
                    unreachable!()
                };
                let ticket = app.outgoing_ticket(&request).unwrap();
                app.in_flight = 1;
                app.reconcile_row_details();
                app.in_flight = 0;
                app.finished_send(crate::board::conversation::outgoing::Completed {
                    ticket: Some(ticket),
                    outcome: Err(crate::core::SquadError::new("REFUSED", "Not accepted.")),
                    what: "Note on worker".into(),
                    to: "worker".into(),
                });
                let draft = app.input.as_ref().unwrap();
                assert_eq!(draft.text.as_str(), "my decision");
                assert_eq!(draft.response_quotes, [selected]);
                app.reconcile_row_details();
                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                    Effect::Act(request),
                    "a fresh explicit retry retains the exact body"
                );
            }
        }
    }
}

#[test]
#[ignore = "exports row-context renderer evidence without changing frozen fixtures"]
fn capture_row_context_packet() {
    use ratatui::{Terminal, backend::TestBackend};
    let output = std::env::var_os("TMT_ROW_CONTEXT_PACKET").expect("task-owned packet path");
    let _commands = crate::runner::forbid_commands();
    let mut frames = Vec::new();
    for (base, depth) in [
        ("tmt", tmt_cli_style::Depth::TrueColor),
        ("tmt-light", tmt_cli_style::Depth::TrueColor),
        ("tmt", tmt_cli_style::Depth::None),
    ] {
        for width in [160, 100, 60] {
            let mut app = source_reader("first exact line\n\nsecond \"raw\" line");
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                depth,
            };
            let view = app.view.as_mut().unwrap();
            view.look = look;
            for scoped in view
                .home
                .as_mut()
                .unwrap()
                .workspace
                .squad_looks
                .values_mut()
            {
                *scoped = look;
            }
            app.perform(&crate::action::Action::parse("annotate member").unwrap());
            app.paste("my comment about this row");
            for scene in ["quoted-draft", "quoted-outgoing"] {
                if scene == "quoted-outgoing" {
                    let Effect::Act(request) =
                        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                    else {
                        panic!("explicit send must freeze the selected source");
                    };
                    assert_eq!(
                        app.outgoing_ticket(&request).unwrap().text,
                        "[product · worker]\n> first exact line\n>\n> second \"raw\" line\n\nmy comment about this row"
                    );
                }
                assert_eq!(app.look().depth, depth);
                assert_eq!(app.look().theme.base.name(), base);
                let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                terminal
                    .draw(|frame| {
                        crate::board::view::prepare(&mut app, frame.area());
                        crate::board::view::render(frame, &app);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let lines = buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>();
                assert!(
                    lines.iter().any(|line| line.contains("my comment")),
                    "{scene} {base} {width}: {lines:?}"
                );
                frames.push(json!({"scene":scene,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),
                    "lines":lines,"cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
}

#[test]
fn reader_enter_inserts_compact_text_without_a_note_attachment_or_dispatch() {
    let mut app = source_reader("first raw line\nsecond line");
    let before = app.view.as_ref().unwrap().document.clone();
    assert_eq!(
        app.reader_event(&key(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.row_details.reader.is_none());
    let input = app.input.as_ref().expect("existing chat composer opens");
    assert_eq!(input.text.as_str(), "> \"first raw line second line\"\n");
    assert!(input.response_quotes.is_empty());
    assert_eq!(app.view.as_ref().unwrap().document, before);
}

#[test]
fn reader_note_key_uses_the_configured_recipient_and_has_no_hard_coded_a() {
    let mut app = source_reader("exact source");
    // The default a action is a note to the lead. This fixture has no lead:
    // the gesture must not silently turn that into a note to the member.
    app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(app.input.is_none());
    assert!(app.row_details.reader.is_some());
    assert_eq!(
        app.notice.as_deref(),
        Some("This squad has no observed lead to receive the note.")
    );
    app.perform(&crate::action::Action::parse("annotate member").unwrap());
    assert_eq!(
        app.input.as_ref().unwrap().response_quotes[0].text,
        "exact source"
    );
}

fn draw_reader(app: &mut crate::board::app::App, width: u16) {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
}

#[test]
fn reader_chip_uses_fresh_geometry_and_changed_source_cannot_copy_or_quote() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for change in ["none", "source", "geometry"] {
        let mut app = source_reader("exact raw text");
        draw_reader(&mut app, 100);
        let hit = app
            .row_details
            .reader
            .as_ref()
            .unwrap()
            .quote_button
            .get()
            .expect("whole chip beside selected end");
        if change == "source" {
            app.view.as_mut().unwrap().replies[0]["response"] = json!("new source");
        }
        if change == "geometry" {
            app.invalidate_frame_geometry();
        }
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.reader_event(&click), Effect::None);
        if change == "none" {
            assert_eq!(
                app.input.as_ref().unwrap().text.as_str(),
                "> \"exact raw text\"\n"
            );
        } else {
            assert!(app.input.is_none(), "{change}");
            let copy = app.reader_event(&key(KeyCode::Char('y'), KeyModifiers::NONE));
            if change == "source" {
                assert!(
                    matches!(copy, Effect::Act(crate::board::app::Request::Copy {text, ..}) if text == "exact raw text"),
                    "frozen read-only text remains explicitly copyable"
                );
            } else {
                assert_eq!(copy, Effect::None, "cleared selection cannot copy");
            }
        }
    }
}

#[test]
fn reader_keyboard_selection_copies_exact_scalars_and_outside_press_clears_it() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = source_reader("漢字 e\u{301}");
    draw_reader(&mut app, 100);
    app.reader_event(&key(KeyCode::Home, KeyModifiers::CONTROL));
    let Effect::Act(crate::board::app::Request::Copy {
        text,
        feedback: crate::board::app::CopyFeedback::Selection,
        ..
    }) = app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::CONTROL))
    else {
        panic!("new keyboard range copies");
    };
    assert_eq!(text, "漢字 e\u{301}");
    let area = app
        .row_details
        .reader
        .as_ref()
        .unwrap()
        .source_area
        .get()
        .unwrap();
    app.reader_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x.saturating_sub(1),
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .quote()
            .is_none()
    );
    assert!(app.input.is_none());
}

#[test]
fn a_non_note_key_never_inserts_characters_in_the_read_only_reader() {
    let mut app = source_reader("read only source");
    app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::ALT));
    assert_eq!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .source
            .as_str(),
        "read only source"
    );
    assert!(app.input.is_none());
}
