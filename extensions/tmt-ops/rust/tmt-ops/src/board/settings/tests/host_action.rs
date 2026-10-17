use super::*;
use crate::board::settings::host_action::Selection;
use crate::config::OpsKeyScope;

fn open(f: &mut Fixture, board_pane: Option<&str>) {
    f.app.open_host_action(Config::read(f.path.clone()).unwrap(), Selection {
        row: Some(json!({"id":"11111111-1111-4111-8111-111111111111", "name":"alpha", "presence":"active", "fields":{"pr_link":"https://example.com/pull/42"}, "pane":{"id":"%3","location":{"host":"tmux"}}})),
        recipient: Some(json!({"id":"11111111-1111-4111-8111-111111111111", "name":"alpha", "presence":"active", "fields":{"pr_link":"https://example.com/pull/42"}, "pane":{"id":"%3","location":{"host":"tmux"}}})),
        default_action: Some(crate::action::Action::parse("jump").unwrap()),
        squad: Some("product".into()), scope: OpsKeyScope::Member, invoker_tmux: true,
    }, board_pane.map(str::to_owned)).unwrap();
}

#[test]
fn preset_preview_cancel_save_and_reopen_use_one_config_owner() {
    let mut f = fixture("host-preset", "[bind]\nj='jump'\no='open {pr_link}'\n");
    open(&mut f, Some("%9"));
    let overlay = f.app.settings.as_mut().unwrap();
    assert_eq!(overlay.picks[0].value, "(current)");
    overlay.surface.borrow_mut().select("pick:host-left");
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Input::Preview
    ));
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    assert!(overlay.draft.is_some());
    let preview = overlay.editing.clone();
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        Input::None
    ));
    assert_eq!(overlay.editing, preview, "preset preview is read-only");
    assert!(
        overlay
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("--into left")
    );
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Input::Preview
    ));
    assert!(!overlay.editing());
    assert!(overlay.notice.is_none());
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    overlay.surface.borrow_mut().select("pick:host-right");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        Input::Save
    ));
    assert!(overlay.save());
    assert_eq!(overlay.picks[2].value, "(current)");
    assert!(overlay.picks[0].value.is_empty());
    assert_eq!(
        Config::read(f.path.clone())
            .unwrap()
            .configured_bindings()
            .unwrap()["j"]
            .text,
        "swap right"
    );
    assert_eq!(
        Config::read(f.path.clone())
            .unwrap()
            .configured_bindings()
            .unwrap()["o"]
            .text,
        "open {pr_link}"
    );
    open(&mut f, Some("%9"));
    assert_eq!(
        f.app.settings.as_ref().unwrap().settings.entries[0].value,
        "swap right"
    );
}

#[test]
fn custom_command_keeps_s_as_text_and_enter_as_save() {
    let mut f = fixture("host-custom-letter", "[bind]\nj='run echo value'\n");
    open(&mut f, Some("%9"));
    let overlay = f.app.settings.as_mut().unwrap();
    overlay.surface.borrow_mut().select("pick:host-custom");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        Input::Preview
    ));
    assert_eq!(overlay.editing.as_ref().unwrap().1, "run echo values");
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Input::Save
    ));
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
}

#[test]
fn focus_preview_uses_the_existing_host_guard_and_names_local_conversation() {
    let mut f = fixture("host-focus-preview", "[bind]\nj='jump'\n");
    open(&mut f, Some("%9"));
    let host = f
        .app
        .settings
        .as_mut()
        .unwrap()
        .host_action
        .as_mut()
        .unwrap();
    assert_eq!(
        host.preview("jump").unwrap(),
        "tmt focus 11111111-1111-4111-8111-111111111111 --json"
    );
    host.selection.recipient.as_mut().unwrap()["presence"] = json!("offline");
    assert!(host.preview("jump").unwrap_err().contains("offline"));
    host.selection.invoker_tmux = false;
    assert_eq!(
        host.preview("jump").unwrap(),
        "UI: open conversation (no shell command)"
    );
}

#[test]
fn custom_preview_keeps_unknown_context_unknown_and_conflict_keeps_the_draft() {
    let mut f = fixture("host-custom", "[bind]\nj='run echo {board_pane} {name}'\n");
    open(&mut f, None);
    let overlay = f.app.settings.as_mut().unwrap();
    overlay.surface.borrow_mut().select("pick:host-custom");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        overlay
            .notice
            .as_ref()
            .unwrap()
            .message
            .contains("board_pane is empty")
    );
    assert!(overlay.draft.is_some());
    std::fs::write(&f.path, format!("{}# another editor\n", f.original)).unwrap();
    assert!(!overlay.save());
    assert!(overlay.editing());
    assert!(overlay.draft.is_some());
    assert!(overlay.notice.as_ref().unwrap().message.contains("changed"));
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .ends_with("# another editor\n")
    );
}

#[test]
fn host_action_presets_and_preview_render_in_terminal_rows_at_each_width() {
    let mut f = fixture("host-render", "[bind]\nj='run echo {board_pane} {name}'\n");
    for (width, height) in [(103, 51), (148, 51), (160, 36), (100, 24), (80, 24)] {
        for look in picker_surface::evidence::looks() {
            open(&mut f, Some("%9"));
            let overlay = f.app.settings.as_mut().unwrap();
            let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
            screen
                .draw(|frame| render(frame, overlay, look, frame.area()))
                .unwrap();
            let text = screen
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("Host action (j)"), "{text}");
            assert!(text.contains("Focus agent"), "{text}");
            assert!(text.contains("Custom command"), "{text}");
            overlay.surface.borrow_mut().select("pick:host-custom");
            overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            screen
                .draw(|frame| render(frame, overlay, look, frame.area()))
                .unwrap();
            let text = screen
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("Custom command · alpha"), "{text}");
            assert!(text.contains("argv"), "{text}");
            assert!(text.contains("%9"), "{text}");
            assert!(!f.root.join("unexpected").exists());
        }
    }
}

#[test]
fn custom_command_is_not_started_by_preview_save_or_cancel() {
    let mut f = fixture("host-no-program", "[bind]\nj='run touch {marker}'\n");
    open(&mut f, Some("%9"));
    let overlay = f.app.settings.as_mut().unwrap();
    overlay.surface.borrow_mut().select("pick:host-custom");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(overlay.notice.as_ref().unwrap().message.contains("argv"));
    assert!(!f.root.join("unexpected").exists());
    assert!(overlay.save());
    assert!(!f.root.join("unexpected").exists());
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Input::Close
    ));
    assert!(!f.root.join("unexpected").exists());
}

#[test]
fn host_action_survives_same_tab_acquisition_before_and_after_save() {
    let mut f = fixture("host-refresh", "[bind]\nj='jump'\n");
    open(&mut f, Some("%9"));
    let refresh = |f: &mut Fixture| {
        f.app.apply(crate::board::app::tests::snapshot(
            "product",
            json!([{"rows":[{"name":"newer", "fields":{}}]}]),
        ));
        let overlay = f.app.settings.as_ref().expect("same tab keeps editor open");
        assert_eq!(overlay.settings.context.as_deref(), Some("product"));
        assert_eq!(
            overlay
                .host_action
                .as_ref()
                .unwrap()
                .selection
                .row
                .as_ref()
                .unwrap()["name"],
            "alpha",
            "refresh keeps the opening binding context"
        );
        assert_eq!(
            overlay
                .host_action
                .as_ref()
                .unwrap()
                .selection
                .recipient
                .as_ref()
                .unwrap()["name"],
            "alpha",
            "refresh keeps the opening recipient preview sample"
        );
    };
    refresh(&mut f);
    let overlay = f.app.settings.as_mut().unwrap();
    overlay.surface.borrow_mut().select("pick:host-left");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(overlay.save());
    refresh(&mut f);
    assert_eq!(
        f.app.settings.as_ref().unwrap().settings.entries[0].value,
        "swap left"
    );
}

#[test]
fn squad_editor_previews_the_lead_and_saves_its_host_binding_without_a_member_section() {
    let text = "[bind]\nj='jump'\n[bind.tmux]\nj='swap left'\n[[squad.product.section]]\ntitle='All'\n[squad.product.section.bind]\nj='swap down'\n";
    let mut f = fixture("squad-host-owner", text);
    let context = json!({"kind":"squad","name":"product","roomId":"room"});
    let recipient = json!({"id":"11111111-1111-4111-8111-111111111111","name":"alpha","presence":"active","pane":{"id":"%3","location":{"host":"tmux"}}});
    f.app
        .open_host_action(
            Config::read(f.path.clone()).unwrap(),
            Selection {
                row: Some(context),
                recipient: Some(recipient),
                default_action: Some(crate::action::Action::parse("jump").unwrap()),
                squad: None,
                scope: OpsKeyScope::Squad,
                invoker_tmux: true,
            },
            Some("%9".into()),
        )
        .unwrap();
    let overlay = f.app.settings.as_mut().unwrap();
    assert_eq!(overlay.settings.entries[0].source, "bind.tmux.j");
    assert_eq!(overlay.settings.entries[0].value, "swap left");
    assert_eq!(
        overlay
            .host_action
            .as_ref()
            .unwrap()
            .preview("swap left")
            .unwrap(),
        "tmt focus 11111111-1111-4111-8111-111111111111 --into left --json"
    );
    overlay.surface.borrow_mut().select("pick:host-right");
    overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(overlay.save());
    let saved = std::fs::read_to_string(&f.path)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    assert_eq!(saved["bind"]["tmux"]["j"].as_str(), Some("swap right"));
    assert_eq!(saved["bind"]["j"].as_str(), Some("jump"));
    assert_eq!(
        saved["squad"]["product"]["section"][0]["bind"]["j"].as_str(),
        Some("swap down")
    );
}

#[test]
fn a_retired_lead_binding_uses_the_effective_default_without_rewriting_the_file() {
    let mut f = fixture("host-lead-binding", "[bind]\nj='jump lead'\n");
    open(&mut f, Some("%9"));
    let overlay = f.app.settings.as_mut().unwrap();
    assert_eq!(overlay.settings.entries[0].value, "jump");
    assert_eq!(overlay.picks[0].value, "(current)");
    let host = overlay.host_action.as_ref().unwrap();
    assert_eq!(host.meaning("jump lead"), "go to the squad lead's pane");
    assert!(
        host.preview("jump lead")
            .unwrap_err()
            .contains("targets the squad lead")
    );
    assert_eq!(
        crate::action::Action::parse("jump lead")
            .unwrap()
            .workspace_description(),
        "go to the squad lead's pane"
    );
    assert!(
        Config::read(f.path.clone())
            .unwrap()
            .preview_host_action("bind.j", "jump lead")
            .is_err(),
        "the editor must preserve the existing retired-action admission guard"
    );
    assert!(matches!(
        overlay.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Input::Close
    ));
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
}

#[test]
fn a_retired_host_binding_keeps_its_effective_value_and_repairs_that_exact_owner() {
    for (index, retired, effective) in [
        (0, "agents-filter needs-you", None),
        (1, "jump lead", Some("jump")),
    ] {
        let mut f = fixture(
            &format!("host-retired-{index}"),
            &format!("[bind]\no='open {{pr_link}}'\n[bind.tmux]\nj='{retired}'\n"),
        );
        open(&mut f, Some("%9"));
        let overlay = f.app.settings.as_mut().unwrap();
        assert_eq!(overlay.settings.entries[0].value, json!(effective));
        assert_eq!(overlay.settings.entries[0].source, "bind.tmux.j");
        assert_eq!(
            overlay.picks[0].value,
            if effective.is_some() { "(current)" } else { "" }
        );
        if effective.is_none() {
            assert!(overlay.picks.iter().all(|pick| pick.value.is_empty()));
        }
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        overlay.surface.borrow_mut().select("pick:host-left");
        overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(overlay.save());
        let saved = std::fs::read_to_string(&f.path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(saved["bind"]["tmux"]["j"].as_str(), Some("swap left"));
        assert!(saved["bind"].get("j").is_none());
        assert_eq!(saved["bind"]["o"].as_str(), Some("open {pr_link}"));
        assert_eq!(overlay.settings.entries[0].value, "swap left");
        assert_eq!(overlay.picks[1].value, "(current)");
        assert_eq!(
            Config::read(f.path.clone())
                .unwrap()
                .workspace_keys(OpsKeyScope::Reader, Some("tmux"), None)
                .unwrap()
                .bindings["j"]
                .text,
            "swap left"
        );
    }
}
