//! The TOK heading of the Agents list is three controls: the sort mark (flat list), the
//! unit word and the window word. Each has its own hit area and lights up alone under the
//! pointer; the unit word switches the whole column like a figure does.
use super::*;
use crate::{
    board::{
        app::{Choice, Request},
        header_sort::Header,
        workspace::AgentsFilter,
    },
    config::{OverviewColumn, SavingsUnit},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn hold(app: &mut App, unit: SavingsUnit) {
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .savings_unit = unit;
}

fn open(filter: AgentsFilter, unit: SavingsUnit) -> App {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    app.workspace.filter = filter;
    hold(&mut app, unit);
    app
}

fn frame(app: &App, width: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn area_of(app: &App, wanted: impl Fn(&Choice) -> bool) -> Option<Rect> {
    app.workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(area, choice)| wanted(choice).then_some(*area))
}

struct Parts {
    mark: Option<Rect>,
    unit: Rect,
    window: Rect,
}

fn parts(app: &App) -> Parts {
    Parts {
        mark: area_of(app, |choice| {
            *choice == Choice::SortBy(Header::Overview(OverviewColumn::TokensPerHour))
        }),
        unit: area_of(app, |choice| *choice == Choice::TokUnit).expect("no unit hit"),
        window: area_of(app, |choice| matches!(choice, Choice::TokRange(_)))
            .expect("no window hit"),
    }
}

fn text(buffer: &Buffer, area: Rect) -> String {
    (area.x..area.x + area.width)
        .map(|x| buffer[(x, area.y)].symbol().to_owned())
        .collect()
}

fn mouse(app: &mut App, kind: MouseEventKind, area: Rect) -> Effect {
    app.mouse(
        MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    )
}

fn hover(app: &mut App, area: Rect) {
    app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
}

fn accented(app: &App, buffer: &Buffer, x: u16, y: u16) -> bool {
    buffer[(x, y)].fg
        == app
            .look()
            .role(tmt_cli_style::Role::Accent)
            .fg
            .unwrap_or_default()
}

const LISTS: [(&str, AgentsFilter); 2] = [
    ("squads", AgentsFilter::All),
    ("agents", AgentsFilter::Agents),
];

#[test]
fn a_click_on_the_unit_word_switches_every_row_and_selects_nothing() {
    for (name, filter) in LISTS {
        for (held, wanted) in [
            (SavingsUnit::Tokens, SavingsUnit::Cost),
            (SavingsUnit::Percent, SavingsUnit::Cost),
            (SavingsUnit::Cost, SavingsUnit::Tokens),
        ] {
            let mut app = open(filter, held);
            paint(&app, 160, 40);
            let unit = parts(&app).unit;
            let (selected, order) = (app.workspace.selected.clone(), app.workspace_order());
            let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
                mouse(&mut app, MouseEventKind::Down(MouseButton::Left), unit)
            else {
                panic!("{name} {held:?}: the unit word must use the preferences writer");
            };
            assert_eq!(next.savings_unit, wanted, "{name} {held:?}");
            let mut rest = next.clone();
            rest.savings_unit = held;
            assert_eq!(rest, expected, "{name}: only the unit changes");
            assert_eq!(app.workspace.selected, selected, "{name}: selects nothing");
            assert_eq!(app.workspace_order(), order, "{name}: sorts nothing");
        }
    }
}

#[test]
fn each_tok_part_lights_up_alone_under_the_pointer() {
    for width in [120, 80] {
        for (name, filter) in LISTS {
            let mut app = open(filter, SavingsUnit::Tokens);
            paint(&app, width, 40);
            let at = parts(&app);
            let y = at.unit.y;
            let idle = frame(&app, width);
            assert!(
                (at.unit.x - 1..at.window.x + at.window.width)
                    .all(|x| !accented(&app, &idle, x, y)),
                "{name} {width}: nothing is lit without the pointer"
            );
            let mut controls = vec![("unit", at.unit), ("window", at.window)];
            if let Some(mark) = at.mark {
                controls.push(("sort", mark));
            }
            assert_eq!(
                at.mark.is_some(),
                filter == AgentsFilter::Agents,
                "{name} {width}: only the flat list sorts by tokens"
            );
            for (part, area) in &controls {
                hover(&mut app, *area);
                paint(&app, width, 40);
                let lit = frame(&app, width);
                for (other, rect) in &controls {
                    let on = (rect.x..rect.x + rect.width).all(|x| accented(&app, &lit, x, y));
                    assert_eq!(
                        on,
                        other == part,
                        "{name} {width}: hover {part} lights {other}"
                    );
                }
                assert_eq!(
                    parts(&app).unit,
                    at.unit,
                    "{name} {width}: a hover never moves the hit areas"
                );
            }
        }
    }
}

#[test]
fn hovering_the_unit_word_names_the_other_unit_and_the_hit_stays_put() {
    for (name, filter) in LISTS {
        for (held, now, other) in [
            (SavingsUnit::Tokens, "TOK", "≈$"),
            (SavingsUnit::Cost, "≈$", "TOK"),
        ] {
            let mut app = open(filter, held);
            paint(&app, 160, 40);
            let at = parts(&app);
            assert_eq!(text(&frame(&app, 160), at.unit), now, "{name} {held:?}");
            hover(&mut app, at.unit);
            paint(&app, 160, 40);
            let lit = frame(&app, 160);
            let row = Rect::new(0, at.unit.y, 160, 1);
            let line = text(&lit, row);
            assert!(
                line.contains(&format!("{other} 1h")),
                "{name} {held:?}: the label names the unit a click gives: {line}"
            );
            assert_eq!(parts(&app).unit, at.unit, "{name} {held:?}");
            assert_eq!(text(&lit, at.window), "1h", "{name} {held:?}");
        }
    }
}

#[test]
fn the_sort_mark_keeps_sorting_and_the_window_keeps_cycling() {
    let mut app = open(AgentsFilter::Agents, SavingsUnit::Tokens);
    paint(&app, 160, 40);
    let at = parts(&app);
    let mark = at.mark.expect("the flat list sorts by tokens");
    let order = app.workspace_order();
    assert!(matches!(
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), mark),
        Effect::None
    ));
    assert_ne!(app.workspace_order(), order, "the mark cell sorts");
    paint(&app, 160, 40);
    let window = parts(&app).window;
    let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), window)
    else {
        panic!("the window word must use the preferences writer");
    };
    assert_eq!(next.tok_range, expected.tok_range.next());
}
