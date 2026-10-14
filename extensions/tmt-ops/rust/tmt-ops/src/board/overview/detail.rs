//! Read-only full values for a selected monitor identity; no command authority.
use super::{actions, columns::Column, monitor};
use crate::board::{
    app::{App, Effect},
    workspace::OpsTarget,
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    text::Line,
};
use std::cell::RefCell;
use tmt_cli_style::Role;
use tmt_tui::components::{Modal, Placement, ScrollState, strip};

pub(in crate::board) struct Details {
    pub target: OpsTarget,
    lines: Vec<String>,
    scroll: RefCell<ScrollState>,
}
impl Details {
    pub fn invalidate(&self) {
        let content = self.scroll.borrow().content();
        self.scroll
            .borrow_mut()
            .update(Rect::default(), content, None);
    }
    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        let look = app.look();
        let modal = Modal {
            title: "Agent details · observed values at open".into(),
            placement: Placement::Body,
        };
        let areas = modal.areas(body, [body.width, body.height], true, true);
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        let retained = app
            .workspace_snapshot()
            .is_none_or(|snapshot| !snapshot.roster.contains(&self.target));
        let lines: Vec<_> = self
            .lines
            .iter()
            .map(String::as_str)
            .chain(retained.then_some("Target no longer observed · retained read-only values"))
            .flat_map(|line| {
                crate::board::notes::wrap(
                    &crate::board::notes::sanitize(line),
                    usize::from(areas.content.width),
                )
            })
            .collect();
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(areas.content, lines.len(), None);
        for (i, line) in lines
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
                Line::styled(line.clone(), look.role(Role::Text)),
            );
        }
        strip::paint_left(
            frame.buffer_mut(),
            areas.footer,
            Line::styled("↑↓/PgUp/PgDn read · Esc close", look.role(Role::Muted)),
        );
    }
    pub fn input(&self, event: &Event) -> bool {
        self.scroll.borrow_mut().input(event)
    }
}
impl App {
    pub(in crate::board) fn overview_details(&mut self) -> Effect {
        if !self.workspace_selection_visible() {
            return self.say("Select a visible agent before opening its details.");
        }
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        let rows = self.workspace_rows();
        let Some(row) = rows
            .iter()
            .find(|row| Some(&row.target) == self.workspace.selected.as_ref())
        else {
            return Effect::None;
        };
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            return self.say("Select an agent for monitor details.");
        };
        let values = monitor::values(self, snapshot, row);
        let memberships = snapshot
            .roster
            .squads
            .iter()
            .filter(|s| s.members.contains(identity_id))
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut lines = vec![
            format!("Identity UUID: {identity_id}"),
            format!(
                "Observed memberships: {}",
                if memberships.is_empty() {
                    "none"
                } else {
                    &memberships
                }
            ),
        ];
        for column in Column::ALL {
            let value = if column == Column::Cron {
                self.cron_glyphs.label(values.cron)
            } else {
                values.text(column, row)
            };
            lines.push(format!("{}: {value}", column.title()));
        }
        let target = crate::board::app::RowTarget::Workspace(row.target.clone());
        if let Some(raw) = self.target_row(&target) {
            let bindings = self.bindings();
            let host_action = bindings.get("j").map(|action| ("j", action)).or_else(|| {
                bindings
                    .iter()
                    .find(|(_, action)| action.verb == crate::action::Verb::Jump)
                    .map(|(key, action)| (key.as_str(), action))
            });
            lines.push(actions::host_preview(raw, self.invoker_tmux, host_action));
            let scope = match self.workspace_scope() {
                crate::config::OpsKeyScope::Overview => "Overview",
                crate::config::OpsKeyScope::Member => "selected squad member",
                crate::config::OpsKeyScope::Squad => "selected squad",
                crate::config::OpsKeyScope::Pool => "Common pool",
                crate::config::OpsKeyScope::Reader => "conversation reader",
            };
            lines.push(format!(
                "Binding scope: {scope}; configuration: {}",
                snapshot.config_path
            ));
            lines.push(
                "Open executes the observed binding; management instructions below do not execute."
                    .into(),
            );
            for (key, action) in self.bindings().iter().filter(|(_, action)| {
                matches!(
                    action.verb,
                    crate::action::Verb::Open | crate::action::Verb::Run
                )
            }) {
                let preview = actions::link_preview(
                    action,
                    raw,
                    self.view.as_ref().and_then(|view| view.opener.as_deref()),
                )
                .unwrap_or_else(|error| format!("Unavailable: {error}"));
                let source = self
                    .workspace_binding_sources()
                    .and_then(|sources| sources.get(key))
                    .map(String::as_str)
                    .unwrap_or("source unavailable");
                lines.push(format!(
                    "{key}: {} · {preview} · source {source}",
                    action.text
                ));
                if source != "source unavailable" && !snapshot.config_path.is_empty() {
                    let table = if source == "workspace default" {
                        "bind"
                    } else {
                        source.strip_suffix(&format!(".{key}")).unwrap_or(source)
                    };
                    lines.push(format!(
                        "Manage binding in {table} of {}: {} = {}; reload before acting.",
                        snapshot.config_path,
                        serde_json::to_string(key).unwrap(),
                        serde_json::to_string(&action.text).unwrap(),
                    ));
                } else {
                    lines.push(
                        "Binding management source unavailable; no edit target inferred.".into(),
                    );
                }
            }
            if let OpsTarget::Member {
                room_id: Some(room),
                ..
            } = &row.target
                && let Some(squad) = snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| &squad.room_id == room)
            {
                lines.push(format!(
                    "Link fields scope: squad {} · room {room} · member {identity_id}",
                    squad.name
                ));
                lines.push(format!(
                    "Manage link (argv; replace field=value): {:?}",
                    [
                        "tmt",
                        "ops",
                        "sq",
                        "set",
                        identity_id,
                        "field=value",
                        "--squad",
                        squad.name.as_str()
                    ]
                ));
            } else {
                lines.push("Common pool: no squad link-management scope; assign a squad before using sq set.".into());
            }
            for (name, value) in raw["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(name, _)| name.as_str() == "link" || name.ends_with("_link"))
            {
                if let Some(value) = value.as_str() {
                    lines.push(format!("{name}: {value}"));
                }
            }
        }
        self.monitor_details = Some(Details {
            target: row.target.clone(),
            lines,
            scroll: Default::default(),
        });
        Effect::None
    }
    pub(in crate::board) fn monitor_details_event(&mut self, event: &Event) -> Option<Effect> {
        if matches!(event,Event::Key(key) if key.kind!=KeyEventKind::Release && key.code==KeyCode::Esc&&!key.modifiers.intersects(KeyModifiers::CONTROL|KeyModifiers::ALT))
        {
            self.monitor_details = None;
        } else {
            self.monitor_details.as_ref()?.input(event);
        }
        Some(Effect::None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::shell::OpsTab;
    #[test]
    fn full_detail_retains_hidden_values_and_is_read_only_after_target_changes() {
        let mut app = crate::board::app::workspace::tests::board();
        app.shell.tab = OpsTab::Overview;
        assert_eq!(app.overview_details(), Effect::None);
        let details = app.monitor_details.as_ref().unwrap();
        assert!(details.lines.iter().any(|line| line == "Runtime: —"));
        assert!(details.lines.iter().any(|line| line == "Model: —"));
        let target = details.target.clone();
        app.workspace.selected = None;
        assert_eq!(app.monitor_details.as_ref().unwrap().target, target);
        let event = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.monitor_details_event(&event), Some(Effect::None));
        assert!(app.monitor_details.is_none());
    }
    #[test]
    fn details_uses_the_renderers_injected_ascii_count_words() {
        let mut app = crate::board::app::workspace::tests::board();
        app.shell.tab = OpsTab::Overview;
        app.cron_glyphs = super::super::cron_counts::GlyphSet::from_ascii(true);
        assert_eq!(app.overview_details(), Effect::None);
        let lines = &app.monitor_details.as_ref().unwrap().lines;
        assert!(lines.iter().any(|line| line == "Cron: CRON unknown"));
        assert!(lines.iter().all(|line| !line.contains('◷')));
    }
}
