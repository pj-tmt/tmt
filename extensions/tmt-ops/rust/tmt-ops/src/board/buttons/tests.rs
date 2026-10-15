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
