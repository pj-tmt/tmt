//! Observable navigation over acquired, exact retained evidence; no Core calls.
use super::*;

fn retained_board(width: u16) -> App {
    let mut app = board();
    fixtures::mixed(&mut app);
    app.usage_chart
        .borrow_mut()
        .apply(Target::Range(10_080))
        .unwrap();
    app.usage_chart
        .borrow_mut()
        .apply(Target::Each(Each::Fixed(60)))
        .unwrap();
    let selection = app.usage_chart.borrow().selection;
    let read = fixtures::retained(&app, selection);
    assert!(app.apply_history(read, std::time::Instant::now()));
    paint(&mut app, width, 51);
    app
}

#[test]
fn scrolling_and_absolute_bucket_selection_keep_one_loaded_series_and_honest_edges() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = retained_board(103);
        let source = Rc::clone(app.usage_chart.borrow().observed.as_ref().unwrap());
        let request = app.retained_history_request().unwrap();
        assert_eq!(request.buckets(), 168);
        let end = app.usage_chart.borrow().viewport.window(168);
        assert_eq!(end.end, 168);
        assert!(end.start > 0);
        app.usage_chart.borrow_mut().set_focus(Focus::Bars);
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let oldest = text(&paint(&mut app, 103, 51));
        assert_eq!(app.usage_chart.borrow().viewport.window(168).start, 0);
        assert!(oldest.contains('▸'));
        assert!(!oldest.contains('◂'));
        assert!(oldest.contains('▲'));
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        paint(&mut app, 103, 51);
        assert_eq!(app.usage_chart.borrow().viewport.window(168), end);
        let hit = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .find(|h| matches!(h.target, Target::Bar(_)))
            .unwrap()
            .area;
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    column: hit.x,
                    row: hit.y,
                    modifiers,
                },
                std::time::Instant::now(),
            );
        }
        let middle = text(&paint(&mut app, 103, 51));
        let window = app.usage_chart.borrow().viewport.window(168);
        assert_eq!(window.start, end.start - 2);
        assert!(middle.contains('◂') && middle.contains('▸'));
        assert!(Rc::ptr_eq(
            &source,
            app.usage_chart.borrow().observed.as_ref().unwrap()
        ));
        assert_eq!(app.retained_history_request(), Some(request));
        assert_eq!(app.usage_chart.borrow().selection.each_minutes, 60);
    });
}

#[test]
fn auto_changes_at_two_widths_and_fixed_selection_preserves_its_grid_on_resize() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = retained_board(100);
        app.usage_chart
            .borrow_mut()
            .apply(Target::Each(Each::Auto))
            .unwrap();
        paint(&mut app, 100, 51);
        let narrow = app.usage_chart.borrow().selection;
        paint(&mut app, 200, 51);
        let wide = app.usage_chart.borrow().selection;
        assert_eq!(narrow.each_minutes, 360);
        assert_eq!(wide.each_minutes, 120);
        assert!(wide.bars().unwrap() > narrow.bars().unwrap());
        app.usage_chart
            .borrow_mut()
            .apply(Target::Each(Each::Fixed(60)))
            .unwrap();
        paint(&mut app, 100, 51);
        let fixed = app.usage_chart.borrow().selection;
        paint(&mut app, 200, 51);
        assert_eq!(fixed.bars(), Ok(168));
        assert_eq!(app.usage_chart.borrow().selection, fixed);
        assert_eq!(app.usage_chart.borrow().each, Each::Fixed(60));
    });
}

#[test]
fn disabled_each_stays_in_the_menu_and_refuses_without_changing_the_grid() {
    let mut state = State::default();
    state.apply(Target::Range(43_200)).unwrap();
    let before = state.selection;
    let mut menu = dropdown::Dropdown::new(&state, Focus::Each);
    menu.selected = 1; // 5m is visible after Auto, despite exceeding 720.
    assert_eq!(menu.choose(), Err("30d needs at least 1h per bar".into()));
    menu.selected = 4; // 1h gives exactly 720 buckets.
    assert_eq!(menu.choose(), Ok(Target::Each(Each::Fixed(60))));
    assert_eq!(state.selection, before);
}

#[test]
fn dropdown_keeps_the_underlying_chart_and_updates_auto_preview_after_resize() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let _no_commands = crate::runner::forbid_commands();
        let mut app = retained_board(103);
        let before = paint(&mut app, 103, 51);
        let source = Rc::clone(app.usage_chart.borrow().observed.as_ref().unwrap());
        super::super::open_dropdown(&mut app, Focus::Each);
        let open = paint(&mut app, 103, 51);
        assert!(text(&open).contains("Automatic (6h per bar)"));
        for y in 5..10 {
            for x in 0..60 {
                assert_eq!(
                    open[(x, y)],
                    before[(x, y)],
                    "dropdown changed uncovered chart cell {x},{y}"
                );
            }
        }
        assert!(Rc::ptr_eq(
            &source,
            app.usage_chart.borrow().observed.as_ref().unwrap()
        ));
        let wide = paint(&mut app, 200, 51);
        assert!(text(&wide).contains("Automatic (2h per bar)"));
        assert_eq!(app.usage_chart.borrow().each, Each::Fixed(60));
    });
}

#[test]
fn time_per_bar_choices_remain_complete_in_narrow_and_themed_menus() {
    use crate::board::overview::chart::format;
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(fixtures::NOW, || {
        for (_, look) in crate::board::home::tests::oracle::looks() {
            for width in [60, 80, 100, 160] {
                let mut app = cases::board_case("usage-each-menu", look, width);
                let shown = text(&paint(&mut app, width, 51));
                let interval = format::interval(
                    Each::Auto
                        .resolve(10_080, app.usage_chart.borrow().viewport.width)
                        .each_minutes,
                );
                assert!(
                    shown.contains(&format!("Automatic ({interval} per bar)")),
                    "{width}: {shown}"
                );
                assert!(shown.contains("1h per bar"));
                assert!(shown.contains("Time per bar"));
                assert!(!shown.contains("Each "));

                let mut app = cases::board_case("usage-auto", look, width);
                let shown = text(&paint(&mut app, width, 51));
                let state = app.usage_chart.borrow();
                if state.hits.iter().any(|hit| hit.target == Target::OpenEach) {
                    let interval = format::interval(state.selection.each_minutes);
                    assert!(shown.contains(&format!("{interval} per bar (auto) ▾")));
                }
            }
        }
    });
}

#[test]
fn reselecting_the_current_interval_preserves_scrolled_and_pinned_evidence() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let app = retained_board(103);
        let mut state = app.usage_chart.borrow_mut();
        state.viewport.scroll(168, -20);
        let source = Rc::clone(state.observed.as_ref().unwrap());
        state.pinned = Some(source.buckets[150].from_ms);
        let window = state.viewport.window(168);
        state.apply(Target::Each(Each::Fixed(60))).unwrap();
        assert_eq!(state.viewport.window(168), window);
        assert_eq!(state.pinned, Some(source.buckets[150].from_ms));
        assert!(Rc::ptr_eq(&source, state.observed.as_ref().unwrap()));
    });
}

#[test]
fn session_motion_repaints_the_fixed_readout_only_when_its_bucket_changes() {
    use crate::board::{BoardEvent, INPUT_WAIT};
    use ratatui::crossterm::event::Event;
    use std::sync::{atomic::AtomicUsize, mpsc::channel};
    use std::time::Duration;
    let (events, input) = channel();
    let (painted, frames) = channel();
    let session = std::thread::spawn(move || {
        crate::status::with_now_ms(fixtures::NOW, || {
            let mut app = retained_board(103);
            app.view.as_mut().unwrap().refresh = None;
            crate::board::tests::session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _, _, _| {},
                |_, _| {},
                |_| panic!("hover must not start checklist work"),
                |_| panic!("hover must not start command work"),
                |app| {
                    let buffer = paint(app, 103, 51);
                    let state = app.usage_chart.borrow();
                    let hit = state
                        .hits
                        .iter()
                        .find(|hit| matches!(hit.target, Target::Bar(_)))
                        .unwrap()
                        .area;
                    let title = buffer
                        .content
                        .chunks(103)
                        .nth(4)
                        .unwrap()
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>();
                    painted.send((state.hover, hit, title)).unwrap();
                    Ok(())
                },
            )
            .unwrap()
        })
    });
    let initial = frames.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(initial.0, None);
    assert!(initial.2.contains("Usage"));
    let moved = |column, row| {
        BoardEvent::Input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }))
    };
    events.send(moved(initial.1.x, initial.1.y)).unwrap();
    let hovered = frames.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(hovered.0.is_some());
    assert!(!hovered.2.contains("Usage"));
    assert!(hovered.2.contains("tok"));
    events.send(moved(initial.1.x, initial.1.y + 1)).unwrap();
    assert!(
        frames.recv_timeout(INPUT_WAIT * 2).is_err(),
        "the same bucket on another provider row does not repaint"
    );
    events.send(moved(0, 0)).unwrap();
    let restored = frames.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(restored.0, None);
    assert_eq!(restored.2, initial.2);
    events
        .send(BoardEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        ))))
        .unwrap();
    assert_eq!(session.join().unwrap(), None);
}

#[test]
fn moving_over_a_disabled_choice_repaints_its_reason_without_selecting_it() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = cases::board_case("usage-disabled-each", crate::look::Look::default(), 103);
        let buffer = paint(&mut app, 103, 51);
        let (x, y) = buffer
            .content
            .chunks(103)
            .enumerate()
            .find_map(|(y, row)| {
                row.windows(2)
                    .position(|cells| cells[0].symbol() == "5" && cells[1].symbol() == "m")
                    .map(|x| (x as u16, y as u16))
            })
            .expect("the actual menu paints its disabled 5m entry");
        let selected = app.usage_chart.borrow().dropdown.as_ref().unwrap().selected;
        let moved = |column, row| MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        // The evidence case starts with the reason already visible.
        assert!(app.move_pointer(moved(0, 0)));
        assert!(app.move_pointer(moved(x, y)));
        assert!(text(&paint(&mut app, 103, 51)).contains("30d needs at least 1h per bar"));
        assert!(!app.move_pointer(moved(x, y)));
        assert_eq!(
            app.usage_chart.borrow().dropdown.as_ref().unwrap().selected,
            selected
        );
        assert!(app.move_pointer(moved(0, 0)));
        assert!(!text(&paint(&mut app, 103, 51)).contains("30d needs at least 1h per bar"));
    });
}

#[test]
fn edge_bucket_markers_keep_their_column_and_both_history_arrows() {
    crate::status::with_now_ms(fixtures::NOW, || {
        let mut app = retained_board(103);
        app.usage_chart.borrow_mut().viewport.scroll(168, -20);
        paint(&mut app, 103, 51);
        let hits: Vec<_> = app
            .usage_chart
            .borrow()
            .hits
            .iter()
            .filter(|hit| matches!(hit.target, Target::Bar(_)))
            .map(|hit| hit.area)
            .collect();
        let left = hits.iter().map(|hit| hit.x).min().unwrap();
        let right = hits.iter().map(|hit| hit.x + hit.width - 1).max().unwrap();
        for x in [left, right] {
            let hit = hits.iter().find(|hit| hit.x == x).unwrap();
            assert!(app.move_pointer(MouseEvent {
                kind: MouseEventKind::Moved,
                column: x,
                row: hit.y,
                modifiers: KeyModifiers::NONE
            }));
            let buffer = paint(&mut app, 103, 51);
            let axis_y = (0..51)
                .find(|y| buffer[(x, *y)].symbol() == "▲")
                .expect("the inspected edge keeps its marker at the actual column");
            assert_eq!(
                buffer[(if x == left { left - 1 } else { left }, axis_y)].symbol(),
                "◂"
            );
            assert_eq!(
                buffer[(if x == right { right + 1 } else { right }, axis_y)].symbol(),
                "▸"
            );
        }
    });
}

#[test]
fn dropdown_fill_covers_keys_labels_and_remainder_without_coloring_the_gutter() {
    crate::status::with_now_ms(fixtures::NOW, || {
        for (_, look) in crate::board::home::tests::oracle::looks() {
            let mut app = cases::board_case("usage-each-menu", look, 103);
            let buffer = paint(&mut app, 103, 51);
            let first = buffer
                .content
                .chunks(103)
                .enumerate()
                .find_map(|(y, row)| {
                    row.windows(4)
                        .position(|cells| {
                            cells.iter().map(|cell| cell.symbol()).collect::<String>() == "Auto"
                        })
                        .map(|x| (x as u16 - 3, y as u16))
                })
                .unwrap();
            let idle = crate::board::input_field::style(look, false);
            // All nine rows, including the gap after each label, form one field.
            for y in first.1..first.1 + 9 {
                for x in first.0..first.0 + 18 {
                    let cell = &buffer[(x, y)];
                    if let Some(bg) = idle.bg {
                        assert_ne!(
                            cell.bg,
                            ratatui::style::Color::Reset,
                            "missing popup fill at {x},{y}"
                        );
                        if y == first.1 {
                            assert_eq!(cell.bg, bg);
                        }
                    } else {
                        assert!(cell.modifier.contains(ratatui::style::Modifier::REVERSED));
                    }
                }
            }
            let gutter = &buffer[(102, first.1 + 2)];
            assert_eq!(gutter.symbol(), " ");
            assert_eq!(gutter.bg, ratatui::style::Color::Reset);
        }
    });
}
