//! Base key hints, notices, link previews and input strip.

use crate::board::app::App;
use crate::config::{BoardMode, Pane};
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
    if app.jobs_focus {
        return crate::board::cronboard::jobs_hints(width);
    }
    if app.view.as_ref().is_some_and(|view| view.home.is_some()) {
        return crate::board::home::hints(width, app.cron_shown());
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
    if let Some((key, _)) = bindings.iter().find(|(key, action)| {
        !matches!(key.as_str(), "click" | "double-click")
            && action.verb == crate::action::Verb::AskLead
    }) {
        // Keep the existing row-action order while reserving the new action
        // ahead of secondary layout, meter and navigation hints.
        hints.insert(0, format!("{key} ask lead"));
    }
    let waiting = app
        .view
        .as_ref()
        .filter(|_| {
            app.current
                .as_deref()
                .is_some_and(|key| !crate::tabs::aggregate(key))
        })
        .filter(|view| crate::attention::Attention::of(&view.document).waiting > 0)
        .map(|view| {
            let count = crate::attention::Attention::of(&view.document).waiting;
            let base = format!("◆ {count} waiting");
            let now = crate::status::now_ms();
            let oldest = view.document["squad"]["lead"]
                .as_object()
                .map(|_| &view.document["squad"]["lead"])
                .into_iter()
                .chain(
                    view.document["sections"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|section| section["rows"].as_array().into_iter().flatten()),
                )
                .filter_map(|row| {
                    Some((row, super::waiting::oldest(row)?["preparedAtMs"].as_u64()?))
                })
                .filter(|(_, since)| *since > 0 && *since <= now)
                .min_by_key(|(_, since)| *since)
                .map(|(row, since)| {
                    format!(
                        "{base} · oldest {} {}",
                        row["name"].as_str().unwrap_or_default(),
                        crate::requests::age(now, since)
                    )
                });
            (base, oldest)
        });
    hints.extend(["/ search", "←→ tab"].map(str::to_owned));
    if !bindings.contains_key("s") {
        hints.push("s switch".into());
    }
    if app.cron_shown() && !bindings.contains_key("c") {
        hints.push("c cron".into());
    }
    hints.push("q quit".into());
    if app.view.as_ref().is_some_and(|view| view.me.is_none()) {
        hints.push(crate::status::UNKNOWN_YOU.to_owned());
    }
    if let Some((base, oldest)) = waiting {
        // Reserve the existing row actions through refresh before spending
        // space on the optional oldest label; secondary tail hints keep their
        // established whole-hint fitting priority.
        let reserved = hints
            .iter()
            .position(|hint| hint == "ctrl-r refresh")
            .map_or(hints.len().min(7), |index| index + 1);
        let minimum = std::iter::once(base.as_str())
            .chain(hints.iter().take(reserved).map(String::as_str))
            .collect::<Vec<_>>()
            .join("  ")
            .width()
            + "  ? more".width();
        let summary = oldest
            .filter(|oldest| minimum + oldest.width().saturating_sub(base.width()) <= width)
            .unwrap_or(base);
        hints.insert(0, summary);
    }
    let more = "? more";
    if width < more.width() {
        return String::new();
    }
    let mut shown = String::new();
    for hint in hints {
        let next = if shown.is_empty() {
            hint
        } else {
            format!("{shown}  {hint}")
        };
        if next.width() + "  ? more".width() > width {
            break;
        }
        shown = next;
    }
    if shown.is_empty() {
        more.into()
    } else {
        format!("{shown}  {more}")
    }
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
    let mut footer_line = if let Some(input) = app
        .input
        .as_ref()
        .filter(|input| !matches!(input.compose, crate::board::app::Compose::AskLead { .. }))
    {
        let mut spans = vec![Span::raw(format!("{} › {}▏", input.prompt, input.text))];
        if let Some(hint) = input
            .hint
            .as_ref()
            .filter(|hint| hint.error || input.text.is_empty())
        {
            spans.push(Span::styled(
                format!("  {}", hint.text),
                look.role(if hint.error { Role::Waiting } else { Role::Dim }),
            ));
        }
        Line::from(spans)
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
    super::strip::paint_line(frame, footer, footer_line, look);
}
