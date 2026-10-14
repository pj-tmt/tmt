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

/// Shared viewport for every message composer. Paint and pointer use the same
/// editor projection; the current frame publishes only its clipped rectangle.
pub(in crate::board) fn editor(
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

/// The conversation reserves this compact editor band.
pub(in crate::board) fn input_lines(
    app: &crate::board::app::App,
    area: ratatui::layout::Rect,
) -> usize {
    if let Some(projection) = app.composer_projection() {
        return projection.text.map_or(1, |text| {
            text.layout(area.width.saturating_sub(15).max(1))
                .lines
                .len()
                .clamp(1, 6)
        }) + 3
            + usize::from(!projection.quotes.is_empty());
    }
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

/// Paint the reserved conversation editor band with opaque chrome and remove
/// the underlying hits before publishing editor geometry.
pub(in crate::board) fn inline_prompt(
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
    app.composer.invalidate();
    if let Some(projection) = app.composer_projection()
        && let Some(band) = app.input_band.get().map(|band| band.intersection(body))
    {
        if band.is_empty() {
            return;
        }
        let look = app.look();
        for y in band.y..band.bottom() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(band.x, y, band.width, 1),
                Line::styled(String::new(), look.role(Role::Text)),
            );
        }
        let border_role = if projection.focused {
            Role::Accent
        } else {
            Role::Dim
        };
        let content = if band.height >= 3 && band.width >= 3 {
            let outline = tmt_tui::components::Outline {
                title: Line::from(format!(" Message → {} ", projection.recipient)),
                border: look.role(border_role),
                title_style: look.role(border_role),
            };
            outline.paint(band, frame.buffer_mut());
            outline.inner(band)
        } else if band.width >= 3 {
            // Preserve one usable input row before reserving decorative chrome.
            for y in band.y..band.bottom() {
                for x in [band.x, band.right() - 1] {
                    strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(x, y, 1, 1),
                        Line::styled("│", look.role(border_role)),
                    );
                }
            }
            Rect::new(band.x + 1, band.y, band.width - 2, band.height)
        } else {
            band
        };
        let content = if content.width >= 5 {
            Rect::new(content.x + 1, content.y, content.width - 2, content.height)
        } else {
            content
        };
        let hint_rows = u16::from(content.height > 1 && band.height >= 4);
        let quote_rows = u16::from(!projection.quotes.is_empty() && content.height > hint_rows + 1);
        if quote_rows > 0 {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(content.x, content.y, content.width, 1),
                Line::styled(
                    format!(
                        "{} quote(s) · {} · Alt-Backspace removes",
                        projection.quotes.len(),
                        projection.quotes.last().unwrap().label()
                    ),
                    look.role(Role::Muted),
                ),
            );
        }
        let prefix = u16::from(content.width >= 3) * 2;
        let button_width = if content.width >= 14 { 9 } else { 0 };
        let editor_area = Rect::new(
            content.x + prefix,
            content.y + quote_rows,
            content
                .width
                .saturating_sub(prefix + button_width)
                .max(1)
                .min(content.width),
            content.height - hint_rows - quote_rows,
        );
        if prefix > 0 {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(content.x, editor_area.y, prefix, 1),
                Line::styled("› ", look.role(border_role)),
            );
        }
        if projection.focused {
            editor(frame, app, editor_area);
        } else if let Some(text) = projection.text.filter(|text| !text.is_empty()) {
            let layout = text.layout(editor_area.width);
            let skip = layout.reveal(0, editor_area.height);
            layout.paint(
                frame.buffer_mut(),
                editor_area,
                skip,
                0..0,
                look.role(Role::Text),
                look.role(Role::Text),
            );
            app.input_viewport.set(Some((editor_area, skip)));
        } else {
            app.input_viewport.set(Some((editor_area, 0)));
        }
        if projection.text.is_none_or(|text| text.is_empty()) {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(editor_area.x, editor_area.y, editor_area.width, 1),
                Line::styled("Write a message…", look.role(Role::Dim)),
            );
        }
        if button_width > 0 {
            let button = Rect::new(content.right() - 8, editor_area.y, 8, 1);
            strip::paint_left(
                frame.buffer_mut(),
                button,
                Line::styled(
                    "[ Send ]",
                    look.role(if app.in_flight == 0 {
                        Role::Accent
                    } else {
                        Role::Dim
                    }),
                ),
            );
            if app.in_flight == 0
                && let Some(key) = projection.key
            {
                *app.composer.send_hit.borrow_mut() =
                    Some(crate::board::composer::SendHit { area: button, key });
            }
        }
        if hint_rows > 0 {
            let hint = if projection.focused {
                "Enter send · Shift+Enter newline · Esc list".to_owned()
            } else {
                app.composer_focus_hint()
            };
            // Keep each hint phrase whole; never paint half of a key name.
            let mut fitted = String::new();
            for phrase in hint.split(" · ") {
                let next = if fitted.is_empty() {
                    phrase.to_owned()
                } else {
                    format!("{fitted} · {phrase}")
                };
                if unicode_width::UnicodeWidthStr::width(next.as_str()) > usize::from(content.width)
                {
                    break;
                }
                fitted = next;
            }
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(content.x, content.bottom() - 1, content.width, 1),
                Line::styled(fitted, look.role(Role::Muted)),
            );
        }
        app.hits.borrow_mut().retain(|hit| {
            Rect::new(hit.x, hit.y, hit.width, 1)
                .intersection(band)
                .is_empty()
        });
        return;
    }
    let (Some(input), Some(reserved)) = (&app.input, app.input_band.get()) else {
        return;
    };
    let inset = app.conversation_input_area().is_some()
        || app.effective_board().is_some_and(|board| board.members);
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
}
