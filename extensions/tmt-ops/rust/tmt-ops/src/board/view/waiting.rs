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
    if app.shell.tab == crate::board::shell::OpsTab::Overview
        && frame.area().width >= 140
        && app.composer_projection().is_some()
    {
        final_input(frame, app, body);
        return;
    }

    if let Some(projection) = app.composer_projection()
        && let Some(band) = app.input_band.get().map(|band| band.intersection(body))
    {
        if band.is_empty() {
            return;
        }
        let look = app.look();
        let blank = " ".repeat(usize::from(band.width));
        for y in band.y..band.bottom() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(band.x, y, band.width, 1),
                Line::styled(blank.as_str(), look.role(Role::Text)),
            );
        }
        // A blocking owner masks the passive band without reflowing the base.
        if projection.focused || !app.modal_open() {
            let border_role = if projection.focused {
                Role::Accent
            } else {
                Role::Dim
            };
            let content = if band.height >= 3 && band.width >= 3 {
                let reason = app
                    .composer_layout_reason()
                    .map_or_else(String::new, |reason| format!(" · {reason}"));
                let name = super::fit(
                    &crate::board::notes::sanitize(projection.recipient),
                    usize::from(band.width).saturating_sub(
                        14 + unicode_width::UnicodeWidthStr::width(reason.as_str()),
                    ),
                );
                let outline = tmt_tui::components::Outline {
                    title: Line::from(format!(" Message → {}{reason} ", name.trim_end())),
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
            let quote_rows =
                u16::from(!projection.quotes.is_empty() && content.height > hint_rows + 1);
            if quote_rows > 0 {
                let quote = projection.quotes.last().unwrap();
                let remove = if content.width >= 5 {
                    crate::board::buttons::paint(
                        app,
                        frame.buffer_mut(),
                        Rect::new(content.x + 2, content.y, content.width - 2, 1),
                        &[crate::board::buttons::Button::secondary("✕")
                            .enabled(projection.key.is_some())],
                        tmt_cli_style::grid::Align::Right,
                        None,
                    )[0]
                } else {
                    None
                };
                let label_width = remove.map_or(content.width, |remove| {
                    remove.x.saturating_sub(content.x).saturating_sub(1)
                });
                let label = app.quote_caption(quote, usize::from(label_width));
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect::new(content.x, content.y, label_width, 1),
                    Line::styled(
                        super::fit(&label, usize::from(label_width)),
                        look.role(Role::Muted),
                    ),
                );
                if let (Some(remove), Some(key)) = (remove, projection.key.clone()) {
                    *app.composer.quote_hit.borrow_mut() = Some(crate::board::composer::QuoteHit {
                        area: remove,
                        key,
                        source: quote.key.clone(),
                        range: quote.range.clone(),
                    });
                }
            }
            let prefix = u16::from(content.width >= 3) * 2;
            // Reserve three editor cells and a gap before fitting the whole Send chip.
            let reserved = prefix + 4;
            let button = if content.width > reserved {
                crate::board::buttons::paint(
                    app,
                    frame.buffer_mut(),
                    Rect::new(
                        content.x + reserved,
                        content.y + quote_rows,
                        content.width - reserved,
                        1,
                    ),
                    &[crate::board::buttons::Button::primary("Send")
                        .focused(projection.focused)
                        .enabled(app.in_flight == 0 && projection.key.is_some())],
                    tmt_cli_style::grid::Align::Right,
                    None,
                )[0]
            } else {
                None
            };
            let editor_width = button.map_or(content.width.saturating_sub(prefix), |button| {
                button
                    .x
                    .saturating_sub(content.x + prefix)
                    .saturating_sub(1)
            });
            let editor_area = Rect::new(
                content.x + prefix,
                content.y + quote_rows,
                editor_width.max(1).min(content.width),
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
                if super::super::member_links::paint_preview(
                    frame,
                    app,
                    editor_area,
                    text.as_str(),
                    Role::Link,
                ) {
                    // Display names have different widths from the exact raw editor.
                    // Clicking the preview focuses editing without fabricating a caret.
                    app.input_viewport.set(None);
                } else {
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
                }
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
            if let Some(button) = button
                && app.in_flight == 0
                && let Some(key) = projection.key
            {
                *app.composer.send_hit.borrow_mut() =
                    Some(crate::board::composer::SendHit { area: button, key });
            }
            if hint_rows > 0 {
                let mut hint = if projection.focused {
                    "Enter send · Shift+Enter newline · Esc list".to_owned()
                } else {
                    app.composer_focus_hint()
                };
                if !projection.quotes.is_empty() {
                    hint.push_str(" · Alt-⌫ remove");
                    if projection.focused
                        && unicode_width::UnicodeWidthStr::width(hint.as_str())
                            > usize::from(content.width)
                    {
                        hint = "Enter send · Esc list · Alt-⌫ remove · Shift+Enter newline".into();
                    }
                }
                // Keep each hint phrase whole; never paint half of a key name.
                let mut fitted = String::new();
                for phrase in hint.split(" · ") {
                    let next = if fitted.is_empty() {
                        phrase.to_owned()
                    } else {
                        format!("{fitted} · {phrase}")
                    };
                    if unicode_width::UnicodeWidthStr::width(next.as_str())
                        > usize::from(content.width)
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

/// Final M11 placement adapts the same editor, quotes and UUID draft admission.
fn final_input(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    body: ratatui::layout::Rect,
) {
    use crate::{
        action::Verb,
        board::{composer::QuoteHit, m11_input_field as field},
    };
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
    if app.modal_open() {
        return;
    }
    let look = app.look();
    let base = field::style(look, projection.focused);
    let start = app.composer_start();
    let key = start
        .as_ref()
        .map_or("click", |(key, _)| super::footer::key_label(key));
    let replying = start
        .as_ref()
        .is_none_or(|(_, action)| action.verb == Verb::Reply);
    let editor_area = field::paint(
        frame.buffer_mut(),
        band,
        projection.recipient,
        key,
        projection.focused,
        replying,
        look,
    );
    if projection.focused {
        editor_with_style(frame, app, editor_area, base);
    } else if let Some(text) = projection.text.filter(|text| !text.is_empty()) {
        if super::super::member_links::paint_preview(
            frame,
            app,
            editor_area,
            text.as_str(),
            Role::Text,
        ) {
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
    if projection.focused
        && editor_area.width > 0
        && projection.text.is_none_or(|text| text.is_empty())
    {
        strip::paint_left(
            frame.buffer_mut(),
            editor_area,
            Line::styled(
                format!(
                    "{} {}…",
                    if replying { "Reply to" } else { "Message" },
                    crate::board::notes::sanitize(projection.recipient)
                ),
                base.patch(look.role(Role::Dim)),
            ),
        );
    }
    if projection.focused
        && band.width >= 23
        && app.in_flight == 0
        && let Some(key) = projection.key.clone()
    {
        let hint_width = unicode_width::UnicodeWidthStr::width("⏎ send · Esc cancel") as u16;
        *app.composer.send_hit.borrow_mut() = Some(crate::board::composer::SendHit {
            area: Rect::new(band.right() - hint_width, band.y, 6, 1),
            key,
        });
    }
    if projection.quotes.is_empty()
        && band.height >= 3
        && let Some(question) = app.input.as_ref().and_then(|input| input.quote.as_deref())
    {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(band.x, band.bottom() - 1, band.width, 1),
            Line::styled(
                super::fit(
                    &format!("Reply: {}", crate::board::notes::single_line(question)),
                    usize::from(band.width),
                ),
                base.patch(look.role(Role::Dim)),
            ),
        );
    }
    if let Some(quote) = projection
        .quotes
        .last()
        .filter(|_| band.height >= 3 && band.width >= 4)
    {
        let row = Rect::new(band.x, band.bottom() - 1, band.width - 2, 1);
        strip::paint_left(
            frame.buffer_mut(),
            row,
            Line::styled(
                app.quote_caption(quote, usize::from(row.width)),
                base.patch(look.role(Role::Muted)),
            ),
        );
        let remove = Rect::new(band.right() - 1, band.bottom() - 1, 1, 1);
        strip::paint_left(frame.buffer_mut(), remove, Line::styled("✕", base));
        if let Some(key) = projection.key {
            *app.composer.quote_hit.borrow_mut() = Some(QuoteHit {
                area: remove,
                key,
                source: quote.key.clone(),
                range: quote.range.clone(),
            });
        }
    }
    app.hits.borrow_mut().retain(|hit| {
        Rect::new(hit.x, hit.y, hit.width, 1)
            .intersection(band)
            .is_empty()
    });
}
