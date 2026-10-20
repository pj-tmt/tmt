//! Effective shortcut labels; callers retain controls, geometry and activation.
use super::app::App;
use crate::{
    action::{OpsUiAction, Verb},
    look::Look,
};
use ratatui::text::{Line, Span};

pub(in crate::board) fn key(app: &App, action: OpsUiAction, preferred: &str) -> Option<String> {
    super::view::footer::effective_key(app, &app.workspace_bindings(), Verb::Ui(action), preferred)
        .map(|key| super::view::footer::key_label(&key).to_owned())
}

pub(in crate::board) fn span(key: &str, selected: bool, look: Look) -> Span<'static> {
    Span::styled(
        format!(" {} ", super::notes::single_line(key)),
        tmt_cli_style::theme::screen::keycap(&look.theme, look.depth, selected),
    )
}

pub(in crate::board) fn label(
    key: Option<&str>,
    label: &str,
    selected: bool,
    look: Look,
) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(key) = key {
        spans.push(span(key, selected, look));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(super::notes::single_line(label)));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unbound_controls_keep_the_label_without_a_fabricated_shortcut() {
        let look = Look::default();
        assert_eq!(
            label(None, "Snapshots", false, look).to_string(),
            "Snapshots"
        );
        let line = label(Some("Ctrl-W"), "Snapshots", false, look);
        assert_eq!(line.to_string(), " Ctrl-W  Snapshots");
        assert_eq!(
            line.spans[0].style,
            tmt_cli_style::theme::screen::keycap(&look.theme, look.depth, false)
        );
    }
}
