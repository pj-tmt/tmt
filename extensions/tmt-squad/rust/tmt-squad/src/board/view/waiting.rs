//! Decision presentation from the acquired row, without additional reads.
use serde_json::Value;
use tmt_tui::components::strip;

pub(super) fn oldest(row: &Value) -> Option<&Value> {
    row["waitingOnYou"]
        .as_array()?
        .iter()
        .min_by_key(|request| request["preparedAtMs"].as_u64().unwrap_or(u64::MAX))
}

pub(in crate::board) fn text(row: &Value) -> Option<&str> {
    row["pending"]
        .as_str()
        .or_else(|| oldest(row)?["preview"].as_str())
}

pub(super) fn age(row: &Value, now: u64) -> Option<String> {
    let since = oldest(row)?["preparedAtMs"].as_u64()?;
    (since > 0 && since <= now).then(|| crate::requests::age(now, since))
}

/// Ask-lead uses the shared opaque docked prompt chrome.
pub(super) fn prompt(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    body: ratatui::layout::Rect,
) {
    use ratatui::{layout::Rect, text::Line};
    use tmt_tui::components::{Modal, Placement};
    let Some(input) = app
        .input
        .as_ref()
        .filter(|input| matches!(input.compose, crate::board::app::Compose::AskLead { .. }))
    else {
        return;
    };
    let look = app.look();
    let modal = Modal {
        title: String::new(),
        placement: Placement::Docked,
    };
    let demand = [body.width, 7];
    let docked = modal.areas(body, demand, true, false);
    // Cover the entire underlying pane band, including the docked margins.
    let band = Rect {
        y: docked.outer.y,
        height: docked.outer.height,
        ..body
    };
    let modal = Modal {
        placement: Placement::Body,
        ..modal
    };
    let areas = modal.areas(band, demand, true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    strip::paint_left(
        frame.buffer_mut(),
        Rect {
            height: 1,
            ..areas.content
        },
        Line::styled(
            input.header(),
            look.role(tmt_cli_style::Role::Accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
    );
    let content = Rect {
        y: areas.content.y + 1,
        height: areas.content.height.saturating_sub(1) + areas.position.height,
        ..areas.content
    };
    let lines = tmt_tui::text::lines(
        &format!("{}▏", input.text),
        content.width,
        tmt_tui::style::TextFlow::Wrap,
    );
    let skip = lines.len().saturating_sub(usize::from(content.height));
    for (index, line) in lines.iter().skip(skip).enumerate() {
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                y: content.y + index as u16,
                height: 1,
                ..content
            },
            Line::styled(line.as_str(), look.role(tmt_cli_style::Role::Text)),
        );
    }
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(
            "Enter send · Esc cancel",
            look.role(tmt_cli_style::Role::Muted),
        ),
    );
}

/// Lines the inline input reserves under `row`, or `None` when it is not anchored
/// there. The reservation joins the row stream, so placement derives from that same
/// stream after scroll reveal, never from a previous frame's hit map.
pub(in crate::board) fn reserved_lines(
    app: &crate::board::app::App,
    row: usize,
    area: ratatui::layout::Rect,
) -> Option<usize> {
    let input = app.input.as_ref()?;
    let target = app.row_target(row)?;
    if input.target()? != &target {
        return None;
    }
    let demand = match input.compose {
        crate::board::app::Compose::ReadLead { .. }
        | crate::board::app::Compose::ReadRow { .. } => {
            let lines = read_lines(app, area.width.saturating_sub(4));
            lines.len().saturating_add(3).max(5)
        }
        crate::board::app::Compose::Status => {
            super::super::status_update::lines(app, area.width.saturating_sub(4))
                .len()
                .saturating_add(3)
                .max(5)
        }
        crate::board::app::Compose::Reply { .. } => 6,
        _ => 5,
    };
    let margin = if app.effective_board().is_some_and(|board| board.members)
        || matches!(&target, crate::board::app::RowTarget::Home(target) if target.section == crate::board::home::LEADS)
    {
        3
    } else {
        2
    };
    Some(demand.min(usize::from(area.height.saturating_sub(margin))))
}

/// Reserve visual lines in a line stream (the home painter's).
pub(in crate::board) fn reserve_input(
    app: &crate::board::app::App,
    row: usize,
    area: ratatui::layout::Rect,
    lines: &mut Vec<ratatui::text::Line<'static>>,
) -> Option<std::ops::Range<usize>> {
    let height = reserved_lines(app, row, area)?;
    let start = lines.len();
    lines.extend((0..height).map(|_| ratatui::text::Line::default()));
    Some(start..lines.len())
}

pub(in crate::board) fn place_input(
    app: &crate::board::app::App,
    range: Option<std::ops::Range<usize>>,
    area: ratatui::layout::Rect,
    offset: usize,
    viewport: usize,
) {
    if let Some(range) = range.filter(|range| {
        !range.is_empty() && range.start >= offset && range.end <= offset + viewport
    }) {
        app.input_band.set(Some(ratatui::layout::Rect {
            y: area.y + (range.start - offset) as u16,
            height: range.len() as u16,
            ..area
        }));
    }
}

/// One band owner paints at the reserved row position. Member bands cover the
/// full body; boxed HOME lead bands cover its full inner width. Both modes share
/// opaque chrome and remove the underlying hits.
pub(super) fn inline_prompt(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    body: ratatui::layout::Rect,
) {
    use ratatui::{
        layout::Rect,
        text::{Line, Span},
    };
    use tmt_cli_style::Role;
    use tmt_tui::components::{Modal, Placement};
    let (Some(input), Some(reserved)) = (&app.input, app.input_band.get()) else {
        return;
    };
    let inset = app.effective_board().is_some_and(|board| board.members)
        || input.row_send.as_ref().is_some_and(|send| {
            matches!(&send.target,
        crate::board::app::RowTarget::Home(target) if target.section == crate::board::home::LEADS)
        });
    let band = if inset {
        reserved.intersection(body)
    } else {
        Rect {
            x: body.x,
            width: body.width,
            ..reserved
        }
    };
    let look = app.look();
    let modal = Modal {
        title: String::new(),
        placement: Placement::Body,
    };
    let areas = modal.areas(band, [band.width, band.height], true, false);
    modal.paint_flat(areas, frame.buffer_mut(), &look.theme, look.depth);
    if matches!(input.compose, crate::board::app::Compose::Status) {
        let content = Rect {
            height: areas.content.height + areas.position.height,
            ..areas.content
        };
        let lines = super::super::status_update::lines(app, content.width);
        let focus = app.status_draft.as_ref().map_or(0, |draft| draft.focus);
        let selected = lines
            .iter()
            .rposition(|(field, _)| *field == Some(focus))
            .unwrap_or(0);
        let skip = app
            .status_draft
            .as_ref()
            .and_then(|draft| draft.scroll)
            .unwrap_or_else(|| {
                selected.saturating_sub(usize::from(content.height).saturating_sub(1))
            })
            .min(lines.len().saturating_sub(usize::from(content.height)));
        for (index, (field, text)) in lines
            .into_iter()
            .skip(skip)
            .take(usize::from(content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    y: content.y + index as u16,
                    height: 1,
                    ..content
                },
                Line::styled(
                    text,
                    look.role(if field == Some(focus) {
                        Role::Accent
                    } else {
                        Role::Text
                    }),
                ),
            );
        }
    } else if let crate::board::app::Compose::ReadLead { offset, .. }
    | crate::board::app::Compose::ReadRow { offset, .. } = input.compose
    {
        let content = Rect {
            height: areas.content.height + areas.position.height,
            ..areas.content
        };
        let lines = read_lines(app, content.width);
        let skip = offset.min(lines.len().saturating_sub(usize::from(content.height)));
        for (index, line) in lines
            .iter()
            .skip(skip)
            .take(usize::from(content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    y: content.y + index as u16,
                    height: 1,
                    ..content
                },
                line.clone(),
            );
        }
    } else {
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                height: 1,
                ..areas.content
            },
            Line::styled(
                super::fit(&input.header(), usize::from(areas.content.width)),
                look.role(Role::Accent)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
        );
        if matches!(input.compose, crate::board::app::Compose::Reply { .. })
            && areas.content.height > 1
        {
            let quote = format!(
                "“{}”",
                input.quote.as_deref().unwrap_or("question unavailable")
            );
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    y: areas.content.y + 1,
                    height: 1,
                    ..areas.content
                },
                Line::from(vec![
                    Span::styled(
                        "◆ ",
                        look.role(Role::Waiting)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    ),
                    Span::styled(
                        super::fit(&quote, usize::from(areas.content.width.saturating_sub(2))),
                        look.role(Role::Muted),
                    ),
                ]),
            );
        }
        // Fit the tail so the cursor remains visible even for a long draft.
        let text = tmt_tui::text::lines(
            &format!("{}▏", input.text),
            areas.position.width,
            tmt_tui::style::TextFlow::Wrap,
        );
        strip::paint_left(
            frame.buffer_mut(),
            areas.position,
            Line::styled(
                text.last().map(String::as_str).unwrap_or("▏"),
                look.role(Role::Text),
            ),
        );
    }
    let hint = if matches!(input.compose, crate::board::app::Compose::Status) {
        format!(
            "↑↓ field · Enter choose · PgUp/PgDn · Esc cancel · Tab {}",
            if areas.footer.width >= tmt_cli_style::breakpoint::LG.cells {
                input.modes().join("/")
            } else {
                "mode".into()
            }
        )
    } else if matches!(input.compose, crate::board::app::Compose::ReadRow { .. }) {
        read_hints(app, areas.footer.width)
    } else if matches!(input.compose, crate::board::app::Compose::ReadLead { .. }) {
        format!(
            "e collapse · a reply to {}",
            input
                .row_send
                .as_ref()
                .map_or("lead", |send| send.name.as_str())
        )
    } else if !input.others.is_empty() {
        format!("Enter send · Esc cancel · Tab {}", input.modes().join("/"))
    } else {
        "Enter send · Esc cancel".into()
    };
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(hint, look.role(Role::Muted)),
    );
    let covered = |area: &Rect| !area.intersection(band).is_empty();
    app.hits
        .borrow_mut()
        .retain(|hit| !(band.y..band.bottom()).contains(&hit.y));
    app.note_hits
        .borrow_mut()
        .retain(|(area, _)| !covered(area));
    app.link_hits
        .borrow_mut()
        .retain(|(area, _)| !covered(area));
    app.title_hits
        .borrow_mut()
        .retain(|hit| !covered(&hit.area));
}

/// One read projection supplies reservation, painting and scroll limits for both
/// HOME messages and squad-row details. No terminal or core read occurs here.
pub(in crate::board) fn read_lines(
    app: &crate::board::app::App,
    width: u16,
) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::text::Line;
    use tmt_cli_style::Role;
    let Some(input) = &app.input else {
        return Vec::new();
    };
    let look = app.look();
    if matches!(input.compose, crate::board::app::Compose::ReadLead { .. }) {
        return crate::board::home_leads::message_lines(&input.text, width)
            .into_iter()
            .map(|line| Line::styled(line, look.role(Role::Text)))
            .collect();
    }
    let Some(row) = app.selected_row() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mut add = |text: String, role| {
        lines.extend(
            crate::board::home_leads::message_lines(&crate::board::notes::sanitize(&text), width)
                .into_iter()
                .map(|line| Line::styled(line, look.role(role))),
        );
    };
    if let Some(waiting) = text(row).filter(|text| !text.is_empty()) {
        add(format!("◆ waits on you: {waiting}"), Role::Waiting);
    }
    if let Some(task) = row["fields"]["task"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        add(format!("task  {task}"), Role::Text);
    }
    let links = row["fields"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| name.as_str() == "link" || name.ends_with("_link"))
        .filter_map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !links.is_empty() {
        add(format!("links  {}", links.join("  ")), Role::Link);
    }
    if let Some(reply) = app.latest_row_reply() {
        let age = reply["submittedAtMs"]
            .as_u64()
            .filter(|at| *at > 0 && *at <= crate::status::now_ms())
            .map(|at| crate::requests::age(crate::status::now_ms(), at));
        add(
            format!(
                "latest reply{}",
                age.map_or(String::new(), |age| format!(" · {age}"))
            ),
            Role::Muted,
        );
        add(
            reply["response"].as_str().unwrap_or(&input.text).to_owned(),
            Role::Text,
        );
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "(no row fields or replies yet)",
            look.role(Role::Dim),
        ));
    }
    lines
}

fn read_hints(app: &crate::board::app::App, width: u16) -> String {
    use crate::action::Verb;
    use unicode_width::UnicodeWidthStr;
    let bindings = app.bindings();
    let key = |verb| {
        bindings
            .iter()
            .find(|(event, action)| {
                action.verb == verb && !matches!(event.as_str(), "click" | "double-click")
            })
            .map(|(event, _)| event.as_str())
    };
    let mut hints = vec![
        key(Verb::HomeMessage)
            .map_or_else(|| "Esc collapse".into(), |key| format!("{key} collapse")),
    ];
    if app.view.as_ref().is_some_and(|view| view.me.is_some())
        && let Some(key) = key(Verb::Annotate)
    {
        hints.push(format!("{key} write"));
    }
    if app
        .selected_row()
        .is_some_and(|row| crate::effects::default_link(row).is_some())
        && let Some(key) = key(Verb::Open)
    {
        let pr = app.selected_row().is_some_and(|row| {
            row["fields"]["pr_link"]
                .as_str()
                .is_some_and(|link| !link.is_empty())
        });
        hints.push(format!("{key} open {}", if pr { "PR" } else { "link" }));
    }
    while hints.join(" · ").width() > usize::from(width) && hints.len() > 1 {
        hints.pop();
    }
    hints.join(" · ")
}
