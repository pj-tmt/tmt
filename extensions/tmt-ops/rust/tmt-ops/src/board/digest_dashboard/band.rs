//! Reserved usage and impact rows over the dashboard's admitted, held evidence.
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
    text::{Line, Span},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

pub(super) const HEIGHT: u16 = 3;
pub(super) const DEFINITION: &str =
    "Gross context reread estimate · uncached input at list price · not billed savings.";

/// Each amount carries its own coverage. Missing quotes never borrow token coverage.
pub(super) fn fields(
    summary: &SavingsSummary,
    window: DigestWindow,
    labels: [&str; 2],
) -> [String; 2] {
    let field = |unit, label, included| {
        let (value, _, conflict) = crate::board::savings::savings_value(summary, window, unit);
        let value = conflict
            .map(str::to_owned)
            .or(value)
            .unwrap_or_else(|| "—".into());
        let value = value.trim_end_matches(" at list price");
        format!(
            "{label} {value} ({included}/{} agents{})",
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

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let rows = app.workspace_rows();
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    usage_row(frame, app, Rect { height: 1, ..area }, &ids);
    if area.height < 2 {
        return;
    }
    impact_row(
        frame,
        app,
        Rect::new(area.x, area.y + 1, area.width, 1),
        &ids,
    );
    if area.height >= HEIGHT {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y + 2, area.width, 1),
            Line::styled(DEFINITION, app.look().role(Role::Muted)),
        );
    }
}

fn usage_row(frame: &mut Frame, app: &App, area: Rect, ids: &[&str]) {
    let range = format!("Usage {}", app.tok_range().argument());
    let mut parts = vec![(
        range,
        Role::Text,
        Some(Choice::TokRange(app.tok_range().next())),
    )];
    parts.push((
        format!(
            "TOK {}",
            metrics::token_text(metrics::tokens(metrics::report(app), ids))
        ),
        Role::Text,
        None,
    ));
    // Reserve health before optional numeric/freshness fields, including in narrow frames.
    if let Some(notice) = model::tick_notice(&app.labels) {
        parts.push((notice, Role::Waiting, None));
    }
    if app.labels.digest().is_none() {
        parts.push(("Digest unavailable".into(), Role::Muted, None));
    }
    if app.loading() {
        parts.push(("Loading workspace…".into(), Role::Muted, None));
    }
    let usage = match app.usage_report.view_for(app.tok_range()) {
        View::Loading => "Usage loading…".into(),
        View::Failed(_) => "Usage unavailable".into(),
        View::Report(_) if app.usage_report.refresh_error(app.tok_range()).is_some() => {
            "Usage stale · refresh failed".into()
        }
        _ => String::new(),
    };
    if !usage.is_empty() {
        parts.push((usage, Role::Text, None));
    }
    if app.labels.digest().is_some_and(|(_, stale)| stale) {
        parts.push(("Digest stale".into(), Role::Waiting, None));
    }
    let switch = app
        .labels
        .digest()
        .and_then(|(status, _)| status.global.as_ref());
    if let Some(global) = switch {
        parts.push((global.text.clone(), Role::Text, Some(Choice::DigestSwitch)));
    }
    if app.labels.digest().is_some() {
        parts.push((
            format!("Held now: {}", super::held_text(app)),
            Role::Text,
            None,
        ));
    }
    if let Some((status, stale)) = app.labels.digest() {
        let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
        let time = |at| model::observation_clock(at, crate::status::now_ms(), &zone);
        parts.push((
            format!(
                "Status {}{}",
                time(Some(status.observed_at_ms)),
                if stale { " · stale" } else { "" }
            ),
            Role::Muted,
            None,
        ));
        if let Some(tick) = status.tick {
            parts.push((format!("tick {}", time(tick.last_at_ms)), Role::Muted, None));
        }
    }
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (text, role, choice) in parts {
        let gap = if used == 0 { 0 } else { 3 };
        if used + gap + text.width() > usize::from(area.width) {
            continue;
        }
        if gap > 0 {
            spans.push(Span::styled(" · ", app.look().role(Role::Muted)));
        }
        if let Some(choice) = choice {
            app.workspace_strip_hits.borrow_mut().push((
                Rect::new(area.x + (used + gap) as u16, area.y, text.width() as u16, 1),
                choice,
            ));
        }
        used += gap + text.width();
        spans.push(Span::styled(text, app.look().role(role)));
    }
    strip::paint_left(frame.buffer_mut(), area, Line::from(spans));
}

fn impact_row(frame: &mut Frame, app: &App, area: Rect, ids: &[&str]) {
    let label = "Impact ";
    strip::paint_left(
        frame.buffer_mut(),
        area,
        Line::styled(label, app.look().role(Role::Text)),
    );
    let window_label = format!("{} ▾", app.digest_window().label());
    let start = area
        .x
        .saturating_add(label.width() as u16)
        .min(area.right());
    let window = crate::board::buttons::paint(
        app,
        frame.buffer_mut(),
        Rect::new(start, area.y, area.right().saturating_sub(start), 1),
        &[Button::secondary(&window_label)],
        Align::Left,
        None,
    )[0];
    if let Some(rect) = window {
        app.workspace_strip_hits
            .borrow_mut()
            .push((rect, Choice::DigestWindowMenu));
    }
    let basis_start = window
        .map_or(area.right(), |rect| rect.right().saturating_add(2))
        .min(area.right());
    let basis = crate::board::buttons::paint(
        app,
        frame.buffer_mut(),
        Rect::new(
            basis_start,
            area.y,
            area.right().saturating_sub(basis_start),
            1,
        ),
        &[Button::secondary("Basis…")],
        Align::Right,
        None,
    )[0];
    if let Some(rect) = basis {
        app.workspace_strip_hits
            .borrow_mut()
            .push((rect, Choice::DigestBasis));
    }
    let mut used = window.map_or(start, |rect| rect.right().saturating_add(2));
    let end = basis.map_or(area.right(), |rect| rect.x.saturating_sub(2));
    let summary = metrics::savings(app, ids);
    for field in fields(
        &summary,
        app.digest_window(),
        ["Est. ctx avoided", "List-price equiv."],
    ) {
        if usize::from(end.saturating_sub(used)) < field.width() {
            break;
        }
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(used, area.y, field.width() as u16, 1),
            Line::styled(&field, app.look().role(Role::Text)),
        );
        used = used.saturating_add(field.width() as u16 + 2);
    }
}
