use super::*;
use crate::board::app::{Request, tests::crew};
use ratatui::crossterm::event::KeyModifiers;
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn draw_completion(app: &App) -> ratatui::buffer::Buffer {
    use ratatui::{Terminal, backend::TestBackend};
    app.input_viewport.set(Some((Rect::new(0, 12, 100, 1), 0)));
    let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}
fn app() -> App {
    let mut app = crew(Default::default());
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
fn every_conversation_mode_names_the_actual_recipient_and_inserts_only_text() {
    for compose in [
        Compose::Talk {
            to: "worker".into(),
        },
        Compose::Annotate {
            to: "lead".into(),
            row: "worker".into(),
        },
        Compose::AskLead {
            to: "lead".into(),
            sender: "Ben".into(),
        },
    ] {
        let recipient = match &compose {
            Compose::Talk { to } | Compose::Annotate { to, .. } | Compose::AskLead { to, .. } => to,
            _ => unreachable!(),
        };
        let mut app = app();
        app.ask(String::new(), compose.clone(), "product".into());
        for ch in "@docs".chars() {
            assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
        }
        let buffer = draw_completion(&app);
        let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(painted.contains(&format!("Mentions are text only · sent to {recipient}")));
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.compose, compose);
        assert_eq!(input.text.as_str(), "@docs ");
        assert!(!app.mentions.completion.open());
    }
}

#[test]
fn clicking_a_painted_candidate_inserts_without_sending_or_retargeting() {
    let mut app = app();
    for ch in "@docs".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    draw_completion(&app);
    let (area, _) = app.mentions.hits.borrow()[0];
    let compose = app.input.as_ref().unwrap().compose.clone();
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now()
        ),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.text.as_str(), "@docs ");
    assert_eq!(input.compose, compose);
    assert!(!app.mentions.completion.open());
}

#[test]
fn fuzzy_picker_bolds_only_matched_label_characters_and_fills_the_selected_row_neutrally() {
    let mut app = app();
    for ch in "@dcs".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    let buffer = draw_completion(&app);
    assert_eq!(app.mentions.completion.matches[0].label, "docs");
    let (area, _) = app.mentions.hits.borrow()[0];
    let label = (area.x..area.right())
        .find(|x| buffer[(*x, area.y)].symbol() == "d")
        .unwrap();
    for (offset, bold) in [(0, true), (1, false), (2, true), (3, true)] {
        assert_eq!(
            buffer[(label + offset, area.y)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD),
            bold
        );
    }
    assert_eq!(
        buffer[(area.right() - 1, area.y)].bg,
        app.look().role(Role::Track).bg.unwrap()
    );
    assert!(
        !buffer[(label, area.y)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    let before = app.input.as_ref().unwrap().compose.clone();
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "@docs ");
    assert_eq!(app.input.as_ref().unwrap().compose, before);
}

#[test]
fn picker_stays_outside_the_whole_composer_band_including_its_header() {
    let mut app = app();
    for ch in "@docs".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    let band = Rect::new(0, 10, 100, 5);
    app.input_band.set(Some(band));
    draw_completion(&app);
    assert!(!app.mentions.hits.borrow().is_empty());
    for (area, _) in app.mentions.hits.borrow().iter() {
        let picker = Rect {
            height: area.height + 1,
            ..*area
        };
        assert!(
            picker.intersection(band).is_empty(),
            "candidate and picker footer must both stay outside {band:?}: {picker:?}"
        );
    }
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
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
    app.key(KeyEvent::new(
        KeyCode::End,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
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

fn message_surface(name: &str) -> App {
    let mut app = app();
    match name {
        "talk" => {}
        "reply" => {
            app.perform(&crate::action::Action::parse("reply").unwrap());
        }
        "annotation" => {
            app.ask(
                String::new(),
                Compose::Annotate {
                    to: "lead".into(),
                    row: "worker".into(),
                },
                "product".into(),
            );
        }
        "ask-lead" => {
            app.ask(
                String::new(),
                Compose::AskLead {
                    to: "lead".into(),
                    sender: "Ben".into(),
                },
                "product".into(),
            );
        }
        "status" => {
            let target = app.row_target(0).unwrap();
            let send = app.send_target(target, true).unwrap();
            app.attach_row(send, "product".into());
            app.input.as_mut().unwrap().compose = Compose::Status;
            app.start_status();
            app.status_draft.as_mut().unwrap().focus = 2;
        }
        "cron" => {
            use crate::board::cronboard::{detail_id, test_cron, test_view};
            let job = test_view("auth-fix", "old message", None);
            let id = detail_id(&job);
            app.cron.replace(Ok(test_cron(
                vec![job],
                tmt_ops::cron::ClockStatus::NoClock,
            )));
            assert_eq!(app.cron_key('E', Some(&id)), Effect::None);
        }
        _ => unreachable!(),
    }
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
    app.key(KeyEvent::new(
        KeyCode::End,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    key(&mut app, KeyCode::Backspace);
    app
}

#[test]
fn every_message_surface_owns_literal_keys_completion_escape_and_painted_mouse_choice() {
    for name in ["talk", "reply", "annotation", "ask-lead", "status", "cron"] {
        let mut app = message_surface(name);
        let compose = app.input.as_ref().unwrap().compose.clone();
        let current = app.current.clone();
        for ch in "@pxoj1".chars() {
            assert_eq!(
                key(&mut app, KeyCode::Char(ch)),
                Effect::None,
                "{name}: {ch}"
            );
        }
        assert_eq!(app.reference_editor().unwrap().as_str(), "@pxoj1");
        assert!(app.mentions.completion.open() && app.mentions.completion.matches.is_empty());
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().compose, compose);
        assert_eq!(app.current, current);
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        assert!(!app.mentions.completion.open() && app.input.is_some());
        assert_eq!(app.reference_editor().unwrap().as_str(), "@pxoj1");
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        app.key(KeyEvent::new(
            KeyCode::End,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        key(&mut app, KeyCode::Backspace);
        for ch in "@docs".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.mentions.completion.matches[0].kind, "agent");
        draw_completion(&app);
        let (area, _) = app.mentions.hits.borrow()[0];
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
        assert_eq!(app.reference_editor().unwrap().as_str(), "@docs ", "{name}");
        assert_eq!(app.input.as_ref().unwrap().compose, compose);
        assert_eq!(app.current, current);
        assert!(!app.mentions.completion.open());
        assert!(
            app.message_field(),
            "{name}: choosing a reference never advances the form"
        );
    }
}

#[test]
fn status_reference_completion_cannot_bypass_the_reason_byte_limit() {
    let mut app = message_surface("status");
    let original = format!("{} @d", "x".repeat(crate::board::app::INPUT_LIMIT - 3));
    assert_eq!(app.paste(&original), Effect::None);
    assert_eq!(app.reference_editor().unwrap().as_str(), original);
    assert_eq!(
        app.reference_editor().unwrap().as_str().len(),
        crate::board::app::INPUT_LIMIT
    );
    assert!(app.mentions.completion.open() && !app.mentions.completion.matches.is_empty());
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(
        app.reference_editor().unwrap().as_str(),
        original,
        "completion uses the same 4,000-byte bound as typing and paste"
    );
    assert!(app.status_draft.as_ref().unwrap().outcome.is_none());
}

#[test]
fn name_and_search_input_do_not_gain_message_completion() {
    let mut app = app();
    app.ask(String::new(), Compose::Workspace, "product".into());
    key(&mut app, KeyCode::Char('@'));
    assert!(!app.message_field() && !app.mentions.completion.open());
    app.input = None;
    app.searching = true;
    key(&mut app, KeyCode::Char('@'));
    assert_eq!(app.search, "@");
    assert!(!app.mentions.completion.open());
}

#[test]
fn message_completion_scrolls_candidates_and_refuses_stale_painted_geometry() {
    for name in ["talk", "reply", "annotation", "ask-lead", "status", "cron"] {
        let mut app = message_surface(name);
        app.references = crate::board::inventory::Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities": (0..12).map(|index| serde_json::json!({
                "id": format!("acquired-id-{index}"), "name": format!("docs-{index:02}")
            })).collect::<Vec<_>>()}),
            ),
        );
        let _no_acquisition = crate::runner::forbid_commands();
        let compose = app.input.as_ref().unwrap().compose.clone();
        app.paste("@docs-");
        assert_eq!(app.mentions.completion.matches.len(), 12, "{name}");
        assert_eq!(key(&mut app, KeyCode::PageDown), Effect::None);
        assert_eq!(app.mentions.completion.selected, 6);
        draw_completion(&app);
        let area = app.mentions.hits.borrow()[0].0;
        let mouse = |kind| MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            app.mouse(mouse(MouseEventKind::ScrollDown), std::time::Instant::now()),
            Effect::None
        );
        assert_eq!(app.mentions.completion.selected, 7);
        draw_completion(&app);
        app.invalidate_frame_geometry();
        // The completion owner relinquishes invalidated hits; outside-editor
        // focus and draft retention are handled by the input owner.
        assert!(!app.mention_mouse(mouse(MouseEventKind::Down(MouseButton::Left))));
        assert_eq!(
            app.reference_editor().unwrap().as_str(),
            "@docs-",
            "{name}: stale choice must not insert"
        );
        assert_eq!(app.input.as_ref().unwrap().compose, compose);
        draw_completion(&app);
        let (last, index) = *app.mentions.hits.borrow().last().unwrap();
        let chosen = app.mentions.completion.matches[index].insertion.clone();
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: last.x,
                    row: last.y,
                    modifiers: KeyModifiers::NONE
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
        assert_eq!(
            app.reference_editor().unwrap().as_str(),
            format!("{chosen} "),
            "{name}"
        );
        assert_eq!(app.input.as_ref().unwrap().compose, compose);
        assert!(app.message_field());
    }
}
