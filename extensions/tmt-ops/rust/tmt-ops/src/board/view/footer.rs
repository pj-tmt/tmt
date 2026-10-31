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
        Verb::Talk if app.terminal_showing() => "type",
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
        Verb::Ui(crate::action::OpsUiAction::Terminal) => {
            if app.terminal_showing() {
                "chat view"
            } else {
                "terminal"
            }
        }
        Verb::Ui(crate::action::OpsUiAction::RevealHidden) => "hidden",
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
    if action.verb == Verb::Ui(crate::action::OpsUiAction::Terminal) {
        return app.terminal_available();
    }
    if action.verb == Verb::Ui(crate::action::OpsUiAction::TerminalZoom) {
        return app.terminal_showing();
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
/// Text projection of the Agents key line for tests that do not paint key roles.
#[cfg(test)]
pub(in crate::board) fn agents_hints(app: &App, width: usize) -> Option<String> {
    agents_line(app, width).map(|hints| hint_text(&hints))
}
/// The Agents tab's one footer line: the keys the selected row needs, then help and quit.
fn agents_line(app: &App, width: usize) -> Option<Vec<Hint>> {
    let hints = agents_legend(app)?;
    Some(fit_hints(hints.keys, hints.tail, width))
}
fn paint_hints(hints: &[Hint], look: crate::look::Look, role: Role) -> Line<'static> {
    let pairs: Vec<_> = hints
        .iter()
        .map(|(key, label)| (key.as_str(), label.as_str()))
        .collect();
    crate::board::key_hints::line(&pairs, look, look.role(role), "  ")
}
struct Hints {
    keys: Vec<Hint>,
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
    configured_key(
        app,
        bindings,
        app.workspace_binding_sources(),
        verb,
        preferred,
    )
}

pub(in crate::board) fn configured_key(
    app: &App,
    bindings: &crate::action::Bindings,
    sources: Option<&std::collections::BTreeMap<String, String>>,
    verb: crate::action::Verb,
    preferred: &str,
) -> Option<String> {
    bindings
        .iter()
        .find(|(key, action)| {
            action.verb == verb
                && row_allows(app, action)
                && sources
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

/// One ordered list per row kind (agent, squad, Common pool, Hidden group). Every other key
/// keeps working and is listed in `?`.
fn agents_legend(app: &App) -> Option<Hints> {
    use crate::action::{OpsUiAction as Ui, Verb};
    use crate::board::workspace::OpsTarget;
    if app.shell.tab != crate::board::shell::OpsTab::Agents
        || app.terminal.typing()
        || app.focused_pane() == Some(crate::config::Pane::Replies)
        || app.usage_chart.borrow().focused()
        || app.modal_open()
        || app.input.is_some()
        || app.searching
    {
        return None;
    }
    if let Some(moving) = app.moving.as_ref().filter(|moving| !moving.applying) {
        // A lifted membership takes only these keys.
        let target = moving.target_name(&app.workspace_rows());
        return Some(Hints {
            keys: vec![
                ("↑↓".into(), "choose squad".into()),
                ("⏎".into(), format!("move to {target}")),
            ],
            tail: vec![("Esc".into(), "cancel".into())],
        });
    }
    let bindings = app.bindings();
    let find = |verb, preferred: &str| effective_key(app, &bindings, verb, preferred);
    let bound = |key: &str, action: Ui| {
        bindings
            .get(key)
            .is_some_and(|found| found.verb == Verb::Ui(action))
    };
    let mut keys: Vec<Hint> = Vec::new();
    if bound("up", Ui::Previous)
        && bound("down", Ui::Next)
        && bound("left", Ui::Fold)
        && bound("right", Ui::Expand)
    {
        keys.push(("↑↓←→".into(), "move".into()));
    } else {
        for (verb, preferred, word) in [
            (Ui::Previous, "up", "previous"),
            (Ui::Next, "down", "next"),
            (Ui::Fold, "left", "fold"),
            (Ui::Expand, "right", "expand"),
        ] {
            if let Some(key) = find(Verb::Ui(verb), preferred) {
                keys.push((key_label(&key).into(), word.into()));
            }
        }
    }
    if let Some(action) = bindings.get("enter").filter(|action| {
        row_allows(app, action)
            && !matches!(action.verb, Verb::Ui(ui) if !ui.footer_hint() || matches!(ui, Ui::Quit | Ui::Help))
    }) {
        keys.push(("⏎".into(), hint_word(app, action)));
    }
    let row: &[(Ui, &str, &str)] = if app.workspace_selected_member().is_some() {
        &[
            (Ui::NewMember, "n", "new"),
            (Ui::Edit, "e", "edit"),
            (Ui::Rename, "r", "rename"),
            (Ui::MoveMembership, "m", "move"),
            (Ui::RemoveMember, "x", "close"),
            (Ui::MemberSession, "R", "resume"),
            (Ui::MemberNotes, "C", "checklist"),
        ]
    } else {
        match app.workspace.selected {
            // A hidden squad, listed inside the unfolded Hidden group, offers Show instead.
            Some(OpsTarget::Squad(ref room))
                if app
                    .workspace_snapshot()
                    .is_some_and(|snapshot| snapshot.preferences.hidden.contains(room)) =>
            {
                &[
                    (Ui::NewMember, "n", "new"),
                    (Ui::Edit, "e", "edit"),
                    (Ui::Rename, "r", "rename"),
                    (Ui::Manage, "m", "manage"),
                    (Ui::CreateSquad, "N", "create squad"),
                    (Ui::ShowSquad, "S", ""),
                ]
            }
            Some(OpsTarget::Squad(_)) => &[
                (Ui::NewMember, "n", "new"),
                (Ui::Edit, "e", "edit"),
                (Ui::Rename, "r", "rename"),
                (Ui::Manage, "m", "manage"),
                (Ui::CreateSquad, "N", "create squad"),
                (Ui::HideSquad, "H", ""),
            ],
            Some(OpsTarget::Pool) => &[
                (Ui::NewMember, "n", "new"),
                (Ui::Manage, "m", "manage"),
                (Ui::CreateSquad, "N", "create squad"),
            ],
            _ => &[(Ui::NewMember, "n", "new")],
        }
    };
    for (action, preferred, word) in row {
        if let Some(key) = find(Verb::Ui(*action), preferred) {
            // An empty word takes the action's own state word, such as hide or show.
            let word = if word.is_empty() {
                hint_word(app, &bindings[&key])
            } else {
                (*word).to_owned()
            };
            keys.push((key_label(&key).into(), word));
        }
    }
    let search_word = if app.search.is_empty() {
        "search"
    } else {
        "edit filter"
    };
    let mut tail = Vec::new();
    for (verb, preferred, word) in [
        (Verb::Ui(Ui::Search), "/", search_word),
        (Verb::Ui(Ui::Help), "?", "help"),
        (Verb::Ui(Ui::Quit), "q", "quit"),
    ] {
        // Search, help and quit always stay; row keys yield from the right first.
        if let Some(key) = find(verb, preferred) {
            tail.push((key_label(&key).into(), word.into()));
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
    Some(Hints { keys, tail })
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
    if app.terminal.typing() {
        // Every other key is the program's: the only hint is the way out.
        return fit_hints(vec![("Ctrl-\\".into(), "back".into())], vec![], width);
    }
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
                && !matches!(action.verb, Verb::Ui(ui) if !ui.footer_hint())
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
                            // The top bar's `g  Web` button carries its key.
                            | OpsUiAction::OpenWeb
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
    let legend = if footer.height > 0 && app.error.is_none() && app.migration_notice.is_none() {
        agents_line(
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
    } else if let Some(hint) = (app.shell.tab == crate::board::shell::OpsTab::Agents
        && legend.is_none())
    .then(|| app.rendered_selection_hint(usize::from(hint_area.width.saturating_sub(1))))
    .flatten()
    {
        // The focused Replies pane's guidance takes the one key line; it names Esc back.
        Line::from(vec![
            Span::raw(" "),
            Span::styled(hint, look.role(Role::Muted)),
        ])
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
            |hints| hint_text(hints),
        );
        let mut line = if let Some(hints) = &legend {
            paint_hints(hints, look, Role::Text)
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

    fn selected(target: crate::board::workspace::OpsTarget) -> App {
        let mut app = crate::board::app::workspace::tests::board();
        app.shell.tab = crate::board::shell::OpsTab::Agents;
        app.workspace.selected = Some(target);
        app
    }

    #[test]
    fn each_row_kind_shows_one_line_of_its_own_keys() {
        use crate::board::app::workspace::tests::{MEMBER, ROOM};
        use crate::board::workspace::OpsTarget;
        let member = selected(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        });
        assert_eq!(
            agents_hints(&member, 1000).unwrap(),
            "↑↓←→ move  ⏎ chat  n new  e edit  r rename  m move  x close  C checklist  / search  ? help  q quit"
        );
        let squad = agents_hints(&selected(OpsTarget::Squad(ROOM.into())), 1000).unwrap();
        assert!(
            squad.starts_with("↑↓←→ move  ⏎ ")
                && squad.contains(
                    "  n new  e edit  r rename  m manage  N create squad  H Hide squad  "
                )
                && squad.ends_with("/ search  ? help  q quit"),
            "{squad}"
        );
        let mut hidden = selected(OpsTarget::Squad(ROOM.into()));
        hidden
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .hidden
            .insert(ROOM.into());
        hidden.workspace.visibility.reveal_hidden = true;
        let hidden = agents_hints(&hidden, 1000).unwrap();
        assert!(
            hidden.contains("  S Show squad  ") && !hidden.contains("Hide squad"),
            "a hidden squad inside the Hidden group offers Show: {hidden}"
        );
        let pool = agents_hints(&selected(OpsTarget::Pool), 1000).unwrap();
        assert!(
            pool.contains("  n new  m manage  N create squad  / search  ? help  q quit"),
            "{pool}"
        );
        for line in [agents_hints(&member, 1000).unwrap(), squad, pool] {
            for gone in [
                "Tab panes",
                "w snapshots",
                "u unread",
                ", settings",
                "c timers",
                "j host",
                "p pin",
                "@ names",
                "←→ fold",
            ] {
                assert!(!line.contains(gone), "{gone} stays in ?: {line}");
            }
        }
    }

    #[test]
    fn narrow_footers_drop_row_keys_from_the_right_and_keep_search_help_and_quit() {
        use crate::board::app::workspace::tests::{MEMBER, ROOM};
        let app = selected(crate::board::workspace::OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        });
        let whole = agents_hints(&app, 1000).unwrap();
        for width in [160, 100, 80, 40] {
            let line = agents_hints(&app, width).unwrap();
            assert!(line.width() <= width, "{width}: {line}");
            assert!(
                line.ends_with("/ search  ? help  q quit"),
                "{width}: {line}"
            );
            let kept = line.trim_end_matches("/ search  ? help  q quit");
            assert!(
                whole.starts_with(kept),
                "{width}: a prefix of the row keys: {line}"
            );
        }
        assert_eq!(agents_hints(&app, 160).unwrap(), whole);
    }

    #[test]
    fn rebound_keys_show_their_own_labels_and_split_the_move_arrows() {
        use crate::board::app::workspace::tests::{MEMBER, ROOM};
        let mut app = selected(crate::board::workspace::OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        });
        let keys = &mut app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .keys;
        for scope in keys.values_mut() {
            scope.remove("left");
            scope.remove("R");
        }
        keys.get_mut(&crate::config::OpsKeyScope::Member)
            .unwrap()
            .insert(
                "K".into(),
                crate::action::Action::parse("member-session").unwrap(),
            );
        let line = agents_hints(&app, 1000).unwrap();
        assert!(
            line.starts_with("up previous  down next  → expand  ⏎ chat"),
            "{line}"
        );
        assert!(line.contains("  K resume  "), "{line}");
    }

    #[test]
    fn help_lists_every_bound_key_the_footer_leaves_out() {
        use crate::board::app::workspace::tests::{MEMBER, ROOM, help_lists};
        use crate::board::workspace::OpsTarget;
        for target in [
            OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: MEMBER.into(),
            },
            OpsTarget::Squad(ROOM.into()),
            OpsTarget::Pool,
        ] {
            let app = selected(target.clone());
            let missing: Vec<_> = app
                .bindings()
                .keys()
                .filter(|key| !matches!(key.as_str(), "click" | "double-click"))
                .filter(|key| !help_lists(&app, key))
                .cloned()
                .collect();
            assert!(missing.is_empty(), "{target:?}: {missing:?}");
        }
    }
}
