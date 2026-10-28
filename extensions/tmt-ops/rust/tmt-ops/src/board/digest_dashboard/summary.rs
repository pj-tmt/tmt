//! The top of the Digest tab: one status line with the tab-wide window, then the usage,
//! saved and list-price tiles beside the allowance tile, over the admitted, held evidence.
use super::{metrics, model};
use crate::{
    board::{
        app::{App, Choice},
        buttons::Button,
        usage_report::View,
    },
    config::{DigestWindow, SavingsUnit},
    labels::digest::SavingsSummary,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// Status, rule, four tile rows and the definition.
const FULL: u16 = 7;
/// Status, one line of amounts and the definition, when tiles do not fit.
const COMPACT: u16 = 3;
/// Rows the table below keeps before the summary gives up its tiles.
const BODY_MINIMUM: u16 = 10;
const TILE_ROWS: u16 = 4;
/// Narrowest amount tile; below three of them the amounts take the one-line form.
const TILE_MINIMUM: u16 = 18;
pub(super) const DEFINITION: &str = "○ Saved = context the agents did not have to reread because messages were delivered together · at list price · not a bill";
const SCOPE: &str = "applies to this whole tab";
const BASIS: &str = "Estimate basis…";

/// The rows the summary takes from `available`.
pub(super) fn height(width: u16, available: u16) -> u16 {
    if available >= FULL + BODY_MINIMUM && width >= TILE_MINIMUM * 3 {
        FULL
    } else {
        COMPACT.min(available)
    }
}

/// Each amount carries its own coverage. Missing quotes never borrow token coverage.
pub(super) fn fields(
    summary: &SavingsSummary,
    window: DigestWindow,
    labels: [&str; 2],
) -> [String; 2] {
    let field = |unit, label, included| {
        format!(
            "{label} {} ({included}/{} agents{})",
            amount(summary, window, unit),
            summary.total,
            if included < summary.total {
                " · partial"
            } else {
                ""
            }
        )
    };
    [
        field(SavingsUnit::Tokens, labels[0], summary.included),
        field(SavingsUnit::Cost, labels[1], summary.cost.included),
    ]
}

/// One unit's amount without its coverage: a conflict names itself, missing is `—`.
fn amount(summary: &SavingsSummary, window: DigestWindow, unit: SavingsUnit) -> String {
    let (value, _, conflict) = crate::board::savings::savings_value(summary, window, unit);
    let value = conflict
        .map(str::to_owned)
        .or(value)
        .unwrap_or_else(|| "—".into());
    value.trim_end_matches(" at list price").to_owned()
}

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let rows = app.workspace_rows();
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    status_row(frame, app, Rect { height: 1, ..area });
    let look = app.look();
    let paint_row = |frame: &mut Frame, offset: u16, line: Line<'_>| {
        if offset < area.height {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(area.x, area.y + offset, area.width, 1),
                line,
            );
        }
    };
    let definition = Line::styled(DEFINITION, look.role(Role::Muted));
    if area.height < FULL {
        let summary = metrics::savings(app, &ids);
        let tok = format!(
            "TOK used {}",
            metrics::token_text(metrics::tokens(metrics::report(app), &ids))
        );
        let [saved, price] = fields(
            &summary,
            app.digest_window(),
            ["Saved", "≈ list price saved"],
        );
        paint_row(
            frame,
            1,
            Line::styled([tok, saved, price].join("  ·  "), look.role(Role::Text)),
        );
        paint_row(frame, 2, definition);
        return;
    }
    paint_row(
        frame,
        1,
        Line::styled("─".repeat(usize::from(area.width)), look.role(Role::Dim)),
    );
    tiles(
        frame,
        app,
        Rect::new(area.x, area.y + 2, area.width, TILE_ROWS),
        &ids,
    );
    paint_row(frame, FULL - 1, definition);
}

/// Supplier state and health on the left, the tab-wide window and the basis on the right.
fn status_row(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let window = format!("{} ▾", app.digest_window().label());
    let chip = |label: &str| label.width() as u16 + 2;
    let controls = "Window  ".width() as u16 + chip(&window) + 3 + chip(BASIS);
    // The scope note is the first thing a short row gives up.
    let scoped = controls + SCOPE.width() as u16 + 3 <= area.width / 2;
    let right = controls + if scoped { SCOPE.width() as u16 + 3 } else { 0 };
    let start = area.right().saturating_sub(right).max(area.x);
    let mut x = start;
    let text = |frame: &mut Frame, x: &mut u16, text: &str| {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(*x, area.y, area.right().saturating_sub(*x), 1),
            Line::styled(text.to_owned(), look.role(Role::Muted)),
        );
        *x = x.saturating_add(text.width() as u16);
    };
    let button = |frame: &mut Frame, x: &mut u16, label: &str, choice: Choice| {
        let rect = crate::board::buttons::paint(
            app,
            frame.buffer_mut(),
            Rect::new(*x, area.y, area.right().saturating_sub(*x), 1),
            &[Button::secondary(label)],
            Align::Left,
            None,
        )[0];
        if let Some(rect) = rect {
            app.workspace_strip_hits.borrow_mut().push((rect, choice));
            *x = rect.right();
        }
    };
    text(frame, &mut x, "Window  ");
    button(frame, &mut x, &window, Choice::DigestWindowMenu);
    x = x.saturating_add(3);
    if scoped {
        text(frame, &mut x, SCOPE);
        x = x.saturating_add(3);
    }
    button(frame, &mut x, BASIS, Choice::DigestBasis);

    let left = Rect::new(area.x, area.y, start.saturating_sub(area.x + 2), 1);
    let mut parts: Vec<(String, Role, Option<Choice>)> = Vec::new();
    // Health comes before the optional fields so a short row keeps it.
    if let Some(notice) = model::tick_notice(&app.labels) {
        parts.push((notice, Role::Waiting, None));
    }
    if app.labels.digest().is_none() {
        parts.push(("unavailable".into(), Role::Muted, None));
    }
    if app.loading() {
        parts.push(("Loading workspace…".into(), Role::Muted, None));
    }
    let usage = match app.usage_report.view_for(app.digest_range()) {
        View::Loading => "Usage loading…",
        View::Failed(_) => "Usage unavailable",
        View::Report(_) if app.usage_report.refresh_error(app.digest_range()).is_some() => {
            "Usage stale · refresh failed"
        }
        _ => "",
    };
    if !usage.is_empty() {
        parts.push((usage.into(), Role::Text, None));
    }
    if let Some((status, stale)) = app.labels.digest() {
        if stale {
            parts.push(("stale".into(), Role::Waiting, None));
        }
        if let Some(global) = status.global.as_ref() {
            parts.push((global.text.clone(), Role::Text, Some(Choice::DigestSwitch)));
        }
        parts.push((format!("{} held", super::held_text(app)), Role::Text, None));
        if let Some(tick) = status.tick {
            let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
            let at = model::observation_clock(tick.last_at_ms, crate::status::now_ms(), &zone);
            parts.push((format!("tick {at}"), Role::Muted, None));
        }
    }
    let lead = "Digest  ";
    let mut spans = vec![Span::styled(lead, strong(app))];
    let mut used = lead.width();
    let mut first = true;
    for (text, role, choice) in parts {
        let gap = if first { 0 } else { 3 };
        if used + gap + text.width() > usize::from(left.width) {
            continue;
        }
        if gap > 0 {
            spans.push(Span::styled(" · ", look.role(Role::Muted)));
        }
        if let Some(choice) = choice {
            app.workspace_strip_hits.borrow_mut().push((
                Rect::new(left.x + (used + gap) as u16, left.y, text.width() as u16, 1),
                choice,
            ));
        }
        used += gap + text.width();
        first = false;
        spans.push(Span::styled(text, look.role(role)));
    }
    strip::paint_left(frame.buffer_mut(), left, Line::from(spans));
}

/// Values read as bright bold text; states keep their mark and word, never a colour.
fn strong(app: &App) -> Style {
    app.look().role(Role::Text).add_modifier(Modifier::BOLD)
}

/// Three amount tiles, then the allowance tile when providers are sampled and it fits.
fn tiles(frame: &mut Frame, app: &App, area: Rect, ids: &[&str]) {
    let look = app.look();
    let allowance = super::allowance::tile_width(app, area.width.saturating_sub(TILE_MINIMUM * 3));
    let tile = area.width.saturating_sub(allowance) / 3;
    let reading = metrics::tokens(metrics::report(app), ids);
    let summary = metrics::savings(app, ids);
    let window = app.digest_window();
    let coverage = |included: usize| format!("{included} of {} agents", summary.total);
    let usage = match reading {
        Some((_, true)) => format!("{} agents · partial", ids.len()),
        Some(_) => format!("{} agents", ids.len()),
        None => "usage unavailable".into(),
    };
    let saved = amount(&summary, window, SavingsUnit::Tokens);
    let saved = if saved == "—" {
        saved
    } else {
        format!("{saved} context tokens")
    };
    let content = [
        ("TOK USED", metrics::token_text(reading), usage),
        ("SAVED · estimate", saved, coverage(summary.included)),
        (
            "≈ LIST PRICE SAVED",
            amount(&summary, window, SavingsUnit::Cost),
            coverage(summary.cost.included),
        ),
    ];
    for (index, (label, value, note)) in content.into_iter().enumerate() {
        let x = area.x + 2 + index as u16 * tile;
        let lines = [
            Line::styled(label, look.role(Role::Muted)),
            Line::styled(value, strong(app)),
            Line::styled(note, look.role(Role::Muted)),
        ];
        for (row, line) in lines.into_iter().enumerate() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(x, area.y + row as u16, tile.saturating_sub(2), 1),
                line,
            );
        }
    }
    if allowance > 0 {
        super::allowance::paint_tile(
            frame,
            app,
            Rect::new(area.right() - allowance, area.y, allowance, area.height),
        );
    }
}
