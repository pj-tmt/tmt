//! M2 TabStrip: fitted neutral destinations; host owns the returned hit rectangles.
use crate::look::Look;
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;

pub(in crate::board) fn line(
    labels: &[&str],
    selected: usize,
    area: Rect,
    look: Look,
) -> (Line<'static>, Vec<(usize, Rect)>) {
    let mut spans = vec![Span::raw(" ")];
    let mut hits = Vec::new();
    let mut x = area.x.saturating_add(1).min(area.right());
    for (index, label) in labels.iter().enumerate() {
        let label = format!(" {} ", super::notes::single_line(label));
        let width = Line::raw(label.clone()).width();
        let painted = width.min(usize::from(area.right().saturating_sub(x))) as u16;
        if painted > 0 && usize::from(painted) == width && area.height > 0 {
            hits.push((index, Rect::new(x, area.y, painted, 1)));
        }
        x = x.saturating_add(painted + 1);
        let style = if selected == index {
            look.role(Role::Text)
                .patch(look.selection())
                .add_modifier(Modifier::BOLD)
        } else {
            look.role(Role::Muted)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    (Line::from(spans), hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_tab_is_neutral_and_only_whole_painted_labels_get_hits() {
        let look = Look::default();
        let (line, hits) = line(
            &["1 Overview", "2 Agents", "3 Digest"],
            0,
            Rect::new(0, 0, 25, 1),
            look,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].1, Rect::new(1, 0, 12, 1));
        assert_eq!(
            line.spans[1].style,
            look.role(Role::Text)
                .patch(look.selection())
                .add_modifier(Modifier::BOLD)
        );
    }
}
