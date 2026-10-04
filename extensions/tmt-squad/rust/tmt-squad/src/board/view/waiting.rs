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
        &look.theme,
        look.depth,
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
            &look.theme,
            look.depth,
        );
    }
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(
            "Enter send · Esc cancel",
            look.role(tmt_cli_style::Role::Muted),
        ),
        &look.theme,
        look.depth,
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
    if input.row_send.as_ref()?.target != target {
        return None;
    }
    let demand = if matches!(input.compose, crate::board::app::Compose::Reply { .. }) {
        6
    } else {
        5
    };
    Some(demand.min(area.height.saturating_sub(2)).into())
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

/// The one input owner paints at the reserved row position, spanning the entire
/// body so a neighboring pane cannot show through its opaque chrome.
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
    let band = Rect {
        x: body.x,
        width: body.width,
        ..reserved
    };
    let look = app.look();
    let modal = Modal {
        title: String::new(),
        placement: Placement::Body,
    };
    let areas = modal.areas(band, [band.width, band.height], true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
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
        &look.theme,
        look.depth,
    );
    if matches!(input.compose, crate::board::app::Compose::Reply { .. }) && areas.content.height > 1
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
            &look.theme,
            look.depth,
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
        &look.theme,
        look.depth,
    );
    let hint = if input.alternative.is_some() {
        "Enter send · Esc cancel · Tab answer/note"
    } else {
        "Enter send · Esc cancel"
    };
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        Line::styled(hint, look.role(Role::Muted)),
        &look.theme,
        look.depth,
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
