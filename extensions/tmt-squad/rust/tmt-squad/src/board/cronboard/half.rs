//! The squad tab's jobs half: a rule line with the clock, then the squad's jobs
//! as an ordinary list below the members. Geometry comes from `composition`;
//! selection and scroll live in the pane's `ListState`.

use super::{
    line::{clock, first_line},
    rows::{Columns, key_of, project, row_id},
};
use crate::{board::app::App, board::view::fit, look::Look};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

/// Below this many body lines the half keeps only its rule line.
const MIN_BODY: u16 = 12;

/// The room of a squad tab; leads, home and user tabs carry none.
pub(in crate::board) fn room(app: &App) -> Option<&str> {
    app.view
        .as_ref()?
        .document
        .pointer("/squad/roomId")?
        .as_str()
}

/// Lines the half wants, or none when the tab has no jobs half.
pub(in crate::board) fn wanted(app: &App, body: Rect) -> Option<u16> {
    let room = room(app)?;
    if !app.cron_shown() {
        return None;
    }
    if body.height < MIN_BODY {
        return Some(1);
    }
    let jobs = app
        .cron
        .cron
        .as_ref()
        .map_or(0, |cron| cron.of_room(room).count());
    // The rule, one line per job (at least one for the empty note), and the
    // detail of the selected job while the half has focus: its wrapped
    // message (at most four lines) and the next-runs line.
    let detail = if app.jobs_focus && jobs > 0 {
        let message = app
            .cron
            .cron
            .iter()
            .flat_map(|cron| cron.of_room(room))
            .find(|view| Some(row_id(&key_of(view))) == app.jobs_selected())
            .map_or(1, |view| {
                tmt_tui::text::lines(
                    &crate::board::notes::sanitize(&view.job.message).replace('\n', " ↵ "),
                    body.width.saturating_sub(9).max(1),
                    tmt_tui::style::TextFlow::Wrap,
                )
                .len()
                .clamp(1, 4)
            });
        message + 1
    } else {
        0
    };
    let want = 1 + jobs.max(1) + detail;
    // Content height, at most two fifths of the body, never fewer than three lines.
    Some(want.min(usize::from(body.height) * 2 / 5).max(3) as u16)
}

fn rule(app: &App, width: usize, jobs: usize, look: Look, now_ms: i64) -> Line<'static> {
    let focused = app.jobs_focus;
    let title = if focused { Role::Accent } else { Role::Muted };
    let mut left = vec![
        Span::styled("── ", look.role(Role::Dim)),
        Span::styled(
            format!("cron · {jobs}"),
            if focused {
                look.role(title)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                look.role(title)
            },
        ),
    ];
    if let Some(failure) = &app.cron.failure {
        let note = if app.cron.cron.is_some() {
            " · ! stale".to_owned()
        } else {
            format!(" · ✗ {}", first_line(failure))
        };
        left.push(Span::styled(note, look.role(Role::Waiting)));
    }
    let right = app.cron.cron.as_ref().map(|cron| {
        let place = app.clock_place();
        let (text, role) = clock(&cron.clock, now_ms, false, app.cron.clock_note(place));
        (format!(" {text} "), role)
    });
    let used = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
    let mut spans = left;
    let mut room = width.saturating_sub(used(&spans) + 1);
    if let Some((text, role)) = right {
        let text = fit(&text, room.min(text.width()));
        room = room.saturating_sub(text.width());
        spans.push(Span::styled(" ", look.role(Role::Dim)));
        spans.push(Span::styled("─".repeat(room), look.role(Role::Dim)));
        spans.push(Span::styled(text, look.role(role)));
    } else {
        spans.push(Span::styled(" ", look.role(Role::Dim)));
        spans.push(Span::styled("─".repeat(room), look.role(Role::Dim)));
    }
    Line::from(spans)
}

/// Paints into `area` (the lower half) and records where its list was drawn.
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(room) = room(app) else { return };
    let look = app.look();
    let now = app.cron.now_ms();
    let jobs: Vec<_> = app
        .cron
        .cron
        .iter()
        .flat_map(|cron| cron.of_room(room))
        .collect();
    let line_area = Rect { height: 1, ..area };
    frame.buffer_mut().set_line(
        line_area.x,
        line_area.y,
        &rule(app, usize::from(area.width), jobs.len(), look, now),
        line_area.width,
    );
    app.jobs_area.set(Rect::default());
    if area.height < 2 {
        return;
    }
    let list_area = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    app.jobs_area.set(area);
    let mut panes = app.jobs.borrow_mut();
    let pane = panes.entry(room.to_owned()).or_default();
    // The selected job expands in place only while the half has focus.
    let selected = app
        .jobs_focus
        .then(|| pane.list.selected().map(str::to_owned))
        .flatten();
    let rows = project(&jobs, selected.as_deref(), now);
    let empty = if app.cron.cron.is_some() {
        "(no jobs · n new)"
    } else {
        "(jobs unavailable)"
    };
    pane.render(
        rows,
        Columns::for_width(false, area.width.saturating_sub(1)),
        empty,
        frame,
        look,
        list_area,
    );
}
