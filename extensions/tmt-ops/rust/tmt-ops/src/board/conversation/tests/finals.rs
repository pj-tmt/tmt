use super::*;
use crate::board::recent_responses::{Item, Observation};

#[test]
fn settled_final_after_older_pane_text_is_its_own_message_with_exact_lines_and_age() {
    crate::status::with_now_ms(3_600_000, || {
        let mut app = workspace_fixture();
        let agent = crate::board::app::workspace::tests::MEMBER;
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":agent,"requestId":"older-request",
            "submittedAtMs":600_000,"response":"Older final first\nOlder final second"
        })];
        draw(&app, 148, 44);
        let key = app.pane_output_read().unwrap();
        app.apply_pane_output(
            key,
            Observation::Observed(vec![Item {
                text: "Earlier pane first\nEarlier pane second".into(),
                ended_ms: 1_200_000,
                truncated: false,
            }]),
        );
        app.scroll_pane(Pane::Replies, Step::Top);
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":agent,"requestId":"later-request",
            "submittedAtMs":3_540_000,"response":"New retained final first\nNew retained final second"
        }));
        draw(&app, 148, 44);
        let state = app.conversation.borrow();
        let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
        let mapped = &session.rendered.as_ref().unwrap().mapped;
        let lines: Vec<_> = mapped.lines.iter().map(Line::to_string).collect();
        let at = |text: &str| {
            lines
                .iter()
                .position(|line| line.contains(text))
                .unwrap_or_else(|| panic!("missing {text}: {lines:#?}"))
        };
        let older = at("Older final first");
        let pane = at("pane output");
        let later = at("New retained final first");
        assert!(older < pane && pane < later, "{lines:#?}");
        assert_eq!(at("Older final second"), older + 1);
        assert_eq!(at("Earlier pane second"), at("Earlier pane first") + 1);
        assert_eq!(at("New retained final second"), later + 1);
        assert!(
            lines[later - 1].contains("worker") && lines[later - 1].contains("1m ago"),
            "{lines:#?}"
        );
        assert!(lines[older - 1].contains("50m ago"), "{lines:#?}");
        assert!(lines[pane].contains("40m ago"));
        assert!(lines[pane].starts_with(" worker · pane output"));
        assert_eq!(
            session
                .response_key_at(mapped.sources[later])
                .unwrap()
                .0
                .request,
            "later-request"
        );
        assert!(
            session
                .response_key_at(mapped.sources[at("Earlier pane first")])
                .is_none()
        );
    });
}

#[test]
fn identical_capture_and_final_remain_two_blocks_with_only_the_final_as_source() {
    crate::status::with_now_ms(3_600_000, || {
        let mut app = workspace_fixture();
        let agent = crate::board::app::workspace::tests::MEMBER;
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":agent,"requestId":"stored-final",
            "submittedAtMs":3_540_000,"response":"Same exact answer\n第二 line"
        })];
        draw(&app, 100, 44);
        let key = app.pane_output_read().unwrap();
        app.apply_pane_output(
            key,
            Observation::Observed(vec![Item {
                text: "Same exact answer\n第二 line".into(),
                ended_ms: 1_200_000,
                truncated: false,
            }]),
        );
        draw(&app, 100, 44);
        let state = app.conversation.borrow();
        let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
        let mapped = &session.rendered.as_ref().unwrap().mapped;
        let rows: Vec<_> = mapped
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.to_string().contains("Same exact answer"))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            rows.len(),
            2,
            "pane text is never deduplication evidence for a stored final"
        );
        assert!(session.response_key_at(mapped.sources[rows[0]]).is_none());
        assert_eq!(
            session
                .response_key_at(mapped.sources[rows[1]])
                .unwrap()
                .0
                .request,
            "stored-final"
        );
        assert!(mapped.lines[rows[0] + 1].to_string().contains("第二 line"));
        assert!(mapped.lines[rows[1] + 1].to_string().contains("第二 line"));
    });
}

#[test]
fn a_pending_body_refresh_keeps_acquired_finals_and_each_header_ages_at_paint() {
    let mut app = workspace_fixture();
    let agent = crate::board::app::workspace::tests::MEMBER;
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":agent,"requestId":"stored-final",
        "submittedAtMs":3_540_000,"response":"Acquired exact body"
    })];
    crate::status::with_now_ms(3_600_000, || {
        draw(&app, 100, 44);
    });
    app.view.as_mut().unwrap().replies[0]["response"] = serde_json::Value::Null;
    crate::status::with_now_ms(3_660_000, || {
        let shown = draw(&app, 100, 44);
        assert!(shown.contains("Acquired exact body"));
        assert!(shown.contains("2m ago"));
        assert!(!shown.contains("reading reply"));
    });
}

#[test]
fn pending_final_text_cannot_supply_an_exact_note_attachment() {
    let mut app = workspace_fixture();
    let agent = crate::board::app::workspace::tests::MEMBER;
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":agent,"requestId":"pending-body","submittedAtMs":3_540_000
    })];
    draw(&app, 100, 44);
    let state = app.conversation.borrow();
    let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
    let rendered = session.rendered.as_ref().unwrap();
    assert_eq!(session.finals.len(), 1);
    assert!(!session.finals[0].acquired);
    assert!(!rendered.final_spans.is_empty());
    let reply = session.finals[0].clone();
    drop(state);
    let quote = crate::board::response::Quote {
        key: reply.key.unwrap(),
        range: 0..reply.body.len(),
        text: reply.body,
    };
    let target = crate::board::row_detail::Target::Row(app.conversation_target().unwrap());
    assert!(!app.response_quote_current(&target, &quote));
}

#[test]
fn final_member_links_hit_the_painted_name_in_both_conversation_placements() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    for grouped in [false, true] {
        let mut app = workspace_fixture();
        let agent = crate::board::app::workspace::tests::MEMBER;
        app.references = crate::board::inventory::Inventory::from_listed(
            &[],
            &Ok(json!({"identities":[{"id":agent,"name":"worker","canonicalName":"worker"}]})),
        );
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":agent,"requestId":"linked-final","submittedAtMs":3_540_000,
            "response":"Ping #tmt:worker / #tmt:worker"
        })];
        let mut terminal = Terminal::new(TestBackend::new(148, 51)).unwrap();
        terminal
            .draw(|frame| {
                if grouped {
                    crate::board::view::prepare(&mut app, frame.area());
                    crate::board::view::render(frame, &app);
                } else {
                    app.member_links.borrow_mut().begin_frame(frame.area());
                    render(frame, &app, frame.area());
                }
            })
            .unwrap();
        let rows: Vec<_> = terminal
            .backend()
            .buffer()
            .content
            .chunks(148)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect();
        let y = rows
            .iter()
            .position(|row| row.contains("Ping worker / worker"))
            .unwrap();
        let byte = rows[y].rfind("worker").unwrap();
        let x = rows[y][..byte].width();
        app.member_link_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x as u16,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        });
        app.member_link_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: x as u16,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        });
        let anchor = app
            .menu
            .as_ref()
            .expect("a click on the painted name opens its existing member menu")
            .anchor
            .unwrap();
        assert_eq!(
            (anchor.x, anchor.y, anchor.width),
            (x as u16, y as u16, 6),
            "grouped={grouped}"
        );
    }
}

#[test]
#[ignore = "writes explicit settled-reply look evidence to a task-owned path only"]
fn capture_settled_reply_packet() {
    let output = std::env::var_os("TMT_SETTLED_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    let mut frames = Vec::new();
    crate::status::with_now_ms(3_600_000, || {
        for (name, base, depth) in [
            (
                "tmt",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "tmt-light",
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "terminal",
                tmt_cli_style::Base::Terminal,
                tmt_cli_style::Depth::Ansi16,
            ),
            (
                "NO_COLOR",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::None,
            ),
        ] {
            for width in [160, 100, 80, 60] {
                for held in [false, true] {
                    let mut app = workspace_fixture();
                    let agent = crate::board::app::workspace::tests::MEMBER;
                    app.view.as_mut().unwrap().look = Look {
                        theme: tmt_cli_style::Theme::new(base),
                        depth,
                    };
                    app.focus = 1;
                    app.view.as_mut().unwrap().replies = vec![json!({
                        "recipientId":agent,"requestId":"older-request","submittedAtMs":600_000,
                        "response":"Earlier stored final.\nIts second line remains separate."
                    })];
                    workspace(&mut app, width, 44);
                    let key = app
                        .pane_output_read()
                        .expect("selected member output scope");
                    app.apply_pane_output(key, Observation::Observed(vec![Item {
                        text: "Earlier pane text.\nA captured bullet list:\n- first item\n- second item".into(),
                        ended_ms: 1_200_000, truncated: false,
                    }]));
                    if held {
                        app.scroll_pane(Pane::Replies, Step::Top);
                    }
                    app.view.as_mut().unwrap().replies.push(json!({
                        "recipientId":agent,"requestId":"later-request","submittedAtMs":3_540_000,
                        "response":"New retained reply.\nThis is its own second line.\nNo pane timestamp is reused."
                    }));
                    let mut terminal = Terminal::new(TestBackend::new(width, 44)).unwrap();
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    frames.push(json!({"scene":if held {"held-reply"} else {"settled-reply"},"base":name,"depth":format!("{depth:?}"),"width":width,"height":44,
                        "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                        "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
                }
            }
        }
    });
    std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
}

#[test]
fn stored_finals_render_in_full_and_keep_their_source_after_wrapping_and_resize() {
    let mut app = workspace_fixture();
    let agent = crate::board::app::workspace::tests::MEMBER;
    let long = (0..24).map(|n| format!("older final line {n:02} with a wrapping tail that remains exact across terminal widths")).collect::<Vec<_>>().join("\n");
    app.view.as_mut().unwrap().replies = vec![
        json!({"recipientId":agent,"requestId":"long-older","submittedAtMs":1000,"response":long}),
        json!({"recipientId":agent,"requestId":"long-newest","submittedAtMs":2000,"response":(0..20).map(|n|format!("newest final {n:02}")).collect::<Vec<_>>().join("\n")}),
    ];
    for width in [148, 60, 100] {
        draw(&app, width, 44);
        let state = app.conversation.borrow();
        let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
        let rendered = session.rendered.as_ref().unwrap();
        let lines: Vec<_> = rendered.mapped.lines.iter().map(Line::to_string).collect();
        assert!(
            !lines.iter().any(|line| line.contains("more lines")),
            "no final folds: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("older final line 23")),
            "the older final renders every line: {lines:#?}"
        );
        let newest = lines
            .iter()
            .position(|line| line.contains("newest final 19"))
            .expect("newest final renders in full");
        assert_eq!(
            session.response_key_at(newest).unwrap().0.request,
            "long-newest"
        );
        let older = lines
            .iter()
            .position(|line| line.contains("older final line 00"))
            .unwrap();
        assert_eq!(
            session.response_key_at(older).unwrap().0.request,
            "long-older"
        );
        drop(state);
        let target = app.conversation_target().unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, 44)).unwrap();
        terminal
            .draw(|frame| {
                super::super::exchange::render_for(
                    frame,
                    &app,
                    frame.area(),
                    &super::super::exchange::Surface::Agents {
                        target: target.clone(),
                        interactive: true,
                    },
                )
            })
            .unwrap();
        let rows: Vec<String> = terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let headers: Vec<_> = rows.iter().filter(|row| row.contains("worker →")).collect();
        assert!(!headers.is_empty());
        assert!(
            headers
                .iter()
                .all(|row| row.trim_start().starts_with("worker →")),
            "a long body must not turn a message header into body text: {headers:?}"
        );
        let row = rows
            .iter()
            .find(|row| row.contains("newest final 19"))
            .unwrap();
        let at = row.find("newest final 19").unwrap();
        assert!(
            row[..at].trim_end().ends_with('│'),
            "a final body row keeps the body gutter: {row}"
        );
    }
}
