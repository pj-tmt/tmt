//! HOME keeps each painted section with the inputs it was painted for. These tests
//! pin the one property that makes that safe: a frame painted from held sections
//! equals a frame painted from nothing, whatever changed since the last frame, and
//! a steady frame paints nothing again.
use super::interaction::{keys, press};
use super::oracle::rich;
use crate::board::{
    app::{App, RowFeedback, RowTarget},
    home::paint,
    view::render_frame,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, crossterm::event::KeyCode::*};
use tmt_cli_style::{Depth, Theme, theme::Base};

const HEIGHT: u16 = 60;

/// Everything a frame shows or answers to: cells with styles, hits, row starts
/// and the input band.
#[derive(PartialEq, Debug)]
struct Frame {
    buffer: Buffer,
    hits: Vec<(u16, u16, u16, usize)>,
    starts: Vec<usize>,
    input: String,
}

fn capture(app: &App, width: u16) -> Frame {
    let mut terminal = Terminal::new(TestBackend::new(width, HEIGHT)).unwrap();
    terminal
        .draw(|frame| render_frame(frame, app, None))
        .unwrap();
    Frame {
        buffer: terminal.backend().buffer().clone(),
        hits: app
            .hits
            .borrow()
            .iter()
            .map(|hit| (hit.y, hit.x, hit.width, hit.row))
            .collect(),
        starts: app.row_starts.borrow().clone(),
        input: format!("{:?}", app.input_band.get()),
    }
}

fn forget(app: &App) {
    let mut derived = app.view.as_ref().unwrap().derived.borrow_mut();
    derived.home = Default::default();
    derived.member_list = Default::default();
}

fn builds(app: &App) -> usize {
    let derived = app.view.as_ref().unwrap().derived.borrow();
    derived.home.builds() + derived.member_list.builds
}

fn look(base: &str, depth: Depth) -> crate::look::Look {
    crate::look::Look {
        theme: Theme::new(Base::parse(base).unwrap()),
        depth,
    }
}

/// Painting after `change` from held sections equals painting from none.
fn agrees(app: &App, width: u16, what: &str) {
    let held = capture(app, width);
    forget(app);
    let cold = capture(app, width);
    assert_eq!(held, cold, "{what} at {width}");
}

#[test]
fn a_frame_from_held_sections_equals_one_from_none() {
    for width in [160, 100, 80, 60] {
        let mut app = rich();
        agrees(&app, width, "the first frame");
        for index in 0..app.home_entries().len() {
            app.select(index);
            agrees(&app, width, &format!("selecting entry {index}"));
        }
        for (name, look) in [
            ("tmt-light", look("tmt-light", Depth::TrueColor)),
            ("NO_COLOR", look("tmt", Depth::None)),
            ("tmt", look("tmt", Depth::TrueColor)),
        ] {
            app.view.as_mut().unwrap().look = look;
            agrees(&app, width, name);
        }
        press(&mut app, Char('t'));
        agrees(&app, width, "hiding the replies");
        press(&mut app, Char('t'));
        agrees(&app, width, "showing the replies");
        app.select(2);
        press(&mut app, Char('a'));
        agrees(&app, width, "opening a composer");
        press(&mut app, Esc);
        let target = app.home_entries()[app.selected].target.clone();
        app.sent = Some(RowFeedback {
            sent: true,
            target: RowTarget::Home(target),
            home: None,
        });
        agrees(&app, width, "sent feedback");
        press(&mut app, Down);
        agrees(&app, width, "the next key clears it");
        keys(&mut app, &[Char('/'), Char('l'), Char('e'), Char('a')]);
        agrees(&app, width, "searching");
        press(&mut app, Esc);
        app.view.as_mut().unwrap().me = None;
        agrees(&app, width, "no user");
        app.cron = Default::default();
        app.cron.replace(Err("storage unreachable".into()));
        agrees(&app, width, "a failed cron read");
    }
}

#[test]
fn a_steady_frame_paints_no_section() {
    let app = rich();
    capture(&app, 160);
    let first = builds(&app);
    assert!(first >= 6, "the first frame paints every section: {first}");
    capture(&app, 160);
    capture(&app, 160);
    assert_eq!(builds(&app), first, "repeated frames build nothing");
}

#[test]
fn a_selection_move_repaints_only_the_sections_it_touches() {
    let mut app = rich();
    capture(&app, 160);
    let entries = app.home_entries();
    let at = |section: &str| {
        entries
            .iter()
            .position(|entry| entry.target.section == section)
            .unwrap()
    };
    let (needs, blocked, leads, squads) = (
        at("needs-you"),
        at("blocked"),
        at(super::super::LEADS),
        at("squads"),
    );
    drop(entries);
    app.select(needs);
    capture(&app, 160);
    let before = builds(&app);
    app.select(needs + 1);
    capture(&app, 160);
    assert_eq!(builds(&app) - before, 1, "within one section");
    let before = builds(&app);
    app.select(blocked);
    capture(&app, 160);
    assert_eq!(
        builds(&app) - before,
        2,
        "into the next section: the one left and the one entered"
    );
    let before = builds(&app);
    app.select(leads);
    capture(&app, 160);
    app.select(squads);
    capture(&app, 160);
    assert_eq!(
        builds(&app) - before,
        4,
        "each move repaints the section left and the one entered"
    );
}

#[test]
fn width_and_look_repaint_the_sections() {
    let mut app = rich();
    capture(&app, 160);
    let first = builds(&app);
    capture(&app, 100);
    assert_eq!(builds(&app), first * 2, "a new width repaints all");
    app.view.as_mut().unwrap().look = look("tmt-light", Depth::TrueColor);
    capture(&app, 100);
    // The key line takes no style, so only it keeps its paint.
    assert_eq!(
        builds(&app),
        first * 2 + first - 1,
        "a new look repaints all but the key line"
    );
}

/// Ages are formatted before the data is bound, so a changed label is a changed key,
/// and an unchanged one is not.
#[test]
fn an_age_label_change_repaints_the_sections_that_show_it() {
    let app = rich();
    let draw = |now: u64| {
        let mut terminal = Terminal::new(TestBackend::new(160, HEIGHT)).unwrap();
        terminal
            .draw(|frame| paint::render_at(frame, &app, frame.area(), now))
            .unwrap();
        terminal.backend().buffer().clone()
    };
    let now = crate::status::now_ms();
    let first = draw(now);
    let built = builds(&app);
    assert_eq!(draw(now + 1_000), first, "a second later reads the same");
    assert_eq!(builds(&app), built, "an unchanged label paints nothing");
    let later = draw(now + 3 * 86_400_000);
    assert_ne!(later, first, "three days later the labels differ");
    assert!(
        builds(&app) > built,
        "the attention and leads sections repaint"
    );
    let cold = {
        forget(&app);
        draw(now + 3 * 86_400_000)
    };
    assert_eq!(later, cold, "and read what a cold paint reads");
}

/// The strips outside the body are held the same way: the usage line follows its
/// data and width, and the key line follows the cron hint.
#[test]
fn the_header_usage_and_key_line_follow_their_inputs() {
    let app = rich();
    let view = app.view.as_ref().unwrap();
    let mut usage = super::interaction::header_usage();
    for width in [160, 140, 100, 99, 80] {
        let held = paint::usage_of(&app, &usage, width, app.look());
        let again = paint::usage_of(&app, &usage, width, app.look());
        assert_eq!(held, paint::usage(&usage, width, app.look()), "{width}");
        assert_eq!(again, held, "{width}");
        for cron in [false, true] {
            assert_eq!(
                paint::hints_of(view, width.into(), cron),
                paint::hints(width.into(), cron),
                "{width} cron {cron}"
            );
        }
    }
    let before = builds(&app);
    paint::usage_of(&app, &usage, 80, app.look());
    paint::hints_of(view, 80, true);
    assert_eq!(builds(&app), before, "the held strips paint nothing");
    usage.unreported += 1;
    paint::usage_of(&app, &usage, 160, app.look());
    assert_eq!(builds(&app), before + 1, "new usage data repaints the line");
}
