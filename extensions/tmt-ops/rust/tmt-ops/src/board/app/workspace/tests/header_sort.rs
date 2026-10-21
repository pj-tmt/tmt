//! The Agents column headers sort by click, show the shown order and light up
//! under the pointer. A click only changes the order: never the selection, the
//! saved preferences or anything sent.
use super::*;
use crate::{
    board::{
        app::Choice, header_sort::Header, overview::cron_counts::GlyphSet, shell::OpsTab,
        workspace::AgentsFilter,
    },
    config::{OpsSort, OverviewColumn, OverviewSort},
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn open(filter: AgentsFilter) -> App {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = filter;
    app
}

fn sorts(app: &App) -> Vec<(Rect, Header)> {
    app.workspace_strip_hits
        .borrow()
        .iter()
        .filter_map(|(area, choice)| match choice {
            Choice::SortBy(header) => Some((*area, *header)),
            _ => None,
        })
        .collect()
}

fn hit(app: &App, header: Header) -> Rect {
    sorts(app)
        .into_iter()
        .find_map(|(area, other)| (other == header).then_some(area))
        .unwrap_or_else(|| panic!("no hit for {header:?}"))
}

fn buffer(app: &App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn header_row(app: &App, header: Header) -> String {
    let rows = paint(app, 160, 40);
    rows[usize::from(hit(app, header).y)].clone()
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

fn click(app: &mut App, area: Rect) -> Effect {
    mouse(app, MouseEventKind::Down(MouseButton::Left), area)
}

fn press(app: &mut App, header: Header) -> Effect {
    let area = hit(app, header);
    click(app, area)
}

#[test]
fn a_flat_header_click_sorts_then_reverses_and_changes_nothing_else() {
    let mut app = open(AgentsFilter::Agents);
    paint(&app, 160, 40);
    let selected = app.workspace.selected.clone();
    let preferences = app.workspace_snapshot().unwrap().preferences.clone();
    assert_eq!(app.workspace_order().overview, None);
    for column in [
        OverviewColumn::Agent,
        OverviewColumn::Squad,
        OverviewColumn::State,
        OverviewColumn::TokensPerHour,
        OverviewColumn::Model,
    ] {
        let header = Header::Overview(column);
        paint(&app, 160, 40);
        let area = hit(&app, header);
        assert_eq!(click(&mut app, area), Effect::None, "{column:?}");
        assert_eq!(
            app.workspace_order().overview,
            Some(OverviewSort::initial(column)),
            "{column:?} starts in its own direction"
        );
        paint(&app, 160, 40);
        let area = hit(&app, header);
        assert_eq!(click(&mut app, area), Effect::None, "{column:?}");
        assert_eq!(
            app.workspace_order().overview,
            Some(OverviewSort::initial(column).reverse()),
            "{column:?} reverses on a second click"
        );
        assert_eq!(app.workspace.selected, selected, "{column:?}");
        assert_eq!(
            app.workspace_snapshot().unwrap().preferences,
            preferences,
            "{column:?}"
        );
    }
}

#[test]
fn a_flat_header_shows_the_shown_order_beside_its_word() {
    let mut app = open(AgentsFilter::Agents);
    let agent = Header::Overview(OverviewColumn::Agent);
    let tok = Header::Overview(OverviewColumn::TokensPerHour);
    assert!(
        !header_row(&app, agent).contains(['↑', '↓', '↕']),
        "no order is shown until a header is clicked"
    );
    let area = hit(&app, agent);
    click(&mut app, area);
    assert!(header_row(&app, agent).contains("AGENT ↑ "));
    press(&mut app, agent);
    assert!(header_row(&app, agent).contains("AGENT ↓ "));
    let area = hit(&app, tok);
    click(&mut app, area);
    let row = header_row(&app, tok);
    assert!(row.contains("↓TOK 1h"), "{row}");
    assert!(
        !row.contains("AGENT ↓"),
        "{row}: only the sorted word shows an arrow"
    );
    press(&mut app, tok);
    assert!(header_row(&app, tok).contains("↑TOK 1h"));
}

#[test]
fn the_tok_word_sorts_and_the_window_word_still_cycles_the_range() {
    let mut app = open(AgentsFilter::Agents);
    paint(&app, 160, 40);
    let tok = hit(&app, Header::Overview(OverviewColumn::TokensPerHour));
    let window = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(area, choice)| matches!(choice, Choice::TokRange(_)).then_some(*area))
        .unwrap();
    assert!(!tok.intersects(window));
    click(&mut app, window);
    assert_eq!(app.workspace_order().overview, None);
    click(&mut app, tok);
    assert_eq!(
        app.workspace_order().overview,
        Some(OverviewSort::initial(OverviewColumn::TokensPerHour))
    );
}

#[test]
fn a_squads_header_click_picks_name_or_attention_and_reverses() {
    let mut app = open(AgentsFilter::All);
    paint(&app, 160, 40);
    let selected = app.workspace.selected.clone();
    assert_eq!(
        sorts(&app).len(),
        2,
        "only SQUAD / AGENT and STATE have a squad order"
    );
    let name = Header::Squads(OpsSort::Name);
    let state = Header::Squads(OpsSort::Attention);
    // STATE first, so the first SQUAD / AGENT click is a change of order whatever the saved one is.
    press(&mut app, state);
    let order = app.workspace_order();
    assert_eq!((order.agents, order.reversed), (OpsSort::Attention, false));
    let row = header_row(&app, state);
    assert!(row.contains("STATE ↓") && !row.contains("AGENT ↑"), "{row}");
    press(&mut app, state);
    assert!(app.workspace_order().reversed);
    assert!(header_row(&app, state).contains("STATE ↑"));
    press(&mut app, name);
    let order = app.workspace_order();
    assert_eq!((order.agents, order.reversed), (OpsSort::Name, false));
    let row = header_row(&app, name);
    assert!(
        row.contains("SQUAD / AGENT ↑") && !row.contains("STATE ↑"),
        "{row}"
    );
    press(&mut app, name);
    assert!(app.workspace_order().reversed);
    assert!(header_row(&app, name).contains("SQUAD / AGENT ↓"));
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn hovering_a_header_shows_the_swap_mark_and_the_accent_until_the_pointer_leaves() {
    let mut app = open(AgentsFilter::Agents);
    let squad = Header::Overview(OverviewColumn::Squad);
    paint(&app, 160, 40);
    let area = hit(&app, squad);
    let idle = buffer(&app)[(area.x, area.y)].clone();
    let other = Rect::new(0, 0, 1, 1);
    assert!(!header_row(&app, squad).contains('↕'));

    assert!(app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.header_hover, Some(squad));
    assert!(header_row(&app, squad).contains("SQUAD ↕"));
    let hovered = buffer(&app)[(area.x, area.y)].clone();
    assert_eq!(hovered.symbol(), idle.symbol());
    assert_eq!(
        hovered.fg,
        app.look()
            .role(tmt_cli_style::Role::Accent)
            .fg
            .unwrap_or_default()
    );
    assert_ne!(hovered.fg, idle.fg);
    assert!(
        !app.move_pointer(MouseEvent {
            kind: MouseEventKind::Moved,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        }),
        "a move inside the same header redraws nothing"
    );

    assert!(app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: other.x,
        row: other.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.header_hover, None);
    assert!(!header_row(&app, squad).contains('↕'));
}

#[test]
fn a_hovered_sorted_header_keeps_its_direction() {
    let mut app = open(AgentsFilter::Agents);
    let squad = Header::Overview(OverviewColumn::Squad);
    paint(&app, 160, 40);
    let area = hit(&app, squad);
    click(&mut app, area);
    paint(&app, 160, 40);
    app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
    let row = header_row(&app, squad);
    assert!(row.contains("SQUAD ↑") && !row.contains('↕'), "{row}");
}

#[test]
fn an_ascii_terminal_gets_ascii_marks() {
    let mut app = open(AgentsFilter::Agents);
    app.cron_glyphs = GlyphSet::Ascii;
    let agent = Header::Overview(OverviewColumn::Agent);
    let tok = Header::Overview(OverviewColumn::TokensPerHour);
    paint(&app, 160, 40);
    press(&mut app, agent);
    assert!(header_row(&app, agent).contains("AGENT ^ "));
    press(&mut app, tok);
    assert!(header_row(&app, tok).contains("vTOK 1h"));
    let area = hit(&app, Header::Overview(OverviewColumn::State));
    app.move_pointer(MouseEvent {
        kind: MouseEventKind::Moved,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(header_row(&app, agent).contains("STATE ~"));
}
