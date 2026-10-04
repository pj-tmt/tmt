//! One home painter; ordinary pane geometry and scalar rows keep their owners.

use super::{
    Age, AgeSource, Counts, Home,
    tiles::{self, TileItem},
};
use crate::{
    board::{
        app::{App, Hit, HomeHeaderUsage, UsageShare},
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
use tmt_cli_style::{
    Role,
    breakpoint::{LG, MD},
    table::escape,
};

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

/// Every section, including quiet and empty ones, reaches the same body edge.
pub(super) fn rule(title: &str, width: usize) -> String {
    let title = format!("── {title} ");
    let tail = width.saturating_sub(unicode_width::UnicodeWidthStr::width(title.as_str()));
    fit(&format!("{title}{}", "─".repeat(tail)), width)
}

pub(crate) fn summary(home: &Home, width: u16, look: Look) -> Line<'static> {
    let wide = width >= 120;
    let mut spans = vec![Span::raw(if wide {
        format!(
            "{} squads · {} members   ",
            home.squads.len(),
            home.summary.members
        )
    } else {
        format!("{} squads  ", home.squads.len())
    })];
    spans.extend(counts(&home.summary, look, wide));
    Line::from(spans)
}

fn percent(share: &UsageShare) -> String {
    format!(
        "{}{:.0}%",
        if share.partial { "~" } else { "" },
        share.fraction * 100.0
    )
}

/// Formats only the accepted meter projection; no acquisition or share arithmetic.
pub(crate) fn usage(usage: &HomeHeaderUsage<'_>, width: u16, look: Look) -> Option<Line<'static>> {
    if width < MD.cells || usage.totals.iter().all(Option::is_none) {
        return None;
    }
    let wide = width >= LG.cells;
    let width = usize::from(width);
    let windows = (usize::from(!wide)..3)
        .map(|index| {
            let number = usage.totals[index].map_or_else(
                || "–".into(),
                |reading| {
                    let number = crate::source::tokens(reading.tokens as f64);
                    format!("{}{number}", if reading.partial { "~" } else { "" })
                },
            );
            format!("{} {number}", usage.windows[index].label())
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let windows = format!("tok {windows}");
    let unreported = wide.then(|| {
        format!(
            "{} {} without data",
            usage.unreported,
            if usage.unreported == 1 {
                "member"
            } else {
                "members"
            }
        )
    });
    let reserved = unreported.as_ref().map_or(0, |text| text.len() + 3);
    let mut member_width = 18;
    let top = loop {
        let top = usage.top.as_ref().map_or_else(
            || format!("share {}: –", usage.windows[2].label()),
            |top| {
                format!(
                    "share {}: {} {}",
                    usage.windows[2].label(),
                    fit(&escape(top.member), member_width).trim_end(),
                    percent(&top.share)
                )
            },
        );
        let used = unicode_width::UnicodeWidthStr::width(windows.as_str())
            + unicode_width::UnicodeWidthStr::width(top.as_str())
            + 3
            + reserved
            + if wide {
                unicode_width::UnicodeWidthStr::width(" · models –")
            } else {
                0
            };
        if used <= width || member_width == 1 {
            break top;
        }
        member_width -= 1;
    };
    let mut spans = vec![
        Span::styled(windows, look.role(Role::Text)),
        Span::styled(" · ", look.role(Role::Dim)),
        Span::styled(
            top,
            look.role(if usage.top.is_some() {
                Role::Accent
            } else {
                Role::Dim
            }),
        ),
    ];
    if let Some(unreported) = unreported {
        let used: usize = spans.iter().map(Span::width).sum();
        let available = width.saturating_sub(used + reserved + 3);
        let mut models = String::from("models ");
        for model in &usage.models {
            let one = format!(
                "{} {}",
                fit(
                    &escape(model.model.map(crate::source::model_name).unwrap_or("–")),
                    12
                )
                .trim_end(),
                percent(&model.share)
            );
            let separator = if models == "models " { "" } else { ", " };
            let next = format!("{models}{separator}{one}");
            if unicode_width::UnicodeWidthStr::width(next.as_str()) > available {
                break;
            }
            models = next;
        }
        if models == "models " {
            models.push('–');
        }
        if unicode_width::UnicodeWidthStr::width(models.as_str()) <= available {
            spans.push(Span::styled(" · ", look.role(Role::Dim)));
            spans.push(Span::styled(models, look.role(Role::Text)));
        }
        spans.push(Span::styled(" · ", look.role(Role::Dim)));
        spans.push(Span::styled(unreported, look.role(Role::Dim)));
    }
    Some(Line::from(spans))
}

pub(crate) fn age_label(age: &Age, now: u64) -> String {
    let age_text = tmt_cli_style::value::relative_time(now.saturating_sub(age.since_ms));
    match age.source {
        AgeSource::Request => age_text,
        AgeSource::Observed => format!("observed {age_text}"),
    }
}

pub(crate) fn hints(width: usize, cron: bool) -> String {
    // Drop complete optional hints, preserving the two exit/help hints at 80.
    let mut optional = vec![
        "↑↓ move",
        "⏎ open",
        "a answer · note",
        if width < usize::from(MD.cells) {
            "A ask"
        } else {
            "A ask lead"
        },
        "e expand",
        "t replies",
        "/ search",
        "←→ tabs",
        "s switch",
    ];
    if cron {
        optional.push("c cron");
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

/// Home: the cron line is one selectable row; its text comes from the cron projection.
fn cron_line(app: &App, selected: bool, width: usize) -> Line<'static> {
    let look = app.look();
    let line = crate::board::cronboard::home_line(&app.cron, app.cron.now_ms(), width as u16, look)
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
    let lead_failures = app
        .home_leads
        .leads
        .iter()
        .filter(|lead| lead.failure.is_some())
        .map(|lead| lead.id())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        + usize::from(app.home_leads.failure.is_some());
    if !home.failures.is_empty()
        || home.incomplete
        || view.me.is_none()
        || view.theme_notice.is_some()
        || lead_failures > 0
        || app.home_leads.incomplete
    {
        let mut notices = view.theme_notice.iter().cloned().collect::<Vec<_>>();
        if !home.failures.is_empty() {
            notices.push(format!("partial: {} failed reads", home.failures.len()));
        }
        if home.incomplete {
            notices.push("older requests not shown".into());
        }
        if lead_failures > 0 {
            notices.push(format!(
                "lead exchanges partial: {lead_failures} failed reads"
            ));
        }
        if app.home_leads.incomplete {
            notices.push("older lead exchanges not shown".into());
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
    let mut regions = Vec::new();
    let mut input_range = None;
    let mut input_area = area;
    let mut selected_range = 0..0;
    let receipt_now = std::time::Instant::now();
    let mut section = "";
    let mut previous_lead_has_exchange = false;
    let chrome = super::leads::Chrome::new(area.width, look);
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
        lines.extend(
            super::attention::paint(
                app,
                look,
                area,
                now,
                &super::attention::Section {
                    first: 0,
                    entries: &[],
                    title: format!("◆ needs you · 0 · {quiet}"),
                    role: Role::Dim,
                    blank: false,
                },
            )
            .lines,
        );
    }
    let mut handled = 0;
    for (index, entry) in entries.iter().enumerate() {
        if index < handled {
            continue;
        }
        if matches!(entry.target.section.as_str(), "needs-you" | "blocked") {
            section = &entry.target.section;
            let count = entries[index..]
                .iter()
                .take_while(|next| next.target.section == section)
                .count();
            handled = index + count;
            let (label, role) = if section == "needs-you" {
                ("◆ needs you", Role::Waiting)
            } else {
                ("✗ blocked", Role::Blocked)
            };
            let block = super::attention::paint(
                app,
                look,
                area,
                now,
                &super::attention::Section {
                    first: index,
                    entries: &entries[index..handled],
                    title: format!("{label} · {count}"),
                    role,
                    blank: true,
                },
            );
            let base = lines.len();
            lines.extend(block.lines);
            for row in block.rows {
                starts.push(base + row.start);
                regions.push((
                    row.index,
                    base + row.start..base + row.start + 1,
                    0,
                    area.width,
                ));
                if row.index == app.selected {
                    selected_range = base + row.start..base + row.end;
                }
                if let Some(range) = row.reserve {
                    input_range = Some(base + range.start..base + range.end);
                }
            }
            continue;
        }
        if entry.target.section == super::LEADS {
            let lead = app
                .home_leads
                .leads
                .iter()
                .find(|lead| {
                    lead.squad == entry.target.squad
                        && entry.target.member.as_deref() == Some(lead.id())
                })
                .expect("a lead target retains its deferred projection");
            if section != super::LEADS {
                section = super::LEADS;
                lines.push(Line::default());
                lines.push(Line::styled(
                    rule(
                        &format!(
                            "leads · latest from each · t {} replies",
                            if view.home_replies { "hides" } else { "shows" }
                        ),
                        width,
                    ),
                    look.role(Role::Muted),
                ));
                lines.push(chrome.top.clone());
            } else if view.home_replies && previous_lead_has_exchange && lead.exchange.is_some() {
                lines.push(chrome.wrap(Line::default(), look, false));
            }
            let selected = index == app.selected;
            let start = lines.len();
            starts.push(start);
            lines.push(chrome.wrap(
                super::leads::heading(lead, area.width, look, selected, now),
                look,
                selected,
            ));
            let reading = app.input.as_ref().is_some_and(|input| {
                matches!(input.compose, crate::board::app::Compose::ReadLead { .. })
                    && input.row_send.as_ref().is_some_and(|send| {
                        send.target == crate::board::app::RowTarget::Home(entry.target.clone())
                    })
            });
            previous_lead_has_exchange = lead.exchange.is_some();
            if view.home_replies && lead.exchange.is_some() && !reading {
                lines.push(chrome.wrap(super::leads::preview(lead, area.width, look), look, false));
            }
            regions.push((
                index,
                start..lines.len(),
                1.min(area.width),
                area.width.saturating_sub(2),
            ));
            if app.sent.as_ref().is_some_and(|feedback| {
                feedback.target == crate::board::app::RowTarget::Home(entry.target.clone())
            }) {
                lines.push(chrome.wrap(
                    Line::styled("  ✓ sent", look.role(Role::Working)),
                    look,
                    false,
                ));
            }
            let inner = Rect {
                x: area.x.saturating_add(1),
                width: area.width.saturating_sub(2),
                ..area
            };
            if let Some(range) =
                crate::board::view::waiting::reserve_input(app, index, inner, &mut lines)
            {
                for line in &mut lines[range.clone()] {
                    *line = chrome.wrap(Line::default(), look, false);
                }
                input_range = Some(range);
                input_area = inner;
            }
            if selected {
                selected_range = start..lines.len();
            }
            if entries
                .get(index + 1)
                .is_none_or(|next| next.target.section != super::LEADS)
            {
                lines.push(chrome.bottom.clone());
            }
            continue;
        }
        if entry.target.section == super::ALL_LEADS {
            section = super::ALL_LEADS;
            let start = lines.len();
            starts.push(start);
            regions.push((index, start..start + 1, 0, area.width));
            let selected = index == app.selected;
            lines.push(Line::styled(
                fit("→ all leads  A to write · @ to pick", width),
                if selected {
                    look.selection().add_modifier(Modifier::BOLD)
                } else {
                    look.role(Role::Muted)
                },
            ));
            if let Some(range) =
                crate::board::view::waiting::reserve_input(app, index, area, &mut lines)
            {
                input_range = Some(range);
                input_area = area;
            }
            if selected {
                selected_range = start..lines.len();
            }
            continue;
        }
        if entry.target.section == super::CRON {
            section = super::CRON;
            lines.push(Line::default());
            starts.push(lines.len());
            regions.push((index, lines.len()..lines.len() + 1, 0, area.width));
            if index == app.selected {
                selected_range = lines.len()..lines.len() + 1;
            }
            lines.push(cron_line(app, index == app.selected, width));
            continue;
        }
        if section != entry.target.section {
            section = &entry.target.section;
            lines.push(Line::default());
            let items = entries[index..]
                .iter()
                .take_while(|entry| entry.target.section == "squads")
                .map(|entry| {
                    let squad = home
                        .squads
                        .iter()
                        .find(|squad| squad.squad == entry.target.squad)
                        .expect("home target retains its acquired squad");
                    TileItem {
                        squad,
                        members: &squad.members,
                        lead_model: app.home_lead_model(&squad.squad),
                        usage: app.home_usage(&squad.squad, receipt_now),
                    }
                })
                .collect::<Vec<_>>();
            let selected = app
                .selected
                .checked_sub(index)
                .filter(|local| *local < items.len());
            let painted = tiles::paint(&items, area.width, look, selected);
            lines.extend(painted.head);
            let start = lines.len();
            let selected_region =
                selected.and_then(|item| painted.regions.iter().find(|region| region.item == item));
            let boundary = selected_region.map(|region| region.lines.end);
            let mut tile_lines = painted.lines;
            let tail = boundary.map(|end| tile_lines.split_off(end));
            lines.extend(tile_lines);
            let before = lines.len();
            if selected_region.is_some() {
                if app.sent.as_ref().is_some_and(|feedback| {
                    feedback.target
                        == crate::board::app::RowTarget::Home(entries[app.selected].target.clone())
                }) {
                    lines.push(Line::styled("   ✓ sent", look.role(Role::Working)));
                }
                input_range =
                    crate::board::view::waiting::reserve_input(app, app.selected, area, &mut lines);
            }
            let inserted = lines.len() - before;
            if let Some(tail) = tail {
                lines.extend(tail);
            }
            for region in painted.regions {
                let shift = if boundary.is_some_and(|end| region.lines.start >= end) {
                    inserted
                } else {
                    0
                };
                let range = start + region.lines.start + shift..start + region.lines.end + shift;
                starts.push(range.start);
                if index + region.item == app.selected {
                    selected_range = range.start..range.end + inserted;
                }
                regions.push((index + region.item, range, region.x, region.width));
            }
        }
    }
    if home.squads.is_empty() {
        lines.push(Line::styled(
            rule("squads · 0", width),
            look.role(Role::Dim),
        ));
    }
    if app.follow && starts.get(app.selected).is_some() {
        app.scrolls
            .reveal_range(Pane::Rows, selected_range, area, lines.len());
    }
    let (offset, shown) = app.scrolls.show(frame, Pane::Rows, area, &lines, look);
    crate::board::view::waiting::place_input(app, input_range, input_area, offset, shown);
    for (row, range, x, width) in regions {
        for line in range.start.max(offset)..range.end.min(offset + shown) {
            if width > 0 {
                app.hits.borrow_mut().push(Hit {
                    y: area.y + (line - offset) as u16,
                    x: area.x + x,
                    width,
                    row,
                });
            }
        }
    }
    *app.row_starts.borrow_mut() = starts;
}
