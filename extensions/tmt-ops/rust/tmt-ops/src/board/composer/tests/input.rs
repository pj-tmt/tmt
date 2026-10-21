use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn band_text(buffer: &ratatui::buffer::Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_box_grows_upward_with_its_content_and_keeps_the_info_line_and_footer_fixed() {
    let _no_io = crate::runner::forbid_commands();
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        let idle = draw(&mut app, width, 36);
        let band = app.input_band.get().expect("an admitted idle composer");
        let idle_view = app.input_viewport.get().unwrap().0;
        assert_eq!(idle_view.height, 1);
        assert_eq!(
            band.height, 4,
            "padding, one text row, padding and the info line"
        );
        let info = Rect::new(band.x, band.bottom() - 1, band.width, 1);
        let info_before = band_text(&idle, info);
        assert!(info_before.contains("worker"));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            crate::board::app::Effect::None
        );
        let focused = draw(&mut app, width, 36);
        assert_eq!(
            app.input_band.get(),
            Some(band),
            "focusing an empty box only changes its fill"
        );
        let editor = app.input_viewport.get().unwrap().0;
        assert_eq!(editor.height, 1);
        assert_eq!(editor.bottom(), idle_view.bottom());
        assert_eq!(band_text(&focused, info), info_before);
        for (text, rows) in [
            ("first\nsecond\nthird\nfourth\nfifth", 5),
            ("\nsixth\nseventh\neighth", 6),
        ] {
            assert_eq!(app.paste(text), crate::board::app::Effect::None);
            let typed = draw(&mut app, width, 36);
            let grown = app.input_band.get().unwrap();
            assert_eq!(grown.bottom(), band.bottom(), "the box grows upward");
            assert_eq!(
                grown.height,
                rows + 3,
                "{rows} rows, their padding and the info line"
            );
            let viewport = app.input_viewport.get().unwrap().0;
            assert_eq!(viewport.height, rows);
            assert_eq!(viewport.bottom(), idle_view.bottom());
            let info_now = Rect::new(grown.x, grown.bottom() - 1, grown.width, 1);
            assert_eq!(band_text(&typed, info_now), info_before);
            let shown = band_text(&typed, grown);
            assert!(
                !shown.contains("as you")
                    && !shown.contains("Reply:")
                    && !shown.contains("Esc cancel")
            );
        }
        // Eight lines exceed the six-row cap: the rest scrolls inside the box,
        // keeping the caret visible and naming what is above.
        let typed = draw(&mut app, width, 36);
        let text = band_text(&typed, app.input_band.get().unwrap());
        assert!(
            text.contains("eighth") && text.contains("↑ 2 more"),
            "{text}"
        );
        assert!(
            app.input_viewport.get().unwrap().0.right() < editor.right(),
            "overflow has its own cells, not cells stolen from typed text"
        );
        // At the cap the box stops growing and keeps its padding rows above and below the text.
        let capped = app.input_band.get().unwrap();
        assert_eq!(capped.height, 6 + 3);
        let view = app.input_viewport.get().unwrap().0;
        let row = |y: u16| band_text(&typed, Rect::new(capped.x, y, capped.width, 1));
        assert!(row(view.y - 1).trim().is_empty() && row(view.bottom()).trim().is_empty());
        assert_eq!(
            (view.y - capped.y, capped.bottom() - 1 - view.bottom()),
            (1, 1)
        );
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "first\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\neighth"
        );
        close(&mut app);
        let held = draw(&mut app, width, 36);
        assert_eq!(
            app.input_viewport.get().unwrap().0.height,
            6,
            "an unfocused draft keeps its content-sized box"
        );
        let held_band = app.input_band.get().unwrap();
        let held_info = Rect::new(held_band.x, held_band.bottom() - 1, held_band.width, 1);
        assert_eq!(band_text(&held, held_info), info_before);
    }
}

#[test]
fn context_comes_from_the_acquired_running_recipient_and_preserves_unknown_window() {
    let _no_io = crate::runner::forbid_commands();
    let mut app = crate::board::app::workspace::tests::board();
    draw(&mut app, 160, 36);
    let id = app.composer_projection().unwrap().key.unwrap().1;
    let listed = |driver: serde_json::Value, window: serde_json::Value| {
        json!({"identities":[{
            "id":id,"name":"worker","runningDriver":driver,
            "resume":{"usage":{"tokens":142000,"windowTokens":window,"observedAtMs":123}}
        }]})
    };
    for (driver, window, wanted, absent) in [
        (json!("codex"), json!(258000), "ctx 142k / 258k", None),
        (
            json!("claude"),
            serde_json::Value::Null,
            "ctx 142k",
            Some("/ 258k"),
        ),
        (
            serde_json::Value::Null,
            json!(258000),
            "worker",
            Some("ctx"),
        ),
    ] {
        app.references =
            crate::board::inventory::Inventory::from_listed(&[], &Ok(listed(driver, window)));
        let buffer = draw(&mut app, 160, 36);
        let area = app.input_band.get().unwrap();
        let info = band_text(&buffer, Rect::new(area.x, area.bottom() - 1, area.width, 1));
        assert!(info.contains(wanted), "{info}");
        if let Some(absent) = absent {
            assert!(!info.contains(absent), "{info}");
        }
    }
}

#[test]
fn info_line_uses_the_acquired_open_request_count_without_showing_a_target_picker() {
    let _no_io = crate::runner::forbid_commands();
    let mut app = crate::board::app::workspace::tests::board();
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
        .unwrap()["waitingOnYou"] = json!([
        {"requestId":"older", "preparedAtMs":1},
        {"requestId":"newer", "preparedAtMs":2}
    ]);
    let buffer = draw(&mut app, 160, 36);
    let band = app.input_band.get().unwrap();
    let info = band_text(&buffer, Rect::new(band.x, band.bottom() - 1, band.width, 1));
    assert!(info.contains("◆ 2 open request"), "{info}");
    assert!(!info.contains("older") && !info.contains("newer"));
    assert!(
        app.input.is_none(),
        "painting counts does not start or send a message"
    );
}

#[test]
fn the_info_line_and_footer_name_the_same_new_line_key() {
    let _no_io = crate::runner::forbid_commands();
    for (enhanced, named) in [(false, "Alt+⏎"), (true, "Shift+⏎")] {
        for width in [160, 100] {
            let mut app = crate::board::app::workspace::tests::board();
            app.keyboard_enhanced = enhanced;
            let idle = draw(&mut app, width, 36);
            let band = app.input_band.get().expect("an admitted idle composer");
            let info = Rect::new(band.x, band.bottom() - 1, band.width, 1);
            let line = band_text(&idle, info);
            assert!(
                line.contains(&format!("{named} new line")),
                "{enhanced} {width}: {line}"
            );
            assert_eq!(
                app.newline_hint().replace("Enter", "⏎"),
                format!("{named} new line"),
                "the footer caption is the same key"
            );
        }
    }
}

#[test]
fn the_padding_follows_the_height_of_the_frame_being_drawn_with_no_stale_frame() {
    let _no_io = crate::runner::forbid_commands();
    let mut app = crate::board::app::workspace::tests::board();
    // Shrinking in place drops the padding in the very next frame, growing restores it.
    for (height, rows) in [(36, 4), (24, 2), (36, 4), (27, 2), (28, 4)] {
        draw(&mut app, 160, height);
        assert_eq!(app.input_band.get().unwrap().height, rows, "{height}");
    }
}
