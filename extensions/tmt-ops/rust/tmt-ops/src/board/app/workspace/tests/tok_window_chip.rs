//! The window word of the Agents `TOK 1h` heading is its own control that
//! cycles the one shared `workspace.tok_range`; it is separate from the rest of
//! the caption.
use super::*;
use crate::{
    board::{
        app::{Choice, Request},
        header_sort::Header,
        shell::OpsTab,
        workspace::AgentsFilter,
    },
    config::TokRange,
};
use ratatui::{
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn hold(app: &mut App, range: TokRange) {
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .tok_range = range;
}

fn chip(app: &App) -> Option<Rect> {
    app.workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(area, choice)| matches!(choice, Choice::TokRange(_)).then_some(*area))
}

fn click_at(app: &mut App, x: u16, y: u16) -> Effect {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    )
}

fn painted(app: &App, area: Rect) -> String {
    paint(app, 160, 40)[usize::from(area.y)]
        .chars()
        .skip(usize::from(area.x))
        .take(usize::from(area.width))
        .collect()
}

fn lists() -> [(&'static str, AgentsFilter); 2] {
    [
        ("squads", AgentsFilter::All),
        ("agents", AgentsFilter::Agents),
    ]
}

#[test]
fn the_window_word_is_a_hit_beside_the_tok_word_in_both_lists() {
    for (name, filter) in lists() {
        for range in TokRange::ALL {
            let mut app = board();
            app.shell.tab = OpsTab::Agents;
            app.workspace.filter = filter;
            hold(&mut app, range);
            let rows = paint(&app, 160, 40);
            let area = chip(&app).unwrap_or_else(|| panic!("{name} {range:?}: no chip hit"));
            assert_eq!(painted(&app, area), range.argument(), "{name} {range:?}");
            let line = &rows[usize::from(area.y)];
            let before: String = line.chars().take(usize::from(area.x)).collect();
            assert!(before.ends_with("TOK "), "{name} {range:?}: {line}");
            // The window hit is its own control: the unit word is the TokUnit hit before the
            // space, and only the flat list's sort hit sits on the mark cell before that.
            assert_eq!(area.width, 2, "{name} {range:?}");
            let unit = Rect::new(area.x - 4, area.y, 3, 1);
            let mark = Rect::new(area.x - 5, area.y, 1, 1);
            assert_eq!(painted(&app, unit), "TOK", "{name} {range:?}");
            for (other, choice) in app.workspace_strip_hits.borrow().iter() {
                if matches!(choice, Choice::TokRange(_)) {
                    continue;
                }
                assert!(!other.intersects(area), "{name} {range:?}: {choice:?}");
                assert_eq!(
                    *other == unit,
                    *choice == Choice::TokUnit,
                    "{name} {range:?}: {choice:?} against the unit word"
                );
                assert_eq!(
                    other.contains((mark.x, mark.y).into()),
                    *choice
                        == Choice::SortBy(Header::Overview(
                            crate::config::OverviewColumn::TokensPerHour
                        )),
                    "{name} {range:?}: {choice:?} on the mark cell"
                );
            }
            assert!(
                app.workspace_strip_hits
                    .borrow()
                    .iter()
                    .any(|(other, choice)| *other == unit && *choice == Choice::TokUnit),
                "{name} {range:?}: no unit hit"
            );
        }
    }
}

#[test]
fn a_click_on_the_window_word_requests_only_the_next_range() {
    for (name, filter) in lists() {
        for range in TokRange::ALL {
            let mut app = board();
            app.shell.tab = OpsTab::Agents;
            app.workspace.filter = filter;
            hold(&mut app, range);
            paint(&app, 160, 40);
            let area = chip(&app).unwrap();
            let selected = app.workspace.selected.clone();
            let effect = click_at(&mut app, area.x + area.width - 1, area.y);
            let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) = effect else {
                panic!("{name} {range:?}: the chip must use the preferences writer");
            };
            assert_eq!(expected.tok_range, range);
            assert_eq!(next.tok_range, range.next(), "{name} {range:?}");
            let mut rest = next.clone();
            rest.tok_range = range;
            assert_eq!(rest, expected, "only the TOK range changes");
            assert_eq!(
                app.workspace.selected, selected,
                "a chip click selects nothing"
            );
            assert_eq!(
                app.tok_range(),
                range,
                "only a saved change moves the range"
            );
        }
    }
}

#[test]
fn the_ranges_cycle_in_the_documented_order() {
    let order = TokRange::ALL.map(TokRange::argument);
    assert_eq!(order, ["1h", "3h", "6h", "1d", "7d"]);
    let mut range = TokRange::Hour;
    let mut seen = vec![range.argument()];
    for _ in 0..5 {
        range = range.next();
        seen.push(range.argument());
    }
    assert_eq!(seen, ["1h", "3h", "6h", "1d", "7d", "1h"]);
}

#[test]
fn the_painted_heading_and_the_digest_range_follow_one_shared_setting() {
    let mut app = board();
    hold(&mut app, TokRange::SixHours);
    app.shell.tab = OpsTab::Agents;
    let agents = paint(&app, 160, 40).join("\n");
    assert!(agents.contains("TOK 6h"), "{agents}");
    let agents_hit = chip(&app).unwrap();
    app.shell.tab = OpsTab::Digest;
    let digest = paint(&app, 160, 40).join("\n");
    assert!(digest.contains("Range 6h"), "{digest}");
    let digest_next = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(_, choice)| match choice {
            Choice::TokRange(range) => Some(*range),
            _ => None,
        })
        .unwrap();
    app.shell.tab = OpsTab::Agents;
    paint(&app, 160, 40);
    let Effect::Act(Request::WorkspacePreferences { next, .. }) =
        click_at(&mut app, agents_hit.x, agents_hit.y)
    else {
        panic!("the chip must use the preferences writer");
    };
    assert_eq!(
        next.tok_range, digest_next,
        "both tabs write the same next range"
    );
}

#[test]
fn a_column_header_too_narrow_for_tok_has_no_window_hit() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    paint(&app, 44, 20);
    assert_eq!(chip(&app), None);
}
