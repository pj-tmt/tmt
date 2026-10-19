use super::*;
use crate::board::app::workspace::tests::MEMBER;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

fn fixture(source: &str) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    app
}
fn draw(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) -> Effect {
    app.mouse(
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    )
}
#[test]
fn the_first_rendered_drag_selects_in_place_and_mouse_up_copies_visible_text() {
    let mut app = fixture("first **styled** passage\nnext line");
    let before = draw(&mut app, 100, 36);
    let (row, column) = (0..before.area.height)
        .find_map(|y| {
            (0..before.area.width)
                .find(|x| {
                    let tail: String = (*x..before.area.width)
                        .map(|column| before[(column, y)].symbol())
                        .collect();
                    tail.starts_with("first styled passage")
                })
                .map(|x| (y, x))
        })
        .expect("rendered message body is visible");
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        column,
        row,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        column + 12,
        row,
    );
    let after = draw(&mut app, 100, 36);
    let painted: String = after.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        !painted.contains("Exact response source"),
        "the first drag must keep the rendered conversation in place"
    );
    let copied = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        column + 12,
        row,
    );
    assert!(
        matches!(copied, Effect::Act(crate::board::app::Request::Copy { text, .. }) if text == "first styled")
    );
}

fn select_words(app: &mut App, words: &str) -> (u16, u16) {
    select_at_width(app, words, 100)
}
fn select_at_width(app: &mut App, words: &str, width: u16) -> (u16, u16) {
    let buffer = draw(app, width, 36);
    let (x, y) = (0..buffer.area.height)
        .find_map(|y| {
            (0..buffer.area.width)
                .find(|x| {
                    let line: String = (*x..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    line.starts_with(words)
                })
                .map(|x| (x, y))
        })
        .unwrap_or_else(|| {
            panic!(
                "the visible passage exists at {width}:\n{}",
                buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(
        app,
        MouseEventKind::Drag(MouseButton::Left),
        x + words.chars().count() as u16,
        y,
    );
    (x, y)
}

#[test]
fn refresh_keeps_the_selected_literal_quote_without_request_identity_checks() {
    let mut app = fixture("literal passage");
    let (x, y) = select_words(&mut app, "literal passage");
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 15, y);
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":MEMBER,"requestId":"replacement","response":"different reply"})];
    app.conversation.borrow_mut().invalidate_source_geometry();
    draw(&mut app, 100, 36);
    let effect = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(effect, Effect::None);
    let input = app
        .input
        .as_ref()
        .expect("quote opens the admitted member draft");
    assert_eq!(input.chat_quotes[0].text, "literal passage");
    assert!(input.text.is_empty(), "a chip does not become editor text");
    assert!(
        input.response_quotes.is_empty(),
        "literal chips grant no exact-source authority"
    );
}

#[test]
fn resize_stops_a_stale_drag_without_clearing_the_frozen_quote() {
    let mut app = fixture("literal passage");
    let (x, y) = select_words(&mut app, "literal");
    draw(&mut app, 60, 24);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 14, y);
    assert_eq!(
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 14, y),
        Effect::None
    );
    let state = app.conversation.borrow();
    let source = state.selection.as_ref().unwrap();
    assert_eq!(source.selected().as_deref(), Some("literal"));
    assert!(!source.dragging);
}

#[test]
fn escape_and_outside_press_clear_only_the_selection() {
    let mut app = fixture("literal passage");
    select_words(&mut app, "literal");
    assert!(app.conversation_source_escape());
    assert!(app.conversation.borrow().selection.is_none());
    select_words(&mut app, "literal");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 0, 0);
    assert!(app.conversation.borrow().selection.is_none());
    assert!(app.input.is_none());
}

#[test]
fn read_focus_shift_arrows_and_y_copy_plain_text() {
    let mut app = fixture("literal passage");
    draw(&mut app, 100, 36);
    app.focus = 1;
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    let selected = app
        .conversation
        .borrow()
        .selection
        .as_ref()
        .unwrap()
        .selected()
        .unwrap();
    assert_eq!(selected.chars().count(), 1);
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Char('y'),KeyModifiers::NONE)),
        Effect::Act(Request::Copy {text, feedback: CopyFeedback::Selection, ..}) if text == selected)
    );
}

#[test]
#[ignore = "writes task-owned selection/chip evidence, never frozen fixtures"]
fn capture_rendered_quote_chip_packet() {
    let output = std::env::var_os("TMT_RENDERED_QUOTES_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut frames = Vec::new();
        for (base_name, base, depth) in [
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
                "NO_COLOR",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::None,
            ),
        ] {
            for width in [160, 100, 60] {
                let mut app = fixture("first **styled** passage\nnext line");
                let look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                app.view.as_mut().unwrap().look = look;
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .squad_looks
                    .clear();
                if width < 140 {
                    app.focus = 1;
                }
                let (x, y) = select_at_width(&mut app, "first styled", width);
                for scene in [
                    "selected-in-place",
                    "one-chip",
                    "three-chips",
                    "chip-focused",
                    "sent",
                ] {
                    match scene {
                        "one-chip" => {
                            mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 12, y);
                            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                            app.paste("my retained comment");
                        }
                        "three-chips" => {
                            app.add_chat_quote("Ben".into(), "Earlier request".into());
                            app.add_chat_quote("worker".into(), "One more reply".into());
                        }
                        "chip-focused" => {
                            app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
                            app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                        }
                        "sent" => {
                            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                            let effect = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                            assert!(
                                matches!(effect, Effect::Act(_)),
                                "send is an effect, not a quote activation"
                            );
                        }
                        _ => (),
                    }
                    let buffer = draw(&mut app, width, 36);
                    frames.push(json!({"scene":scene,"base":base_name,"depth":format!("{depth:?}"),"width":width,"height":36,
                        "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                        "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                        "inputBand":format!("{:?}",app.input_band.get())}));
                }
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    });
}

#[test]
fn task_fields_select_their_rendered_text_without_opening_a_source_view() {
    let mut app = fixture("separate reply");
    let selected = app.workspace.selected.clone().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&selected)
        .unwrap()["fields"] = json!({"task":"task proof"});
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .lead = Some(MEMBER.into());
    app.choose(crate::board::app::Choice::WorkspaceTarget(
        crate::board::workspace::OpsTarget::Squad(crate::board::app::workspace::tests::ROOM.into()),
    ));
    app.focus = 1;
    let buffer = draw(&mut app, 100, 36);
    let area = app
        .conversation
        .borrow()
        .source_surfaces
        .iter()
        .find(|surface| surface.text().contains("task proof"))
        .expect("the actual task painter publishes its displayed cells")
        .area;
    let line: String = (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect();
    let x = area.x + line.find("task proof").unwrap() as u16;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, area.y);
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        x + 10,
        area.y,
    );
    assert!(
        matches!(mouse(&mut app,MouseEventKind::Up(MouseButton::Left),x+10,area.y),
        Effect::Act(Request::Copy{text,..}) if text=="task proof")
    );
}

#[test]
fn literal_selection_keeps_wide_and_combining_glyphs_whole() {
    let mut app = fixture("漢字 e\u{301} end");
    let buffer = draw(&mut app, 100, 36);
    let (x, y) = (0..buffer.area.height)
        .find_map(|y| {
            (0..buffer.area.width)
                .find(|x| buffer[(*x, y)].symbol() == "漢")
                .map(|x| (x, y))
        })
        .unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 6, y);
    assert!(
        matches!(mouse(&mut app,MouseEventKind::Up(MouseButton::Left),x+6,y),
        Effect::Act(Request::Copy{text,..}) if text=="漢字 e\u{301}")
    );
}

#[test]
fn a_linked_name_can_be_drag_selected_and_still_opens_its_menu_on_click() {
    let mut app = fixture("Mention #tmt:worker here");
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[{"id":MEMBER,"name":"worker","canonicalName":"worker"}]})),
    );
    let buffer = draw(&mut app, 100, 36);
    let (x, y) = (0..buffer.area.height)
        .find_map(|y| {
            (0..buffer.area.width)
                .find(|x| {
                    let line: String = (*x..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    line.starts_with("Mention worker here")
                })
                .map(|x| (x + 8, y))
        })
        .expect("resolved mention is painted in the body");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    assert!(app.menu.is_none(), "Down must permit a first drag");
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 6, y);
    assert!(
        matches!(mouse(&mut app,MouseEventKind::Up(MouseButton::Left),x+6,y),Effect::Act(Request::Copy{text,..}) if text=="worker")
    );
    assert!(app.menu.is_none());
    draw(&mut app, 100, 36);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, y);
    assert!(
        app.menu.is_some(),
        "an actual click still opens the existing member menu"
    );
    assert!(app.conversation.borrow().selection.is_none());
}

#[test]
fn the_actual_read_focus_footer_explains_rendered_selection() {
    let mut app = fixture("literal passage");
    app.focus = 1;
    let buffer = draw(&mut app, 100, 36);
    let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(painted.contains("select text to copy"), "{painted}");
    assert!(
        painted.contains("Esc back"),
        "effective Escape hint stays visible"
    );
}

#[test]
fn leaving_the_conversation_never_reuses_its_old_frame_rectangle() {
    let mut app = fixture("literal passage");
    draw(&mut app, 100, 36);
    assert!(!app.conversation.borrow().source_surfaces.is_empty());
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    draw(&mut app, 100, 36);
    let state = app.conversation.borrow();
    assert!(state.response_area.is_none());
    assert!(state.source_surfaces.is_empty());
}

#[test]
fn changed_text_keeps_the_literal_quote_without_stray_partial_highlights() {
    let mut app = fixture("literal passage");
    let (x, y) = select_words(&mut app, "literal passage");
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 15, y);
    let replacement = "lXteral pXssage";
    app.view.as_mut().unwrap().replies[0]["response"] = json!(replacement);
    let changed = draw(&mut app, 100, 36);
    let mut plain = fixture(replacement);
    let expected = draw(&mut plain, 100, 36);
    for column in x..x + replacement.len() as u16 {
        assert_eq!(
            changed[(column, y)],
            expected[(column, y)],
            "stale selection must not highlight coincidental glyphs"
        );
    }
    assert_eq!(
        app.conversation
            .borrow()
            .selection
            .as_ref()
            .unwrap()
            .selected()
            .as_deref(),
        Some("literal passage")
    );
}
