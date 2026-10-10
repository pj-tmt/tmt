//! Named pane dispatch and existing split/tab chrome.

use super::{
    detail::render_detail, notes::render_notes, replies::render_replies, rows::render_rows,
    tabs::pane_tab,
};
use crate::board::app::App;
use crate::board::app::TitleHit;
use crate::board::notes::sanitize;
use crate::config::{BoardMode, Pane};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::{Outline, strip};

/// The existing pane is receiving keys only while no input or overlay owns them.
pub(super) fn receiving_pane(app: &App) -> Option<Pane> {
    if app.input.is_some()
        || app.searching
        || app.help
        || app.menu.is_some()
        || app.settings.is_some()
        || app.view_picker.is_some()
        || app.theme_picker.is_some()
        || app.switcher.is_some()
        || app.cron_list.is_some()
        || app.checklist_shown()
    {
        None
    } else {
        app.focused_pane()
    }
}

/// Split mode tiles the configured panes; tabs mode shows the focused pane
/// under a tab bar. The focused pane's border is highlighted.
pub(super) fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    let jobs = body_halves(app, area);
    match jobs {
        Some((members, jobs)) => {
            render_members(frame, app, members, true);
            crate::board::cronboard::render_half(frame, app, jobs);
        }
        None => render_members(frame, app, area, false),
    }
}

fn body_halves(app: &App, area: Rect) -> Option<(Rect, Rect)> {
    app.view
        .as_ref()
        .filter(|view| view.home.is_none())
        .and_then(|_| crate::board::cronboard::half_wanted(app, area))
        // A failed split leaves the whole body to the members.
        .and_then(|lower| crate::board::composition::halves(area, lower).ok())
}

/// Solve the same members rectangle before reconciling input focus.
pub(super) fn prepare(app: &App, area: Rect) {
    let Some(view) = app.view.as_ref().filter(|view| view.home.is_none()) else {
        return;
    };
    let Some(board) = app.effective_board() else {
        return;
    };
    let (area, rule_below) =
        body_halves(app, area).map_or((area, false), |(members, _)| (members, true));
    let mut composition = view.composition.borrow_mut();
    if crate::board::composition::layout(
        &mut composition,
        board,
        &app.folds_at(area.width),
        app.focused(),
        area,
        rule_below,
    )
    .is_err()
    {
        *composition = None;
    }
}

/// `rule_below` says the jobs half's rule is drawn on the row under `area`: a
/// framed pane resting on it ends its frame there, so the boundary has one rule.
fn render_members(frame: &mut Frame, app: &App, area: Rect, rule_below: bool) {
    let look = app.look();
    let Some(view) = &app.view else {
        render_rows(frame, app, area, Shared::default());
        return;
    };
    if view.home.is_some() {
        crate::board::home::render(frame, app, area);
        return;
    }
    let board = app.effective_board().expect("loaded view has a board");
    let (placement, chrome) = {
        let mut composition = view.composition.borrow_mut();
        let placement = crate::board::composition::layout(
            &mut composition,
            board,
            &app.folds_at(area.width),
            app.focused(),
            area,
            rule_below,
        );
        let chrome = composition
            .as_ref()
            .map(|cache| cache.chrome.clone())
            .unwrap_or_default();
        (placement, chrome)
    };
    let slots = match placement {
        Ok(slots) => slots,
        Err(error) => {
            for (offset, line) in error.lines().take(usize::from(area.height)).enumerate() {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect {
                        y: area.y + offset as u16,
                        height: 1,
                        ..area
                    },
                    Line::styled(line, look.role(Role::Dim)),
                );
            }
            return;
        }
    };
    let focused = receiving_pane(app);
    let pane_block = |pane: Pane| {
        let focus = if Some(pane) == focused { "Focus: " } else { "" };
        let title = match pane {
            // The lead's notes nobody updated for a while say how long.
            Pane::Notes => {
                let lead = view.document["squad"]["lead"]["name"]
                    .as_str()
                    .unwrap_or("no lead");
                let mut title = vec![Span::raw(format!(" {focus}notes · {lead} "))];
                if let Some(age) =
                    crate::staleness::label(&view.document["squad"]["notesStaleness"])
                {
                    title.push(Span::styled(format!("· {age} "), look.role(Role::Waiting)));
                }
                Line::from(title)
            }
            other => Line::from(format!(" {focus}{} ", other.title())),
        };
        Outline {
            title,
            border: look.role(Role::Dim),
            title_style: if Some(pane) == focused && board.panes.len() > 1 {
                // Receiving titles start from their own role, so incidental
                // frame effects cannot replace a configured Accent effect.
                Style::reset()
                    .patch(look.role(Role::Accent))
                    .add_modifier(Modifier::BOLD)
            } else {
                look.role(Role::Muted)
            },
        }
    };
    match board.mode {
        BoardMode::Split if board.panes.len() == 1 && app.collapsed_panes().is_empty() => {
            let shared = Shared {
                below: rule_below,
                ..Shared::default()
            };
            render_pane(frame, app, board.panes[0], slots[0].1, shared)
        }
        BoardMode::Split => render_split(frame, app, &chrome, &pane_block),
        BoardMode::Tabs => {
            let focused = app.focused();
            let bar = slots[0].1;
            let rest = slots[1].1;
            let mut spans = Vec::new();
            for pane in &board.panes {
                spans.push(pane_tab(look, pane.title(), *pane == focused));
                spans.push(Span::raw(" "));
            }
            strip::paint_left(frame.buffer_mut(), bar, Line::from(spans));
            if board.members && focused == Pane::Rows {
                let shared = Shared {
                    below: rule_below,
                    ..Shared::default()
                };
                render_pane(frame, app, focused, rest, shared);
                return;
            }
            let outline = pane_block(focused);
            let framed = if rule_below { reach_rule(rest) } else { rest };
            let inner = outline.inner(framed);
            outline.paint_flat(framed, frame.buffer_mut());
            render_pane(frame, app, focused, inner, Shared::default());
        }
    }
}

/// The slot extended over the row below it, where the jobs half's rule is painted.
fn reach_rule(area: Rect) -> Rect {
    Rect {
        height: area.height + 1,
        ..area
    }
}

/// Named pane slots retain their rich painter, borders and title hits.
fn render_split(
    frame: &mut Frame,
    app: &App,
    chrome: &[crate::board::composition::Chrome],
    pane_block: &dyn Fn(Pane) -> Outline<'static>,
) {
    let collapsed = app.collapsed_panes();
    // Top to bottom, so a pane's heading is painted over the edge above it.
    let mut order = (0..chrome.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| (chrome[index].area.y, chrome[index].area.x));
    for index in order {
        let geometry = chrome[index];
        let pane = geometry.pane;
        let area = geometry.area;
        if area.is_empty() {
            continue;
        }
        let title = Rect { height: 1, ..area };
        app.title_hits
            .borrow_mut()
            .push(TitleHit { pane, area: title });
        if collapsed.contains(&pane) {
            let mut text = format!("▸ {}", pane.title());
            if pane == Pane::Notes
                && let Some(view) = &app.view
            {
                if let Some(lead) = view.document["squad"]["lead"]["name"].as_str() {
                    text.push_str(&format!(" · {}", sanitize(lead)));
                }
                if let Some(age) =
                    crate::staleness::label(&view.document["squad"]["notesStaleness"])
                {
                    text.push_str(&format!(" · {age}"));
                }
            }
            let look = app.look();
            strip::paint_left(
                frame.buffer_mut(),
                title,
                Line::styled(text, look.role(Role::Muted)),
            );
        } else {
            if let Some(painted) = geometry.painted {
                pane_block(pane).paint_flat(painted, frame.buffer_mut());
                if !geometry.inner.is_empty() {
                    render_pane(frame, app, pane, geometry.inner, Shared::default());
                }
            } else {
                render_pane(frame, app, pane, geometry.inner, geometry.shared);
            }
        }
    }
}

pub(in crate::board) use crate::board::composition::Shared;

fn render_pane(frame: &mut Frame, app: &App, pane: Pane, area: Rect, shared: Shared) {
    match pane {
        Pane::Rows => render_rows(frame, app, area, shared),
        Pane::Notes => render_notes(frame, app, area),
        Pane::Detail => render_detail(frame, app, area),
        Pane::Replies => render_replies(frame, app, area),
    }
}
