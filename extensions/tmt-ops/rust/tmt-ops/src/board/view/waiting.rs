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

/// Shared viewport for every message composer. Paint and pointer use the same
/// editor projection; the current frame publishes only its clipped rectangle.
pub(super) fn editor(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    area: ratatui::layout::Rect,
) {
    use ratatui::style::Modifier;
    use tmt_cli_style::Role;
    let Some(input) = app.input.as_ref() else {
        return;
    };
    if area.is_empty() {
        return;
    }
    let text = app.reference_editor().unwrap_or(&input.text);
    let layout = text.layout(area.width);
    let skip = layout.reveal(0, area.height);
    let look = app.look();
    if let Some(caret) = layout.paint(
        frame.buffer_mut(),
        area,
        skip,
        text.selection(),
        look.role(Role::Text),
        look.role(Role::Text).add_modifier(Modifier::REVERSED),
    ) {
        frame.set_cursor_position(caret);
    }
    app.input_viewport.set(Some((area, skip)));
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
    editor(frame, app, content);
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(
            "Enter send · Esc keep draft · Ctrl-N newline · Ctrl-P paste",
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
    app.input.as_ref()?;
    if app.conversation_input_area().is_some() {
        return None;
    }
    let target = app.row_target(row)?;
    reserved_target_lines(app, &target, area)
}

pub(in crate::board) fn reserved_target_lines(
    app: &crate::board::app::App,
    target: &crate::board::app::RowTarget,
    area: ratatui::layout::Rect,
) -> Option<usize> {
    if app.input.as_ref()?.target()? != target {
        return None;
    }
    let demand = input_lines(app, area);
    let margin = if app.effective_board().is_some_and(|board| board.members)
        || matches!(target, crate::board::app::RowTarget::Home(target) if target.section == crate::board::home::LEADS)
    {
        3
    } else {
        2
    };
    Some(demand.min(usize::from(area.height.saturating_sub(margin))))
}

/// Both inline rows and the conversation reserve the same compact editor band.
pub(in crate::board) fn input_lines(
    app: &crate::board::app::App,
    area: ratatui::layout::Rect,
) -> usize {
    let Some(input) = &app.input else { return 0 };
    let demand = match input.compose {
        crate::board::app::Compose::Status => {
            super::super::status_update::lines(app, area.width.saturating_sub(4))
                .len()
                .saturating_add(3)
                .max(5)
        }
        crate::board::app::Compose::Reply { .. } => {
            5 + input
                .text
                .layout(area.width.saturating_sub(4))
                .lines
                .len()
                .clamp(1, 6)
        }
        _ => {
            4 + input
                .text
                .layout(area.width.saturating_sub(4))
                .lines
                .len()
                .clamp(1, 6)
        }
    };
    demand + usize::from(!input.response_quotes.is_empty())
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
    let inset = app.conversation_input_area().is_some()
        || app.effective_board().is_some_and(|board| board.members)
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
            if field == Some(2) && focus == 2 {
                let row = Rect {
                    y: content.y + index as u16,
                    height: 1,
                    ..content
                };
                strip::paint_left(
                    frame.buffer_mut(),
                    row,
                    Line::styled("› Reason: ", look.role(Role::Accent)),
                );
                editor(
                    frame,
                    app,
                    Rect {
                        x: row.x + 10.min(row.width),
                        width: row.width.saturating_sub(10),
                        ..row
                    },
                );
                continue;
            }
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
        let mut header_lines = if matches!(input.compose, crate::board::app::Compose::Reply { .. })
        {
            2
        } else {
            1
        };
        if let Some(quote) = input.response_quotes.last() {
            let hint = "Alt-Backspace removes";
            let source = super::fit(
                &format!(
                    "{} quote(s) · {}",
                    input.response_quotes.len(),
                    quote.label()
                ),
                usize::from(areas.content.width).saturating_sub(hint.len() + 3),
            );
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    y: areas.content.y + header_lines,
                    height: 1,
                    ..areas.content
                },
                Line::styled(
                    format!("{} · {hint}", source.trim_end()),
                    look.role(Role::Muted),
                ),
            );
            header_lines += 1;
        }
        editor(
            frame,
            app,
            Rect {
                y: areas.content.y + header_lines,
                height: (areas.content.height + areas.position.height).saturating_sub(header_lines),
                ..areas.content
            },
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
    } else if !input.others.is_empty() {
        format!(
            "Enter send · Esc keep draft · Ctrl-N newline · Ctrl-P paste · Tab {}",
            input.modes().join("/")
        )
    } else {
        "Enter send · Esc keep draft · Ctrl-N newline · Ctrl-P paste".into()
    };
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(hint, look.role(Role::Muted)),
    );
    let covered = |area: &Rect| !area.intersection(band).is_empty();
    app.hits
        .borrow_mut()
        .retain(|hit| !covered(&Rect::new(hit.x, hit.y, hit.width, 1)));
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
