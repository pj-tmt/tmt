use super::*;
use crate::board::app::workspace::tests::MEMBER;
use crate::board::scroll::Step;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

fn fixture(source: &str) -> App {
    fixture_with_look(source, crate::look::Look::default())
}

fn fixture_with_look(source: &str, look: crate::look::Look) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    view.replies = vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    app
}

fn selection_looks() -> impl Iterator<Item = crate::look::Look> {
    [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight]
        .into_iter()
        .flat_map(|base| {
            [
                tmt_cli_style::Depth::TrueColor,
                tmt_cli_style::Depth::Ansi16,
                tmt_cli_style::Depth::None,
            ]
            .into_iter()
            .map(move |depth| crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            })
        })
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
    assert_eq!(input.literal_quotes()[0].text, "literal passage");
    assert_eq!(
        input.text.as_str(),
        "literal passage\n",
        "Editor owns the full quote source"
    );
    assert!(
        input.exact_quotes().next().is_none(),
        "literal chips grant no exact-source authority"
    );
}

#[test]
fn a_resize_that_rewraps_the_transcript_ends_the_selection_and_its_keys() {
    let mut app = fixture("literal passage");
    let (x, y) = select_words(&mut app, "literal");
    assert!(app.conversation.borrow().selection.is_some());
    // The wrap width changes: the columns no longer count in the same text, so the
    // selection cannot be shown and must not stay live behind the user's back.
    draw(&mut app, 60, 24);
    assert!(
        app.conversation.borrow().selection.is_none(),
        "a selection that cannot be seen ends"
    );
    assert_eq!(
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 14, y),
        Effect::None
    );
    assert_eq!(
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 14, y),
        Effect::None
    );
    app.focus = 1;
    assert!(
        !matches!(
            app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            Effect::Act(Request::Copy { .. })
        ),
        "y copies nothing once the selection ended"
    );
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

fn task_field_fixture(look: crate::look::Look) -> App {
    let mut app = fixture_with_look("separate reply", look);
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
    app
}

fn task_field_start(app: &App, buffer: &ratatui::buffer::Buffer) -> (u16, u16) {
    let area = app
        .conversation
        .borrow()
        .source_surfaces
        .iter()
        .find(|surface| surface.text().contains("task proof"))
        .expect("the actual task painter publishes its displayed cells")
        .area;
    for y in area.y..area.bottom() {
        let line: String = (area.x..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(x) = line.find("task proof") {
            return (area.x + x as u16, y);
        }
    }
    panic!("the published task field contains its displayed text");
}

#[test]
fn task_fields_select_their_rendered_text_without_opening_a_source_view() {
    for look in selection_looks() {
        let mut app = task_field_fixture(look);
        let buffer = draw(&mut app, 100, 36);
        let (x, y) = task_field_start(&app, &buffer);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 10, y);
        let highlighted = draw(&mut app, 100, 36);
        assert!(matches!(
            app.conversation.borrow().selection,
            Some(Selection::Cells(_))
        ));
        assert_ne!(highlighted[(x, y)], buffer[(x, y)], "{look:?}");
        if look.depth == tmt_cli_style::Depth::None {
            assert!(
                highlighted[(x, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            );
        }
        assert_eq!(selected(&app).as_deref(), Some("task proof"));
        assert!(
            matches!(mouse(&mut app,MouseEventKind::Up(MouseButton::Left),x+10,y),
            Effect::Act(Request::Copy{text,..}) if text=="task proof")
        );
    }
}

#[test]
fn literal_selection_keeps_wide_and_combining_glyphs_whole() {
    for look in selection_looks() {
        let mut app = fixture_with_look("漢字 e\u{301} end", look);
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
fn changed_text_ends_the_selection_without_stray_partial_highlights() {
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
    assert!(
        app.conversation.borrow().selection.is_none(),
        "a selection whose text changed can no longer be seen, so it ends"
    );
}

/// Tall enough to show a good part of an 80-line reply.
const TALL: u16 = 80;

fn long(count: usize) -> String {
    (0..count).map(|n| format!("passage {n:03}  \n")).collect()
}

fn locate(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    (0..buffer.area.height).find_map(|y| {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        row.find(needle)
            .map(|at| (row[..at].chars().count() as u16, y))
    })
}

/// Whether `needle` is painted as selected: the same frame without a selection
/// styles its first glyph differently.
fn marked(app: &mut App, needle: &str) -> bool {
    let with = draw(app, 100, TALL);
    let Some((x, y)) = locate(&with, needle) else {
        return false;
    };
    let kept = app.conversation.borrow_mut().selection.take();
    let without = draw(app, 100, TALL);
    app.conversation.borrow_mut().selection = kept;
    with[(x, y)] != without[(x, y)]
}

fn selected(app: &App) -> Option<String> {
    app.conversation
        .borrow()
        .selection
        .as_ref()
        .and_then(Selection::selected)
}

fn quote_button(app: &App) -> Option<Rect> {
    app.conversation
        .borrow()
        .selection
        .as_ref()
        .and_then(Selection::control)
}

/// Press on `words` and drag to their end; the button is not released.
fn press(app: &mut App, words: &str) -> (u16, u16) {
    let (x, y) = locate(&draw(app, 100, TALL), words).expect("the passage is in view");
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(
        app,
        MouseEventKind::Drag(MouseButton::Left),
        x + words.len() as u16,
        y,
    );
    (x, y)
}

fn pick(app: &mut App, words: &str) {
    let (x, y) = press(app, words);
    mouse(
        app,
        MouseEventKind::Up(MouseButton::Left),
        x + words.len() as u16,
        y,
    );
}

fn replace_reply(app: &mut App, text: String) {
    app.view.as_mut().unwrap().replies[0]["response"] = json!(text);
}

#[test]
fn a_selection_follows_its_text_through_a_new_message_and_scrolling() {
    for look in selection_looks() {
        let mut app = fixture_with_look(&long(80), look);
        pick(&mut app, "passage 070");
        assert!(marked(&mut app, "passage 070"), "{look:?}");
        assert!(quote_button(&app).is_some());

        replace_reply(&mut app, format!("{}fresh news\n", long(80)));
        assert!(marked(&mut app, "passage 070"), "{look:?}");
        assert!(quote_button(&app).is_some());
        assert_eq!(selected(&app).as_deref(), Some("passage 070"));

        let before = locate(&draw(&mut app, 100, TALL), "passage 070").unwrap().1;
        app.scroll_pane(Pane::Replies, Step::Lines(-4));
        let after = locate(&draw(&mut app, 100, TALL), "passage 070").unwrap().1;
        assert_eq!(after, before + 4);
        assert!(marked(&mut app, "passage 070"), "it moves with its text");
        assert!(quote_button(&app).is_some());

        app.scroll_pane(Pane::Replies, Step::Top);
        assert!(!marked(&mut app, "passage 070"));
        assert!(
            quote_button(&app).is_none(),
            "no button for text out of view"
        );
        assert_eq!(selected(&app).as_deref(), Some("passage 070"));

        app.scroll_pane(Pane::Replies, Step::Bottom);
        assert!(
            marked(&mut app, "passage 070"),
            "restored when scrolled back"
        );
        assert!(quote_button(&app).is_some());
    }
}

#[test]
fn a_selection_follows_its_text_when_lines_are_inserted_above_it() {
    for look in selection_looks() {
        let mut app = fixture_with_look(&long(80), look);
        pick(&mut app, "passage 070");
        replace_reply(&mut app, format!("one\ntwo\nthree\n\n{}", long(80)));
        assert!(marked(&mut app, "passage 070"), "{look:?}");
        assert_eq!(selected(&app).as_deref(), Some("passage 070"));
    }
}

#[test]
fn a_selection_survives_the_input_band_growing_and_still_quotes() {
    for look in selection_looks() {
        let mut app = fixture_with_look(&long(80), look);
        pick(&mut app, "passage 070");
        app.perform(&crate::action::Action::parse("talk").unwrap());
        assert!(app.input.is_some(), "the draft opens under the transcript");
        app.paste("a\nb\nc\nd\ne");
        assert!(marked(&mut app, "passage 070"), "{look:?}");
        let button = quote_button(&app).expect("the button stays");
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            button.x,
            button.y,
        );
        let quotes = &app.input.as_ref().unwrap().literal_quotes();
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].text, "passage 070");
    }
}

#[test]
fn quote_does_not_depend_on_what_the_agent_is_doing() {
    use crate::board::activity::{ObservedActivity, Phase};
    for phase in [Phase::Working, Phase::Idle, Phase::Unknown] {
        let mut app = fixture(&long(10));
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .members
            .get_mut(MEMBER)
            .unwrap()
            .activity = ObservedActivity {
            phase,
            since_ms: Some(1),
            last_ms: Some(1),
        };
        pick(&mut app, "passage 005");
        draw(&mut app, 100, TALL);
        assert!(quote_button(&app).is_some(), "{phase:?}");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let quotes = &app.input.as_ref().unwrap().literal_quotes();
        assert_eq!(quotes[0].text, "passage 005", "{phase:?}");
    }
}

#[test]
fn a_drag_keeps_working_while_the_history_updates() {
    let mut app = fixture(&long(80));
    let (x, y) = press(&mut app, "passage 070");
    replace_reply(&mut app, format!("{}tick\n", long(80)));
    draw(&mut app, 100, TALL);
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        x + 11,
        y + 2,
    );
    draw(&mut app, 100, TALL);
    let effect = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        x + 11,
        y + 2,
    );
    assert!(
        matches!(&effect, Effect::Act(Request::Copy { text, .. })
            if text == "passage 070\n │ passage 071\n │ passage 072"),
        "{effect:?}"
    );
}

#[test]
fn dragging_to_the_edge_scrolls_and_keeps_extending() {
    let mut app = fixture(&long(80));
    draw(&mut app, 100, TALL);
    app.scroll_pane(Pane::Replies, Step::Top);
    let (x, y) = locate(&draw(&mut app, 100, TALL), "passage 001").unwrap();
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    let area = app.conversation.borrow().response_area.unwrap();
    let before = app.scrolls.offset(Pane::Replies);
    let edge = area.y + (area.height - 2);
    for _ in 0..3 {
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            x + 11,
            edge,
        );
        draw(&mut app, 100, TALL);
    }
    assert_eq!(app.scrolls.offset(Pane::Replies), before + 3);
    let effect = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        x + 11,
        edge,
    );
    let Effect::Act(Request::Copy { text, .. }) = effect else {
        panic!("{effect:?}");
    };
    assert!(text.starts_with("passage 001"), "{text}");
    let last = text.lines().last().unwrap();
    let rows = usize::from(edge - y);
    assert!(
        text.lines().count() >= rows + 3,
        "{} rows of {text}",
        text.lines().count()
    );
    assert!(last.contains("passage"), "{last}");

    // Back at the top edge it scrolls up again.
    let top = area.y;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, edge);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x, top);
    assert_eq!(app.scrolls.offset(Pane::Replies), before + 2);
}

#[test]
fn a_keyboard_selection_quotes_whole_lines_without_the_mouse() {
    let mut app = fixture(&long(10));
    draw(&mut app, 100, TALL);
    app.focus = 1;
    for _ in 0..2 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    }
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    draw(&mut app, 100, TALL);
    assert!(quote_button(&app).is_some());
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let quotes = &app.input.as_ref().unwrap().literal_quotes();
    let text = &quotes[0].text;
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains(" │ passage 000"), "{text}");
}

#[test]
fn a_selection_ends_when_its_lines_are_gone() {
    let mut app = fixture(&long(80));
    pick(&mut app, "passage 070");
    replace_reply(&mut app, "short".into());
    draw(&mut app, 100, TALL);
    assert!(app.conversation.borrow().selection.is_none());
}

/// A busy agent streams into the message being read (#4344): the selected words stay,
/// whatever follows them on the same line grows, and Copy and Quote keep working.
#[test]
fn a_selection_survives_streaming_into_the_line_it_ends_on() {
    let mut app = fixture("alpha beta gamma");
    pick(&mut app, "beta");
    assert_eq!(selected(&app).as_deref(), Some("beta"));
    replace_reply(&mut app, "alpha beta gamma delta epsilon".into());
    draw(&mut app, 100, TALL);
    assert_eq!(
        selected(&app).as_deref(),
        Some("beta"),
        "streamed words after the selection do not end it"
    );
    assert!(quote_button(&app).is_some());
}

/// The same, with the selection ending on the last (still growing) line of a longer message.
#[test]
fn a_selection_in_the_growing_last_paragraph_keeps_copy_and_quote() {
    let mut app = fixture(&format!("{}tail words", long(10)));
    let (x, y) = press(&mut app, "tail words");
    replace_reply(&mut app, format!("{}tail words and more", long(10)));
    draw(&mut app, 100, TALL);
    let copied = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        x + "tail words".len() as u16,
        y,
    );
    assert!(
        matches!(&copied, Effect::Act(Request::Copy { text, .. }) if text == "tail words"),
        "{copied:?}"
    );
    draw(&mut app, 100, TALL);
    assert_eq!(
        selected(&app).as_deref(),
        Some("tail words"),
        "still selected"
    );
    assert!(quote_button(&app).is_some());
}

/// New messages that make the transcript taller than the pane add a scrollbar column,
/// which narrows the wrap: the selection must not end just because it got longer.
#[test]
fn a_selection_survives_the_transcript_outgrowing_the_pane() {
    let mut app = fixture(&long(3));
    pick(&mut app, "passage 001");
    assert!(selected(&app).is_some());
    replace_reply(&mut app, long(120));
    draw(&mut app, 100, TALL);
    assert_eq!(
        selected(&app).as_deref(),
        Some("passage 001"),
        "a longer transcript keeps the selection"
    );
}

/// A stamp that ticks after the selected words (`3s ago` → `4s ago`) is the same line.
#[test]
fn a_selection_ends_before_a_ticking_stamp_and_survives_it() {
    let mut app = fixture("Ben to auth-fix 3s ago");
    pick(&mut app, "Ben to auth-fix");
    replace_reply(&mut app, "Ben to auth-fix 4s ago".into());
    draw(&mut app, 100, TALL);
    assert_eq!(selected(&app).as_deref(), Some("Ben to auth-fix"));
    // A selection that reaches the changing digits ends rather than show another passage.
    let mut app = fixture("Ben to auth-fix 3s ago");
    pick(&mut app, "Ben to auth-fix 3s");
    replace_reply(&mut app, "Ben to auth-fix 4s ago".into());
    draw(&mut app, 100, TALL);
    assert!(app.conversation.borrow().selection.is_none());
}

/// The pane-output read for the open conversation, as the worker would deliver it.
fn deliver_output(app: &mut App, text: &str) {
    use crate::board::recent_responses::{Item, Observation};
    let key = app
        .pane_output_read()
        .expect("the open conversation reads pane output");
    app.apply_pane_output(
        key,
        Observation::Observed(vec![Item {
            provider: "codex".into(),
            text: text.into(),
            ended_ms: 1_000,
            truncated: false,
        }]),
    );
}

/// A busy agent's output lands between two pointer events, before the next frame has
/// rebuilt the transcript (#4344): the drag goes on and the release still copies.
#[test]
fn a_drag_and_its_release_survive_output_landing_before_the_next_frame() {
    let mut app = fixture(&long(80));
    let (x, y) = press(&mut app, "passage 070");
    // One observation lands per frame (`pane_output_fresh`); here it lands mid-gesture.
    deliver_output(&mut app, "the agent is writing");
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        x + 11,
        y + 2,
    );
    let copied = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        x + 11,
        y + 2,
    );
    assert!(
        matches!(&copied, Effect::Act(Request::Copy { text, .. })
            if text.starts_with("passage 070") && text.contains("passage 072")),
        "{copied:?}"
    );
}

/// A press that lands in the same gap still starts a selection.
#[test]
fn a_press_before_the_frame_that_follows_new_output_still_selects() {
    let mut app = fixture(&long(80));
    let (x, y) = locate(&draw(&mut app, 100, TALL), "passage 070").unwrap();
    deliver_output(&mut app, "the agent is writing");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 11, y);
    let copied = mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 11, y);
    assert!(
        matches!(&copied, Effect::Act(Request::Copy { text, .. }) if text == "passage 070"),
        "{copied:?}"
    );
}

mod click_focus {
    use super::*;
    use crate::board::shell::RightTab;
    use crate::config::Pane;

    const WIDE: u16 = 100;
    const HIGH: u16 = 36;

    fn click(app: &mut App, column: u16, row: u16) -> Effect {
        mouse(app, MouseEventKind::Down(MouseButton::Left), column, row);
        mouse(app, MouseEventKind::Up(MouseButton::Left), column, row)
    }
    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    /// The painted footer: the key hints that follow the focused pane.
    fn footer(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        (buffer.area.height - 3..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }
    fn long_history() -> String {
        (0..120).map(|n| format!("line {n:03}\n\n")).collect()
    }
    fn offset(app: &App) -> usize {
        app.scrolls.offset(Pane::Replies)
    }

    #[test]
    fn a_click_on_the_history_gives_the_reader_focus_as_tab_does() {
        let mut tabbed = fixture(&long_history());
        draw(&mut tabbed, WIDE, HIGH);
        press(&mut tabbed, KeyCode::Tab);
        let mut clicked = fixture(&long_history());
        let buffer = draw(&mut clicked, WIDE, HIGH);
        assert_eq!(clicked.focused(), Pane::Rows);
        let (x, y) = locate(&buffer, "line 11").expect("history text is painted");
        let selected = clicked.selected;
        assert_eq!(click(&mut clicked, x, y), Effect::None);
        assert_eq!(clicked.focused(), Pane::Replies);
        assert!(clicked.input.is_none(), "reading mode, not the input");
        assert_eq!(clicked.selected, selected, "the row selection is unchanged");
        // The same footer hints, and the same keys scroll it.
        assert_eq!(
            footer(&draw(&mut clicked, WIDE, HIGH)),
            footer(&draw(&mut tabbed, WIDE, HIGH))
        );
        let start = offset(&clicked);
        for code in [KeyCode::PageUp, KeyCode::Up] {
            press(&mut clicked, code);
            press(&mut tabbed, code);
            draw(&mut clicked, WIDE, HIGH);
            draw(&mut tabbed, WIDE, HIGH);
        }
        assert!(offset(&clicked) < start, "the arrows and PgUp scroll it");
        assert_eq!(offset(&clicked), offset(&tabbed));
    }

    #[test]
    fn a_click_on_blank_history_or_the_pane_title_focuses_the_conversation() {
        for (what, x, y) in [("blank history", 98, 27), ("pane title", 98, 15)] {
            let mut app = fixture("first passage");
            draw(&mut app, WIDE, HIGH);
            assert_eq!(app.focused(), Pane::Rows);
            assert_eq!(click(&mut app, x, y), Effect::None, "{what}");
            assert_eq!(app.focused(), Pane::Replies, "{what}");
            assert!(app.input.is_none(), "{what}");
        }
        // A Talk or Measure chip is part of the pane's title rule: it switches the view and the
        // pane keeps the focus.
        let mut app = fixture("first passage");
        draw(&mut app, WIDE, HIGH);
        let chip = app
            .workspace_strip_hits
            .borrow()
            .iter()
            .find_map(|(area, choice)| {
                (*choice == crate::board::app::Choice::RightTab(RightTab::Measure)).then_some(*area)
            })
            .expect("painted Measure chip");
        click(&mut app, chip.x, chip.y);
        assert_eq!(app.right_tab, RightTab::Measure);
        assert_eq!(app.focused(), Pane::Replies);
    }

    #[test]
    fn a_click_in_the_input_focuses_it_and_places_the_caret_keeping_the_draft() {
        let mut app = fixture("first passage");
        draw(&mut app, WIDE, HIGH);
        app.focus_composer();
        for letter in "hello world".chars() {
            press(&mut app, KeyCode::Char(letter));
        }
        draw(&mut app, WIDE, HIGH);
        let (area, _) = app.input_viewport.get().expect("painted input viewport");
        // The caret goes to the clicked cell.
        assert_eq!(click(&mut app, area.x + 5, area.y), Effect::None);
        assert_eq!(app.focused(), Pane::Replies);
        let input = app.input.as_ref().expect("the input keeps the focus");
        assert_eq!(input.text.as_str(), "hello world");
        assert_eq!(input.text.cursor(), 5);
        // A click past the end of the text puts it at the end.
        draw(&mut app, WIDE, HIGH);
        click(&mut app, area.x + area.width - 1, area.y);
        assert_eq!(
            app.input.as_ref().unwrap().text.cursor(),
            "hello world".len()
        );
        // Leaving for the list and clicking the input again keeps the draft.
        let (list_x, list_y) = locate(&draw(&mut app, WIDE, HIGH), "worker").unwrap();
        click(&mut app, list_x, list_y);
        assert_eq!(app.focused(), Pane::Rows);
        assert!(app.input.is_none());
        draw(&mut app, WIDE, HIGH);
        let band = app.input_band.get().expect("painted input band");
        click(&mut app, band.x + 2, band.y);
        assert_eq!(app.focused(), Pane::Replies);
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "hello world");
    }

    #[test]
    fn a_drag_still_selects_and_copies_and_leaves_the_reader_focused() {
        let mut app = fixture("first styled passage");
        let buffer = draw(&mut app, WIDE, HIGH);
        let (x, y) = locate(&buffer, "first styled").expect("history text is painted");
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 12, y);
        let copied = mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 12, y);
        assert!(
            matches!(copied, Effect::Act(crate::board::app::Request::Copy { text, .. }) if text == "first styled")
        );
        assert_eq!(app.focused(), Pane::Replies);
    }

    #[test]
    fn a_click_on_the_list_focuses_the_list_and_esc_steps_back() {
        let mut app = fixture("first passage");
        let buffer = draw(&mut app, WIDE, HIGH);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focused(), Pane::Replies);
        draw(&mut app, WIDE, HIGH);
        let (x, y) = locate(&buffer, "worker").expect("the member row is painted");
        let target = app.conversation_target();
        assert_eq!(click(&mut app, x, y), Effect::None);
        assert_eq!(app.focused(), Pane::Rows);
        assert_eq!(
            app.conversation_target(),
            target,
            "the clicked row is the member"
        );
        // Esc still steps back one level: from the reader to the list.
        draw(&mut app, WIDE, HIGH);
        click(&mut app, 98, 27);
        assert_eq!(app.focused(), Pane::Replies);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focused(), Pane::Rows);
    }

    #[test]
    fn a_click_never_sends_or_changes_the_recipient() {
        let mut app = fixture("first passage");
        let buffer = draw(&mut app, WIDE, HIGH);
        let target = app.conversation_target();
        app.focus_composer();
        for letter in "keep me".chars() {
            press(&mut app, KeyCode::Char(letter));
        }
        draw(&mut app, WIDE, HIGH);
        let (x, y) = locate(&buffer, "first passage").expect("history text is painted");
        for (column, row) in [(x, y), (98, 27), (98, 15), (2, 12)] {
            let effect = click(&mut app, column, row);
            assert_eq!(effect, Effect::None, "click at {column},{row}");
            assert_eq!(app.conversation_target(), target, "click at {column},{row}");
            draw(&mut app, WIDE, HIGH);
        }
    }
}

#[test]
#[ignore = "writes task-owned selected-text styles, never frozen fixtures"]
fn capture_selection_fallback_packet() {
    let output = std::env::var_os("TMT_SELECTION_FALLBACK_PACKET")
        .expect("task-owned selection packet path");
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut frames = Vec::new();
        for look in selection_looks() {
            let mut app = fixture_with_look(&long(80), look);
            pick(&mut app, "passage 070");
            for scene in [
                "selected",
                "append",
                "insert",
                "input-grown",
                "out-of-view",
                "scrolled-back",
            ] {
                match scene {
                    "append" => replace_reply(&mut app, format!("{}fresh news\n", long(80))),
                    "insert" => replace_reply(&mut app, format!("one\ntwo\nthree\n\n{}", long(80))),
                    "input-grown" => {
                        app.perform(&crate::action::Action::parse("talk").unwrap());
                        app.paste("a\nb\nc\nd\ne");
                    }
                    "out-of-view" => {
                        app.scroll_pane(Pane::Replies, Step::Top);
                    }
                    "scrolled-back" => {
                        app.scroll_pane(Pane::Replies, Step::Bottom);
                    }
                    _ => (),
                }
                frames.push(selection_frame(
                    &mut app,
                    look,
                    scene,
                    100,
                    TALL,
                    "passage 070",
                ));
            }
            let mut app = task_field_fixture(look);
            let buffer = draw(&mut app, 100, 36);
            let (x, y) = task_field_start(&app, &buffer);
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
            mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 10, y);
            assert!(matches!(
                app.conversation.borrow().selection,
                Some(Selection::Cells(_))
            ));
            assert_eq!(selected(&app).as_deref(), Some("task proof"));
            frames.push(selection_frame(
                &mut app,
                look,
                "task-field",
                100,
                36,
                "task proof",
            ));
        }
        std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    });
}

fn selection_frame(
    app: &mut App,
    look: crate::look::Look,
    scene: &str,
    width: u16,
    height: u16,
    needle: &str,
) -> serde_json::Value {
    let buffer = draw(app, width, height);
    let start = if scene == "task-field" {
        Some(task_field_start(app, &buffer))
    } else {
        locate(&buffer, needle)
    };
    let highlighted: Vec<_> = start
        .into_iter()
        .flat_map(|(x, y)| (x..x + needle.len() as u16).map(move |x| (x, y)))
        .collect();
    json!({
        "scene": scene, "base": format!("{:?}", look.theme.base), "depth": format!("{:?}", look.depth),
        "width": width, "height": height,
        "lines": buffer.content.chunks(usize::from(width)).map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
        "cells": buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(), "fg":format!("{:?}", cell.fg), "bg":format!("{:?}", cell.bg), "modifier":format!("{:?}", cell.modifier)})).collect::<Vec<_>>(),
        "highlightCells": highlighted, "selected": selected(app),
        "quoteControl": format!("{:?}", quote_button(app)), "inputBand": format!("{:?}", app.input_band.get()),
        "scroll": app.scrolls.offset(Pane::Replies),
    })
}

/// One actual backend frame in a fixture-owned terminal; no tmux command runs here.
#[test]
#[ignore = "requires an explicitly isolated private-terminal capture"]
fn selection_private_terminal_evidence() {
    use std::io::{Read, Write};
    assert!(std::env::var_os("TMUX").is_none());
    assert!(std::env::var_os("TMUX_PANE").is_none());
    let ready = std::env::var("TMT_SELECTION_TERMINAL_READY").unwrap();
    assert!(
        ready.starts_with("selection-4242-")
            && ready.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    );
    let width: u16 = std::env::var("TMT_SELECTION_TERMINAL_WIDTH")
        .unwrap()
        .parse()
        .unwrap();
    assert!([80, 100, 160].contains(&width));
    let look = match std::env::var("TMT_SELECTION_TERMINAL_LOOK")
        .unwrap()
        .as_str()
    {
        "tmt" => crate::look::Look::default(),
        "tmt-light" => crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight),
            depth: tmt_cli_style::Depth::TrueColor,
        },
        "NO_COLOR" => crate::look::Look {
            depth: tmt_cli_style::Depth::None,
            ..crate::look::Look::default()
        },
        other => panic!("unsupported fixture look: {other}"),
    };
    let scene = std::env::var("TMT_SELECTION_TERMINAL_SCENE").unwrap();
    assert!(
        [
            "selected-top",
            "selected-bottom",
            "input-grown",
            "task-field",
            "help"
        ]
        .contains(&scene.as_str())
    );
    let _no_commands = crate::runner::forbid_commands();
    crate::board::terminal::restore_before_panic_reports();
    let mut screen =
        crate::board::terminal::Guard::enter(crate::board::terminal::Crossterm::default()).unwrap();
    let mut terminal =
        Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout())).unwrap();
    assert_eq!(
        terminal.size().unwrap(),
        ratatui::layout::Size::new(width, 36)
    );
    let position = crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = if scene == "task-field" {
            task_field_fixture(look)
        } else {
            fixture_with_look(&long(80), look)
        };
        draw(&mut app, width, 36);
        let needle = match scene.as_str() {
            "task-field" => "task proof",
            "selected-top" => {
                app.scroll_pane(Pane::Replies, Step::Top);
                "passage 001"
            }
            _ => "passage 078",
        };
        if scene == "task-field" {
            let buffer = draw(&mut app, width, 36);
            let (x, y) = task_field_start(&app, &buffer);
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
            mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 10, y);
            assert!(matches!(
                app.conversation.borrow().selection,
                Some(Selection::Cells(_))
            ));
        } else {
            select_at_width(&mut app, needle, width);
        }
        assert_eq!(selected(&app).as_deref(), Some(needle));
        if scene == "input-grown" {
            app.perform(&crate::action::Action::parse("talk").unwrap());
            app.paste("a\nb\nc\nd\ne");
            app.scroll_pane(Pane::Replies, Step::Bottom);
        }
        if scene == "help" {
            app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        }
        let preview = draw(&mut app, width, 36);
        let position = if scene == "help" {
            (0, 0)
        } else if scene == "task-field" {
            task_field_start(&app, &preview)
        } else {
            locate(&preview, needle).expect("selected passage is visible")
        };
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        position
    });
    // The private-server window-renamed hook runs after all preceding frame bytes.
    write!(
        std::io::stdout(),
        "\x1bk{ready}-{}-{}\x1b\\",
        position.0,
        position.1
    )
    .unwrap();
    std::io::stdout().flush().unwrap();
    let mut quit = [0];
    std::io::stdin().read_exact(&mut quit).unwrap();
    assert_eq!(quit, [b'q']);
    screen.restore().unwrap();
}

#[test]
fn selected_text_survives_a_retained_frame_and_the_single_refresh_swap() {
    let mut app = fixture("literal passage kept through refresh");
    let (x, y) = select_words(&mut app, "literal passage");
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 15, y);
    app.selected_request = crate::board::refresh::SelectedRequest::Ready(None);
    app.refresh_sent();
    app.conversation.borrow_mut().refresh_pane_output();
    draw(&mut app, 100, 36);
    assert!(app.conversation.borrow().selection.is_some());
    let plan = app.conversation_read().expect("one selected refresh");
    app.apply_conversation(
        plan.clone(),
        plan.test_projection("not a replacement", None),
    );
    draw(&mut app, 100, 36);
    assert!(app.conversation.borrow().selection.is_some());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    let input = app
        .input
        .as_ref()
        .expect("selection still opens the same draft");
    assert_eq!(input.literal_quotes()[0].text, "literal passage");
    assert!(
        input.exact_quotes().next().is_none(),
        "cached display grants no new authority"
    );
}
