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
fn focus_and_five_lines_keep_the_info_line_and_footer_fixed_without_box_chrome() {
    let _no_io = crate::runner::forbid_commands();
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        let idle = draw(&mut app, width, 36);
        let band = app.input_band.get().expect("an admitted idle composer");
        let idle_view = app.input_viewport.get().unwrap().0;
        assert_eq!(idle_view.height, 1);
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
            "focus changes fill/box height, never its envelope"
        );
        let editor = app.input_viewport.get().unwrap().0;
        assert_eq!(editor.height, 3);
        assert_eq!(editor.bottom(), idle_view.bottom());
        assert_eq!(band_text(&focused, info), info_before);
        assert_eq!(
            app.paste("first\nsecond\nthird\nfourth\nfifth"),
            crate::board::app::Effect::None
        );
        let typed = draw(&mut app, width, 36);
        assert_eq!(app.input_band.get(), Some(band));
        let viewport = app.input_viewport.get().unwrap().0;
        assert_eq!(viewport.height, 3);
        assert!(
            viewport.right() < editor.right(),
            "overflow has its own cells, not cells stolen from typed text"
        );
        let text = band_text(&typed, band);
        assert!(
            text.contains("fifth") && text.contains("↑ 2 more"),
            "{text}"
        );
        assert!(
            !text.contains("as you") && !text.contains("Reply:") && !text.contains("Esc cancel")
        );
        assert_eq!(band_text(&typed, info), info_before);
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "first\nsecond\nthird\nfourth\nfifth"
        );
        close(&mut app);
        let held = draw(&mut app, width, 36);
        assert_eq!(
            app.input_viewport.get().unwrap().0.height,
            3,
            "an unfocused draft retains its full box"
        );
        assert_eq!(band_text(&held, info), info_before);
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
