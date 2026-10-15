//! SectionRule: one neutral rule, a focused title and an optional right note.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) fn plain(title: &str, width: usize) -> String {
    let title = crate::board::notes::single_line(title);
    if title.is_empty() {
        return "─".repeat(width);
    }
    let title = format!("── {title} ");
    let tail = width.saturating_sub(title.width());
    crate::board::view::fit(&format!("{title}{}", "─".repeat(tail)), width)
}

pub(in crate::board) fn line(
    title: &str,
    note: Line<'static>,
    width: usize,
    focused: bool,
    look: Look,
) -> Line<'static> {
    let title = crate::board::notes::single_line(title);
    let heading = format!("── {title} ");
    let note_width = note.width() + 4;
    if note.width() == 0 || heading.width() + note_width + 1 > width {
        let prefix = "── ".chars().take(width.min(3)).collect::<String>();
        let shown = crate::board::view::fit(
            &format!("{title} "),
            width.saturating_sub(prefix.width()).min(title.width() + 1),
        );
        let fill = width.saturating_sub(prefix.width() + shown.width());
        return Line::from(vec![
            Span::styled(prefix, look.role(Role::Dim)),
            Span::styled(
                shown,
                look.role(if focused { Role::Text } else { Role::Muted }),
            ),
            Span::styled("─".repeat(fill), look.role(Role::Dim)),
        ]);
    }
    let mut spans = vec![
        Span::styled("── ", look.role(Role::Dim)),
        Span::styled(
            format!("{title} "),
            look.role(if focused { Role::Text } else { Role::Muted }),
        ),
        Span::styled(
            "─".repeat(width - heading.width() - note_width),
            look.role(Role::Dim),
        ),
        Span::raw(" "),
    ];
    spans.extend(note.spans);
    spans.push(Span::styled(" ──", look.role(Role::Dim)));
    Line::from(spans)
}

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    note: Line<'static>,
    focused: bool,
    look: Look,
) {
    tmt_tui::components::strip::paint_left(
        buffer,
        Rect {
            height: area.height.min(1),
            ..area
        },
        line(title, note, usize::from(area.width), focused, look),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notes_yield_to_the_title_and_every_rule_fits() {
        let look = Look::new(tmt_cli_style::Theme::default());
        for width in [0, 1, 8, 40, 85, 103, 148] {
            let row = line(
                "Squads",
                Line::styled("8 squads · 28 agents", look.role(Role::Muted)),
                width,
                true,
                look,
            );
            assert!(row.width() <= width);
            if width >= 40 {
                assert!(row.to_string().ends_with("8 squads · 28 agents ──"));
            }
            assert!(!row.to_string().contains('━'));
        }
        assert_eq!(plain("Timers", 20), "── Timers ──────────");
    }
}
