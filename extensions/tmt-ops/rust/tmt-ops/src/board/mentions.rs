//! Plain-text reference completion over acquired board data. No lookup performs
//! IO and choosing a candidate never changes the composer's recipient.
use super::app::{App, Compose, Effect, MESSAGE_LIMIT};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    text::{Line, Span},
};
use std::cell::RefCell;
use tmt_cli_style::Role;
use tmt_tui::components::{Completion, Editor, strip};

/// Chosen mentions use this prefix: `@name` reads as a file reference to Claude Code.
pub(super) const TRIGGER: &str = "tmt/";
pub(super) const TRIGGER_HINT: &str = "@ / tmt/";
/// A chosen mention is an expanded fold over its `tmt/<name>` text whose label holds the
/// agent's identity, so a rename never retargets it; editing the text drops the fold.
const IDENTITY: &str = "tmt-mention:";

/// The identity a chosen mention's fold stands for.
pub(super) fn mentioned(fold: &tmt_tui::components::Fold) -> Option<&str> {
    fold.label.strip_prefix(IDENTITY)
}

#[derive(Default)]
pub(super) struct State {
    pub completion: Completion,
    hits: RefCell<Vec<(Rect, usize)>>,
}

impl State {
    pub(super) fn invalidate(&self) {
        self.hits.borrow_mut().clear();
    }
}

impl App {
    pub(super) fn reference_editor(&self) -> Option<&Editor> {
        if matches!(self.input.as_ref()?.compose, Compose::Status) {
            self.status_draft
                .as_ref()
                .filter(|draft| draft.editable_reason())
                .map(|draft| &draft.reason)
        } else {
            Some(&self.input.as_ref()?.text)
        }
    }
    pub(super) fn reference_editor_mut(&mut self) -> Option<&mut Editor> {
        if matches!(self.input.as_ref()?.compose, Compose::Status) {
            self.status_draft
                .as_mut()
                .filter(|draft| draft.editable_reason())
                .map(|draft| &mut draft.reason)
        } else {
            Some(&mut self.input.as_mut()?.text)
        }
    }
    fn accept_reference(&mut self) -> bool {
        let mut completion = std::mem::take(&mut self.mentions.completion);
        let status_reason = self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status));
        let limit = if self.message_field() && !status_reason {
            MESSAGE_LIMIT
        } else {
            super::app::INPUT_LIMIT
        };
        let accepted = completion.chosen().is_some_and(|(range, candidate)| {
            self.reference_editor_mut()
                .is_some_and(|editor| insert_mention(editor, range, &candidate, limit))
        });
        if accepted {
            completion.close();
        }
        self.mentions.completion = completion;
        accepted
    }
    pub(super) fn message_field(&self) -> bool {
        self.input
            .as_ref()
            .is_some_and(|input| match input.compose {
                Compose::Talk { .. } | Compose::Annotate { .. } | Compose::AskLead { .. } => true,
                Compose::Status => self.reference_editor().is_some(),
                Compose::Cron => self
                    .cron_draft
                    .as_ref()
                    .is_some_and(|draft| draft.message_field()),
                _ => false,
            })
    }
    pub(super) fn update_mentions(&mut self) {
        if !self.message_field() && !self.workspace_reference_field() {
            self.mentions = State::default();
            return;
        }
        let mut completion = std::mem::take(&mut self.mentions.completion);
        if let Some(editor) = self.reference_editor() {
            // Select the trigger only; Completion owns token boundaries, folds and dismissal.
            let token = editor.as_str()[..editor.cursor()]
                .rsplit(char::is_whitespace)
                .next()
                .unwrap_or_default();
            let trigger = if token.starts_with('@') { "@" } else { TRIGGER };
            completion.update(editor, &self.references.candidates, trigger);
        }
        self.mentions.completion = completion;
        self.mentions.hits.borrow_mut().clear();
    }
    /// A mentioned agent's current name, from the held identity inventory.
    pub(super) fn mention_name(&self, identity: &str) -> Option<String> {
        self.references
            .agents
            .get(identity)
            .map(|agent| agent.mention().to_owned())
    }

    /// A picker row's squad (`· lead` for its lead) and STATE cell, from the snapshot.
    fn mention_detail(&self, candidate: &str) -> (String, Option<super::activity::Label>) {
        let (Some(id), Some(snapshot)) =
            (candidate.strip_prefix("agent:"), self.workspace_snapshot())
        else {
            return (String::new(), None);
        };
        let squad = snapshot
            .roster
            .squads
            .iter()
            .find(|squad| squad.members.iter().any(|member| member == id));
        let label = snapshot.roster.members.get(id).map(|member| {
            let state = squad.and_then(|squad| squad.member_states.get(id));
            super::activity::Member {
                activity: member.activity,
                waiting: state.map_or(member.waiting, |state| state.waiting),
                idle_over: state.is_some_and(|state| state.idle_over),
                task: None,
            }
            .label(
                self.activity_stale(std::time::Instant::now()),
                crate::status::now_ms(),
            )
        });
        // The age ticks with the frame clock, like the Agents STATE cell.
        if let Some(label) = &label {
            self.admit_activity_clock(label.at);
        }
        let squad = squad.map_or_else(String::new, |squad| {
            let name = super::notes::single_line(&squad.name);
            if squad.lead.as_deref() == Some(id) {
                format!("{name} · lead")
            } else {
                name
            }
        });
        (squad, label)
    }

    pub(super) fn mention_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if !self.mentions.completion.open() {
            return None;
        }
        match key.code {
            KeyCode::Up => self.mentions.completion.move_by(-1),
            KeyCode::Down => self.mentions.completion.move_by(1),
            KeyCode::PageUp => self.mentions.completion.move_by(-6),
            KeyCode::PageDown => self.mentions.completion.move_by(6),
            KeyCode::Enter | KeyCode::Tab => {
                self.accept_reference();
            }
            KeyCode::Esc => {
                let mut completion = std::mem::take(&mut self.mentions.completion);
                if let Some(editor) = self.reference_editor() {
                    completion.dismiss(editor);
                }
                self.mentions.completion = completion;
            }
            _ => return None,
        }
        Some(Effect::None)
    }
    pub(super) fn mention_mouse(&mut self, event: MouseEvent) -> bool {
        if !self.mentions.completion.open() {
            return false;
        }
        let hit = self
            .mentions
            .hits
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains((event.column, event.row).into()))
            .map(|(_, index)| *index);
        let Some(index) = hit else {
            return false;
        };
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.mentions.completion.selected = index;
                self.accept_reference();
            }
            MouseEventKind::ScrollUp => self.mentions.completion.move_by(-1),
            MouseEventKind::ScrollDown => self.mentions.completion.move_by(1),
            _ => {}
        }
        true
    }
}

/// Replace the typed token with the candidate's `tmt/<name>` as a mention fold, then a
/// space, all or nothing.
fn insert_mention(
    editor: &mut Editor,
    range: std::ops::Range<usize>,
    candidate: &tmt_tui::components::Candidate,
    limit: usize,
) -> bool {
    let Some(identity) = candidate.id.strip_prefix("agent:") else {
        return false;
    };
    let mut next = editor.clone();
    let start = range.start;
    if !next.replace_range(range, "", limit)
        || !next.insert_folded(&candidate.insertion, format!("{IDENTITY}{identity}"), limit)
    {
        return false;
    }
    let Some(id) = next
        .folds()
        .iter()
        .find(|fold| fold.source.start == start && mentioned(fold).is_some())
        .map(|fold| fold.id)
    else {
        return false;
    };
    if !next.set_expanded(id, true) || !next.insert(" ", limit) {
        return false;
    }
    *editor = next;
    true
}

pub(super) fn render(frame: &mut Frame, app: &App) {
    app.mentions.hits.borrow_mut().clear();
    let completion = &app.mentions.completion;
    if !completion.open() {
        return;
    }
    let Some((editor, _)) = app.input_viewport.get() else {
        return;
    };
    let screen = frame.area();
    let count = completion.matches.len().clamp(1, 6);
    let height = (count as u16 + 1).min(screen.height);
    let anchor = app.input_band.get().unwrap_or(editor).intersection(screen);
    let y = if screen.bottom().saturating_sub(anchor.bottom()) >= height {
        anchor.bottom()
    } else if anchor.y.saturating_sub(screen.y) >= height {
        anchor.y - height
    } else {
        editor.bottom().min(screen.bottom().saturating_sub(height))
    };
    let width = anchor.width.min(68);
    let x = anchor.x.min(screen.right().saturating_sub(width));
    let look = app.look();
    let start = completion.selected.saturating_sub(count - 1);
    let name_cells = completion
        .matches
        .iter()
        .map(|candidate| unicode_width::UnicodeWidthStr::width(candidate.label.as_str()))
        .max()
        .unwrap_or(0)
        .min(24);
    let squad_cells = 20;
    for row in 0..height {
        let rect = Rect::new(x, y + row, width, 1);
        let line = if row == height - 1 {
            let recipient = app.input.as_ref().and_then(|input| match &input.compose {
                Compose::Talk { to }
                | Compose::AskLead { to, .. }
                | Compose::Annotate { to, .. } => Some(to.as_str()),
                _ => None,
            });
            let sent = recipient.map_or_else(String::new, |to| format!(", sent to {to}"));
            // A narrow picker drops the key hints before it cuts the recipient.
            let info = [
                format!("Agents · {TRIGGER_HINT} · ↑↓ choose · Tab insert · text only{sent}"),
                format!("Agents · {TRIGGER_HINT} · Tab insert · text only{sent}"),
                format!("{TRIGGER_HINT}  Tab insert  text only{sent}"),
                format!("text only{sent}"),
            ]
            .into_iter()
            .find(|info| unicode_width::UnicodeWidthStr::width(info.as_str()) <= usize::from(width))
            .unwrap_or_else(|| format!("text only{sent}"));
            Line::styled(info, look.role(Role::Muted))
        } else if let Some(candidate) = completion.matches.get(start + usize::from(row)) {
            let selected = start + usize::from(row) == completion.selected;
            let style = if selected {
                look.role(Role::Text).patch(look.role(Role::Track))
            } else {
                look.role(Role::Text)
            };
            app.mentions
                .hits
                .borrow_mut()
                .push((rect, start + usize::from(row)));
            let mut spans = vec![Span::styled(
                format!("{} {TRIGGER}", if selected { "›" } else { " " }),
                style,
            )];
            let mut at = 0;
            for matched in completion.matched_ranges(start + usize::from(row)) {
                spans.push(Span::styled(&candidate.label[at..matched.start], style));
                spans.push(Span::styled(
                    &candidate.label[matched.clone()],
                    style.add_modifier(ratatui::style::Modifier::BOLD),
                ));
                at = matched.end;
            }
            spans.push(Span::styled(&candidate.label[at..], style));
            // The name column, then the agent's squad and its STATE cell, as in Agents.
            let shown = unicode_width::UnicodeWidthStr::width(candidate.label.as_str());
            spans.push(Span::styled(
                " ".repeat(name_cells.saturating_sub(shown) + 2),
                style,
            ));
            let (squad, state) = app.mention_detail(&candidate.id);
            spans.push(Span::styled(
                format!("{squad:<squad_cells$}  "),
                style.patch(look.role(Role::Dim)),
            ));
            if let Some(label) = state {
                let mut mark = crate::board::state_cell::mark(Some(label.state), look);
                mark.style = style.patch(mark.style);
                spans.push(mark);
                spans.push(Span::styled(
                    format!(
                        " {}{}",
                        label.state,
                        label.age.map(|age| format!(" {age}")).unwrap_or_default()
                    ),
                    style.patch(look.role(Role::Muted)),
                ));
            }
            Line::from(spans)
        } else {
            Line::styled(
                if app.references.error.is_some() {
                    "Agent inventory unavailable"
                } else {
                    "No matching agent"
                },
                look.role(Role::Muted),
            )
        };
        // Explicit opaque cell fill prevents underlying pane text leaking into
        // short candidates; strip owns escaping and cell clipping.
        for col in rect.x..rect.right() {
            let cell = &mut frame.buffer_mut()[(col, rect.y)];
            cell.reset();
            if row != height - 1
                && start + usize::from(row) == completion.selected
                && !completion.matches.is_empty()
            {
                cell.set_style(look.role(Role::Text).patch(look.role(Role::Track)));
            }
        }
        strip::paint_left(frame.buffer_mut(), rect, line);
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
