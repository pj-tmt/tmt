//! Held row-field adapter. It acquires nothing and never manufactures a request
//! source: task quotations remain plain text in the existing chat draft.
use super::{App, Quote, RowTarget, Target};
use crate::board::{app::Effect, workspace::OpsTarget};
use ratatui::{
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board) enum Field {
    TaskAndNext,
    OverviewTask,
    Task,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Key {
    pub target: RowTarget,
    pub sender: Option<String>,
    pub field: Field,
    pub text: String,
}

/// One held-field projection feeds both paint and source validation.
pub(in crate::board) fn text(row: &serde_json::Value, field: Field) -> Option<String> {
    match field {
        Field::Task => row["fields"]["task"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        Field::OverviewTask => crate::board::overview::monitor::task_source(row)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        Field::TaskAndNext => {
            let task = row["fields"]["task"].as_str().unwrap_or("");
            let next = row["fields"]["next"].as_str().unwrap_or("");
            match (task.is_empty(), next.is_empty()) {
                (true, true) => None,
                (false, true) => Some(task.into()),
                (true, false) => Some(next.into()),
                _ => Some(format!("{task} → {next}")),
            }
        }
    }
}

impl App {
    pub(super) fn task_quote_current(&self, target: &Target, quote: &Quote<super::Origin>) -> bool {
        let super::Origin::Task(key) = &quote.key else {
            return false;
        };
        target == &Target::Row(key.target.clone())
            && match &key.target {
                RowTarget::Workspace(target) => self
                    .workspace_snapshot()
                    .is_some_and(|snapshot| snapshot.roster.contains(target)),
                _ => false,
            }
            && self.view.as_ref().and_then(|view| view.me_id.clone()) == key.sender
            && self
                .target_row(&key.target)
                .and_then(|row| text(row, key.field))
                .as_deref()
                == Some(key.text.as_str())
            && key.text.get(quote.range.clone()) == Some(quote.text.as_str())
    }
}

#[derive(Clone)]
pub(in crate::board) struct Hit {
    pub(super) area: Rect,
    target: OpsTarget,
    sender: Option<String>,
    field: Field,
    offset: usize,
}
pub(in crate::board) struct Drag {
    hit: Hit,
    key: Key,
}

impl Field {
    fn pane(self) -> crate::config::Pane {
        match self {
            Self::TaskAndNext | Self::Task => crate::config::Pane::Replies,
            Self::OverviewTask => crate::config::Pane::Rows,
        }
    }
}

pub(in crate::board) fn begin_frame(app: &App) {
    app.conversation.borrow_mut().task_hits.clear();
}

/// Frame-local field geometry has no byte-range or request-source authority.
pub(in crate::board) fn publish(app: &App, target: &OpsTarget, field: Field, area: Rect) {
    if area.is_empty() || !matches!(target, OpsTarget::Member { .. }) {
        return;
    }
    app.conversation.borrow_mut().task_hits.push(Hit {
        area,
        target: target.clone(),
        sender: app.view.as_ref().and_then(|view| view.me_id.clone()),
        field,
        offset: app.scrolls.offset(field.pane()),
    });
}

impl App {
    fn task_drag_current(&self, drag: &Drag) -> bool {
        drag.hit.offset == self.scrolls.offset(drag.hit.field.pane())
            && self.view.as_ref().and_then(|view| view.me_id.clone()) == drag.key.sender
            && self
                .workspace_snapshot()
                .is_some_and(|snapshot| snapshot.roster.contains(&drag.hit.target))
            && self
                .target_row(&drag.key.target)
                .and_then(|row| text(row, drag.key.field))
                .as_deref()
                == Some(drag.key.text.as_str())
    }

    pub(in crate::board) fn task_source_mouse(&mut self, event: MouseEvent) -> Option<Effect> {
        if self.searching || self.input.is_some() || self.modal_open() {
            return None;
        }
        let point = (event.column, event.row).into();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .conversation
                    .borrow()
                    .task_hits
                    .iter()
                    .find(|hit| hit.area.contains(point))
                    .cloned()?;
                if hit.offset != self.scrolls.offset(hit.field.pane())
                    || hit.sender != self.view.as_ref().and_then(|view| view.me_id.clone())
                {
                    return Some(Effect::None);
                }
                let target = RowTarget::Workspace(hit.target.clone());
                let text = text(self.target_row(&target)?, hit.field)?;
                if text.len() > crate::board::app::MESSAGE_LIMIT {
                    return Some(self.say("Task source exceeds 1 MiB; selection unavailable."));
                }
                self.conversation.borrow_mut().task_drag = Some(Drag {
                    key: Key {
                        target,
                        sender: hit.sender.clone(),
                        field: hit.field,
                        text,
                    },
                    hit,
                });
                Some(Effect::None)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let drag = self.conversation.borrow_mut().task_drag.take()?;
                if !drag.hit.area.contains(point) || !self.task_drag_current(&drag) {
                    return Some(Effect::None);
                }
                self.choose(crate::board::app::Choice::WorkspaceTarget(
                    drag.hit.target.clone(),
                ));
                if self.workspace_message_target().as_ref() != Some(&drag.hit.target) {
                    return Some(Effect::None);
                }
                let (identity, target) = self.receiving_conversation()?;
                let source = drag.key.text.clone();
                let mut state = self.conversation.borrow_mut();
                let session = state.sessions.entry(identity).or_default();
                session.exact_source = Some(super::Source {
                    target: Target::Row(target),
                    selection: crate::board::response::Selection::new(
                        super::Origin::Task(drag.key),
                        source,
                    ),
                    anchor: super::Anchor::Byte(0),
                    layout_width: None,
                    painted: None,
                    quote_button: None,
                    copy_pending: false,
                });
                session.follow = false;
                drop(state);
                self.focus = 1;
                self.invalidate_frame_geometry();
                self.scrolls.restore([(crate::config::Pane::Replies, 0)]);
                Some(Effect::None)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let drag = self.conversation.borrow_mut().task_drag.take()?;
                if self.task_drag_current(&drag) && drag.hit.area.contains(point) {
                    Some(self.choose(crate::board::app::Choice::WorkspaceTarget(drag.hit.target)))
                } else {
                    Some(Effect::None)
                }
            }
            _ => None,
        }
    }
}
