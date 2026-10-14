//! Base key hints, notices, link previews and input strip.

use crate::board::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// The word a hint shows for an action: its verb name, except where the name
/// is an internal spelling (`next-pane`) or the target matters.
fn hint_word(action: &crate::action::Action) -> &'static str {
    use crate::action::Verb;
    let lead = action.args.first().and_then(|arg| arg.literal()) == Some("lead");
    match action.verb {
        Verb::NextPane => "pane",
        Verb::PickTab => "tabs",
        Verb::TokenWindow => "window",
        Verb::AskLead => "ask lead",
        Verb::Jump if lead => "jump lead",
        Verb::Jump => "host",
        Verb::Menu => "menu",
        Verb::Annotate if action.args.first().and_then(|arg| arg.literal()) == Some("member") => {
            "note member"
        }
        Verb::Annotate => "write",
        Verb::HomeMessage => "expand",
        Verb::ViewReply => "view",
        Verb::Ui(action) => action.spec().label,
        verb => verb.name(),
    }
}

/// The key a hint shows for an event.
fn key_label(event: &str) -> &str {
    match event {
        "enter" => "⏎",
        "backspace" => "⌫",
        other => other,
    }
}

/// Whether the selected row allows the action. Board-level actions always do;
/// a row action needs a row, and `reply` and `open` need something to act on.
fn row_allows(app: &App, action: &crate::action::Action) -> bool {
    use crate::action::Verb;
    if app.workspace_snapshot().is_some()
        && action.verb.acts_on_member()
        && app.workspace_selected_member().is_none()
    {
        return false;
    }
    let lead = action.args.first().and_then(|arg| arg.literal()) == Some("lead");
    if action.verb == Verb::ViewReply {
        return app.reply_hint();
    }
    let acts_on_row = !(action.verb == Verb::Jump && lead)
        && matches!(
            action.verb,
            Verb::Jump
                | Verb::HomeMessage
                | Verb::Talk
                | Verb::Annotate
                | Verb::Reply
                | Verb::Open
                | Verb::Copy
                | Verb::Run
        );
    if !acts_on_row {
        return true;
    }
    let Some(row) = app.selected_row() else {
        return false;
    };
    match action.verb {
        Verb::Reply => crate::attention::waits_on_you(row),
        Verb::Open => match action.args.first() {
            Some(template) => template.fill(row).is_ok(),
            None => crate::effects::default_link(row).is_some(),
        },
        Verb::Copy => action.args.first().is_none_or(|t| t.fill(row).is_ok()),
        _ => true,
    }
}

/// The footer names what the most used keys do for the selected row, highest
/// priority first. Whole hints drop from the end; `q quit` and the effective Help hint always
/// stay (Help alone when only it fits), as the guideline requires.
pub(super) fn hints(app: &App, width: usize) -> String {
    workspace_hints(app, width)
}

fn workspace_hints(app: &App, width: usize) -> String {
    if app.usage_chart_available() && app.usage_chart.borrow().focused() {
        return crate::board::view::fit(
            "←→ choose · Enter apply/details · Tab Each/Range/buckets · Esc list · ? help",
            width,
        )
        .trim_end()
        .to_owned();
    }
    use crate::action::{OpsUiAction, Verb};
    let bindings = app.bindings();
    let mut entries: Vec<_> = bindings
        .iter()
        .filter(|(event, action)| {
            !matches!(event.as_str(), "click" | "double-click")
                && !matches!(
                    action.verb,
                    Verb::Ui(
                        OpsUiAction::Overview
                            | OpsUiAction::Agents
                            | OpsUiAction::DigestDashboard
                            | OpsUiAction::First
                            | OpsUiAction::Last
                            | OpsUiAction::PageUp
                            | OpsUiAction::PageDown
                            | OpsUiAction::ScrollTop
                            | OpsUiAction::ScrollBottom
                            | OpsUiAction::ScrollPageUp
                            | OpsUiAction::ScrollPageDown
                    )
                )
                && row_allows(app, action)
        })
        .collect();
    entries.sort_by_key(|(_, action)| action.order());
    let mut seen = std::collections::BTreeSet::new();
    let mut hints = Vec::new();
    let mut tail = Vec::new();
    let mut help_hint = None;
    for (key, action) in entries {
        if !seen.insert(action.text.as_str()) {
            continue;
        }
        let word = hint_word(action);
        let hint = format!("{} {word}", key_label(key));
        if matches!(action.verb, Verb::Ui(OpsUiAction::Help | OpsUiAction::Quit)) {
            if action.verb == Verb::Ui(OpsUiAction::Help) {
                help_hint = Some(hint.clone());
            }
            tail.push(hint);
        } else {
            hints.push(hint);
        }
    }
    if !bindings
        .values()
        .any(|action| action.verb == Verb::Ui(OpsUiAction::Quit))
    {
        tail.push("Ctrl-C quit".into());
    }
    let mut shown = tail.join("  ");
    if shown.width() > width {
        return help_hint
            .filter(|hint| hint.width() <= width)
            .unwrap_or_default();
    }
    let mut chosen: Vec<String> = Vec::new();
    for hint in hints {
        let next = chosen
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(hint.as_str()))
            .chain(tail.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("  ");
        if next.width() <= width {
            chosen.push(hint);
        } else {
            break;
        }
    }
    if !chosen.is_empty() {
        chosen.extend(tail);
        shown = chosen.join("  ");
    }
    shown
}

/// Style the fitted hint list shared by board, HOME, read and jobs footers.
/// Informational attention summaries remain muted rather than becoming keys.
pub(in crate::board) fn hint_line(text: &str, look: crate::look::Look) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, hint) in text.split("  ").enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", look.role(Role::Muted)));
        }
        if let Some((key, label)) = hint.split_once(' ')
            && key != "◆"
        {
            spans.push(Span::styled(
                key.to_owned(),
                look.role(Role::Accent).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!(" {label}"), look.role(Role::Muted)));
        } else {
            spans.push(Span::styled(hint.to_owned(), look.role(Role::Muted)));
        }
    }
    Line::from(spans)
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
    if let Some(input) = app.input.as_ref().filter(|input| {
        !matches!(
            input.compose,
            crate::board::app::Compose::AskLead { .. }
                | crate::board::app::Compose::Status
                | crate::board::app::Compose::Workspace
        ) && app.input_band.get().is_none()
    }) {
        let hint = input
            .hint
            .as_ref()
            .filter(|hint| hint.error || input.text.is_empty())
            .map(|hint| {
                (
                    hint.text.as_str(),
                    if hint.error { Role::Waiting } else { Role::Dim },
                )
            })
            .or_else(|| app.notice.as_deref().map(|notice| (notice, Role::Waiting)));
        let hint_width = hint.map_or(0, |(text, _)| {
            (text.width() + 2).min(usize::from(footer.width / 3)) as u16
        });
        let prompt = format!("{} › ", input.prompt);
        let width = prompt.width().min(usize::from(footer.width / 3)) as u16;
        strip::paint_left(
            frame.buffer_mut(),
            Rect { width, ..footer },
            Line::styled(prompt, look.role(Role::Accent)),
        );
        super::waiting::editor(
            frame,
            app,
            Rect {
                x: footer.x + width,
                width: footer.width.saturating_sub(width + hint_width),
                ..footer
            },
        );
        if let Some((text, role)) = hint {
            strip::paint_left(
                frame.buffer_mut(),
                Rect {
                    x: footer.right() - hint_width,
                    width: hint_width,
                    ..footer
                },
                Line::styled(format!("  {text}"), look.role(role)),
            );
        }
        return;
    }
    let mut footer_line = if let Some(input) = app.input.as_ref().filter(|input| {
        !matches!(input.compose, crate::board::app::Compose::AskLead { .. })
            && app.input_band.get().is_none()
    }) {
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
    } else if let Some(notice) = app.migration_notice.as_ref().or(app.notice.as_ref()) {
        Line::from(Span::styled(notice.as_str(), look.role(Role::Waiting)))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), look.role(Role::Blocked)))
    } else if app.modal_open() || app.input_band.get().is_some() {
        // Notices and errors above stay; the base key hints would misname the keys.
        Line::default()
    } else {
        hint_line(&hints(app, usize::from(footer.width)), look)
    };
    if app.settings.is_some() {
        for span in &mut footer_line.spans {
            span.style = look.role(Role::Dim);
        }
    }
    // A prompt or overlay is holding a due reload back; say so beside it.
    if app.reload_waiting() {
        footer_line.spans.push(Span::styled(
            crate::board::reload::WAITING,
            look.role(Role::Dim),
        ));
    }
    strip::paint_left(frame.buffer_mut(), footer, footer_line);
}
