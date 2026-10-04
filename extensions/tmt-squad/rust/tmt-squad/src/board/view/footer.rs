//! Base key hints, notices, link previews and input strip.

use crate::board::app::App;
use crate::config::{BoardMode, Pane};
use ratatui::widgets::Paragraph;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

/// One state label for the effective toggle in footer and help.
pub(in crate::board) fn toggle_label(app: &App, action: &crate::action::Action) -> Option<String> {
    let board = app.effective_board()?;
    if board.mode != BoardMode::Split {
        return None;
    }
    let panes: Vec<_> = action
        .args
        .iter()
        .filter_map(|arg| arg.literal().and_then(Pane::parse))
        .filter(|pane| board.panes.contains(pane))
        .collect();
    if panes.is_empty() {
        return None;
    }
    let collapsed = app.collapsed_panes();
    let state = if panes.iter().all(|pane| collapsed.contains(pane)) {
        "▸"
    } else {
        "▾"
    };
    Some(format!(
        "{} {state}",
        panes
            .iter()
            .map(|pane| pane.title())
            .collect::<Vec<_>>()
            .join("+")
    ))
}

/// The footer names what the most used keys do for the selected row.
pub(super) fn hints(app: &App, width: usize) -> String {
    if app.view.as_ref().is_some_and(|view| view.home.is_some()) {
        return crate::board::home::hints(width);
    }
    let bindings = app.bindings();
    let mut hints: Vec<String> = [
        ("enter", "⏎"),
        ("o", "o"),
        ("y", "y"),
        ("d", "d"),
        ("tab", "tab"),
        ("ctrl-r", "ctrl-r"),
        ("l", "l"),
        ("T", "T"),
        ("w", "w"),
    ]
    .into_iter()
    .filter_map(|(event, label)| {
        let action = bindings.get(event)?;
        if event == "w"
            && !app
                .meter
                .as_ref()
                .is_some_and(|meter| meter.settings.enabled)
        {
            return None;
        }
        if action.verb == crate::action::Verb::TokenWindow {
            return app
                .meter
                .as_ref()
                .filter(|meter| meter.settings.enabled)
                .map(|_| format!("{label} window"));
        }
        if event == "d" && action.verb == crate::action::Verb::Toggle {
            return toggle_label(app, action).map(|label| format!("d {label}"));
        }
        Some(format!("{label} {}", action.verb.name()))
    })
    .collect();
    hints.extend(["/ search", "←→ tab"].map(str::to_owned));
    if !bindings.contains_key("s") {
        hints.push("s switch".into());
    }
    hints.extend(["? more", "q quit"].map(str::to_owned));
    if app.view.as_ref().is_some_and(|view| view.me.is_none()) {
        hints.push(crate::status::UNKNOWN_YOU.to_owned());
    }
    let mut shown = String::new();
    for hint in hints {
        let next = if shown.is_empty() {
            hint
        } else {
            format!("{shown}  {hint}")
        };
        if next.width() > width {
            break;
        }
        shown = next;
    }
    shown
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
    let mut footer_line = if let Some(input) = &app.input {
        Line::from(format!("{} › {}▏", input.prompt, input.text))
    } else if app.searching {
        Line::from(format!("/{}▏", app.search))
    } else if let Some(notice) = &app.notice {
        Line::from(Span::styled(notice.as_str(), look.role(Role::Waiting)))
    } else if let Some(link) = app
        .selected_link()
        .filter(|_| app.focused_pane() == Some(Pane::Notes))
    {
        Line::from(Span::styled(
            format!(
                "{} · {} · Enter/click to activate",
                link.kind.label(),
                link.target
            ),
            look.role(Role::Link),
        ))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), look.role(Role::Blocked)))
    } else {
        Line::from(Span::styled(
            hints(app, usize::from(footer.width)),
            look.role(Role::Muted),
        ))
    };
    if app.settings.is_some() {
        for span in &mut footer_line.spans {
            span.style = look.role(Role::Dim);
        }
    }
    frame.render_widget(Paragraph::new(footer_line), footer);
}
