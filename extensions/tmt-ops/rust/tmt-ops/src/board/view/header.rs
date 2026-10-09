//! Summary, token meter and frame invalidation text.

use crate::board::app::App;
use crate::config::{BoardMode, Pane};
use crate::requests::age;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

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

/// Request and reply ages are clock-derived text. Hidden replies do not
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
            .chain(
                home.sections
                    .iter()
                    .flat_map(|section| &section.rows)
                    .filter_map(|row| crate::focus::label(&row.member, now)),
            )
            .chain(
                app.home_leads
                    .leads
                    .iter()
                    .filter_map(|lead| crate::focus::label(&lead.row, now)),
            )
            .chain(app.home_leads.leads.iter().filter_map(|lead| {
                lead.exchange
                    .as_ref()?
                    .since_ms
                    .map(|at| crate::requests::age(now, at))
            }))
            .collect();
    }
    let board = app.effective_board().expect("loaded view has a board");
    let visible = match board.mode {
        BoardMode::Split => {
            board.panes.contains(&Pane::Replies) && !app.collapsed_panes().contains(&Pane::Replies)
        }
        BoardMode::Tabs => app.focused() == Pane::Replies,
    };
    let mut marks: Vec<String> = app
        .rows()
        .into_iter()
        .filter_map(|(_, row)| super::waiting::age(row, now))
        .collect();
    marks.extend(
        app.rows()
            .into_iter()
            .filter_map(|(_, row)| crate::focus::label(row, now)),
    );
    if board.members {
        marks.extend(app.rows().into_iter().filter_map(|(_, row)| {
            row["staleness"]["unchangedSinceMs"]
                .as_u64()
                .filter(|at| *at <= now)
                .map(|at| tmt_cli_style::value::relative_time(now - at))
        }));
    }
    if visible {
        marks.extend(
            view.replies
                .iter()
                .filter_map(|reply| reply["submittedAtMs"].as_u64().map(|at| age(now, at))),
        );
    }
    marks
}

/// The second header line: the shown squad's summary or delayed loading indicator.
pub(super) fn summary_line(app: &App) -> Line<'_> {
    let look = app.look();
    if app.loading_since.is_some() {
        let target = app
            .current
            .as_deref()
            .map(crate::tabs::label)
            .unwrap_or("your squads");
        let frame = spinner_frame(app, std::time::Instant::now());
        return Line::from(vec![
            Span::styled(
                frame.map_or(" ", |frame| SPINNER[frame]),
                look.role(Role::Dim),
            ),
            Span::styled("  Opening ", look.role(Role::Muted)),
            Span::styled(tmt_cli_style::table::escape(target), look.role(Role::Text)),
        ]);
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
    let mut spans = vec![Span::styled(summary, look.role(Role::Text))];
    let waiting = app
        .current
        .as_ref()
        .and_then(|squad| app.attention.get(squad))
        .map_or(0, |attention| attention.waiting);
    if waiting > 0 {
        spans.push(Span::styled(
            format!(" · {waiting} waiting on you"),
            look.role(Role::Text),
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
            look.role(Role::Text),
        ));
    }
    Line::from(spans)
}

/// Both meter rows stay reserved while enabled, so gaining/losing coverage
/// does not move member rows or their hit identities.
pub(super) fn meter_enabled(app: &App) -> bool {
    !app.loading()
        && app
            .current
            .as_deref()
            .is_some_and(|tab| !crate::board::tabs::aggregate(tab))
        && app
            .meter
            .as_ref()
            .is_some_and(|meter| meter.settings.enabled)
}

pub(super) fn render_meter_status(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    if meter_enabled(app)
        && app
            .meter
            .as_ref()
            .is_some_and(|meter| meter.digits().is_none())
    {
        let text = if app.view.as_ref().is_some_and(|view| view.history_pending) {
            "Updating usage…"
        } else {
            "no usage reported yet"
        };
        let width = area.width.min(text.len() as u16);
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                x: area.right() - width,
                width,
                ..area
            },
            Line::styled(text, look.role(Role::Text)),
        );
    }
}

/// Reserve the current mode first; clip the lead/attention summary if needed.
/// The normal render/diff owns all terminal writes.
pub(super) fn meter_region(
    app: &App,
    summary: Rect,
) -> Option<(Rect, crate::board::meter::Layout)> {
    if !meter_enabled(app) {
        return None;
    }
    let left = summary_line(app).width() + 2;
    let meter = app.meter.as_ref()?;
    let layout = meter
        .layout(usize::from(summary.width).saturating_sub(left))
        .or_else(|| meter.layout(usize::from(summary.width)))?;
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
    let look = app.look();
    let meter = app.meter.as_ref().expect("visible meter");
    let hovered = app.meter_hover.is_some();
    let selected_bar = app.meter_hover.flatten().filter(|_| layout.spark);
    let digits = meter.digits();
    let readout_width = usize::from(area.width).saturating_sub(if layout.spark { 9 } else { 0 });
    let mut spans = if let Some(readout) =
        selected_bar.and_then(|bar| meter.bar_readout(bar, std::time::Instant::now()))
    {
        vec![Span::raw(readout)]
    } else {
        let mut spans = vec![
            Span::raw(digits.as_deref().unwrap_or("–")),
            Span::styled(layout.unit, look.role(Role::Muted)),
        ];
        if let Some(label) = layout.label {
            spans.push(Span::styled(format!(" {label}"), look.role(Role::Muted)));
        }
        spans
    };
    let readout = Line::from(spans.clone()).width();
    spans.insert(
        0,
        Span::raw(" ".repeat(readout_width.saturating_sub(readout))),
    );
    if layout.spark {
        spans.push(Span::raw(" "));
        for (index, bar) in meter.sparkline().chars().enumerate() {
            spans.push(Span::styled(
                bar.to_string(),
                look.role(if selected_bar == Some(index) {
                    Role::Text
                } else {
                    Role::Muted
                }),
            ));
        }
    }
    let mut line = Line::from(spans);
    if hovered {
        line.style = look.selection();
        for span in &mut line.spans {
            span.style = look.row_span(true, span.style, false);
        }
    }
    strip::paint_right(frame.buffer_mut(), area, line);
    let mut hits = app.hits.borrow_mut();
    hits.push(crate::board::app::Hit {
        y: area.y,
        x: area.x,
        width: area.width,
        target: crate::board::app::HitTarget::Meter(None),
    });
    if layout.spark {
        for index in 0..8 {
            hits.push(crate::board::app::Hit {
                y: area.y,
                x: area.right() - 8 + index as u16,
                width: 1,
                target: crate::board::app::HitTarget::Meter(Some(index)),
            });
        }
    }
}

/// Slice bars retain their own tones on the hover background after the general
/// selected-word pass; only the pointed slice changes to the text role.
pub(super) fn finish_meter_styles(frame: &mut Frame, app: &App, summary: Rect) {
    if app.meter_hover.is_none() || app.look().selection().bg.is_none() {
        return;
    }
    let Some((area, _)) = meter_region(app, summary).filter(|(_, layout)| layout.spark) else {
        return;
    };
    let look = app.look();
    for index in 0..8 {
        let role = if app.meter_hover.flatten() == Some(index) {
            Role::Text
        } else {
            Role::Muted
        };
        frame.buffer_mut()[(area.right() - 8 + index as u16, area.y)].set_style(look.role(role));
    }
}
