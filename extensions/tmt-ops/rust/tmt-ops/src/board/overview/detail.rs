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
        let mut modal = Modal {
            title: "Agent details · observed values at open".into(),
            placement: Placement::Body,
        };
        let areas = modal.areas(body, [body.width, body.height], true, true);
        if areas.content.height == 0 {
            modal.title = "Enlarge terminal · Esc closes agent details".into();
        }
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.peek_areas
            .set(crate::board::peek::Peek::Monitor, areas.outer);
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
            } else if column == Column::State
                && let Some(label) =
                    self.activity_label_at(row, std::time::Instant::now(), crate::status::now_ms())
            {
                let (head, task) = label.fit(usize::MAX);
                head + task.as_deref().unwrap_or("")
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
                crate::config::OpsKeyScope::Reader | crate::config::OpsKeyScope::OverviewReader => {
                    "conversation reader"
                }
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
        lines.extend(self.quote_provenance(identity_id));
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
    fn quote_details_freeze_provenance_without_changing_draft_focus_or_selection() {
        let mut app = crate::board::app::workspace::tests::board();
        app.key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        app.paste("kept draft");
        let id = crate::board::app::workspace::tests::MEMBER;
        app.input
            .as_mut()
            .unwrap()
            .response_quotes
            .push(crate::board::response::Quote {
                key: crate::board::home_leads::MessageKey {
                    sender: "owner".into(),
                    lead: id.into(),
                    squad: "team".into(),
                    request: "req-frozen-quote".into(),
                    kind: crate::board::home_leads::Kind::Reply,
                },
                range: 0..5,
                text: "quote".into(),
            });
        let before = app.input.as_ref().unwrap().response_quotes.clone();
        let selected = app.workspace.selected.clone();
        let focus = app.focus;
        assert_eq!(app.overview_details(), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().response_quotes, before);
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "kept draft");
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(app.focus, focus);
        let frozen = app.monitor_details.as_ref().unwrap().lines.clone();
        assert!(frozen.iter().any(|line| line.contains("req-frozen-quote")
            && line.contains(id)
            && line.contains("bytes 0..5")));
        app.input.as_mut().unwrap().response_quotes.clear();
        assert_eq!(app.monitor_details.as_ref().unwrap().lines, frozen);
    }
    #[test]
    fn full_detail_retains_hidden_values_and_is_read_only_after_target_changes() {
        let mut app = crate::board::app::workspace::tests::board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
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
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Overview;
        app.cron_glyphs = super::super::cron_counts::GlyphSet::from_ascii(true);
        assert_eq!(app.overview_details(), Effect::None);
        let lines = &app.monitor_details.as_ref().unwrap().lines;
        assert!(lines.iter().any(|line| line == "Cron: CRON unknown"));
        assert!(lines.iter().all(|line| !line.contains('◷')));
    }
    #[test]
    fn a_zero_content_viewport_explains_how_to_read_and_close_details() {
        let mut app = crate::board::app::workspace::tests::board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Overview;
        assert_eq!(app.overview_details(), Effect::None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 8)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Enlarge terminal"));
        assert!(text.contains("Esc closes agent details"));
        let escape = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.monitor_details_event(&escape), Some(Effect::None));
        assert!(app.monitor_details.is_none());
    }
    #[test]
    fn details_retains_complete_observed_metadata_and_literal_idn_link() {
        let mut app = crate::board::app::workspace::tests::board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Overview;
        let target = app.workspace.selected.clone().unwrap();
        let OpsTarget::Member { identity_id, .. } = &target else {
            panic!("member")
        };
        let model = "model-界界-with-a-long-observed-label";
        let link = "https://例.test/a/very/long/path?literal=$(id)&value=unchanged";
        app.references = crate::board::inventory::Inventory::from_listed(
            &[],
            &Ok(serde_json::json!({
                "identities":[{"id":identity_id,"name":"worker","runningDriver":"claude","resume":{"model":model}}]
            })),
        );
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
            .unwrap()["fields"]["pr_link"] = serde_json::json!(link);
        assert_eq!(app.overview_details(), Effect::None);
        let lines = &app.monitor_details.as_ref().unwrap().lines;
        assert!(lines.iter().any(|line| line == &format!("Model: {model}")));
        assert!(lines.iter().any(|line| line == &format!("pr_link: {link}")));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("o: open · ") && line.contains(link))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Manage link (argv; replace field=value)"))
        );
    }

    #[test]
    fn details_quote_provenance_is_available_while_the_action_menu_is_open() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = crate::board::app::workspace::tests::board();
        let id = crate::board::app::workspace::tests::MEMBER;
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.paste("retained draft");
        let sender = app
            .input
            .as_ref()
            .unwrap()
            .draft_key
            .as_ref()
            .unwrap()
            .0
            .clone();
        app.input
            .as_mut()
            .unwrap()
            .response_quotes
            .push(crate::board::response::Quote {
                key: crate::board::home_leads::MessageKey {
                    sender,
                    lead: id.into(),
                    squad: "team".into(),
                    request: "req-menu-quote-source".into(),
                    kind: crate::board::home_leads::Kind::Reply,
                },
                range: 0..6,
                text: "source".into(),
            });
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _no_acquisition = crate::runner::forbid_commands();
        assert_eq!(
            app.perform(&crate::action::Action::parse("menu").unwrap()),
            Effect::None
        );
        assert!(app.menu.is_some() && app.modal_open());
        assert_eq!(app.overview_details(), Effect::None);
        let lines = &app.monitor_details.as_ref().unwrap().lines;
        assert!(
            lines
                .iter()
                .any(|line| line.contains("req-menu-quote-source")
                    && line.contains(id)
                    && line.contains("bytes 0..6"))
        );
        assert_eq!(
            app.composer_projection().unwrap().text.unwrap().as_str(),
            "retained draft"
        );
    }
}
