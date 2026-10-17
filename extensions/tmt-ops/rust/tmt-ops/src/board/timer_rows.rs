//! TimerRows: fitted, inert timer observations and a shared maximum row budget.
use crate::look::Look;
use ratatui::text::{Line, Span};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::style::TextFlow;

pub(in crate::board) const MAX_ROWS: usize = 3;
/// Identity, squad, schedule and the complete next-time field remain usable.
pub(in crate::board) const MIN_CELLS: u16 = 1 + (2 + 1) + (10 + 1) + (10 + 1) + 8;

pub(in crate::board) struct Row {
    pub id: String,
    pub squad: String,
    pub schedule: String,
    pub next: Option<String>,
    pub purpose: String,
    pub dim: bool,
}

pub(in crate::board) fn line(row: &Row, width: usize, look: Look) -> Line<'static> {
    let role = if row.dim { Role::Dim } else { Role::Muted };
    let fit = |text: &str, cells: usize, align| {
        tmt_tui::text::fit_line(
            &crate::board::notes::sanitize(text),
            cells as u16,
            TextFlow::Truncate,
            align,
        )
    };
    // Approved tracks: id 1, squad 4, schedule 15; right-aligned "in 19m" starts
    // at 28 within the next field (26..34), followed by purpose at 36.
    let fields = [
        (&row.id, 2, Align::Left, 1),
        (&row.squad, 10, Align::Left, 1),
        (&row.schedule, 10, Align::Left, 1),
    ];
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for (text, cells, align, gap) in fields {
        let available = width.saturating_sub(used).min(cells);
        spans.push(Span::styled(fit(text, available, align), look.role(role)));
        used += available;
        let gap = width.saturating_sub(used).min(gap);
        spans.push(Span::raw(" ".repeat(gap)));
        used += gap;
    }
    let next_width = width.saturating_sub(used).min(8);
    spans.push(Span::styled(
        fit(row.next.as_deref().unwrap_or("—"), next_width, Align::Right),
        look.role(if row.dim || row.next.is_none() {
            Role::Dim
        } else {
            Role::Text
        }),
    ));
    used += next_width;
    let gap = width.saturating_sub(used).min(2);
    spans.push(Span::raw(" ".repeat(gap)));
    used += gap;
    spans.push(Span::styled(
        fit(&row.purpose, width.saturating_sub(used), Align::Left),
        look.role(role),
    ));
    if width == 0 {
        Line::default()
    } else {
        Line::from(spans)
    }
}

/// Whole hints only; the host associates the visible footer with its action.
pub(in crate::board) fn footer(
    more: usize,
    key: Option<&str>,
    width: usize,
    look: Look,
) -> Line<'static> {
    use unicode_width::UnicodeWidthStr;
    let available = width.saturating_sub(2);
    let more = (more > 0).then(|| format!("+{more} more"));
    let shortcut = key.map(|key| format!("{} timers", crate::board::view::footer::key_label(key)));
    let mut spans = vec![Span::raw(" ")];
    match (more, shortcut) {
        (Some(more), Some(shortcut)) if more.width() + 3 + shortcut.width() <= available => {
            spans.push(Span::styled(more, look.role(Role::Muted)));
            spans.push(Span::styled(" · ", look.role(Role::Dim)));
            spans.extend(
                crate::board::key_hints::line(
                    &[(
                        crate::board::view::footer::key_label(key.unwrap()),
                        "timers",
                    )],
                    look,
                    look.role(Role::Text),
                    " · ",
                )
                .spans,
            );
        }
        (_, Some(shortcut)) if shortcut.width() <= available => spans.extend(
            crate::board::key_hints::line(
                &[(
                    crate::board::view::footer::key_label(key.unwrap()),
                    "timers",
                )],
                look,
                look.role(Role::Text),
                " · ",
            )
            .spans,
        ),
        (Some(more), _) if more.width() <= available => {
            spans.push(Span::styled(more, look.role(Role::Muted)))
        }
        (None, None) if available >= 11 => {
            spans.push(Span::styled("Open timers", look.role(Role::Muted)))
        }
        _ => return Line::default(),
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timer_tracks_match_the_mock_and_paused_rows_are_neutral() {
        let look = Look::default();
        let mut row = Row {
            id: "c4".into(),
            squad: "tmt-core".into(),
            schedule: "every :05".into(),
            next: Some("in 19m".into()),
            purpose: "hourly check".into(),
            dim: false,
        };
        let shown = line(&row, 62, look).to_string();
        for (text, x) in [
            ("c4", 1),
            ("tmt-core", 4),
            ("every :05", 15),
            ("in 19m", 28),
            ("hourly check", 36),
        ] {
            assert_eq!(shown.find(text), Some(x));
        }
        for width in [0, 1, 12, 36, 60, 62] {
            assert!(line(&row, width, look).width() <= width);
        }
        row.dim = true;
        assert!(
            line(&row, 62, look)
                .spans
                .iter()
                .filter(|s| !s.content.trim().is_empty())
                .all(|s| s.style == look.role(Role::Dim))
        );
    }
}
