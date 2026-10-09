//! One home painter; ordinary pane geometry and scalar rows keep their owners.

use super::{
    Age, AgeSource,
    tiles::{self, TileItem},
};
use crate::{
    board::{
        app::{App, Hit, HomeHeaderUsage, View},
        view::{fit, scene::rule},
    },
    config::Pane,
    look::Look,
};
use ratatui::{Frame, layout::Rect, text::Line};
use std::ops::Range;
use tmt_cli_style::Role;

/// The strips outside the body, held in the view like the body's sections.
pub(crate) fn summary_of(view: &View, width: u16, look: Look) -> Line<'static> {
    let home = view.home.as_ref().expect("a home view has a model");
    super::bar::summary_in(
        &mut view.derived.borrow_mut().home.summary,
        home,
        width,
        look,
    )
}

pub(crate) fn usage_of(
    view: &View,
    usage: &HomeHeaderUsage<'_>,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    super::bar::usage_in(
        &mut view.derived.borrow_mut().home.usage,
        usage,
        width,
        look,
    )
}

pub(crate) fn hints_of(
    view: &View,
    width: usize,
    overflow: bool,
    receiving: bool,
    view_reply: bool,
) -> String {
    super::bar::hints_in(
        &mut view.derived.borrow_mut().home.hints,
        width,
        overflow,
        receiving,
        view_reply,
    )
}

/// The strips painted from nothing, for tests of the strips themselves.
#[cfg(test)]
pub(crate) use super::bar::{hints, summary, usage};

pub(crate) fn age_label(age: &Age, now: u64) -> String {
    let age_text = tmt_cli_style::value::relative_time(now.saturating_sub(age.since_ms));
    match age.source {
        AgeSource::Request => age_text,
        AgeSource::Observed => format!("observed {age_text}"),
    }
}

pub(crate) fn render(frame: &mut Frame, app: &App, area: Rect) {
    render_at(frame, app, area, crate::status::now_ms());
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
    let mut derived = view.derived.borrow_mut();
    let derived = &mut *derived;
    let (scenes, member_list) = (&mut derived.home, &mut derived.member_list);
    let mut starts = Vec::new();
    let mut regions = Vec::new();
    let mut input_range = None;
    let mut input_area = area;
    let mut selected_range = 0..0;
    let receipt_now = std::time::Instant::now();
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
                scenes.attention.entry("quiet").or_default(),
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
                scenes
                    .attention
                    .entry(if section == "needs-you" {
                        "needs-you"
                    } else {
                        "blocked"
                    })
                    .or_default(),
            );
            let base = lines.len();
            lines.extend(block.lines);
            for row in block.rows {
                let at = index + row.local;
                starts.push(base + row.start);
                regions.push((
                    at,
                    base + row.start
                        ..base + row.reserve.as_ref().map_or(row.end, |range| range.start),
                    0,
                    area.width,
                ));
                if at == app.selected {
                    selected_range = base + row.start..base + row.end;
                }
                if let Some(range) = row.reserve {
                    input_range = Some(base + range.start..base + range.end);
                }
            }
            continue;
        }
        if entry.target.section == super::LEADS {
            section = super::LEADS;
            let count = entries[index..]
                .iter()
                .take_while(|next| next.target.section == super::LEADS)
                .count();
            handled = index + count;
            let group = &entries[index..handled];
            let leads = group
                .iter()
                .map(|entry| {
                    app.home_leads
                        .leads
                        .iter()
                        .find(|lead| {
                            lead.squad == entry.target.squad
                                && entry.target.member.as_deref() == Some(lead.id())
                        })
                        .expect("a lead target retains its deferred projection")
                })
                .collect::<Vec<_>>();
            let block = super::leads::paint(
                app,
                look,
                area,
                now,
                &super::leads::Section {
                    first: index,
                    entries: group,
                    leads: &leads,
                },
                member_list,
            );
            let base = lines.len();
            lines.extend(block.lines);
            for lead in block.leads {
                let at = index + lead.local;
                starts.push(base + lead.start);
                regions.push((
                    at,
                    base + lead.hit.start..base + lead.hit.end,
                    1.min(area.width),
                    area.width.saturating_sub(2),
                ));
                if at == app.selected {
                    selected_range = base + lead.start..base + lead.end;
                }
                if let Some(range) = lead.reserve {
                    input_range = Some(base + range.start..base + range.end);
                    input_area = Rect {
                        x: area.x.saturating_add(1),
                        width: area.width.saturating_sub(2),
                        ..area
                    };
                }
            }
            continue;
        }
        if entry.target.section == super::ALL_LEADS || entry.target.section == super::CRON {
            let kind = if entry.target.section == super::CRON {
                super::rows::Kind::Cron
            } else {
                super::rows::Kind::AllLeads
            };
            section = &entry.target.section;
            let slot = match kind {
                super::rows::Kind::Cron => &mut scenes.cron,
                super::rows::Kind::AllLeads => &mut scenes.audience,
            };
            let block = super::rows::paint(app, look, area, kind, index, slot);
            let base = lines.len();
            lines.extend(block.lines);
            starts.push(base + block.row);
            regions.push((index, base + block.row..base + block.row + 1, 0, area.width));
            if index == app.selected {
                selected_range = base + block.row
                    ..base + block.row + 1 + block.reserve.as_ref().map_or(0, Range::len);
            }
            if let Some(range) = block.reserve {
                input_range = Some(base + range.start..base + range.end);
                input_area = area;
            }
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
            let painted = tiles::paint_in(&mut scenes.squads, &items, area.width, look, selected);
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
                    feedback.sent
                        && feedback.target
                            == crate::board::app::RowTarget::Home(
                                entries[app.selected].target.clone(),
                            )
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
    for (row, start) in starts.iter().enumerate() {
        let boxed = entries
            .get(row)
            .is_some_and(|entry| entry.target.section == super::LEADS);
        let width = area.width.saturating_sub(if boxed { 2 } else { 0 });
        crate::board::row_detail::more_hit(
            app,
            row,
            &app.detail_value(row, &[]),
            width,
            3,
            start + 1,
            area,
            offset,
            shown,
        );
    }

    for (row, range, x, width) in regions {
        for line in range.start.max(offset)..range.end.min(offset + shown) {
            if width > 0 {
                app.hits.borrow_mut().push(Hit {
                    y: area.y + (line - offset) as u16,
                    x: area.x + x,
                    width,
                    target: crate::board::app::HitTarget::Row(row),
                });
            }
        }
    }
    *app.row_starts.borrow_mut() = starts;
}
