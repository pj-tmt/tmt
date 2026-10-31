//! The existing workspace overlay owns one shared Confirm dialog and its painted hits.
use super::*;
use crate::board::app::Overlay;
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::{Button, Dialog, DialogKind, Placement, strip};

pub(super) fn button_labels(draft: &Draft) -> Vec<&'static str> {
    if draft.stage == Stage::Pending {
        return vec!["Cancel"];
    }
    match draft.close.as_ref().and_then(|c| c.result.as_ref()) {
        Some(Outcome::Refused { .. }) => vec!["Cancel", "Jump to pane", "Delete identity"],
        Some(_) => vec!["Close"],
        None => vec!["Cancel", "Close and delete"],
    }
}
fn text(draft: &Draft, app: &App) -> Vec<(String, Role)> {
    let member = draft.member.as_ref().expect("frozen member");
    let mut rows = vec![("←→ action · Enter choose · Esc close".into(), Role::Muted)];
    let mut push = |text: String, role| rows.push((text, role));
    if draft.stage == Stage::Pending {
        push(
            "◌ Checking and applying the reviewed operation…".into(),
            Role::Waiting,
        );
        push(
            "Closing this dialog keeps the operation running. Nothing is retried.".into(),
            Role::Muted,
        );
    } else if let Some(result) = draft.close.as_ref().and_then(|c| c.result.as_ref()) {
        match result {
            Outcome::Changed { pane_reason } => {
                push(
                    format!("✗ {} changed or was removed; reopen to retry.", member.name),
                    Role::Blocked,
                );
                push(
                    "No same-name successor is selected and nothing is retried.".into(),
                    Role::Muted,
                );
                if let Some(reason) = pane_reason {
                    push(format!("Pane left open · {reason}"), Role::Muted);
                }
            }
            Outcome::Refused {
                step,
                error,
                pane_reason,
            } => {
                push(
                    format!(
                        "✗ Could not {} {}",
                        if *step == Step::Stop {
                            "stop"
                        } else {
                            "delete"
                        },
                        member.name
                    ),
                    Role::Blocked,
                );
                let unavailable = *step == Step::Stop
                    && error.code == "RESUME_UNAVAILABLE"
                    && error.signalled == Some(false);
                if unavailable {
                    push(
                        "No recorded agent session · nothing was signalled.".into(),
                        Role::Text,
                    );
                    push("Delete may follow; Core checks it.".into(), Role::Text);
                } else if error.code == "AGENT_RUNNING" {
                    push(
                        "Still running or unproven · quit it in its pane, then Delete.".into(),
                        Role::Text,
                    );
                } else if *step == Step::Delete {
                    let stopped = pane_reason.is_some()
                        || matches!(
                            draft.operation.as_ref(),
                            Some(Change::CloseMember(Request {
                                mode: Mode::CloseAndDelete,
                                ..
                            }))
                        );
                    push(
                        format!(
                            "{}Deletion was not confirmed: {}",
                            if stopped { "Agent stopped. " } else { "" },
                            error.message
                        ),
                        Role::Blocked,
                    );
                    push(
                        "Delete asks Core to check again; nothing is retried automatically.".into(),
                        Role::Text,
                    );
                } else {
                    push(error.message.clone(), Role::Blocked);
                    push(
                        match error.signalled {
                            Some(true) => {
                                "A stop was requested · quit it in its pane, then Delete."
                            }
                            Some(false) => "Nothing was signalled.",
                            None => "Whether a stop was requested is unknown.",
                        }
                        .into(),
                        Role::Muted,
                    );
                    if error.signalled != Some(true) {
                        push("Delete may follow; Core checks it.".into(), Role::Text);
                    }
                }
                if let Some(reason) = pane_reason {
                    push(format!("Pane left open · {reason}"), Role::Muted);
                }
            }
            Outcome::Removed { .. } => unreachable!("removed closes the dialog"),
        }
    } else {
        let host = app
            .references
            .agents
            .get(&member.id)
            .and_then(|agent| agent.parameters["location"]["host"].as_str())
            .unwrap_or("its host");
        push(
            format!("1. Stop {} through {host}.", member.name),
            Role::Text,
        );
        push(
            match host {
                "pty" => "The built-in host ends the agent program.",
                "tmux" => "Close only a pane TMT opened; a pane you opened stays open.",
                _ => "The host driver handles closing; a pane you opened stays open.",
            }
            .into(),
            Role::Muted,
        );
        push(format!("2. Delete identity {}.", member.name), Role::Text);
        push(
            "Conversation history and notes are kept.".into(),
            Role::Muted,
        );
        push(
            "If stop refuses, nothing is deleted automatically.".into(),
            Role::Muted,
        );
    }
    rows
}
pub(in crate::board::workspace) fn render(frame: &mut Frame, app: &App, draft: &Draft, body: Rect) {
    let close = draft.close.as_ref().expect("close dialog");
    let look = app.look();
    let compact = body.height < 8 || body.width < 34;
    let body = if compact { frame.area() } else { body };
    let labels = if compact {
        vec!["Cancel"]
    } else {
        button_labels(draft)
    };
    let make_dialog = |enabled: bool| Dialog {
        title: format!("Close and delete · {}", draft.member.as_ref().unwrap().name),
        kind: DialogKind::Confirm,
        placement: Placement::Center,
        buttons: labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let enabled = i == 0
                    || (enabled
                        && (i != 1
                            || labels.len() != 3
                            || app
                                .references
                                .agents
                                .get(&draft.member.as_ref().unwrap().id)
                                .is_some_and(|agent| agent.parameters["pane"].as_str().is_some())));
                if i == labels.len() - 1 && labels.len() > 1 {
                    Button::primary(label).enabled(enabled)
                } else {
                    Button::secondary(label).enabled(enabled)
                }
            })
            .collect(),
    };
    let probe = make_dialog(false).areas(body, [90, 30], false);
    let content = if compact {
        vec![("Enlarge to review · Esc close".into(), Role::Muted)]
    } else {
        text(draft, app)
    };
    let lines: Vec<_> = content
        .into_iter()
        .flat_map(|(text, role)| {
            crate::board::notes::wrap(
                &super::super::render::safe(&text),
                usize::from(probe.content.width),
            )
            .into_iter()
            .map(move |line| (line, role))
        })
        .collect();
    let areas = make_dialog(false).areas(
        body,
        [
            90,
            lines.len().saturating_add(4).min(u16::MAX as usize) as u16,
        ],
        false,
    );
    let mut scroll = draft.scroll.borrow_mut();
    scroll.update(areas.content, lines.len(), None);
    let readable = areas.content.width >= 20
        && areas.content.height >= 3
        && scroll
            .offset()
            .saturating_add(usize::from(areas.content.height))
            >= lines.len();
    let dialog = make_dialog(readable);
    let hits = crate::board::buttons::paint_dialog(
        app,
        frame.buffer_mut(),
        &dialog,
        areas,
        &draft.review,
        Overlay::Workspace,
    );
    for (i, (line, role)) in lines
        .iter()
        .skip(scroll.offset())
        .take(usize::from(areas.content.height))
        .enumerate()
    {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(
                areas.content.x,
                areas.content.y + i as u16,
                areas.content.width,
                1,
            ),
            Line::styled(line, look.role(*role)),
        );
    }
    if areas.position.height > 0 && scroll.limit() > 0 {
        strip::paint_left(
            frame.buffer_mut(),
            areas.position,
            Line::styled(
                format!("Read · {}", scroll.position()),
                look.role(Role::Muted),
            ),
        );
    }
    let mut controls = [None; 3];
    for (i, rect) in hits.buttons.iter().enumerate().take(3) {
        controls[i] = *rect;
    }
    draft.controls.set(controls);
    *close.hits.borrow_mut() = hits;
    app.input_band.set(Some(areas.outer));
    app.peek_areas.set(Overlay::Workspace, Some(areas.outer));
}
