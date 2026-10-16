//! KeyHints: one painter for already fitted effective-binding hints.
use crate::look::Look;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;

pub(in crate::board) fn line(
    hints: &[(&str, &str)],
    look: Look,
    key_style: Style,
    separator: &str,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, label)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(separator.to_owned(), look.role(Role::Muted)));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            key_style.add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}"), look.role(Role::Muted)));
    }
    Line::from(spans)
}

// Free-form hints from legacy readers are not fitted workspace legends.
pub(in crate::board) fn legacy_line(
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_labels_preserve_spaces_without_manufacturing_keys() {
        let look = Look::default();
        let line = line(
            &[("Ctrl-X", "label  with  spaces"), ("?", "help")],
            look,
            look.role(Role::Text),
            " · ",
        );
        assert_eq!(line.to_string(), "Ctrl-X label  with  spaces · ? help");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(!line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[3].content, "?");
    }
}
