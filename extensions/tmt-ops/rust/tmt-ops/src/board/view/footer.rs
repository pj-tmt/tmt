//! Base key hints, notices, link previews and input strip.

use crate::board::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// The word a hint shows for an action: its verb name, except where the name
/// is an internal spelling (`next-pane`) or the target matters.
pub(in crate::board) fn hint_word(app: &App, action: &crate::action::Action) -> &'static str {
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
        Verb::Talk => "chat",
        Verb::Annotate if action.args.first().and_then(|arg| arg.literal()) == Some("member") => {
            "note member"
        }
        Verb::Annotate => "write",
        Verb::HomeMessage => "expand",
        Verb::ViewReply => "reader",
        Verb::Ui(crate::action::OpsUiAction::Pin)
            if app.shell.tab == crate::board::shell::OpsTab::Overview =>
        {
            "pin agent"
        }
        Verb::Ui(crate::action::OpsUiAction::Pin) => "pin squad",
        Verb::Ui(crate::action::OpsUiAction::RevealHidden) => "show hidden",
        Verb::Ui(action) => action.spec().label,
        verb => verb.name(),
    }
}

/// The key a hint shows for an event.
pub(in crate::board) fn key_label(event: &str) -> &str {
    match event {
        "enter" => "⏎",
        "esc" => "Esc",
        "backspace" => "⌫",
        "left" => "←",
        "right" => "→",
        "tab" => "Tab",
        other => other,
    }
}

/// Whether the selected row allows the action. Board-level actions always do;
/// a row action needs a row, and `reply` and `open` need something to act on.
pub(in crate::board) fn row_allows(app: &App, action: &crate::action::Action) -> bool {
    use crate::action::Verb;
    if app.workspace_snapshot().is_some()
        && action.verb.acts_on_member()
        && app.workspace_message_target().is_none()
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
    if app.shell.tab == crate::board::shell::OpsTab::Overview
        && app.overview_view == crate::board::overview::monitor::View::Squads
        && matches!(
            action.verb,
            Verb::Ui(crate::action::OpsUiAction::Sort | crate::action::OpsUiAction::ReverseSort)
        )
    {
        return false;
    }
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

type Hint = (String, String);
fn hint_text(hints: &[Hint]) -> String {
    hints
        .iter()
        .map(|(key, label)| format!("{key} {label}"))
        .collect::<Vec<_>>()
        .join("  ")
}
fn overview_pairs(app: &App, width: usize) -> Option<Vec<Hint>> {
    let hints = workspace_legend(app, crate::board::shell::OpsTab::Overview)?;
    Some(fit_hints(
        hints.first.into_iter().chain(hints.second).collect(),
        hints.tail,
        width,
    ))
}
/// Text projection for tests and consumers that do not paint key roles.
pub(in crate::board) fn overview_hints(app: &App, width: usize) -> Option<String> {
    overview_pairs(app, width).map(|pairs| hint_text(&pairs))
}
fn agents_pairs(app: &App, width: usize) -> Option<[Vec<Hint>; 2]> {
    let hints = workspace_legend(app, crate::board::shell::OpsTab::Agents)?;
    Some([
        fit_hints(hints.first, vec![], width),
        fit_hints(hints.second, hints.tail, width),
    ])
}
fn paint_hints(hints: &[Hint], look: crate::look::Look, role: Role) -> Line<'static> {
    let pairs: Vec<_> = hints
        .iter()
        .map(|(key, label)| (key.as_str(), label.as_str()))
        .collect();
    crate::board::key_hints::line(&pairs, look, look.role(role), "  ")
}
struct Hints {
    first: Vec<Hint>,
    second: Vec<Hint>,
    tail: Vec<Hint>,
}
fn fit_hints(mut hints: Vec<Hint>, tail: Vec<Hint>, width: usize) -> Vec<Hint> {
    while hint_text(&hints.iter().chain(&tail).cloned().collect::<Vec<_>>()).width() > width {
        if hints.pop().is_none() {
            return tail
                .first()
                .filter(|hint| hint_text(std::slice::from_ref(hint)).width() <= width)
                .cloned()
                .into_iter()
                .collect();
        }
    }
    hints.extend(tail);
    hints
}

fn workspace_legend(app: &App, tab: crate::board::shell::OpsTab) -> Option<Hints> {
    use crate::action::{OpsUiAction as Ui, Verb};
    if app.shell.tab != tab
        || app.focused_pane() == Some(crate::config::Pane::Replies)
        || app.usage_chart.borrow().focused()
        || app.modal_open()
        || app.input.is_some()
        || app.searching
    {
        return None;
    }
    let bindings = app.bindings();
    let find = |verb, preferred: &str| {
        bindings
            .get(preferred)
            .filter(|action| action.verb == verb && row_allows(app, action))
            .map(|_| preferred.to_owned())
            .or_else(|| {
                bindings
                    .iter()
                    .find(|(_, action)| action.verb == verb && row_allows(app, action))
                    .map(|(key, _)| key.clone())
            })
    };
    let mut seen = Vec::new();
    let mut first = Vec::new();
    if bindings
        .get("up")
        .is_some_and(|action| action.verb == Verb::Ui(Ui::Previous))
        && bindings
            .get("down")
            .is_some_and(|action| action.verb == Verb::Ui(Ui::Next))
    {
        first.push(("↑↓".into(), "select".into()));
    } else {
        for (verb, preferred, word) in [
            (Verb::Ui(Ui::Previous), "up", "previous"),
            (Verb::Ui(Ui::Next), "down", "next"),
        ] {
            if let Some(key) = find(verb, preferred) {
                first.push((key_label(&key).into(), word.into()));
            }
        }
    }
    if tab == crate::board::shell::OpsTab::Agents {
        if bindings
            .get("left")
            .is_some_and(|a| a.verb == Verb::Ui(Ui::Fold))
            && bindings
                .get("right")
                .is_some_and(|a| a.verb == Verb::Ui(Ui::Expand))
        {
            first.push(("←→".into(), "fold".into()));
        } else {
            for (verb, preferred, word) in [
                (Verb::Ui(Ui::Fold), "left", "fold"),
                (Verb::Ui(Ui::Expand), "right", "expand"),
            ] {
                if let Some(key) = find(verb, preferred) {
                    first.push((key_label(&key).into(), word.into()));
                }
            }
        }
    }
    if let Some(action) = bindings.get("enter").filter(|action| {
        row_allows(app, action) && !matches!(action.verb, Verb::Ui(Ui::Quit | Ui::Help))
    }) {
        let word = if tab == crate::board::shell::OpsTab::Overview
            && matches!(
                app.workspace.selected,
                Some(crate::board::workspace::OpsTarget::Squad(_))
            )
            && action.verb == Verb::Talk
        {
            "message lead"
        } else {
            hint_word(app, action)
        };
        first.push(("⏎".into(), word.into()));
        seen.push(action.verb);
    }
    for (verb, preferred, word) in [
        (Verb::Talk, "t", "chat"),
        (Verb::ViewReply, "v", "reader"),
        (Verb::Jump, "j", "host"),
        (Verb::Open, "o", "link"),
        (Verb::Ui(Ui::Pin), "p", "pin"),
    ] {
        if (tab == crate::board::shell::OpsTab::Overview
            && matches!(verb, Verb::ViewReply | Verb::Open | Verb::Ui(Ui::Pin)))
            || seen.contains(&verb)
            || (verb == Verb::Ui(Ui::Pin)
                && tab == crate::board::shell::OpsTab::Overview
                && app.workspace_selected_member().is_none())
        {
            continue;
        }
        if let Some(key) = find(verb, preferred) {
            first.push((key_label(&key).into(), word.into()));
            seen.push(verb);
        }
    }
    for (verb, preferred) in [
        (Ui::HideSquad, "H"),
        (Ui::ShowSquad, "S"),
        (Ui::RevealHidden, "h"),
    ] {
        if tab == crate::board::shell::OpsTab::Agents
            && let Some(key) = find(Verb::Ui(verb), preferred)
            && let Some(action) = bindings.get(&key)
        {
            first.push((key_label(&key).into(), hint_word(app, action).into()));
        }
    }
    let mut second = Vec::new();
    let mut tail = Vec::new();
    let filter = if tab == crate::board::shell::OpsTab::Agents {
        app.workspace.filter.next().label().to_owned()
    } else {
        app.overview_view.next().label().to_ascii_lowercase()
    };
    let filter_verb = if tab == crate::board::shell::OpsTab::Agents {
        Ui::AgentsFilter
    } else {
        Ui::OverviewView
    };
    if tab == crate::board::shell::OpsTab::Agents
        && let Some(key) = find(Verb::NextPane, "tab")
    {
        second.push((key_label(&key).into(), "panes".into()));
    }
    let choices = if tab == crate::board::shell::OpsTab::Overview {
        vec![
            (Verb::Ui(filter_verb), "v", "squads/agents"),
            (Verb::Ui(Ui::Search), "/", "search"),
            (Verb::Settings, ",", "settings"),
            (Verb::Ui(Ui::Help), "?", "help"),
            (Verb::Ui(Ui::Quit), "q", "quit"),
        ]
    } else {
        vec![
            (Verb::Ui(Ui::Search), "/", "search"),
            (Verb::Ui(Ui::Timers), "c", "timers"),
            (Verb::Ui(filter_verb), "f", filter.as_str()),
            (Verb::Ui(Ui::SavingsUnit), "$", "unit"),
            (Verb::Ui(Ui::UsageControls), "u", "chart"),
            (Verb::Settings, ",", "settings"),
            (Verb::Ui(Ui::Help), "?", "help"),
            (Verb::Ui(Ui::Quit), "q", "quit"),
        ]
    };
    for (verb, preferred, word) in choices {
        if seen.contains(&verb) || (verb == Verb::Ui(Ui::SavingsUnit) && !app.savings_visible()) {
            continue;
        }
        if let Some(key) = find(verb, preferred) {
            let hint = (key_label(&key).into(), word.into());
            if matches!(verb, Verb::Ui(Ui::Help | Ui::Quit)) {
                tail.push(hint);
            } else {
                second.push(hint);
            }
        }
    }
    if !bindings
        .values()
        .any(|action| action.verb == Verb::Ui(Ui::Quit))
    {
        tail.push(("Ctrl-C".into(), "quit".into()));
    }
    Some(Hints {
        first,
        second,
        tail,
    })
}

fn workspace_hints(app: &App, width: usize) -> String {
    if let Some(hints) = overview_hints(app, width) {
        return hints;
    }
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
    let group_scroll = app.focused_pane() == Some(crate::config::Pane::Replies)
        && [
            ("up", OpsUiAction::ScrollUp),
            ("k", OpsUiAction::ScrollUp),
            ("down", OpsUiAction::ScrollDown),
            ("j", OpsUiAction::ScrollDown),
        ]
        .iter()
        .all(|(key, verb)| {
            bindings
                .get(*key)
                .is_some_and(|action| action.verb == Verb::Ui(*verb))
        });
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
                && (action.verb != Verb::Ui(OpsUiAction::SavingsUnit) || app.savings_visible())
                && row_allows(app, action)
        })
        .collect();
    entries.sort_by_key(|(_, action)| action.order());
    let mut seen = std::collections::BTreeSet::new();
    let mut hints = Vec::new();
    let mut tail = Vec::new();
    if app.focused_pane() == Some(crate::config::Pane::Replies) {
        tail.push("Esc back".into());
    }
    let mut help_hint = None;
    for (key, action) in entries {
        if group_scroll
            && matches!(
                action.verb,
                Verb::Ui(OpsUiAction::ScrollUp | OpsUiAction::ScrollDown)
            )
        {
            if seen.insert("reader-scroll") {
                hints.push("↑↓/jk scroll".into());
            }
            continue;
        }
        if !seen.insert(action.text.as_str()) {
            continue;
        }
        let word = hint_word(app, action);
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
    crate::board::key_hints::legacy_line(text, look, look.role(Role::Accent), "  ")
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
    let legend = if footer.height >= 2
        && app.notice.is_none()
        && app.error.is_none()
        && app.migration_notice.is_none()
    {
        agents_pairs(app, usize::from(footer.width))
    } else {
        None
    };
    if let Some(input) = app.input.as_ref().filter(|input| {
        !matches!(
            input.compose,
            crate::board::app::Compose::AskLead { .. }
                | crate::board::app::Compose::Status
                | crate::board::app::Compose::Workspace
        ) && app.input_band.get().is_none()
    }) {
        let footer = Rect::new(
            footer.x,
            footer.bottom().saturating_sub(1),
            footer.width,
            u16::from(footer.height > 0),
        );
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
        Line::from(Span::styled(
            notice.as_str(),
            look.role(if app.shell.tab == crate::board::shell::OpsTab::Overview {
                Role::Muted
            } else {
                Role::Waiting
            }),
        ))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(
            error.as_str(),
            look.role(if app.shell.tab == crate::board::shell::OpsTab::Overview {
                Role::Muted
            } else {
                Role::Blocked
            }),
        ))
    } else if app.modal_open() || (app.input.is_some() && app.input_band.get().is_some()) {
        // Notices and errors above stay; the base key hints would misname the keys.
        Line::default()
    } else if let Some(hint) = app.member_link_hint() {
        if app.shell.tab == crate::board::shell::OpsTab::Overview {
            crate::board::m12_key_hints::line(hint, look, look.role(Role::Text), "  ")
        } else {
            hint_line(hint, look)
        }
    } else {
        let text = legend.as_ref().map_or_else(
            || {
                hints(
                    app,
                    usize::from(footer.width.saturating_sub(u16::from(
                        app.shell.tab == crate::board::shell::OpsTab::Overview,
                    ))),
                )
            },
            |lines| hint_text(&lines[1]),
        );
        if app.shell.tab == crate::board::shell::OpsTab::Overview {
            let pairs = overview_pairs(app, usize::from(footer.width.saturating_sub(1)))
                .unwrap_or_default();
            let refs: Vec<_> = pairs
                .iter()
                .map(|(key, label)| (key.as_str(), label.as_str()))
                .collect();
            let mut line = crate::board::key_hints::line(&refs, look, look.role(Role::Text), "  ");
            line.spans.insert(0, Span::raw(" "));
            line
        } else {
            if let Some(lines) = &legend {
                paint_hints(&lines[1], look, Role::Accent)
            } else {
                hint_line(&text, look)
            }
        }
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
    if let Some([first, _]) = legend {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(footer.x, footer.bottom() - 2, footer.width, 1),
            paint_hints(&first, look, Role::Accent),
        );
    }
    let footer = Rect::new(
        footer.x,
        footer.bottom().saturating_sub(1),
        footer.width,
        u16::from(footer.height > 0),
    );
    strip::paint_left(frame.buffer_mut(), footer, footer_line);
}
