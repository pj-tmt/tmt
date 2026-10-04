//! Base key hints, notices, link previews and input strip.

use crate::board::app::App;
use crate::config::{BoardMode, Pane};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
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

/// The word a hint shows for a verb: its name, except where the name is an
/// internal spelling (`next-pane`).
fn hint_word(verb: crate::action::Verb) -> &'static str {
    use crate::action::Verb;
    match verb {
        Verb::NextPane => "pane",
        Verb::PickTab => "tabs",
        Verb::TokenWindow => "window",
        Verb::AskLead => "ask lead",
        verb => verb.name(),
    }
}

/// Whether the selected row allows the action. Board-level actions always do;
/// a row action needs a row, and `reply` and `open` need something to act on.
fn row_allows(app: &App, action: &crate::action::Action) -> bool {
    use crate::action::Verb;
    let acts_on_row = matches!(
        action.verb,
        Verb::Jump
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
/// priority first. Whole hints drop from the end; `q quit` and `? more` always
/// stay (`? more` alone when only it fits), as the guideline requires.
pub(super) fn hints(app: &App, width: usize) -> String {
    if app.jobs_focus {
        return crate::board::cronboard::jobs_hints(width);
    }
    if app.view.as_ref().is_some_and(|view| view.home.is_some()) {
        return crate::board::home::hints(width, app.cron_shown());
    }
    let bindings = app.bindings();
    // The effective action of an event, as `key word`; unbound events, a
    // disabled meter and actions the selected row does not allow show nothing.
    let bound = |event: &str, label: &str| -> Option<String> {
        let action = bindings.get(event)?;
        if (event == "w" || action.verb == crate::action::Verb::TokenWindow)
            && !app
                .meter
                .as_ref()
                .is_some_and(|meter| meter.settings.enabled)
        {
            return None;
        }
        if !row_allows(app, action) {
            return None;
        }
        if event == "d" && action.verb == crate::action::Verb::Toggle {
            return toggle_label(app, action).map(|label| format!("d {label}"));
        }
        Some(format!("{label} {}", hint_word(action.verb)))
    };
    let mut hints: Vec<String> = Vec::new();
    if let Some((key, action)) = bindings.iter().find(|(key, action)| {
        !matches!(key.as_str(), "click" | "double-click")
            && action.verb == crate::action::Verb::AskLead
    }) {
        hints.push(format!("{key} {}", hint_word(action.verb)));
    }
    // `r` only shows on a row that waits on you, so it ranks with the decision.
    hints.extend(bound("enter", "⏎"));
    hints.extend(bound("r", "r"));
    hints.extend(bound("backspace", "⌫"));
    hints.push("/ search".into());
    hints.extend(
        [("t", "t"), ("a", "a"), ("o", "o"), ("y", "y")]
            .into_iter()
            .filter_map(|(event, label)| bound(event, label)),
    );
    // Row actions end here; the oldest-waiting label may take space only if
    // they all still fit.
    let row_actions = hints.len();
    hints.extend(
        [
            ("d", "d"),
            ("tab", "tab"),
            ("ctrl-r", "ctrl-r"),
            ("l", "l"),
            ("T", "T"),
            ("w", "w"),
        ]
        .into_iter()
        .filter_map(|(event, label)| bound(event, label)),
    );
    hints.push("←→ tab".into());
    if !bindings.contains_key("s") {
        hints.push("s switch".into());
    }
    if app.cron_shown() && !bindings.contains_key("c") {
        hints.push("c cron".into());
    }
    if app.view.as_ref().is_some_and(|view| view.me.is_none()) {
        hints.push(crate::status::UNKNOWN_YOU.to_owned());
    }
    let reserved = ["q quit", "? more"];
    let tail = reserved.join("  ");
    if let Some(waiting) = waiting_summary(app) {
        let minimum = std::iter::once(waiting.base.as_str())
            .chain(hints.iter().take(row_actions).map(String::as_str))
            .chain(reserved)
            .collect::<Vec<_>>()
            .join("  ")
            .width();
        let summary = waiting
            .oldest
            .filter(|oldest| minimum + oldest.width().saturating_sub(waiting.base.width()) <= width)
            .unwrap_or(waiting.base);
        hints.insert(0, summary);
    }
    if width < tail.width() {
        // Below both reserved hints: `? more` first, then nothing.
        return if width < reserved[1].width() {
            String::new()
        } else {
            reserved[1].to_owned()
        };
    }
    let mut shown = String::new();
    for hint in hints {
        let next = if shown.is_empty() {
            hint
        } else {
            format!("{shown}  {hint}")
        };
        if next.width() + "  ".width() + tail.width() > width {
            break;
        }
        shown = next;
    }
    if shown.is_empty() {
        tail
    } else {
        format!("{shown}  {tail}")
    }
}

struct Waiting {
    base: String,
    oldest: Option<String>,
}

/// `◆ N waiting`, with the oldest request's member and age when known.
fn waiting_summary(app: &App) -> Option<Waiting> {
    let view = app.view.as_ref().filter(|_| {
        app.current
            .as_deref()
            .is_some_and(|key| !crate::tabs::aggregate(key))
    })?;
    let count = crate::attention::Attention::of(&view.document).waiting;
    if count == 0 {
        return None;
    }
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
        .filter_map(|row| Some((row, super::waiting::oldest(row)?["preparedAtMs"].as_u64()?)))
        .filter(|(_, since)| *since > 0 && *since <= now)
        .min_by_key(|(_, since)| *since)
        .map(|(row, since)| {
            format!(
                "{base} · oldest {} {}",
                row["name"].as_str().unwrap_or_default(),
                crate::requests::age(now, since)
            )
        });
    Some(Waiting { base, oldest })
}

pub(super) fn render(frame: &mut Frame, app: &App, footer: Rect, look: crate::look::Look) {
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
    strip::paint_left(
        frame.buffer_mut(),
        footer,
        footer_line,
        &look.theme,
        look.depth,
    );
}
