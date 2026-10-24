//! A scoped idle-alert editor. Paint/input use held observations; effects use the lane.
mod load;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;
use super::{
    app::{App, Choice, Effect, Menu, MenuEntry},
    workspace::OpsTarget,
};
use crate::config::Config;
pub(in crate::board) use load::{Completed, Task, complete};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    layout::Rect,
};
use std::cell::{Cell, RefCell};
use tmt_tui::components::Editor;

pub(in crate::board) struct Controller {
    owner: u64,
    room: String,
    name: String,
    config: Option<Config>,
    enabled: bool,
    after: Editor,
    focus: usize,
    choice: usize,
    pending: bool,
    notice: Option<String>,
    readable: Cell<bool>,
    hits: RefCell<Vec<(Rect, Control)>>,
    editor_width: Cell<u16>,
}
#[derive(Clone, Copy, Debug)]
enum Control {
    On(bool),
    After,
    Preset(usize),
    Save,
    Cancel,
}
const PRESETS: [&str; 4] = ["15m", "30m", "1h", "2h"];
const INVALID: &str = "✗ invalid · use a time like 20m or 2h";
impl Controller {
    pub fn invalidate(&self) {
        self.hits.borrow_mut().clear();
        self.readable.set(false);
    }
    fn task(&self) -> Task {
        Task {
            owner: self.owner,
            room: self.room.clone(),
            name: self.name.clone(),
            save: None,
        }
    }
    fn draft(&self) -> Result<crate::config::IdleAlert, String> {
        let config = self.config.as_ref().ok_or("◌ Loading idle alert…")?;
        config
            .preview_setting(Some(&self.name), "idle_alert.after", self.after.as_str())
            .and_then(|draft| draft.idle_alert(&self.name))
            .map(|mut setting| {
                setting.enabled = self.enabled;
                setting
            })
            .map_err(|_| INVALID.to_owned())
    }
    fn impact(&self, app: &App) -> Vec<String> {
        let Some(snapshot) = app.workspace_snapshot() else {
            return vec!["? Current activity unavailable.".into()];
        };
        let Some(squad) = snapshot
            .roster
            .squads
            .iter()
            .find(|s| s.room_id == self.room && s.name == self.name)
        else {
            return vec!["✗ Changed · squad no longer observed. Close and reopen.".into()];
        };
        let lead = squad
            .lead
            .as_ref()
            .and_then(|id| snapshot.roster.members.get(id))
            .map(|m| m.name.as_str());
        let mut lines = vec![match lead {
            Some(lead) => format!(
                "Tells *{lead} once when a member stays idle longer than After. Not sent while it waits on you or rests (hold)."
            ),
            None => "? No lead observed; no idle alert can be sent.".into(),
        }];
        if squad.observation != super::workspace::OpsObservation::Complete
            || app.activity_stale(std::time::Instant::now())
        {
            lines.push("? Current activity unavailable; preview kept read-only.".into());
            return lines;
        }
        let alerted: Vec<_> = squad
            .member_states
            .iter()
            .filter(|(_, state)| state.idle_over)
            .filter_map(|(id, _)| snapshot.roster.members.get(id))
            .collect();
        let Some(config) = &self.config else {
            return lines;
        };
        let original = config.idle_alert(&self.name).ok();
        let now = crate::status::now_ms();
        let named = |member: &super::workspace::OpsMember| {
            let age = member
                .activity
                .since_ms
                .filter(|since| *since <= now)
                .map(|since| crate::requests::age(now, since))
                .unwrap_or_else(|| "?".into());
            format!(
                "{}{} ({age})",
                if squad.lead.as_ref() == Some(&member.id) {
                    "*"
                } else {
                    ""
                },
                member.name
            )
        };
        let current = if alerted.is_empty() {
            "◷ No members alerted now.".into()
        } else {
            format!(
                "◷ {} alerted now: {}",
                alerted.len(),
                alerted
                    .iter()
                    .map(|m| named(m))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        match self.draft() {
            Ok(setting) if original != Some(setting) => {
                let after_ms = u64::try_from(setting.after.as_millis()).unwrap_or(u64::MAX);
                let removed: Vec<_> = alerted
                    .iter()
                    .filter(|m| {
                        !setting.enabled
                            || m.activity
                                .since_ms
                                .and_then(|since| now.checked_sub(since))
                                .is_some_and(|age| age < after_ms)
                    })
                    .collect();
                lines.push(if removed.is_empty() {
                    "◷ No currently alerted member leaves the threshold.".into()
                } else {
                    format!(
                        "{}: {} no longer {} as over",
                        if setting.enabled {
                            format!("◷ at {}", self.after)
                        } else {
                            "○ Off".into()
                        },
                        removed
                            .iter()
                            .map(|m| named(m))
                            .collect::<Vec<_>>()
                            .join(", "),
                        if removed.len() == 1 {
                            "counts"
                        } else {
                            "count"
                        }
                    )
                });
            }
            _ => lines.push(current),
        }
        lines
    }
}
impl App {
    pub(in crate::board) fn open_idle_alert(&mut self, room: Option<String>) -> Effect {
        if self.shell.tab != super::shell::OpsTab::Agents
            || self.input.is_some()
            || self.searching
            || self.modal_open() && self.menu.is_none()
        {
            return self.say("Close the current input or overlay before opening Idle alert.");
        }
        if !self.workspace_selection_visible() {
            return self.say("Select a visible squad or member for Idle alert.");
        }
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("? Squad observations unavailable.");
        };
        let scopes: Vec<_> = snapshot
            .roster
            .squads
            .iter()
            .filter(|s| {
                if let Some(room) = &room {
                    return &s.room_id == room;
                }
                match &self.workspace.selected {
                    Some(OpsTarget::Squad(id)) => &s.room_id == id,
                    Some(OpsTarget::Member { identity_id, .. }) if self.flat_agents() => {
                        s.members.contains(identity_id)
                    }
                    Some(OpsTarget::Member {
                        room_id: Some(id), ..
                    }) => &s.room_id == id,
                    Some(OpsTarget::Member {
                        room_id: None,
                        identity_id,
                    }) => s.members.contains(identity_id),
                    _ => false,
                }
            })
            .map(|s| (s.room_id.clone(), s.name.clone()))
            .collect();
        if scopes.len() > 1 {
            self.menu = Some(Menu {
                row_send: None,
                link: None,
                prefill: String::new(),
                title: "Idle alert · choose squad".into(),
                entries: scopes
                    .into_iter()
                    .map(|(room, name)| MenuEntry {
                        key: String::new(),
                        label: name,
                        choice: Choice::IdleAlert(room),
                    })
                    .collect(),
                selected: 0,
                anchor: None,
                surface: RefCell::new(None),
            });
            return Effect::None;
        }
        let Some((room, name)) = scopes.into_iter().next() else {
            return self.say("Common pool has no squad idle-alert setting.");
        };
        self.idle_alert_generation += 1;
        let c = Controller {
            owner: self.idle_alert_generation,
            room,
            name,
            config: None,
            enabled: false,
            after: Editor::default(),
            focus: 0,
            choice: 0,
            pending: true,
            notice: None,
            readable: Cell::new(false),
            hits: RefCell::new(Vec::new()),
            editor_width: Cell::new(12),
        };
        let task = c.task();
        self.idle_alert_editor = Some(c);
        self.menu = None;
        Effect::IdleAlert(task)
    }
    pub(in crate::board) fn finished_idle_alert(&mut self, completed: Completed) -> Effect {
        let Some(c) = &mut self.idle_alert_editor else {
            return Effect::None;
        };
        if c.owner != completed.task.owner {
            return Effect::None;
        }
        c.pending = false;
        if let Some(config) = completed.recovered {
            c.config = Some(*config);
        }
        match completed.result {
            Ok(config) => {
                if completed.task.save.is_some() {
                    c.config = Some(*config);
                    c.notice = Some("✓ Saved · squad idle alert".into());
                    c.invalidate();
                    return Effect::Refresh;
                }
                if let Ok(setting) = config.idle_alert(&c.name) {
                    c.enabled = setting.enabled;
                    c.after = Editor::from(crate::idle_alert::threshold(setting.after));
                    c.config = Some(*config);
                }
            }
            Err(error) => c.notice = Some(format!("✗ Refused · {}", error.message)),
        }
        c.invalidate();
        Effect::None
    }
    pub(in crate::board) fn idle_alert_event(&mut self, event: &Event) -> Option<Effect> {
        let c = self.idle_alert_editor.as_mut()?;
        if matches!(event, Event::Resize(..)) {
            c.invalidate();
            return Some(Effect::None);
        }
        let mut action = None;
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc && (!c.pending || c.config.is_none()) {
                    self.idle_alert_editor = None;
                    return Some(Effect::None);
                }
                if c.pending {
                    return Some(Effect::None);
                }
                match key.code {
                    KeyCode::Up | KeyCode::BackTab => {
                        c.focus = (c.focus + 2) % 3;
                        c.choice = 0;
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        c.focus = (c.focus + 1) % 3;
                        c.choice = 0;
                    }
                    KeyCode::Left | KeyCode::Right => {
                        let back = key.code == KeyCode::Left;
                        if c.focus < 2 {
                            c.notice = None;
                        }
                        match c.focus {
                            0 => c.enabled = !c.enabled,
                            1 => {
                                c.choice = (c.choice + if back { 4 } else { 1 }) % 5;
                                if c.choice > 0 {
                                    c.after = Editor::from(PRESETS[c.choice - 1]);
                                }
                            }
                            _ => c.choice = 1 - c.choice.min(1),
                        }
                    }
                    KeyCode::Enter => {
                        action = Some(if c.focus != 2 || c.choice == 0 {
                            Control::Save
                        } else {
                            Control::Cancel
                        })
                    }
                    _ if c.focus == 1 => {
                        c.choice = 0;
                        c.after.key(*key, c.editor_width.get(), 32);
                        c.notice = None;
                    }
                    _ => {}
                }
            }
            Event::Paste(text) if !c.pending && c.focus == 1 => {
                c.after.insert(text, 32);
                c.choice = 0;
                c.notice = None;
            }
            Event::Mouse(mouse)
                if !c.pending && mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
            {
                let hit = c
                    .hits
                    .borrow()
                    .iter()
                    .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                    .copied();
                if let Some((area, Control::After)) = hit {
                    c.after.place(
                        c.editor_width.get(),
                        0,
                        usize::from(mouse.column.saturating_sub(area.x.saturating_add(1))),
                        false,
                    );
                }
                action = hit.map(|(_, a)| a);
            }
            _ => {}
        }
        if let Some(action) = action {
            match action {
                Control::Cancel => {
                    self.idle_alert_editor = None;
                    return Some(Effect::None);
                }
                Control::On(on) => {
                    c.notice = None;
                    c.enabled = on;
                    c.focus = 0;
                }
                Control::After => {
                    c.focus = 1;
                    c.choice = 0;
                }
                Control::Preset(i) => {
                    c.notice = None;
                    c.focus = 1;
                    c.choice = i + 1;
                    c.after = Editor::from(PRESETS[i]);
                }
                Control::Save => {
                    if !c.readable.get() {
                        c.notice = Some("✗ Refused · enlarge to review the whole form.".into());
                    } else if let Err(error) = c.draft() {
                        c.notice = Some(error);
                    } else {
                        let mut task = c.task();
                        task.save = c
                            .config
                            .clone()
                            .map(|config| (Box::new(config), c.enabled, c.after.to_string()));
                        c.pending = true;
                        c.notice = None;
                        c.invalidate();
                        return Some(Effect::IdleAlert(task));
                    }
                }
            }
        }
        c.invalidate();
        Some(Effect::None)
    }
}
