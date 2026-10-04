//! Decision presentation from the acquired row, without additional reads.
use serde_json::Value;

pub(super) fn oldest(row: &Value) -> Option<&Value> {
    row["waitingOnYou"]
        .as_array()?
        .iter()
        .min_by_key(|request| request["preparedAtMs"].as_u64().unwrap_or(u64::MAX))
}

pub(super) fn text(row: &Value) -> Option<&str> {
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
        title: input.prompt.clone(),
        placement: Placement::Docked,
    };
    let demand = [body.width, 7];
    let areas = modal.areas(body, demand, true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    let lines = tmt_tui::text::lines(
        &format!("{}▏", input.text),
        areas.content.width,
        tmt_tui::style::TextFlow::Wrap,
    );
    let skip = lines
        .len()
        .saturating_sub(usize::from(areas.content.height));
    for (index, line) in lines.iter().skip(skip).enumerate() {
        crate::markup::paint_line(
            frame,
            Rect {
                y: areas.content.y + index as u16,
                height: 1,
                ..areas.content
            },
            Line::styled(line.as_str(), look.role(tmt_cli_style::Role::Text)),
            look,
        );
    }
    crate::markup::paint_line(
        frame,
        areas.footer,
        Line::styled(
            "Enter send · Esc cancel",
            look.role(tmt_cli_style::Role::Muted),
        ),
        look,
    );
}
