//! KeyHints: one painter for already fitted effective-binding hints.
use crate::look::Look;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;

pub(in crate::board) fn line(
    text: &str,
    look: Look,
    key_style: Style,
    separator: &str,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, hint) in text.split("  ").enumerate() {
        if index > 0 {
            spans.push(Span::styled(separator.to_owned(), look.role(Role::Muted)));
        }
        if let Some((key, label)) = hint.split_once(' ')
            && key != "◆"
        {
            spans.push(Span::styled(
                key.to_owned(),
                key_style.add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!(" {label}"), look.role(Role::Muted)));
        } else {
            spans.push(Span::styled(hint.to_owned(), look.role(Role::Muted)));
        }
    }
    Line::from(spans)
}
