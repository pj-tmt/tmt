use super::*;

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn draw(app: &mut App, width: u16, height: u16) -> bool {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().cursor_visible()
}

fn click(area: Rect) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn preview_is_read_only_scrollable_and_outside_close_can_remove_the_chip() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    let raw = (0..100)
        .map(|line| format!("line {line} 漢字\r\n"))
        .collect::<String>();
    app.paste(&raw);
    draw(&mut app, 100, 36);
    let hit = app.composer.paste_hits.borrow()[0].clone();
    app.mouse(click(hit.area), std::time::Instant::now());
    assert!(app.paste_preview_shown());
    assert!(
        !draw(&mut app, 100, 36),
        "a read-only preview hides the underlying editor caret"
    );
    let before = app.input.as_ref().unwrap().text.clone();
    key(&mut app, KeyCode::Char('z'));
    key(&mut app, KeyCode::Backspace);
    app.paste("must not enter the draft");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().text, before);
    key(&mut app, KeyCode::PageDown);
    assert!(
        app.current_paste_preview()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    key(&mut app, KeyCode::Esc);
    assert!(!app.paste_preview_shown());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), raw);
    assert!(
        draw(&mut app, 100, 36),
        "closing the preview restores the editor caret"
    );
    app.mouse(click(hit.area), std::time::Instant::now());
    draw(&mut app, 100, 36);
    app.mouse(click(hit.remove), std::time::Instant::now());
    assert!(!app.paste_preview_shown());
    assert!(
        app.input.as_ref().unwrap().text.is_empty(),
        "outside press also reaches the chip remove control"
    );
}

#[test]
fn chips_between_typed_text_are_atomic_and_old_geometry_cannot_edit_a_new_draft() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let first = "a".repeat(501);
    let second = "b".repeat(501);
    app.paste(&first);
    app.paste(" between ");
    app.paste(&second);
    app.paste(" after");
    draw(&mut app, 100, 36);
    let stale = app.composer.paste_hits.borrow()[0].clone();
    for _ in 0..6 {
        key(&mut app, KeyCode::Left);
    }
    key(&mut app, KeyCode::Left);
    assert_eq!(
        app.input.as_ref().unwrap().text.cursor(),
        "before ".len() + first.len() + " between ".len()
    );
    key(&mut app, KeyCode::Delete);
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("before {first} between  after")
    );
    app.input.as_mut().unwrap().draft_key.as_mut().unwrap().1 = "replacement".into();
    assert!(!app.paste_hit_is_current(&stale));
    app.open_paste_preview(stale);
    assert!(!app.paste_preview_shown());
}

#[test]
fn selected_chip_replacement_and_failed_admission_preserve_surrounding_bytes() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("left ");
    let raw = "漢字\0\r\n".repeat(100);
    app.paste(&raw);
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(
        app.input.as_ref().unwrap().text.selection(),
        5..5 + raw.len()
    );
    app.paste("replacement");
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "left replacement"
    );
    assert!(app.input.as_ref().unwrap().text.folds().is_empty());
    let before = app.input.as_ref().unwrap().text.clone();
    app.paste(&"x".repeat(crate::board::app::MESSAGE_LIMIT));
    assert_eq!(
        app.input.as_ref().unwrap().text,
        before,
        "oversized paste is atomic"
    );
    app.paste(&raw);
    draw(&mut app, 100, 36);
    app.preview_paste_at_cursor();
    assert!(app.paste_preview_shown());
    app.remember_draft();
    app.input = None;
    assert!(!app.paste_preview_shown());
    app.focus_composer();
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("left replacement{raw}")
    );
    assert!(
        !app.paste_preview_shown(),
        "reopening the draft never resurrects a preview"
    );
}

#[test]
fn exclusive_thresholds_count_unicode_and_logical_breaks_without_normalizing_source() {
    for chars in [499, 500, 501] {
        let source = "漢".repeat(chars);
        assert_eq!(label(&source).is_some(), chars > 500);
        assert_eq!(counts(&source), (chars, 1));
    }
    for separator in ["\n", "\r", "\r\n"] {
        for lines in [9, 10, 11] {
            let source = vec!["x"; lines].join(separator);
            assert_eq!(counts(&source).1, lines);
            assert_eq!(label(&source).is_some(), lines > 10);
        }
    }
    assert_eq!(counts("x\r\n"), (3, 2));
    assert!(label(&"x".repeat(1123)).unwrap().contains("1,123 chars"));
    assert!(label(&"x\n".repeat(10)).unwrap().contains("11 lines"));
}

#[test]
fn ascii_and_clipped_chips_keep_the_admitted_editor_width_and_remove_target() {
    let label = label(&"x".repeat(501)).unwrap();
    assert_eq!(crate::board::glyph_guard::glyph_error(&label), None);
    for ascii in [false, true] {
        for width in 1..=label.width() {
            let (painted, remove) = display(&label, width, ascii);
            assert_eq!(painted.width(), width);
            assert_eq!(
                painted.chars().nth(remove),
                Some(if ascii { 'x' } else { '×' })
            );
            if ascii {
                assert!(painted.is_ascii());
            }
        }
    }
    let (painted, _) = display(&label, label.width(), true);
    assert!(painted.starts_with("[Pasted ") && painted.trim_end().ends_with(" x]"));
}

#[test]
fn preview_anchors_above_then_below_and_stays_inside_the_conversation() {
    let bounds = Rect::new(40, 5, 80, 30);
    let above = preview_area(bounds, Rect::new(50, 30, 25, 1), 100).unwrap();
    assert_eq!(above.x, 50);
    assert_eq!(above.bottom(), 30);
    assert_eq!(above.height, bounds.height / 2);
    let below = preview_area(bounds, Rect::new(50, 6, 25, 1), 100).unwrap();
    assert_eq!(below.y, 7);
    assert_eq!(below.height, bounds.height / 2);
    assert_eq!(below.intersection(bounds), below);
    assert!(preview_area(bounds, Rect::new(119, 20, 1, 1), 100).is_none());
    assert!(preview_area(Rect::new(0, 0, 80, 3), Rect::new(0, 2, 20, 1), 10).is_none());
}

#[test]
fn paste_admission_uses_event_size_and_strict_line_threshold_in_the_actual_composer() {
    for separator in ["\n", "\r", "\r\n"] {
        for lines in [10, 11] {
            let mut app = crate::board::app::workspace::tests::board();
            key(&mut app, KeyCode::Enter);
            let raw = vec!["漢字"; lines].join(separator);
            app.paste(&raw);
            let editor = &app.input.as_ref().unwrap().text;
            assert_eq!(editor.as_str(), raw);
            assert_eq!(editor.folds().len(), usize::from(lines > 10));
        }
    }
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste(&"漢".repeat(500));
    app.paste(&"字".repeat(500));
    assert!(
        app.input.as_ref().unwrap().text.folds().is_empty(),
        "each ordinary paste stays text even when the complete draft exceeds the threshold"
    );
    app.paste(&"a".repeat(501));
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
}

pub(in crate::board) const SCENES: &[&str] = &[
    "paste-single",
    "paste-mixed",
    "paste-selected",
    "paste-preview",
    "paste-scrolled",
    "paste-removed",
    "paste-retained",
    "paste-ascii",
    "paste-tiny",
];

/// One scene owner for decoded review and the eventual post-#4100 oracle.
pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
    width: u16,
    height: u16,
) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().look = look;
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for squad in &workspace.roster.squads {
        workspace.squad_looks.insert(squad.room_id.clone(), look);
    }
    if scene == "paste-ascii" {
        app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::Ascii;
    }
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let source = if matches!(scene, "paste-preview" | "paste-scrolled") {
        (0..100)
            .map(|index| format!("source line {index:03}"))
            .collect::<Vec<_>>()
            .join("\r\n")
    } else {
        "x".repeat(1123)
    };
    app.paste(&source);
    if matches!(
        scene,
        "paste-mixed" | "paste-removed" | "paste-retained" | "paste-ascii"
    ) {
        app.paste(" between ");
        app.paste(&["second source"; 11].join("\n"));
        app.paste(" after");
    }
    if scene == "paste-selected" {
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    }
    draw(&mut app, width, height);
    if matches!(scene, "paste-preview" | "paste-scrolled") {
        app.preview_paste_at_cursor();
        draw(&mut app, width, height);
        if scene == "paste-scrolled" {
            key(&mut app, KeyCode::PageDown);
        }
    } else if scene == "paste-removed" {
        let hit = app.composer.paste_hits.borrow().last().cloned().unwrap();
        app.mouse(click(hit.remove), std::time::Instant::now());
    } else if scene == "paste-retained" {
        key(&mut app, KeyCode::Esc);
    }
    app
}

#[test]
#[ignore = "exports task-owned paste review evidence; never records a frozen fixture"]
fn capture_paste_packet() {
    let output = std::env::var("TMT_PASTE_CAPTURE").expect("task-owned evidence path");
    let mut captures = Vec::new();
    for (name, look) in crate::board::home::tests::oracle::looks() {
        for scene in SCENES {
            let height = if *scene == "paste-tiny" { 8 } else { 36 };
            for width in [160, 100, 80] {
                let app = capture_case(scene, look, width, height);
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .unwrap();
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let cells = buffer.content.iter().map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                let lines = buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>();
                captures.push(serde_json::json!({"scene":scene,"look":name,"width":width,"height":height,"cells":cells,"lines":lines,"inputBand":format!("{:?}",app.input_band.get()),"editor":format!("{:?}",app.input_viewport.get()),"paste":capture_metadata(&app)}));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn grapheme_join_fallback_keeps_every_byte_when_pasting_or_typing_a_combining_mark() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("e");
    let raw = format!("\u{301}{}", "x".repeat(501));
    app.paste(&raw);
    let text = &app.input.as_ref().unwrap().text;
    assert_eq!(text.as_str(), format!("e{raw}"));
    assert!(
        text.folds().is_empty(),
        "joining the preceding grapheme retains ordinary source text"
    );

    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    let raw = "x".repeat(501);
    app.paste(&raw);
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    key(&mut app, KeyCode::Char('\u{301}'));
    let text = &app.input.as_ref().unwrap().text;
    assert_eq!(text.as_str(), format!("{raw}\u{301}"));
    assert!(
        text.folds().is_empty(),
        "a joining edit removes only the chip projection, never source bytes"
    );
}

/// Draft/fold provenance and clipped geometry accompany every new scene.
pub(in crate::board) fn capture_metadata(app: &App) -> serde_json::Value {
    let rect = |area: Rect| [area.x, area.y, area.width, area.height];
    let hits = app.composer.paste_hits.borrow().iter().map(|hit| serde_json::json!({
        "area":rect(hit.area),"remove":rect(hit.remove),"draft":[hit.key.0,hit.key.1],"fold":hit.id,"source":[hit.source.start,hit.source.end],"label":hit.label
    })).collect::<Vec<_>>();
    let preview = app.current_paste_preview().map(|preview| {
        let scroll = preview.scroll.borrow();
        serde_json::json!({"fold":preview.id,"viewport":rect(scroll.viewport()),"offset":scroll.offset(),"content":scroll.content()})
    });
    serde_json::json!({"chips":hits,"preview":preview})
}
