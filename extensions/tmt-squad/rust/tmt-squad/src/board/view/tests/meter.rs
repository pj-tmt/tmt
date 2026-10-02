use super::*;
use crate::board::{meter::Meter, rate::tests::input};
use ratatui::{
    backend::{Backend, ClearType, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use std::{
    convert::Infallible,
    time::{Duration, Instant},
};

/// Record exactly the cells handed to Backend::draw, after ratatui's diff.
struct Recording {
    inner: TestBackend,
    emitted: Vec<(u16, u16)>,
}
impl Backend for Recording {
    type Error = Infallible;
    fn draw<'a, I>(&mut self, content: I) -> Result<(), Infallible>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let cells: Vec<_> = content.collect();
        self.emitted.extend(cells.iter().map(|(x, y, _)| (*x, *y)));
        self.inner.draw(cells.into_iter())
    }
    fn hide_cursor(&mut self) -> Result<(), Infallible> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> Result<(), Infallible> {
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> Result<Position, Infallible> {
        self.inner.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, p: P) -> Result<(), Infallible> {
        self.inner.set_cursor_position(p)
    }
    fn clear(&mut self) -> Result<(), Infallible> {
        self.inner.clear()
    }
    fn clear_region(&mut self, region: ClearType) -> Result<(), Infallible> {
        self.inner.clear_region(region)
    }
    fn size(&self) -> Result<Size, Infallible> {
        self.inner.size()
    }
    fn window_size(&mut self) -> Result<WindowSize, Infallible> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> Result<(), Infallible> {
        self.inner.flush()
    }
}
fn redraw(terminal: &mut Terminal<Recording>, app: &App) {
    terminal.backend_mut().emitted.clear();
    terminal.draw(|frame| render(frame, app)).unwrap();
}
fn with_meter(now: Instant, reduced: bool) -> App {
    let mut app = preset_board();
    let settings = crate::config::TokenRate {
        enabled: true,
        reduced_motion: reduced,
        ..Default::default()
    };
    app.meter = Some(Meter::new(settings, &input(100), now));
    app
}

#[test]
fn sample_and_animation_emit_only_meter_cells_in_normal_render() {
    for width in [80, 120, 200] {
        for reduced in [false, true] {
            let now = Instant::now();
            let mut app = with_meter(now, reduced);
            let mut terminal = Terminal::new(Recording {
                inner: TestBackend::new(width, 24),
                emitted: vec![],
            })
            .unwrap();
            redraw(&mut terminal, &app);
            let sample = now + Duration::from_secs(10);
            app.meter.as_mut().unwrap().sample(Ok(&input(200)), sample);
            redraw(&mut terminal, &app);
            let area = meter_region(&app, Rect::new(0, 1, width, 1)).unwrap().0;
            assert!(
                !terminal.backend().emitted.is_empty(),
                "sample must emit cells"
            );
            assert!(
                terminal
                    .backend()
                    .emitted
                    .iter()
                    .all(|(x, y)| area.contains(Position::new(*x, *y)))
            );
            let mut total = terminal.backend().emitted.len();
            for elapsed in [250, 500, 600] {
                app.meter
                    .as_mut()
                    .unwrap()
                    .tick(sample + Duration::from_millis(elapsed));
                redraw(&mut terminal, &app);
                assert!(
                    terminal
                        .backend()
                        .emitted
                        .iter()
                        .all(|(x, y)| area.contains(Position::new(*x, *y)))
                );
                total += terminal.backend().emitted.len();
            }
            assert_eq!(app.meter.as_ref().unwrap().digits().as_deref(), Some("15"));
            let summary: String = (0..width)
                .map(|x| terminal.backend().inner.buffer()[(x, 1)].symbol())
                .collect();
            assert!(summary.ends_with("15 tok/s 10s        █"), "{summary}");
            assert_eq!(area.width, 26, "seven-cell maximum number region");
            redraw(&mut terminal, &app);
            assert!(
                terminal.backend().emitted.is_empty(),
                "idle frame writes no cells"
            );
            println!(
                "meter width={width} reduced={reduced} region={area:?} sample+motion cells={total}"
            );
            // New trend data keeps the reserved region and summary in place.
            app.meter
                .as_mut()
                .unwrap()
                .sample(Ok(&input(300)), now + Duration::from_secs(20));
            assert_eq!(
                meter_region(&app, Rect::new(0, 1, width, 1)).unwrap().0,
                area
            );
            redraw(&mut terminal, &app);
            assert!(!terminal.backend().emitted.is_empty());
            assert!(
                terminal
                    .backend()
                    .emitted
                    .iter()
                    .all(|(x, y)| area.contains(Position::new(*x, *y)))
            );
        }
    }
}

#[test]
fn narrow_drops_spark_then_meter_and_hidden_ticks_emit_nothing() {
    let now = Instant::now();
    let mut app = with_meter(now, false);
    app.meter
        .as_mut()
        .unwrap()
        .sample(Ok(&input(100)), now + Duration::from_secs(60));
    let left = summary_line(&app).width() + 2;
    assert!(
        meter_region(&app, Rect::new(0, 1, (left + 9) as u16, 1))
            .is_some_and(|(_, layout)| !layout.spark)
    );
    let mut compact = Terminal::new(TestBackend::new((left + 9) as u16, 24)).unwrap();
    compact.draw(|frame| render(frame, &app)).unwrap();
    let summary = (0..compact.backend().buffer().area.width)
        .map(|x| compact.backend().buffer()[(x, 1)].symbol())
        .collect::<String>();
    assert!(summary.ends_with("0/s"), "{summary}");
    let width = (left + 9 - 1) as u16;
    assert!(meter_region(&app, Rect::new(0, 1, width, 1)).is_none());
    let mut terminal = Terminal::new(Recording {
        inner: TestBackend::new(width, 24),
        emitted: vec![],
    })
    .unwrap();
    redraw(&mut terminal, &app);
    app.meter
        .as_mut()
        .unwrap()
        .sample(Ok(&input(200)), now + Duration::from_secs(70));
    redraw(&mut terminal, &app);
    assert!(terminal.backend().emitted.is_empty());
    app.current = Some(crate::board::ALL.into());
    assert!(meter_region(&app, Rect::new(0, 1, 200, 1)).is_none());
    app.current = Some(crate::board::LEADS.into());
    assert!(meter_region(&app, Rect::new(0, 1, 200, 1)).is_none());
}

#[test]
fn disabled_meter_keeps_board_buffers_and_crossterm_bytes_identical() {
    let now = Instant::now();
    let baseline = preset_board();
    let mut disabled = with_meter(now, false);
    disabled.meter.as_mut().unwrap().settings.enabled = false;
    for width in [80, 120, 200] {
        assert_eq!(draw(&baseline, width, 24), draw(&disabled, width, 24));
        let bytes = |app: &App| {
            let mut bytes = Vec::<u8>::new();
            let backend = ratatui::backend::CrosstermBackend::new(&mut bytes);
            let options = ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, width, 24)),
            };
            let mut terminal = Terminal::with_options(backend, options).unwrap();
            terminal.draw(|frame| render(frame, app)).unwrap();
            drop(terminal);
            bytes
        };
        assert_eq!(bytes(&baseline), bytes(&disabled));
    }
}

#[test]
fn window_hint_is_conditional_whole_and_help_discloses_semantics() {
    let now = Instant::now();
    let mut app = with_meter(now, true);
    assert!(hints(&app, 200).contains("w window"));
    let complete = hints(&app, 200);
    for width in 0..200 {
        let text = hints(&app, width);
        assert!(text.width() <= width);
        assert!(
            text.split("  ")
                .filter(|hint| !hint.is_empty())
                .all(|hint| complete.split("  ").any(|whole| whole == hint))
        );
    }
    app.view.as_mut().unwrap().token_rate = Some(crate::board::app::RateView {
        settings: app.meter.as_ref().unwrap().settings,
        input: input(100),
    });
    let help = help_lines(&app).join("\n");
    assert!(help.contains("5s window -> 40s trend, 1m -> 80s"));
    assert!(help.contains("no data hides; measured zero is 0"));
    app.meter.as_mut().unwrap().settings.enabled = false;
    app.view.as_mut().unwrap().token_rate = None;
    assert!(!hints(&app, 200).contains("w window"));
    assert!(
        !help_lines(&app)
            .iter()
            .any(|line| line.contains("token-window"))
    );
}

#[test]
fn help_roster_and_disabled_custom_w_do_not_change_on_meter_only_ticks() {
    let now = Instant::now();
    let mut app = with_meter(now, true);
    app.help = true;
    app.view.as_mut().unwrap().token_rate = Some(crate::board::app::RateView {
        settings: app.meter.as_ref().unwrap().settings,
        input: input(100),
    });
    app.excluded_counters = vec!["never".into()];
    let before = help_lines(&app);
    app.meter
        .as_mut()
        .unwrap()
        .sample(Ok(&input(200)), now + Duration::from_secs(10));
    assert_eq!(help_lines(&app), before);
    app.meter.as_mut().unwrap().settings.enabled = false;
    app.view.as_mut().unwrap().token_rate = None;
    app.view.as_mut().unwrap().bindings.extend(
        crate::action::parse_bindings([("w", Some("refresh"))].into_iter(), "bind").unwrap(),
    );
    assert!(
        !hints(&app, 200).contains("w refresh"),
        "disabled footer preserves pre-meter hint set"
    );
}
