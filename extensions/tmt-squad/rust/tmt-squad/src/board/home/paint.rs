//! Flat home painting; ordinary pane geometry and scalar rows keep their owners.

use super::{Age, AgeSource, Counts, Home};
use crate::{
    board::{
        app::{App, Hit},
        view::fit,
    },
    config::Pane,
    look::Look,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::{Role, table::escape};

fn counts(counts: &Counts, look: Look, words: bool) -> Vec<Span<'static>> {
    [
        ("◆", counts.waiting, Role::Waiting, "waiting on you"),
        ("✗", counts.blocked, Role::Blocked, "blocked"),
        ("◐", counts.review, Role::Review, "review"),
        ("●", counts.working, Role::Working, "working"),
        ("○", counts.idle, Role::Dim, "idle"),
    ]
    .into_iter()
    .map(|(mark, count, role, label)| {
        let suffix = if words {
            format!(" {label}")
        } else {
            String::new()
        };
        Span::styled(
            format!("{mark} {count}{suffix}  "),
            look.role(if count == 0 { Role::Dim } else { role }),
        )
    })
    .collect()
}

pub(crate) fn summary(home: &Home, width: u16, look: Look) -> Line<'static> {
    let wide = width >= 120;
    let mut spans = vec![
        Span::styled("① ", look.role(Role::Dim)),
        Span::raw(if wide {
            format!(
                "{} squads · {} members   ",
                home.squads.len(),
                home.summary.members
            )
        } else {
            format!("{} squads  ", home.squads.len())
        }),
    ];
    spans.extend(counts(&home.summary, look, wide));
    Line::from(spans)
}

pub(crate) fn age_label(age: &Age, now: u64) -> String {
    let age_text = crate::requests::age(now, age.since_ms);
    match age.source {
        AgeSource::Request => age_text,
        AgeSource::Observed => format!("obs {age_text}"),
    }
}

pub(crate) fn hints(width: usize, cron: bool) -> String {
    // Drop complete optional hints, preserving the two exit/help hints at 80.
    let mut optional = vec![
        "↑↓ move",
        "tab section",
        "⏎ open",
        "a answer · note",
        "←→ tabs",
        "s switch",
        "/ search",
    ];
    if cron {
        optional.insert(4, "c cron");
    }
    loop {
        let text = optional
            .iter()
            .copied()
            .chain(["? more", "q quit"])
            .collect::<Vec<_>>()
            .join("  ");
        if unicode_width::UnicodeWidthStr::width(text.as_str()) <= width || optional.is_empty() {
            return text;
        }
        optional.pop();
    }
}

pub(crate) fn render(frame: &mut Frame, app: &App, area: Rect) {
    render_at(frame, app, area, crate::status::now_ms());
}

/// ⑤: the cron line is one selectable row; its text comes from the cron projection.
fn cron_line(app: &App, selected: bool, width: usize, now: u64) -> Line<'static> {
    let look = app.look();
    let line = crate::board::cronboard::home_line(&app.cron, now as i64, width as u16, look)
        .expect("a cron target exists only with a read or its failure");
    let mut spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|span| Span::styled(span.content, look.row_span(selected, span.style, false)))
        .collect();
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let mut line = Line::from(spans);
    line.style = if selected {
        look.selection().add_modifier(Modifier::BOLD)
    } else {
        look.role(Role::Text)
    };
    line
}

pub(super) fn render_at(frame: &mut Frame, app: &App, area: Rect, now: u64) {
    let view = app.view.as_ref().expect("home dispatcher has a view");
    let home = view.home.as_ref().expect("home dispatcher has a model");
    let entries = app.home_entries();
    let look = app.look();
    let width = usize::from(area.width);
    let mut lines = vec![Line::default()];
    if !home.failures.is_empty()
        || home.incomplete
        || view.me.is_none()
        || view.theme_notice.is_some()
    {
        let mut notices = view.theme_notice.iter().cloned().collect::<Vec<_>>();
        if !home.failures.is_empty() {
            notices.push(format!("partial: {} failed reads", home.failures.len()));
        }
        if home.incomplete {
            notices.push("older requests not shown".into());
        }
        if view.me.is_none() {
            notices.push(crate::status::UNKNOWN_YOU.into());
        }
        lines.push(Line::styled(
            fit(&notices.join(" · "), width),
            look.role(Role::Waiting),
        ));
    }
    let mut starts = Vec::new();
    let mut section = "";
    if !entries
        .iter()
        .any(|entry| entry.target.section == "needs-you")
    {
        let quiet = if view.me.is_none() {
            "user not set"
        } else if app.search.is_empty() {
            "nothing waits on you"
        } else {
            "no matching members"
        };
        lines.push(Line::styled(
            fit(&format!("── ② ◆ needs you · 0 · {quiet}"), width),
            look.role(Role::Dim),
        ));
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.target.section == super::CRON {
            section = super::CRON;
            lines.push(Line::default());
            starts.push(lines.len());
            lines.push(cron_line(app, index == app.selected, width, now));
            continue;
        }
        if section != entry.target.section {
            section = &entry.target.section;
            let count = entries
                .iter()
                .filter(|e| e.target.section == section)
                .count();
            let (label, role) = match section {
                "needs-you" => ("② ◆ needs you", Role::Waiting),
                "blocked" => ("② ✗ blocked", Role::Blocked),
                _ => ("③ squads", Role::Muted),
            };
            lines.push(Line::default());
            let title = format!("── {label} · {count} ");
            let tail = width.saturating_sub(unicode_width::UnicodeWidthStr::width(title.as_str()));
            lines.push(Line::styled(
                fit(&format!("{title}{}", "─".repeat(tail)), width),
                look.role(role),
            ));
        }
        starts.push(lines.len());
        let selected = index == app.selected;
        let name = escape(entry.row["name"].as_str().unwrap_or_default());
        let mut spans = Vec::new();
        let text_span = |text: String, role: Role, emphasize| {
            Span::styled(text, look.row_span(selected, look.role(role), emphasize))
        };
        if let Some(one) = entry.counts {
            let (badge, role) = if one.waiting > 0 {
                ("◆", Role::Waiting)
            } else if one.blocked > 0 {
                ("✗", Role::Blocked)
            } else {
                (" ", Role::Dim)
            };
            let name_width = (width / 5).min(22);
            let lead_width = (width / 5).min(24);
            spans.push(text_span(format!(" {badge} "), role, true));
            spans.push(text_span(fit(&name, name_width), Role::Accent, true));
            spans.push(text_span(
                fit(&escape(entry.lead.unwrap_or("no lead")), lead_width),
                Role::Text,
                false,
            ));
            let distribution = counts(one, look, false);
            let demand: usize = distribution.iter().map(Span::width).sum();
            let room = width.saturating_sub(3 + name_width + lead_width);
            if demand <= room {
                spans.extend(distribution.into_iter().map(|span| {
                    Span::styled(span.content, look.row_span(selected, span.style, true))
                }));
                let pressing = entry
                    .pressing
                    .filter(|_| one.waiting > 0 || one.blocked > 0 || one.review > 0)
                    .and_then(|row| row["name"].as_str())
                    .unwrap_or("");
                spans.push(text_span(
                    fit(&escape(pressing), room - demand),
                    Role::Muted,
                    false,
                ));
            } else {
                spans.push(text_span(
                    fit(&format!("{} members", one.members), room),
                    Role::Dim,
                    false,
                ));
            }
        } else {
            let waiting = entry.target.section == "needs-you";
            let age = entry.age.map(|age| age_label(age, now)).unwrap_or_default();
            let age_width = unicode_width::UnicodeWidthStr::width(age.as_str());
            let available = width.saturating_sub(4 + age_width);
            let name_width = (available / 2).min(24);
            spans.push(text_span(
                format!(" {} ", if waiting { "◆" } else { "✗" }),
                if waiting {
                    Role::Waiting
                } else {
                    Role::Blocked
                },
                true,
            ));
            spans.push(text_span(fit(&name, name_width), Role::Text, false));
            spans.push(text_span(
                fit(
                    &escape(&entry.target.squad),
                    available.saturating_sub(name_width),
                ),
                Role::Muted,
                false,
            ));
            spans.push(text_span(format!(" {age}"), Role::Dim, false));
        }
        let mut line = Line::from(spans);
        let padding = width.saturating_sub(line.width());
        line.spans.push(Span::raw(" ".repeat(padding)));
        line.style = if selected {
            look.selection().add_modifier(Modifier::BOLD)
        } else {
            look.role(Role::Text)
        };
        lines.push(line);
    }
    if home.squads.is_empty() {
        lines.push(Line::styled("③ squads · 0", look.role(Role::Dim)));
    }
    if app.follow
        && let Some(start) = starts.get(app.selected)
    {
        app.scrolls
            .reveal_range(Pane::Rows, *start..start + 1, area, lines.len());
    }
    let (offset, shown) = app
        .scrolls
        .show(frame, Pane::Rows, area, &lines, look.role(Role::Dim));
    for (row, start) in starts
        .iter()
        .enumerate()
        .filter(|(_, start)| **start >= offset && **start < offset + shown)
    {
        app.hits.borrow_mut().push(Hit {
            y: area.y + (*start - offset) as u16,
            x: area.x,
            width: area.width,
            row,
        });
    }
    *app.row_starts.borrow_mut() = starts;
}
