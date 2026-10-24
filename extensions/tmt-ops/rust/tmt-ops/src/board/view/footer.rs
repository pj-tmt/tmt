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
pub(in crate::board) fn hint_word(app: &App, action: &crate::action::Action) -> String {
    use crate::action::Verb;
    if action.verb == Verb::Swap {
        return format!(
            "swap {}",
            action
                .args
                .first()
                .and_then(|arg| arg.literal())
                .unwrap_or("?")
        );
    }
    if app.shell.tab == crate::board::shell::OpsTab::Digest {
        match action.verb {
            Verb::Talk | Verb::Ui(crate::action::OpsUiAction::Inspect) => {
                return "chat in Agents".into();
            }
            Verb::NextPane => return "Exchanges".into(),
            Verb::Ui(crate::action::OpsUiAction::MonitorDetails) => return "details".into(),
            _ => {}
        }
    }
    if app.workspace_snapshot().is_some() && action.verb == Verb::Run {
        return format!(
            "run {}",
            action
                .args
                .first()
                .and_then(|arg| arg.literal())
                .unwrap_or("?")
        );
    }
    let lead = action.args.first().and_then(|arg| arg.literal()) == Some("lead");
    match action.verb {
        Verb::NextPane => "pane",
        Verb::PickTab => "tabs",
        Verb::TokenWindow => "window",
        Verb::AskLead => "ask lead",
        Verb::Jump if lead => "jump lead",
        Verb::Jump => "host",
        Verb::Menu => "menu",
        Verb::Talk | Verb::Reply => "chat",
        Verb::Annotate if action.args.first().and_then(|arg| arg.literal()) == Some("member") => {
            "note member"
        }
        Verb::Annotate if app.workspace_snapshot().is_some() => "note",
        Verb::Annotate => "write",
        Verb::HomeMessage => "expand",
        Verb::ViewReply => "reader",
        Verb::Ui(crate::action::OpsUiAction::Pin) => {
            if app.workspace_is_pinned() == Some(true) {
                "unpin"
            } else {
                "pin"
            }
        }
        Verb::Ui(crate::action::OpsUiAction::RevealHidden) => "show hidden",
        Verb::Ui(crate::action::OpsUiAction::Search) if !app.search.is_empty() => "edit filter",
        Verb::Ui(action) => action.spec().label,
        verb => verb.name(),
    }
    .to_owned()
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
        "pageup" => "PgUp",
        "pagedown" => "PgDn",
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
    if action.verb == Verb::Ui(crate::action::OpsUiAction::Pin) {
        return app.workspace_is_pinned().is_some();
    }
    let lead = action.args.first().and_then(|arg| arg.literal()) == Some("lead");
    if action.verb == Verb::ViewReply {
        return app.reply_hint();
    }
    let acts_on_row = !(action.verb == Verb::Jump && lead)
        && matches!(
            action.verb,
            Verb::Jump
                | Verb::Swap
                | Verb::HomeMessage
                | Verb::Talk
                | Verb::Annotate
                | Verb::UpdateStatus
                | Verb::Reply
                | Verb::Open
                | Verb::Copy
                | Verb::Run
        );
    if !acts_on_row {
        return true;
    }
    let target = action
        .verb
        .acts_on_member()
        .then(|| app.workspace_message_target())
        .flatten()
        .map(crate::board::app::RowTarget::Workspace);
    let Some(row) = target
        .as_ref()
        .and_then(|target| app.target_row(target))
        .or_else(|| app.selected_row())
    else {
        return false;
    };
    let ops = app.parameter_observations(row);
    let parameters = crate::template::Parameters {
        row,
        ops: Some(&ops),
        board_pane: None,
    };
    match action.verb {
        Verb::Reply => true,
        Verb::UpdateStatus => target.as_ref().is_none_or(|target| {
            app.target_squad(target)
                .is_some_and(|squad| !squad.is_empty())
        }),
        Verb::Open => match action.args.first() {
            Some(template) => template.fill_parameters(parameters).is_ok(),
            None => crate::effects::default_link(row).is_some(),
        },
        Verb::Copy => action
            .args
            .first()
            .is_none_or(|t| t.fill_parameters(parameters).is_ok()),
        _ => true,
    }
}

/// The footer names what the most used keys do for the selected row, highest
/// priority first. Core hints drop from the end; the optional note is dropped first.
/// `q quit` and effective Help stay (Help alone when only it fits).
pub(super) fn hints(app: &App, width: usize) -> String {
    hint_text(&workspace_pairs(app, width))
}

type Hint = (String, String);
fn hint_text(hints: &[Hint]) -> String {
    hints
        .iter()
        .map(|(key, label)| format!("{key} {label}"))
        .collect::<Vec<_>>()
        .join("  ")
}
fn names_hint(app: &App, mut hints: Vec<Hint>, width: usize) -> Vec<Hint> {
    if app.row_names_hint() {
        let at = hints
            .iter()
            .position(|(_, label)| matches!(label.as_str(), "help" | "quit"))
            .unwrap_or(hints.len());
        hints.insert(at, ("@".into(), "names".into()));
        if hint_text(&hints).width() > width {
            hints.remove(at);
        }
    }
    hints
}
/// Text projection of the second legend line for tests that do not paint key roles.
#[cfg(test)]
pub(in crate::board) fn agents_hints(app: &App, width: usize) -> Option<String> {
    agents_pairs(app, width).map(|[_, second]| hint_text(&second))
}
fn agents_pairs(app: &App, width: usize) -> Option<[Vec<Hint>; 2]> {
    let hints = workspace_legend(app, crate::board::shell::OpsTab::Agents)?;
    Some([
        fit_hints(hints.first, vec![], width),
        names_hint(app, fit_hints(hints.second, hints.tail, width), width),
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
    // The note stays beside Chat when it fits, but never evicts a core hint.
    let note = hints
        .iter()
        .position(|(_, word)| word == "note")
        .map(|at| (at, hints.remove(at)));
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
    if let Some((at, note)) = note.filter(|(at, _)| *at <= hints.len()) {
        hints.insert(at, note);
        if hint_text(&hints.iter().chain(&tail).cloned().collect::<Vec<_>>()).width() > width {
            hints.remove(at);
        }
    }
    hints.extend(tail);
    hints
}

pub(in crate::board) fn effective_key(
    app: &App,
    bindings: &crate::action::Bindings,
    verb: crate::action::Verb,
    preferred: &str,
) -> Option<String> {
    bindings
        .iter()
        .find(|(key, action)| {
            action.verb == verb
                && row_allows(app, action)
                && app
                    .workspace_binding_sources()
                    .and_then(|sources| sources.get(*key))
                    .is_some_and(|source| source != "workspace default")
        })
        .map(|(key, _)| key.clone())
        .or_else(|| {
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
        })
}

fn resize_hints(app: &App, axis: &str, width: usize) -> Vec<Hint> {
    use crate::action::{OpsUiAction as Ui, Verb};
    let bindings = app.bindings();
    let mut tail = Vec::new();
    for (action, preferred, word) in [(Ui::Help, "?", "help"), (Ui::Quit, "q", "quit")] {
        if let Some(key) = effective_key(app, &bindings, Verb::Ui(action), preferred) {
            tail.push((key_label(&key).into(), word.into()));
        } else if action == Ui::Quit {
            tail.push(("Ctrl-C".into(), "quit".into()));
        }
    }
    fit_hints(
        vec![
            (axis.into(), "drag to resize".into()),
            ("double-click".into(), "reset".into()),
        ],
        tail,
        width,
    )
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
    let find = |verb, preferred: &str| effective_key(app, &bindings, verb, preferred);
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
        let word = hint_word(app, action);
        first.push(("⏎".into(), word));
        seen.push(action.verb);
    }
    if tab == crate::board::shell::OpsTab::Agents {
        if let Some(key) = find(Verb::Ui(Ui::IdleAlert), "i") {
            first.push((key_label(&key).into(), "idle alert".into()));
            seen.push(Verb::Ui(Ui::IdleAlert));
        }
        for (action, preferred, word) in [
            (Ui::NewMember, "n", "new"),
            (Ui::RenameMember, "r", "rename"),
            (Ui::MoveMembership, "m", "move"),
            (Ui::MemberSession, "R", "resume"),
            (Ui::MemberNotes, "N", "notes"),
            (Ui::RemoveMember, "d", "remove"),
            (Ui::RemoveMembership, "x", "leave squad"),
            (Ui::CreateSquad, "N", "create squad"),
        ] {
            let member = app.workspace_selected_member().is_some();
            if action != Ui::NewMember && (action == Ui::CreateSquad) == member {
                continue;
            }
            if let Some(key) = find(Verb::Ui(action), preferred) {
                first.push((key_label(&key).into(), word.into()));
                seen.push(Verb::Ui(action));
            }
        }
    }
    for (verb, preferred, word) in [
        (Verb::Talk, "t", "chat"),
        (Verb::Copy, "y", "copy"),
        (Verb::Annotate, "a", "note"),
        (Verb::ViewReply, "v", "reader"),
        (Verb::Jump, "j", "host"),
        (Verb::Open, "o", "link"),
        (Verb::Ui(Ui::Pin), "p", "pin"),
    ] {
        if seen.contains(&verb) {
            continue;
        }
        let host_key = (verb == Verb::Jump)
            .then(|| bindings.get("j"))
            .flatten()
            .filter(|action| {
                matches!(action.verb, Verb::Jump | Verb::Swap | Verb::Run)
                    && row_allows(app, action)
            })
            .map(|_| "j".to_owned());
        if let Some(key) = host_key.or_else(|| find(verb, preferred)) {
            let action = &bindings[&key];
            let word = if matches!(verb, Verb::Jump | Verb::Ui(Ui::Pin)) {
                hint_word(app, action)
            } else {
                word.to_owned()
            };
            first.push((key_label(&key).into(), word));
            seen.push(action.verb);
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
            first.push((key_label(&key).into(), hint_word(app, action)));
        }
    }
    let mut second = Vec::new();
    let mut tail = Vec::new();
    if tab == crate::board::shell::OpsTab::Agents
        && let Some(key) = find(Verb::NextPane, "tab")
    {
        second.push((key_label(&key).into(), "panes".into()));
    }
    let search_word = if app.search.is_empty() {
        "search"
    } else {
        "edit filter"
    };
    let mut choices = vec![
        (Verb::Ui(Ui::Search), "/", search_word),
        (Verb::Ui(Ui::TalkMeasure), "M", "talk/measure"),
        (Verb::Ui(Ui::Timers), "c", "timers"),
        (Verb::Ui(Ui::AgentsFilter), "f", "filter"),
        (Verb::Ui(Ui::SavingsUnit), "$", "unit"),
        (Verb::Ui(Ui::UsageControls), "u", "chart"),
        (Verb::Settings, ",", "settings"),
        (Verb::Ui(Ui::Help), "?", "help"),
        (Verb::Ui(Ui::Quit), "q", "quit"),
    ];
    choices.insert(0, (Verb::Ui(Ui::WorkspaceSnapshots), "w", "snapshots"));
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
    if !app.search.is_empty() {
        tail.push(("Esc".into(), "clear filter".into()));
    }
    Some(Hints {
        first,
        second,
        tail,
    })
}

fn workspace_pairs(app: &App, width: usize) -> Vec<Hint> {
    if app.usage_chart_available() && app.usage_chart.borrow().focused() {
        let scrollable = app.usage_chart.borrow().scrollable();
        return fit_hints(
            [
                ("←→", if scrollable { "scroll" } else { "select" }),
                ("Enter", "pin/menu"),
            ]
            .into_iter()
            .chain(scrollable.then_some(("Home/End", "ends")))
            .chain([("Tab", "fields"), ("Esc", "unpin/list"), ("?", "help")])
            .map(|(key, label)| (key.into(), label.into()))
            .collect(),
            vec![],
            width,
        );
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
            if app.shell.tab == crate::board::shell::OpsTab::Digest {
                let useful = matches!(
                    action.verb,
                    Verb::NextPane
                        | Verb::Talk
                        | Verb::Digest
                        | Verb::Refresh
                        | Verb::Settings
                        | Verb::Theme
                        | Verb::Ui(
                            OpsUiAction::Inspect
                                | OpsUiAction::MonitorDetails
                                | OpsUiAction::UsageControls
                                | OpsUiAction::WorkspaceSnapshots
                                | OpsUiAction::Search
                                | OpsUiAction::Help
                                | OpsUiAction::Quit
                                | OpsUiAction::CycleTokRange
                        )
                );
                if !useful
                    || (action.verb == Verb::Ui(OpsUiAction::Inspect)
                        && app.workspace_message_target().is_none())
                {
                    return false;
                }
            }

            !matches!(event.as_str(), "click" | "double-click")
                && !matches!(
                    action.verb,
                    Verb::Ui(
                        OpsUiAction::Agents
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
    entries.sort_by_key(|(event, action)| {
        (
            action.order(),
            app.shell.tab == crate::board::shell::OpsTab::Digest && event.as_str() == "backtab",
        )
    });
    let mut seen = std::collections::BTreeSet::new();
    let mut hints = Vec::new();
    let mut tail = Vec::new();
    if app.focused_pane() == Some(crate::config::Pane::Replies) {
        tail.push(("Esc".into(), "back".into()));
    }
    if !app.search.is_empty() {
        tail.push(("Esc".into(), "clear filter".into()));
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
                hints.push(("↑↓/jk".into(), "scroll".into()));
            }
            continue;
        }
        if !seen.insert(action.text.as_str()) {
            continue;
        }
        let word = hint_word(app, action);
        let hint = (key_label(key).to_owned(), word);
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
        tail.push(("Ctrl-C".into(), "quit".into()));
    }
    if hint_text(&tail).width() > width {
        return help_hint
            .filter(|hint| hint_text(std::slice::from_ref(hint)).width() <= width)
            .into_iter()
            .collect();
    }
    fit_hints(hints, tail, width)
}

/// Style the fitted hint list shared by board, HOME, read and jobs footers.
/// Informational attention summaries remain muted rather than becoming keys.
pub(in crate::board) fn hint_line(text: &str, look: crate::look::Look) -> Line<'static> {
    crate::board::key_hints::legacy_line(text, look, look.role(Role::Accent), "  ")
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
    if !app.modal_open()
        && app.input.is_none()
        && !app.searching
        && app.shown_notice().is_none()
        && app.error.is_none()
        && app.migration_notice.is_none()
        && let Some(hint) = app.pane_resize.hint()
    {
        let area = Rect::new(
            footer.x + u16::from(footer.width > 0),
            footer.y,
            footer.width.saturating_sub(1),
            footer.height.min(1),
        );
        let hints = resize_hints(app, hint, usize::from(area.width));
        strip::paint_left(
            frame.buffer_mut(),
            area,
            paint_hints(&hints, look, Role::Muted),
        );
        return;
    }
    let last_row = Rect::new(
        footer.x,
        footer.bottom().saturating_sub(1),
        footer.width,
        u16::from(footer.height > 0),
    );
    // Persistent migration/acquisition failures retain their existing priority.
    // Fit whole key hints into the remaining width before any text is painted.
    let hint_area = app
        .shown_notice()
        .filter(|_| app.migration_notice.is_none() && app.error.is_none())
        .map_or(last_row, |text| {
            crate::board::notice::paint_links(frame, app, last_row, text, app.notice_clock.failed)
        });
    let legend = if footer.height >= 2 && app.error.is_none() && app.migration_notice.is_none() {
        agents_pairs(
            app,
            usize::from(hint_area.width.saturating_sub(u16::from(
                app.shell.tab == crate::board::shell::OpsTab::Agents,
            ))),
        )
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
        let footer = hint_area;
        let hint = input
            .hint
            .as_ref()
            .filter(|hint| hint.error || input.text.is_empty())
            .map(|hint| {
                (
                    hint.text.as_str(),
                    if hint.error { Role::Waiting } else { Role::Dim },
                )
            });
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
        Line::styled("↑↓ move · ⏎ done · Esc clear", look.role(Role::Text))
    } else if let Some(notice) = app.migration_notice.as_ref() {
        Line::from(Span::styled(notice.as_str(), look.role(Role::Waiting)))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), look.role(Role::Blocked)))
    } else if app.modal_open() {
        Line::default()
    } else if app
        .composer_projection()
        .is_some_and(|projection| projection.focused)
    {
        // Conversation editing keys live in its stable information line.
        Line::default()
    } else if app.input.is_some() && app.input_band.get().is_some() {
        // The filled input owns Send/Cancel; capability-derived editing keys
        // remain in the footer rather than reviving the old composer frame.
        let (key, label) = app
            .newline_hint()
            .split_once(' ')
            .expect("newline key and label");
        let hints = fit_hints(
            vec![
                (key.into(), label.into()),
                ("Ctrl-P".into(), "paste".into()),
            ],
            vec![],
            usize::from(hint_area.width.saturating_sub(1)),
        );
        let mut line = paint_hints(&hints, look, Role::Text);
        line.spans.insert(0, Span::raw(" "));
        line
    } else if let Some(hint) = app.member_link_hint() {
        if matches!(app.shell.tab, crate::board::shell::OpsTab::Agents) {
            crate::board::key_hints::legacy_line(hint, look, look.role(Role::Text), "  ")
        } else {
            hint_line(hint, look)
        }
    } else {
        let text = legend.as_ref().map_or_else(
            || {
                hints(
                    app,
                    usize::from(hint_area.width.saturating_sub(u16::from(matches!(
                        app.shell.tab,
                        crate::board::shell::OpsTab::Agents
                    )))),
                )
            },
            |lines| hint_text(&lines[1]),
        );
        let mut line = if let Some(lines) = &legend {
            paint_hints(&lines[1], look, Role::Text)
        } else if app.shell.tab == crate::board::shell::OpsTab::Agents {
            paint_hints(
                &workspace_pairs(app, usize::from(hint_area.width.saturating_sub(1))),
                look,
                Role::Text,
            )
        } else {
            hint_line(&text, look)
        };
        if app.shell.tab == crate::board::shell::OpsTab::Agents {
            line.spans.insert(0, Span::raw(" "));
        }
        line
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
    // Read guidance uses the already reserved spare row, never the effective
    // action-hint budget (including authored keys and the Escape ladder).
    if footer.height >= 2
        && legend.is_none()
        && app.error.is_none()
        && app.migration_notice.is_none()
        && let Some(hint) = app.rendered_selection_hint(usize::from(footer.width))
    {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(footer.x, footer.y, footer.width, 1),
            Line::styled(hint, look.role(Role::Muted)),
        );
    }
    if let Some([first, _]) = legend {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(
                footer.x
                    + u16::from(
                        app.shell.tab == crate::board::shell::OpsTab::Agents && footer.width > 0,
                    ),
                footer.bottom() - 2,
                footer.width.saturating_sub(u16::from(
                    app.shell.tab == crate::board::shell::OpsTab::Agents,
                )),
                1,
            ),
            paint_hints(
                &first,
                look,
                if app.shell.tab == crate::board::shell::OpsTab::Agents {
                    Role::Text
                } else {
                    Role::Accent
                },
            ),
        );
    }
    strip::paint_left(frame.buffer_mut(), hint_area, footer_line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_note_never_drops_an_existing_hint() {
        let core: Vec<Hint> = [
            ("↑↓", "select"),
            ("⏎", "chat"),
            ("p", "pin"),
            ("v", "squads/agents"),
            ("/", "search"),
            ("f", "filter"),
            ("e", "details"),
        ]
        .into_iter()
        .map(|(key, word)| (key.into(), word.into()))
        .collect();
        let tail = vec![("?".into(), "help".into()), ("q".into(), "quit".into())];
        let mut with_note = core.clone();
        with_note.insert(2, ("a".into(), "note".into()));
        let mut shown = 0;
        let mut omitted = 0;
        for width in 0..=160 {
            let baseline = fit_hints(core.clone(), tail.clone(), width);
            let mut actual = fit_hints(with_note.clone(), tail.clone(), width);
            if let Some(at) = actual.iter().position(|hint| hint.0 == "a") {
                assert_eq!(at, 2, "the note stays after Chat");
                actual.remove(at);
                shown += 1;
            } else {
                omitted += 1;
            }
            assert_eq!(actual, baseline, "width={width}");
        }
        assert!(shown > 0 && omitted > 0);
    }
}
