//! Summary, token meter and frame invalidation text.

use crate::board::app::App;
use crate::config::{BoardMode, Pane};
use crate::requests::age;
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use ratatui::{layout::Alignment, widgets::Paragraph};
use tmt_cli_style::Role;

/// Shown only when a switch takes long enough to notice.
pub(in crate::board) const SPINNER_DELAY: std::time::Duration =
    std::time::Duration::from_millis(100);
pub(super) const SPINNER_TICK: std::time::Duration = std::time::Duration::from_millis(80);
pub(super) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(in crate::board) fn spinner_frame(app: &App, now: std::time::Instant) -> Option<usize> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    (elapsed >= SPINNER_DELAY).then(|| {
        ((elapsed - SPINNER_DELAY).as_millis() / SPINNER_TICK.as_millis()) as usize % SPINNER.len()
    })
}

pub(in crate::board) fn spinner_wait(
    app: &App,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    Some(if elapsed < SPINNER_DELAY {
        SPINNER_DELAY - elapsed
    } else {
        SPINNER_TICK
            - std::time::Duration::from_millis(
                ((elapsed - SPINNER_DELAY).as_millis() % SPINNER_TICK.as_millis()) as u64,
            )
    })
}

/// Reply age is the board's only clock-derived text. Hidden replies do not
/// invalidate a frame, and minute/hour marks redraw only when their text changes.
pub(in crate::board) fn time_marks(app: &App, now: u64) -> Vec<String> {
    let Some(view) = &app.view else {
        return Vec::new();
    };
    if let Some(home) = &view.home {
        return home
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter_map(|row| row.age.as_ref())
            .map(|age| crate::board::home::age_label(age, now))
            .collect();
    }
    let board = app.effective_board().expect("loaded view has a board");
    let visible = match board.mode {
        BoardMode::Split => {
            board.panes.contains(&Pane::Replies) && !app.collapsed_panes().contains(&Pane::Replies)
        }
        BoardMode::Tabs => app.focused() == Pane::Replies,
    };
    if !visible {
        return Vec::new();
    }
    view.replies
        .iter()
        .filter_map(|reply| reply["submittedAtMs"].as_u64().map(|at| age(now, at)))
        .collect()
}

/// One pane tab (tabs mode), the same width selected or not: the selected
/// one is bracketed, the others padded.
/// The second header line: the shown squad's summary or delayed loading indicator.
pub(super) fn summary_line(app: &App) -> Line<'_> {
    let look = app.look();
    if let Some(frame) = spinner_frame(app, std::time::Instant::now()) {
        return Line::from(Span::styled(
            format!("{} loading", SPINNER[frame]),
            look.role(Role::Accent).add_modifier(Modifier::BOLD),
        ));
    }
    let Some(view) = app.view.as_ref().filter(|_| !app.loading()) else {
        return Line::default();
    };
    let lead = match view.document["squad"]["lead"]["name"].as_str() {
        Some(lead) => format!("lead {lead}"),
        None => "no lead".into(),
    };
    // A member can appear in several user sections; count people once.
    let count = view.document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["rows"].as_array().into_iter().flatten())
        .filter_map(|row| row["name"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    // The built-in tabs have one row per squad: a person leading two squads
    // is two rows on the leads tab.
    let rows = || {
        view.document["sections"][0]["rows"]
            .as_array()
            .map_or(0, Vec::len)
    };
    let summary = if app.current.as_deref() == Some(crate::board::LEADS) {
        plural(rows(), "squad lead", "squad leads")
    } else if app.current.as_deref() == Some(crate::board::ALL) {
        plural(rows(), "squad", "squads")
    } else if app
        .current
        .as_deref()
        .and_then(crate::board::tabs::user_name)
        .is_some()
    {
        plural(count, "member", "members")
    } else {
        format!("{lead} · {}", plural(count, "member", "members"))
    };
    let mut spans = vec![Span::styled(summary, look.role(Role::Muted))];
    let waiting = app
        .current
        .as_ref()
        .and_then(|squad| app.attention.get(squad))
        .map_or(0, |attention| attention.waiting);
    if waiting > 0 {
        spans.push(Span::styled(
            format!(" · {waiting} waiting on you"),
            look.named(&view.tab_colors.waiting),
        ));
    }
    if view.document["olderRequestsNotShown"] == true {
        spans.push(Span::styled(
            " · older requests not shown",
            look.role(Role::Dim),
        ));
    }
    if let Some(notice) = &view.theme_notice {
        spans.push(Span::styled(
            format!(" · {notice}"),
            look.role(Role::Waiting),
        ));
    }
    if view.document["partial"] == true {
        spans.push(Span::styled(
            " · partial: failed reads",
            look.role(Role::Waiting),
        ));
    }
    Line::from(spans)
}

/// Reserve a fixed band only when the complete lead/attention summary fits.
/// The normal ratatui render/diff owns all terminal writes.
pub(super) fn meter_region(
    app: &App,
    summary: Rect,
) -> Option<(Rect, crate::board::meter::Layout)> {
    if app.loading()
        || app
            .current
            .as_deref()
            .is_none_or(crate::board::tabs::aggregate)
    {
        return None;
    }
    let left = summary_line(app).width() + 2;
    let meter = app.meter.as_ref()?;
    let layout = meter.layout(usize::from(summary.width).saturating_sub(left))?;
    let area = Rect {
        x: summary.right() - layout.width as u16,
        width: layout.width as u16,
        ..summary
    };
    Some((area, layout))
}

pub(super) fn render_meter(frame: &mut Frame, app: &App, summary: Rect) {
    let Some((area, layout)) = meter_region(app, summary) else {
        return;
    };
    let meter = app.meter.as_ref().expect("visible meter");
    let mut spans = vec![Span::raw(meter.digits().expect("visible digits"))];
    spans.push(Span::styled(layout.unit, app.look().role(Role::Muted)));
    if let Some(label) = layout.label {
        spans.push(Span::styled(
            format!(" {label}"),
            app.look().role(Role::Muted),
        ));
    }
    if layout.spark {
        // Keep empty slices: the eight-slot trend grows from the right.
        spans.push(Span::styled(
            format!(" {}", meter.sparkline()),
            app.look().role(Role::Muted),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        area,
    );
}
