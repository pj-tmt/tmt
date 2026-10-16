//! Places the panel's rows. Hits for the window chips go where the other strip
//! hits go, so a click is a `Choice` like every other.

use super::{Since, panel::Row};
use crate::board::{
    app::{App, Choice as Chosen},
    chip_group::{self, ChipGroup, Choice},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use std::sync::OnceLock;
use tmt_cli_style::Role;
use tmt_tui::components::strip;

/// The zone clock times are shown in, read once.
pub fn zone() -> &'static jiff::tz::TimeZone {
    static ZONE: OnceLock<jiff::tz::TimeZone> = OnceLock::new();
    ZONE.get_or_init(jiff::tz::TimeZone::system)
}

fn chips(since: Since) -> Vec<ChipGroup> {
    vec![ChipGroup {
        label: String::new(),
        choices: Since::ALL
            .into_iter()
            .map(|option| Choice {
                id: option.text().into(),
                actionable: true,
                text: format!(" {} ", option.text()),
                gap: " ".into(),
                selected: option == since,
                disabled: false,
                cursor: false,
            })
            .collect(),
    }]
}

/// Paints `rows` into `area`; when they do not all fit, the last line says how
/// many are left out.
pub fn paint(buffer: &mut Buffer, app: &App, area: Rect, rows: &[Row]) {
    let look = app.look();
    let room = usize::from(area.height);
    let clipped = rows.len() > room && room > 1;
    let shown = if clipped {
        room - 1
    } else {
        rows.len().min(room)
    };
    for (offset, row) in rows.iter().take(shown).enumerate() {
        let rect = Rect::new(area.x, area.y + offset as u16, area.width, 1);
        match row {
            Row::Rule(text) => strip::paint_left(
                buffer,
                rect,
                Line::styled(text.clone(), look.role(Role::Muted)),
            ),
            Row::Chips(since) => {
                for hit in chip_group::paint(buffer, rect, &chips(*since), app, None) {
                    if let Some(option) = Since::ALL.into_iter().find(|s| s.text() == hit.id) {
                        app.workspace_strip_hits
                            .borrow_mut()
                            .push((hit.area, Chosen::UsageReportSince(option)));
                    }
                }
            }
            Row::Line(pieces) => strip::paint_left(
                buffer,
                rect,
                Line::from(
                    pieces
                        .iter()
                        .map(|(text, role)| Span::styled(text.clone(), look.role(*role)))
                        .collect::<Vec<_>>(),
                ),
            ),
        }
    }
    if clipped {
        strip::paint_left(
            buffer,
            Rect::new(area.x, area.y + shown as u16, area.width, 1),
            Line::styled(
                format!("… {} more rows", rows.len() - shown),
                look.role(Role::Dim),
            ),
        );
    }
}
