//! Decision presentation from the acquired row, without additional reads.
use serde_json::Value;
use tmt_tui::components::strip;

pub(in crate::board) fn oldest(row: &Value) -> Option<&Value> {
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
    editor_with_style(frame, app, area, app.look().role(tmt_cli_style::Role::Text));
}

fn editor_with_style(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    area: ratatui::layout::Rect,
    base: ratatui::style::Style,
) {
    use ratatui::style::Modifier;
    let Some(input) = app.input.as_ref() else {
        return;
    };
    if area.is_empty() {
        return;
    }
    let text = app.reference_editor().unwrap_or(&input.text);
    let layout = text.layout(area.width);
    let skip = layout.reveal(0, area.height);
    if let Some(caret) = layout.paint(
        frame.buffer_mut(),
        area,
        skip,
        text.selection(),
        base,
        base.add_modifier(Modifier::REVERSED),
    ) && input.chat_quote_focus.is_none()
        && !app.paste_preview_shown()
    {
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
            format!(
                "Enter send · Esc keep draft · {} · Ctrl-P paste",
                app.newline_hint()
            ),
            look.role(tmt_cli_style::Role::Muted),
        ),
    );
}

/// The conversation reserves this compact editor band.
pub(in crate::board) fn input_lines(
    app: &crate::board::app::App,
    area: ratatui::layout::Rect,
) -> usize {
    if app.composer_projection().is_some() {
        return usize::from(
            app.composer_rows(
                area.width
                    .saturating_sub(crate::board::input_field::GUTTER * 2),
            ),
        );
    }
    let Some(input) = &app.input else { return 0 };
    let demand = match input.compose {
        crate::board::app::Compose::Status => {
            super::super::status_update::lines(app, area.width.saturating_sub(4))
                .len()
                .saturating_add(3)
                .max(5)
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
    use ratatui::{layout::Rect, text::Line};
    use tmt_cli_style::Role;
    use tmt_tui::components::{Modal, Placement};
    app.composer.invalidate();
    if app.composer_projection().is_some() {
        final_input(frame, app, body);
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
        let mut header_lines = 1;
        if let Some(quote) = input.response_quotes.last() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    y: areas.content.y + header_lines,
                    height: 1,
                    ..areas.content
                },
                Line::styled(
                    super::fit(
                        &app.quote_caption(quote, usize::from(areas.content.width)),
                        usize::from(areas.content.width),
                    ),
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
    let mut hint = if matches!(input.compose, crate::board::app::Compose::Status) {
        "↑↓ field · Enter choose · PgUp/PgDn · Esc cancel".to_owned()
    } else {
        format!(
            "Enter send · Esc keep draft · {} · Ctrl-P paste",
            app.newline_hint()
        )
    };
    if !input.response_quotes.is_empty() {
        hint.push_str(" · Alt-⌫ remove");
    }
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

/// Project the active typed note without changing the draft or routing owner.
fn composer_caption<'a>(
    app: &'a crate::board::app::App,
    projection: &crate::board::composer::Projection<'a>,
) -> crate::board::input_field::Caption<'a> {
    if projection.focused
        && let Some(crate::board::app::Input {
            compose: crate::board::app::Compose::Annotate { to, .. },
            ..
        }) = app.input.as_ref()
    {
        crate::board::input_field::Caption::Note { to }
    } else {
        crate::board::input_field::Caption::Message(projection.recipient)
    }
}

/// Final input-field placement adapts the same editor, quotes and UUID draft admission.
fn final_input(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    body: ratatui::layout::Rect,
) {
    use crate::board::{composer::QuoteHit, input_field as field};
    use ratatui::{layout::Rect, text::Line};
    use tmt_cli_style::Role;
    let Some(projection) = app.composer_projection() else {
        return;
    };
    let Some(band) = app
        .input_band
        .get()
        .map(|area| area.intersection(body))
        .filter(|area| !area.is_empty())
    else {
        return;
    };
    if app.modal_open()
        && !app.paste_preview_shown()
        && (app.shell.tab != crate::board::shell::OpsTab::Agents || !projection.focused)
    {
        return;
    }
    let look = app.look();
    let base = field::style(look, projection.focused);
    let start = app.composer_start();
    let key = start
        .as_ref()
        .map_or("click", |(key, _)| super::footer::key_label(key));
    let field_rows = field::rows(projection.text, band.width);
    let exact_rows = u16::from(!projection.quotes.is_empty() && band.height > field_rows);
    let chat_rows =
        u16::from(!projection.chat_quotes.is_empty() && band.height > field_rows + exact_rows);
    let quote_rows = exact_rows + chat_rows;
    let field_band = Rect::new(
        band.x,
        band.y + quote_rows,
        band.width,
        band.height - quote_rows,
    );
    let content_area = field::paint(
        frame.buffer_mut(),
        field_band,
        composer_caption(app, &projection),
        key,
        projection.focused,
        projection
            .text
            .is_some_and(|text| !text.is_empty())
            .then(|| field::box_rows(projection.text, band.width)),
        look,
    );
    let (editor_area, overflow) = projection.text.map_or((content_area, None), |text| {
        field::viewport(content_area, text)
    });
    if projection.focused {
        editor_with_style(frame, app, editor_area, base);
    } else if let Some(text) = projection.text.filter(|text| !text.is_empty()) {
        if text.folds().is_empty()
            && super::super::member_links::paint_preview(
                frame,
                app,
                editor_area,
                text.as_str(),
                Role::Text,
            )
        {
            app.input_viewport.set(None);
            for y in editor_area.y..editor_area.bottom() {
                for x in editor_area.x..editor_area.right() {
                    frame.buffer_mut()[(x, y)].set_style(base);
                }
            }
        } else {
            let layout = text.layout(editor_area.width);
            let skip = layout.reveal(0, editor_area.height);
            layout.paint(frame.buffer_mut(), editor_area, skip, 0..0, base, base);
            app.input_viewport.set(Some((editor_area, skip)));
        }
    } else {
        app.input_viewport.set(Some((editor_area, 0)));
    }
    app.paint_paste_chips(frame, &projection, editor_area);
    let send = app
        .composer_info(&projection)
        .paint(frame.buffer_mut(), field_band, look);
    if projection.focused
        && app.in_flight == 0
        && let (Some(area), Some(key)) = (send, projection.key.clone())
    {
        *app.composer.send_hit.borrow_mut() = Some(crate::board::composer::SendHit { area, key });
    }
    if let Some((area, label)) = overflow {
        strip::paint_right(
            frame.buffer_mut(),
            area,
            Line::styled(label, field::muted_style(look, projection.focused)),
        );
    }
    if let Some(quote) = projection
        .quotes
        .last()
        .filter(|_| exact_rows > 0 && content_area.width >= 2)
    {
        let row = Rect::new(content_area.x, band.y, content_area.width - 2, 1);
        strip::paint_left(
            frame.buffer_mut(),
            row,
            Line::styled(
                app.quote_caption(quote, usize::from(row.width)),
                field::muted_style(look, projection.focused),
            ),
        );
        let remove = Rect::new(content_area.right() - 1, band.y, 1, 1);
        strip::paint_left(frame.buffer_mut(), remove, Line::styled("✕", base));
        if let Some(key) = projection.key.clone() {
            *app.composer.quote_hit.borrow_mut() = Some(QuoteHit {
                area: remove,
                key,
                source: quote.key.clone(),
                range: quote.range.clone(),
            });
        }
    }
    if chat_rows > 0 {
        paint_chat_quotes(
            frame,
            app,
            &projection,
            Rect::new(content_area.x, band.y + exact_rows, content_area.width, 1),
        );
    }
    app.hits.borrow_mut().retain(|hit| {
        Rect::new(hit.x, hit.y, hit.width, 1)
            .intersection(band)
            .is_empty()
    });
}

/// Each literal attachment is a bounded presentation chip; its retained text
/// remains intact for submission. Clipped chips never publish activation hits.
fn paint_chat_quotes(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    projection: &crate::board::composer::Projection<'_>,
    row: ratatui::layout::Rect,
) {
    use crate::board::{
        buttons::{self, Button},
        composer::ChatQuoteHit,
    };
    use tmt_cli_style::grid::Align;
    let Some(key) = &projection.key else { return };
    if row.is_empty() {
        return;
    }
    let slots = projection
        .chat_quotes
        .len()
        .min(usize::from(row.width) / 10)
        .max(1);
    let width = usize::from(row.width).saturating_sub(slots.saturating_sub(1)) / slots;
    let first = projection
        .chat_quote_focus
        .unwrap_or(0)
        .saturating_sub(slots - 1);
    for (position, (index, quote)) in projection
        .chat_quotes
        .iter()
        .enumerate()
        .skip(first)
        .take(slots)
        .enumerate()
    {
        let author = super::super::notes::single_line(&quote.author);
        let excerpt: String = quote.text.chars().take(120).collect();
        let excerpt = super::super::notes::single_line(&excerpt);
        let label = format!(
            "{} ×",
            super::fit(
                &format!("{author}: {excerpt}"),
                width.saturating_sub(4).min(38)
            )
            .trim_end()
        );
        let area = ratatui::layout::Rect::new(
            row.x + (position * (width + 1)) as u16,
            row.y,
            width as u16,
            1,
        );
        let hit = buttons::paint(
            app,
            frame.buffer_mut(),
            area,
            &[Button::secondary(&label)
                .focused(projection.focused && projection.chat_quote_focus == Some(index))],
            Align::Left,
            None,
        )[0];
        if app.button_owner_is_active(None)
            && let Some(area) = hit
        {
            app.composer
                .chat_quote_hits
                .borrow_mut()
                .push(ChatQuoteHit {
                    area,
                    key: key.clone(),
                    index,
                    quote: quote.clone(),
                });
        }
    }
}
