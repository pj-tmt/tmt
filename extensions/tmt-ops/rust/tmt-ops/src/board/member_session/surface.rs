//! Shared modal geometry, scrolling and currently painted hit ownership.
use super::*;
use crate::look::Look;
use ratatui::{
    Frame,
    crossterm::event::{Event, MouseButton, MouseEventKind},
    layout::Rect,
};
use serde_json::json;
use std::sync::OnceLock;
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput};
const FILE: &str = "ops.member-session.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="member-session" title="Session" placement="center" class="w-110 h-32"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" wrap="true"/></tmt-repeat><tmt-repeat each="$.recovery" as="note"><tmt-text id-bind="note.id" bind="note.text" token="muted" wrap="true"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty=""><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" wrap="true"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" wrap="true"/></tmt-modal></tmt-view>"#;
fn safe(text: &str) -> String {
    super::super::notes::single_line(text)
}
impl Controller {
    fn rows(&self) -> Vec<(&'static str, &'static str, bool)> {
        if self.confirmation.is_some() {
            vec![
                ("cancel", "Cancel", false),
                ("confirm", "Close board and continue", false),
            ]
        } else {
            let unavailable =
                !self.task.evidence.remembered() || !matches!(self.loaded, Some(Ok(_)));
            if self.task.evidence.is_unbound() {
                return vec![
                    ("copy", "Copy command", unavailable),
                    ("close", "Close", false),
                ];
            }
            vec![
                ("resume", "Resume here…", unavailable),
                ("move", "Move here…", unavailable),
                ("close", "Close", false),
            ]
        }
    }
    pub(super) fn list_rows(&self) -> Vec<ListRow> {
        self.rows()
            .iter()
            .map(|(id, _, disabled)| ListRow {
                id: (*id).into(),
                disabled: *disabled,
            })
            .collect()
    }
    pub(super) fn choice(&self, event: &Event) -> Option<String> {
        let mut ui = self.ui.borrow_mut();
        ui.reconcile(self.list_rows());
        let clicked = if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            ui.frame
                .as_ref()
                .and_then(|m| m.list.as_ref())
                .filter(|frame| {
                    frame.offset == ui.picker.list.scroll.offset()
                        && frame.viewport.contains((mouse.column, mouse.row).into())
                })
                .and_then(|frame| {
                    frame
                        .geometry
                        .iter()
                        .find(|r| r.visible.contains((mouse.column, mouse.row).into()))
                })
                .and_then(|r| {
                    self.rows()
                        .into_iter()
                        .find(|(id, _, disabled)| *id == r.id && !disabled)
                })
                .map(|(id, _, _)| id.to_owned())
        } else {
            None
        };
        let input = ui.input(event, PickerField::List);
        clicked.or(match input {
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => Some(id),
            _ => None,
        })
    }
    fn notes(&self, app: &App) -> Vec<String> {
        let OpsTarget::Member { identity_id, .. } = &self.task.target else {
            unreachable!()
        };
        if let Some(command) = &self.confirmation {
            let mut lines = vec![
                format!("Identity UUID: {identity_id}"),
                "Destination: this board pane. The board closes here before the command starts.".into(),
                format!("Command: {}", command.line()),
                "Core checks the remembered session again and may refuse. No automatic retry or fresh launch.".into(),
            ];
            if command.args.iter().any(|a| a == "--move") {
                lines.push("Move may stop the verified old agent: one TERM, up to three seconds, no force-kill.".into());
            }
            return lines;
        }
        let mut lines = vec![format!("Identity UUID: {identity_id}")];
        let mut evidence = self.task.evidence.clone();
        if let Some(Err(error)) = &self.loaded {
            evidence.error = Some(error.message.clone());
        }
        lines.extend(evidence.lines(
            crate::status::now_ms(),
            app.member_last_activity(identity_id),
        ));
        match &self.loaded {
            None => {
                lines.push("Loading session… you can cancel while the public read finishes.".into())
            }
            Some(Err(_)) => {}
            Some(Ok(observed)) => lines.push(match &observed.preset {
                Some(executable) => {
                    format!("Launch preset: {executable} (preset only; not current resumability)")
                }
                None => {
                    "Launch preset: none associated; Core checks the registered driver at launch."
                        .into()
                }
            }),
        }
        if let Some(command) = self.recovery_command() {
            lines.push(format!("Command: {command}"));
        }
        lines
    }
    pub fn render(&self, frame: &mut Frame, app: &App, look: Look, body: Rect) {
        static TEMPLATE: OnceLock<tmt_tui::components::surface::Template<()>> = OnceLock::new();
        static UNBOUND_TEMPLATE: OnceLock<tmt_tui::components::surface::Template<()>> =
            OnceLock::new();
        let unbound = self.task.evidence.is_unbound();
        let template = if unbound {
            &UNBOUND_TEMPLATE
        } else {
            &TEMPLATE
        }
        .get_or_init(|| {
            let mut schema = picker_surface::schema(&["cursor", "label"]);
            let tmt_tui::binding::Schema::Object(fields) = &mut schema else {
                unreachable!("picker scene schema");
            };
            let notes = fields["notes"].clone();
            fields.insert("recovery".into(), notes);
            let markup = if unbound {
                MARKUP.replace("class=\"w-110 h-32\"", "class=\"w-110\"")
            } else {
                MARKUP.to_owned()
            };
            picker_surface::compile(FILE, &markup, schema)
        });
        let mut ui = self.ui.borrow_mut();
        ui.reconcile(self.list_rows());
        let notes: Vec<_> = self.notes(app).iter().map(|s| safe(s)).collect();
        let projected_notes: Vec<_> = notes
            .iter()
            .enumerate()
            .map(|(i, text)| json!({"id":format!("note:{i}"),"text":text}))
            .collect();
        let recovery: Vec<_> = self
            .task
            .evidence
            .is_unbound()
            .then(|| json!({"id":"recovery:unbound", "text":"Run it in the agent's pane"}))
            .into_iter()
            .collect();
        let rows: Vec<_> = self.rows().iter().map(|(id, label, disabled)| json!({"id":id,"label":label,"disabled":disabled,"cursor":if ui.picker.list.selected()==Some(*id) {"›"}else{" "}})).collect();
        let short = frame.area().height < 16;
        let footer = if short && self.confirmation.is_some() {
            "Esc cancel · c copy command · enlarge to continue"
        } else if self.confirmation.is_some() {
            "Esc cancel · c copy command · ↑↓ select · Enter continue"
        } else if self.task.evidence.is_unbound() {
            "↑↓ select · Enter copy · Esc close"
        } else {
            "↑↓ select · Enter review · Esc close"
        };
        let status = self.status.as_deref().unwrap_or(if short {
            if self.loaded.is_none() {
                "Loading session…"
            } else {
                "Enlarge terminal to read session details."
            }
        } else {
            ""
        });
        let body = if frame.area().height < 16 {
            frame.area()
        } else {
            body
        };
        ui.render(FILE, template, json!({"rows":rows,"notes":projected_notes,"recovery":recovery,"query":"","status":safe(status),"footer":footer}), frame, look, body);
        self.readable.set(
            self.confirmation.is_some()
                && ui.frame.as_ref().is_some_and(|map| {
                    map.areas.content.width >= 38
                        && notes.iter().enumerate().all(|(i, text)| {
                            let id = format!("note:{i}");
                            let height = tmt_tui::text::measure(
                                text,
                                tmt_tui::style::TextFlow::Wrap,
                                tmt_tui::geometry::Space::Cells(map.areas.content.width),
                            )[1];
                            map.hits
                                .iter()
                                .any(|hit| hit.id.last() == Some(&id) && hit.rect.height == height)
                        })
                        && map.list.as_ref().is_some_and(|list| {
                            ["cancel", "confirm"].iter().all(|id| {
                                list.geometry.iter().any(|r| {
                                    r.id == *id && usize::from(r.visible.height) == r.lines.len()
                                })
                            })
                        })
                }),
        );
    }
}
