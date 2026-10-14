//! Held Overview strips must preserve current workspace cells and pointer authority.
//! Body geometry is exercised through the real renderer, never a retired painter.
use crate::board::{app::App, app::workspace::tests::board, home::paint, view::render_frame};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use tmt_cli_style::{Depth, Theme, theme::Base};

#[derive(Debug, PartialEq)]
struct Capture {
    buffer: Buffer,
    hits: String,
    actions: String,
    starts: Vec<usize>,
    input: String,
}
fn capture(app: &App, width: u16) -> Capture {
    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
    terminal
        .draw(|frame| render_frame(frame, app, None, Vec::new()))
        .unwrap();
    Capture {
        buffer: terminal.backend().buffer().clone(),
        hits: format!("{:?}", app.hits.borrow()),
        actions: format!("{:?}", app.workspace_action_hits.borrow()),
        starts: app.row_starts.borrow().clone(),
        input: format!("{:?}", app.input_band.get()),
    }
}
fn forget(app: &App) {
    app.view.as_ref().unwrap().derived.borrow_mut().home = Default::default();
}
fn builds(app: &App) -> usize {
    app.view.as_ref().unwrap().derived.borrow().home.builds()
}
fn agrees(app: &App, width: u16, transition: &str) {
    let held = capture(app, width);
    forget(app);
    assert_eq!(held, capture(app, width), "{transition} at {width}");
}
fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn current_workspace_frames_match_after_discarding_held_header_strips() {
    for width in [160, 100, 80, 60] {
        let mut app = board();
        agrees(&app, width, "initial");
        for index in 0..app.workspace_rows().len() {
            app.select(index);
            agrees(&app, width, "selection");
        }
        for (base, depth) in [
            ("tmt-light", Depth::TrueColor),
            ("tmt", Depth::None),
            ("tmt", Depth::TrueColor),
        ] {
            let look = crate::look::Look {
                theme: Theme::new(Base::parse(base).unwrap()),
                depth,
            };
            let view = app.view.as_mut().unwrap();
            view.look = look;
            for value in view
                .home
                .as_mut()
                .unwrap()
                .workspace
                .squad_looks
                .values_mut()
            {
                *value = look;
            }
            agrees(&app, width, base);
        }
        app.select(app.workspace_rows().len() - 1);
        press(&mut app, KeyCode::Char('t'));
        assert!(app.input.is_some());
        agrees(&app, width, "composer");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('w'));
        agrees(&app, width, "search owns input");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('?'));
        agrees(&app, width, "help owns input");
    }
}

#[test]
fn selection_does_not_rebuild_unchanged_overview_summary_strips() {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    capture(&app, 160);
    let initial = builds(&app);
    assert!(initial > 0);
    capture(&app, 160);
    assert_eq!(builds(&app), initial, "steady frames retain strips");
    for index in 0..app.workspace_rows().len() {
        app.select(index);
        capture(&app, 160);
        assert_eq!(
            builds(&app),
            initial,
            "selection does not change aggregate data"
        );
    }
    let held = capture(&app, 160);
    forget(&app);
    assert_eq!(held, capture(&app, 160));
}

#[test]
fn header_width_and_look_are_part_of_the_admitted_cache_key() {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    capture(&app, 160);
    let first = builds(&app);
    capture(&app, 100);
    assert!(builds(&app) > first, "new width must rebuild the strip");
    let before = builds(&app);
    app.view.as_mut().unwrap().look.theme = Theme::new(Base::TmtLight);
    capture(&app, 100);
    assert!(builds(&app) > before, "new look must rebuild the strip");
    agrees(&app, 100, "changed look and width");
}

#[test]
fn held_header_usage_matches_uncached_formatting_and_invalidates_on_observation_change() {
    let app = board();
    let mut usage = super::interaction::header_usage();
    for width in [160, 140, 100, 99, 80] {
        let held = paint::usage_of(&app, &usage, width, app.look());
        assert_eq!(held, paint::usage(&usage, width, app.look()));
        assert_eq!(paint::usage_of(&app, &usage, width, app.look()), held);
    }
    let before = builds(&app);
    paint::usage_of(&app, &usage, 80, app.look());
    assert_eq!(builds(&app), before);
    let before_text = paint::usage_of(&app, &usage, 80, app.look());
    usage.totals[2].as_mut().unwrap().partial = false;
    usage.top.as_mut().unwrap().share.partial = false;
    let updated = paint::usage_of(&app, &usage, 80, app.look());
    assert_ne!(
        updated, before_text,
        "visible coverage changes at the same width"
    );
    assert_eq!(updated, paint::usage(&usage, 80, app.look()));
    assert_eq!(builds(&app), before + 1);
}
