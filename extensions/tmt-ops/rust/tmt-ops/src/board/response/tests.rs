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
    for case in ["attached", "changed", "limit", "boundary"] {
        let source = if case == "boundary" {
            "exact source ending in CR\r"
        } else {
            "exact **raw** 漢字 e\u{301}\n\x1b[31m"
        };
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
        } else if matches!(case, "limit" | "boundary") {
            let target = app.conversation_target().unwrap();
            let squad = app.target_squad(&target).unwrap();
            let send = app.send_target(target, true).unwrap();
            app.compose_row(send, crate::action::Verb::Talk, squad);
            // Keep the Reader open while a retained draft exercises admission failure.
            app.input.as_mut().unwrap().text = if case == "limit" {
                "x".repeat(crate::board::app::MESSAGE_LIMIT).into()
            } else {
                "kept draft".into()
            };
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
            "limit" | "boundary" => {
                assert!(app.row_details.reader.is_none());
                let input = app.input.as_ref().unwrap();
                assert!(input.text.folds().is_empty());
                assert_eq!(
                    input.text.as_str(),
                    if case == "limit" {
                        "x".repeat(crate::board::app::MESSAGE_LIMIT)
                    } else {
                        "kept draft".to_owned()
                    }
                );
                assert_eq!(
                    app.notice.as_deref(),
                    Some("Quote could not be added; not attached.")
                );
            }
            _ => {
                assert!(app.row_details.reader.is_none());
                assert_eq!(
                    app.input
                        .as_ref()
                        .unwrap()
                        .exact_quotes()
                        .collect::<Vec<_>>(),
                    [expected]
                );
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
            reader.input.as_ref().map(|input| (
                &input.draft_key,
                &input.text,
                &input.inline_quotes
            )),
            inline.input.as_ref().map(|input| (
                &input.draft_key,
                &input.text,
                &input.inline_quotes
            ))
        );
        assert_eq!(reader.row_details.reader.is_some(), changed);
    }
}

#[test]
fn changed_exact_source_is_refused_at_send_without_losing_the_draft() {
    for change in ["request", "body", "range"] {
        let mut app = source_reader("held exact passage");
        app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
        app.paste("my unsent comment");
        match change {
            "request" => app.view.as_mut().unwrap().replies[0]["requestId"] = json!("replaced"),
            "body" => app.view.as_mut().unwrap().replies[0]["response"] = json!("different body"),
            _ => {
                let source = app
                    .input
                    .as_mut()
                    .unwrap()
                    .inline_quotes
                    .values_mut()
                    .next()
                    .unwrap();
                let crate::board::composer::QuoteSource::Exact { range, .. } = source else {
                    panic!("exact source")
                };
                *range = 0..usize::MAX;
            }
        }
        let before = app.input.as_ref().unwrap().clone();
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None,
            "{change}"
        );
        let retained = app.input.as_ref().unwrap();
        assert_eq!(retained.text.as_str(), before.text.as_str());
        assert_eq!(retained.inline_quotes, before.inline_quotes);
        assert_eq!(retained.compose, before.compose);
        assert_eq!(
            app.notice.as_deref(),
            Some("The response source changed; nothing sent. Draft retained.")
        );
    }
}

#[test]
fn final_body_limit_counts_the_row_tag_and_quote_markup_and_retains_the_source() {
    let mut app = source_reader(&"x".repeat(crate::board::app::MESSAGE_LIMIT - 1));
    app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
    let quotes = app
        .input
        .as_ref()
        .unwrap()
        .exact_quotes()
        .collect::<Vec<_>>();
    assert_eq!(quotes.len(), 1);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(
        app.input
            .as_ref()
            .unwrap()
            .exact_quotes()
            .collect::<Vec<_>>(),
        quotes
    );
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
        app.shell.tab = OpsTab::Agents;
        app.agents_filter(crate::board::workspace::AgentsFilter::Agents);
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
        app.workspace_reconcile();
        // The Replies pane must be painted before it can ask for a body.
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36))
            .unwrap()
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        app.perform(&crate::action::Action::parse("view-reply").unwrap());
        eprintln!(
            "DBG reader={} notice={:?} idx={} vis={} tgt={:?}",
            app.row_details.reader.is_some(),
            app.notice,
            app.selected,
            app.scrolls.visible(crate::config::Pane::Replies),
            app.conversation_target()
        );
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
        app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
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
            assert_eq!(
                app.input.as_ref().unwrap().text.as_str(),
                format!("{RAW}\nmy decision")
            );
            assert_eq!(
                app.input
                    .as_ref()
                    .unwrap()
                    .exact_quotes()
                    .collect::<Vec<_>>(),
                [selected]
            );
        } else {
            let sent = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(
                matches!(&sent, Effect::Act(Request::Annotate { text, .. }) if text == "[product · worker]\n> full question\n>\n> \"漢字\"\nmy decision"),
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
                assert_eq!(draft.text.as_str(), format!("{RAW}\nmy decision"));
                assert_eq!(draft.exact_quotes().collect::<Vec<_>>(), [selected]);
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
            app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
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
fn exact_and_literal_quotes_expand_in_place_with_the_existing_formats() {
    use crate::board::app::Request;
    let raw = "exact **raw**\r\n漢字\n";
    let mut app = source_reader(raw);
    app.reader_event(&key(KeyCode::Char('a'), KeyModifiers::NONE));
    app.paste("answer one");
    let literal = format!("{}\r\nsecond line", "語".repeat(250));
    app.add_chat_quote("worker".into(), literal.clone());
    app.paste("answer two");
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.text.folds().len(), 2);
    assert_eq!(input.exact_quotes().next().unwrap().text, raw);
    assert_eq!(input.literal_quotes()[0].text, literal);
    let expected = format!(
        "[product · worker]\n> exact **raw**\r\n> 漢字\n>\nanswer one\n> \"{}…\"\nanswer two",
        "語".repeat(199)
    );
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)),Effect::Act(Request::Annotate{text,..}) if text==expected)
    );
}
