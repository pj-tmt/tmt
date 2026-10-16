use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

fn moved(app: &mut App, x: u16, y: u16) -> bool {
    app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn shared_groups_use_the_same_padding_style_hover_and_disabled_hit_contract() {
    use crate::board::chip_group::{self, ChipGroup, Choice};
    use tmt_cli_style::{Base, Depth, Theme};
    for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
        let mut app = App::default();
        app.initial_look = Some(crate::look::Look {
            theme: Theme::new(Base::Tmt),
            depth,
        });
        let groups = [ChipGroup {
            label: "Each ".into(),
            choices: [
                ("5m", true, false),
                ("15m", false, false),
                ("30m", false, true),
            ]
            .into_iter()
            .map(|(label, selected, disabled)| Choice {
                id: label.into(),
                actionable: true,
                text: format!(" {label:>3} "),
                gap: " ".into(),
                selected,
                disabled,
                cursor: false,
            })
            .collect(),
        }];
        let area = Rect::new(3, 2, 24, 1);
        let mut buffer = Buffer::empty(area);
        let hits = chip_group::paint(&mut buffer, area, &groups, &app, None);
        assert_eq!(
            hits.iter().map(|hit| hit.id.as_str()).collect::<Vec<_>>(),
            ["5m", "15m"]
        );
        let selected = hits[0].area;
        let look = app.look();
        let hot = screen::chip(&look.theme, ChipKind::Secondary, ChipState::Hot, depth);
        for x in selected.x..selected.right() {
            assert_eq!(buffer[(x, selected.y)].modifier, hot.add_modifier);
            assert_eq!(buffer[(x, selected.y)].bg, hot.bg.unwrap_or_default());
        }
        let idle = hits[1].area;
        assert!(moved(&mut app, idle.x, idle.y));
        app.buttons.begin_frame();
        let hovered = chip_group::paint(&mut buffer, area, &groups, &app, None);
        assert_eq!(hovered[1].area, idle, "hover must not move the group");
        for x in idle.x..idle.right() {
            assert_eq!(buffer[(x, idle.y)].modifier, hot.add_modifier);
            assert_eq!(buffer[(x, idle.y)].bg, hot.bg.unwrap_or_default());
        }
        app.buttons.invalidate();
        let clipped = Rect { width: 13, ..area };
        let hits = chip_group::paint(&mut buffer, clipped, &groups, &app, None);
        assert_eq!(hits.len(), 1, "a partially clipped chip is not actionable");
    }
}

#[test]
fn padding_hover_repaints_only_the_whole_chip_and_leaving_restores_idle() {
    let mut app = App::default();
    app.initial_look = Some(crate::look::Look {
        theme: tmt_cli_style::Theme::default(),
        depth: tmt_cli_style::Depth::TrueColor,
    });
    let area = Rect::new(0, 0, 20, 1);
    let mut buffer = Buffer::empty(area);
    let controls = [Button::secondary("Details"), Button::secondary("Links")];
    let hits = paint(&app, &mut buffer, area, &controls, Align::Left, None);
    assert_eq!(
        hits,
        vec![Some(Rect::new(0, 0, 9, 1)), Some(Rect::new(10, 0, 7, 1))]
    );
    let idle = buffer[(0, 0)].bg;
    assert!(moved(&mut app, 0, 0), "left padding is part of the chip");
    app.buttons.begin_frame();
    paint(&app, &mut buffer, area, &controls, Align::Left, None);
    let look = app.look();
    assert_eq!(
        buffer[(0, 0)].bg,
        screen::chip(&look.theme, ChipKind::Secondary, ChipState::Hot, look.depth)
            .bg
            .unwrap()
    );
    assert_ne!(buffer[(0, 0)].bg, idle);
    assert_eq!(
        buffer[(9, 0)].bg,
        ratatui::style::Color::Reset,
        "the gap never brightens"
    );
    assert!(moved(&mut app, 19, 0));
    assert!(
        !moved(&mut app, 18, 0),
        "moving outside chips needs no repaint"
    );
    app.buttons.begin_frame();
    paint(&app, &mut buffer, area, &controls, Align::Left, None);
    assert_eq!(buffer[(0, 0)].bg, idle);
    assert!(
        app.hits.borrow().is_empty(),
        "presentation registers no activation action"
    );
}

#[test]
fn disabled_and_covered_buttons_cannot_hover_and_modal_owner_uses_its_own_geometry() {
    let mut app = App::default();
    let area = Rect::new(0, 0, 12, 1);
    let mut buffer = Buffer::empty(area);
    paint(
        &app,
        &mut buffer,
        area,
        &[Button::primary("Send").enabled(false)],
        Align::Left,
        None,
    );
    assert!(app.buttons.regions.borrow().is_empty());
    assert!(!moved(&mut app, 1, 0));
    app.help = true;
    app.buttons.begin_frame();
    paint(
        &app,
        &mut buffer,
        area,
        &[Button::secondary("Details")],
        Align::Left,
        None,
    );
    assert!(
        app.buttons.regions.borrow().is_empty(),
        "covered base controls have no hover region"
    );
    paint(
        &app,
        &mut buffer,
        area,
        &[Button::secondary("Close")],
        Align::Left,
        Some(Overlay::Help),
    );
    assert!(moved(&mut app, 1, 0));
    assert_eq!(
        app.buttons.hovered.get(),
        Some((Rect::new(0, 0, 7, 1), Some(Overlay::Help)))
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.buttons.hovered.get().is_none());
    assert!(!app.help);
}

#[test]
fn keyboard_and_frame_invalidation_clear_hover_without_retaining_stale_regions() {
    let mut app = App::default();
    let area = Rect::new(0, 0, 12, 1);
    let mut buffer = Buffer::empty(area);
    paint(
        &app,
        &mut buffer,
        area,
        &[Button::secondary("Links")],
        Align::Right,
        None,
    );
    assert!(moved(&mut app, 11, 0));
    app.key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE));
    assert!(app.buttons.hovered.get().is_none());
    assert!(moved(&mut app, 11, 0));
    app.invalidate_frame_geometry();
    assert!(app.buttons.regions.borrow().is_empty());
    assert!(app.buttons.hovered.get().is_none());
    assert!(
        !moved(&mut app, 11, 0),
        "an invalidated frame cannot invent hover"
    );
}

#[test]
fn chip_cells_use_the_shared_style_at_each_depth_without_relayout_on_hover() {
    use tmt_cli_style::{Base, Depth, Theme};
    for (base, depth) in [
        (Base::Tmt, Depth::TrueColor),
        (Base::TmtLight, Depth::TrueColor),
        (Base::Terminal, Depth::Ansi16),
        (Base::Tmt, Depth::None),
    ] {
        let mut app = App::default();
        app.initial_look = Some(crate::look::Look {
            theme: Theme::new(base),
            depth,
        });
        let look = app.look();
        let area = Rect::new(0, 0, 18, 1);
        let mut buffer = Buffer::empty(area);
        let buttons = [
            Button::primary("Send"),
            Button::secondary("Cancel").enabled(false),
        ];
        let before = paint(&app, &mut buffer, area, &buttons, Align::Right, None);
        let send = before[0].unwrap();
        let disabled = before[1].unwrap();
        assert!(moved(&mut app, send.right() - 1, send.y));
        app.buttons.begin_frame();
        assert_eq!(
            paint(&app, &mut buffer, area, &buttons, Align::Right, None),
            before
        );
        for (rect, kind, state) in [
            (send, ChipKind::Primary, ChipState::Hot),
            (disabled, ChipKind::Secondary, ChipState::Disabled),
        ] {
            let expected = screen::chip(&look.theme, kind, state, depth);
            for x in rect.x..rect.right() {
                let cell = &buffer[(x, rect.y)];
                assert_eq!(
                    cell.bg,
                    expected.bg.unwrap_or_default(),
                    "{base:?}/{depth:?}"
                );
                assert_eq!(
                    cell.fg,
                    expected.fg.unwrap_or_default(),
                    "{base:?}/{depth:?}"
                );
                assert_eq!(cell.modifier, expected.add_modifier, "{base:?}/{depth:?}");
            }
        }
        assert!(moved(&mut app, disabled.x, disabled.y));
        assert!(
            app.buttons.hovered.get().is_none(),
            "disabled padding cannot hover"
        );
    }
}

#[test]
#[ignore = "explicit real-App chip look packet; writes only to a task-owned evidence directory"]
fn capture_chip_controls() {
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use tmt_cli_style::{Base, Depth, Theme};
    let root = std::path::PathBuf::from(
        std::env::var_os("TMT_OPS_CHIP_CAPTURE").expect("task-owned evidence path"),
    );
    std::fs::create_dir_all(&root).unwrap();
    let draw = |app: &mut App, width, height| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(app, frame.area());
                crate::board::view::render(frame, app);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    };
    crate::status::with_now_ms(1_791_600_000_000, || {
        let _no_commands = crate::runner::forbid_commands();
        for (base, depth) in [
            (Base::Tmt, Depth::TrueColor),
            (Base::TmtLight, Depth::TrueColor),
            (Base::Tmt, Depth::None),
        ] {
            for width in [160, 100, 60] {
                for state in ["idle", "hover", "focused", "busy", "squad"] {
                    let height = if state == "idle" { 44 } else { 24 };
                    let look = crate::look::Look {
                        theme: Theme::new(base),
                        depth,
                    };
                    let mut app = crate::board::app::workspace::tests::capture_case("agents", look);
                    if state == "squad" {
                        let index = app
                            .workspace_rows()
                            .iter()
                            .position(|row| {
                                matches!(row.target, crate::board::workspace::OpsTarget::Squad(_))
                            })
                            .unwrap();
                        app.select(index);
                    }
                    draw(&mut app, width, height);
                    if state == "hover" {
                        let hit = app.composer.send_hit.borrow().clone().unwrap();
                        assert!(moved(&mut app, hit.area.x, hit.area.y));
                    } else if state == "focused" || state == "busy" {
                        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        assert!(app.input.is_some());
                        app.paste("A retained message draft");
                        if state == "busy" {
                            app.in_flight = 1;
                        }
                    }
                    let buffer = draw(&mut app, width, height);
                    if state == "busy" {
                        assert!(app.composer.send_hit.borrow().is_none());
                    }
                    let cells: Vec<_> = buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                    let lines: Vec<_> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect();
                    let value = json!({"width":width,"height":height,"theme":if base == Base::TmtLight {"tmt-light"} else {"tmt"},"depth":format!("{depth:?}"),"state":state,"cells":cells,"lines":lines,"send":format!("{:?}",app.composer.send_hit.borrow().as_ref().map(|hit|hit.area)),"controls":format!("{:?}",app.buttons.regions.borrow())});
                    std::fs::write(
                        root.join(format!("{state}-{base:?}-{depth:?}-{width}.json")),
                        serde_json::to_vec(&value).unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    });
}
