//! Typed projection into the shared modal/list surface and current-frame hits.
use super::*;
use crate::{
    checklist::model::{Action, Mutation},
    look::Look,
};
use ratatui::{Frame, layout::Rect};
use serde_json::json;
use tmt_tui::components::ListRow;
const FILE: &str = "squad.checklist.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="checklist" title="Checklist" placement="center" class="w-100 h-30"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no matching items)"><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" token-bind="row.role" wrap="true"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="query" bind="$.query" token="accent" wrap="true"/><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#;
const READING: &str = r#"<tmt-view version="1"><tmt-modal id="checklist" title="Checklist" placement="center" class="w-100 h-30"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text slot="query" bind="$.query" token="accent" wrap="true"/><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#;
const CONFIRM: &str = r#"<tmt-view version="1"><tmt-modal id="checklist" title="Checklist" placement="center" class="w-100 h-30"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" wrap="true"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty="(no matching items)"><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" token-bind="row.role" wrap="true"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="query" bind="$.query" token="accent" wrap="true"/><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#;

impl Controller {
    pub(in crate::board) fn render(&self, frame: &mut Frame, look: Look, body: Rect) {
        let reading = self.screen == Screen::Unknown
            || matches!(self.screen, Screen::Details(_)) && self.focus == 0;
        let title = format!(
            "Checklist · {}",
            self.current()
                .and_then(|c| c.room.name.as_deref())
                .unwrap_or("choose room")
        );
        let confirm = self.screen == Screen::Confirm;
        let form = self.screen == Screen::Form;
        let key = format!("{title}/{reading}/{confirm}/{form}");
        let mut cached = self.template.borrow_mut();
        if cached.as_ref().is_none_or(|(old, _)| old != &key) {
            let escaped = tmt_cli_style::table::escape(&title)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;");
            let markup = if confirm {
                CONFIRM
            } else if reading {
                READING
            } else {
                MARKUP
            };
            let markup = if form {
                markup.replace(
                    "token-bind=\"row.role\" wrap=\"true\"",
                    "token-bind=\"row.role\" class=\"truncate\"",
                )
            } else {
                markup.into()
            };
            let markup = markup.replace("title=\"Checklist\"", &format!("title=\"{escaped}\""));
            *cached = Some((key, picker_surface::compile(FILE, &markup, schema())));
        }
        let template = &cached.as_ref().unwrap().1;
        let all_rows = self.rows();
        let notes: Vec<_> = all_rows
            .iter()
            .filter(|row| reading || confirm && row.disabled)
            .collect();
        let rows = self.choice_rows();
        let mut state = if reading {
            self.reading.borrow_mut()
        } else if self.screen == Screen::List {
            self.list.borrow_mut()
        } else {
            self.panel.borrow_mut()
        };
        state.reconcile(
            rows.iter()
                .map(|r| ListRow {
                    id: r.id.clone(),
                    disabled: r.disabled,
                })
                .collect(),
        );
        let selected = if reading {
            None
        } else {
            state.picker.list.selected()
        };
        let rows_value:Vec<_>=rows.iter().map(|r|json!({"id":r.id,"disabled":r.disabled,"label":r.label,"role":if r.muted{"muted"}else{"text"},"cursor":if selected==Some(r.id.as_str()){"›"}else{""}})).collect();
        let status = if self.pending.is_some() {
            "Loading…".into()
        } else if let Some(error) = &self.failure {
            format!(
                "{}: {} · Refresh explicitly",
                error.code.as_str(),
                error.message
            )
        } else {
            self.notice.clone().unwrap_or_default()
        };
        let list = self.screen == Screen::List;
        let value = json!({"rows":rows_value,"query":self.heading(),"status":status,"notes":notes.iter().flat_map(|r|r.label.split('\n').enumerate().map(|(index,text)|json!({"id":format!("{}:{index}",r.id),"text":text,"role":if r.muted{"muted"}else{"text"}}))).collect::<Vec<_>>(),
            "footer":if list{format!("{}Filter · {}Actions · Tab focus · Enter opens · Esc board",if self.focus==1{"›"}else{" "},if self.focus==2{"›"}else{" "})}else if self.screen==Screen::Unknown{"↑↓ / PgUp/PgDn read · Esc list".into()}else if matches!(self.screen,Screen::Details(_)){"↑↓ / PgUp/PgDn read · Tab actions · Esc list".into()}else if self.screen==Screen::Form{"Tab fields · type / Ctrl-U clear · Esc retains".into()}else{"↑↓ / Tab choose · Enter select · Esc list".into()}});
        if reading {
            state.render_modal(FILE, template, value, frame, look, body);
        } else {
            state.render(FILE, template, value, frame, look, body);
        }
        let mut controls = [Rect::default(); 2];
        if list && let Some(map) = &state.frame {
            let footer = map.areas.footer;
            for (index, (start, width)) in [(0, 7), (10, 8)].into_iter().enumerate() {
                if footer.height > 0 && footer.width >= start + width {
                    controls[index] = Rect::new(footer.x + start, footer.y, width, 1);
                }
            }
        }
        self.controls.set(controls);
        // All exact-target lines and Cancel/Confirm must fit simultaneously.
        let readable = confirm
            && state.frame.as_ref().is_some_and(|map| {
                map.areas.content.width >= 38
                    && notes.iter().all(|row| {
                        row.label.split('\n').enumerate().all(|(index, text)| {
                            let id = format!("{}:{index}", row.id);
                            let height = tmt_tui::text::measure(
                                text,
                                tmt_tui::style::TextFlow::Wrap,
                                tmt_tui::geometry::Space::Cells(map.areas.content.width),
                            )[1];
                            map.hits
                                .iter()
                                .any(|hit| hit.id.last() == Some(&id) && hit.rect.height == height)
                        })
                    })
                    && map.list.as_ref().is_some_and(|list| {
                        ["cancel", "confirm"].iter().all(|id| {
                            list.geometry.iter().any(|row| {
                                row.id == *id
                                    && usize::from(row.visible.height) == row.lines.len()
                                    && row.visible.width >= 38
                            })
                        })
                    })
            });
        self.readable.set(readable);
    }
    fn deleting(&self) -> bool {
        self.submit.as_ref().is_some_and(|request| {
            matches!(
                request.action,
                Action::Item {
                    mutation: Mutation::Delete { .. },
                    ..
                }
            )
        })
    }
    fn heading(&self) -> String {
        if self.screen == Screen::Rooms {
            return "Choose an exact Squad room · no room selected".into();
        }
        let Some(current) = self.current() else {
            return "Awaiting admitted room state".into();
        };
        let name = current.room.name.as_deref().unwrap_or("Unavailable room");
        if self.screen != Screen::List {
            return format!(
                "{name} · {}",
                match &self.screen {
                    Screen::Unknown => "Original unknown outcomes",
                    Screen::Details(_) => "Details",
                    Screen::Actions => "Actions",
                    Screen::Filters => "Filters",
                    Screen::Members(Some(_)) => "Assign to member",
                    Screen::Members(None) => "Member filter",
                    Screen::Form => "Authored fields",
                    Screen::Confirm if self.deleting() => "Review delete",
                    Screen::Confirm => "Review exact update",
                    Screen::Reorder => "Full inventory order (including archived)",
                    Screen::OrderActions(_) => "Move item",
                    _ => "Checklist",
                }
            );
        }
        let total = current
            .items
            .iter()
            .filter(|v| self.archived || !v.item.archived)
            .count();
        let visible = current.items.iter().filter(|v| self.matches(v)).count();
        let completion = match self.completion {
            Some(Completion::Open) => "Open",
            Some(Completion::Complete) => "Complete",
            None => "All completion",
        };
        let assignment = match &self.assignment {
            Assignment::All => "Squad-wide".into(),
            Assignment::Unassigned => "Unassigned".into(),
            Assignment::Member(id) => format!(
                "Member {}",
                self.preview
                    .as_ref()
                    .and_then(|p| p.members.iter().find(|(i, _)| i == id))
                    .map_or(id.as_str(), |(_, name)| name)
            ),
        };
        format!(
            "{name} · {completion} · {assignment} · archived {} · {visible} visible / {total} total{}",
            if self.archived {
                "included"
            } else {
                "excluded"
            },
            if current.room.available {
                ""
            } else {
                " · read-only"
            }
        )
    }
    pub(super) fn target_text(&self) -> String {
        let Some(request) = &self.submit else {
            return "No submitted update".into();
        };
        let actor = self
            .draft
            .as_ref()
            .map(|d| &d.actor)
            .or_else(|| self.preview.as_ref().map(|p| &p.actor_id));
        let target = match &request.action {
            Action::Create {
                item_id, inventory, ..
            } => format!("item {}\ninventory {inventory:?}", item_id.as_str()),
            Action::Item {
                item_id,
                revision,
                mutation,
            } => format!(
                "item {}\nitem revision {revision}{}",
                item_id.as_str(),
                match mutation {
                    Mutation::Delete {
                        inventory_revision, ..
                    } => format!("\ninventory revision {inventory_revision}"),
                    _ => String::new(),
                }
            ),
            Action::Reorder {
                inventory_revision,
                order,
            } => format!(
                "inventory revision {inventory_revision}\nfull order: {}",
                order.iter().map(Id::as_str).collect::<Vec<_>>().join(", ")
            ),
        };
        format!(
            "{}\nactor {}\nroom {}\nchecklist {}\n{target}",
            super::controller::action_name(request),
            actor.map_or("unavailable", Id::as_str),
            request.room_id.as_str(),
            request.checklist_id.as_str()
        ) + &self
            .target_label
            .as_ref()
            .map(|title| format!("\nTitle: {title}"))
            .unwrap_or_default()
    }
    pub(super) fn choice_rows(&self) -> Vec<Row> {
        self.rows()
            .into_iter()
            .filter(|row| self.screen != Screen::Confirm || !row.disabled)
            .collect()
    }
    pub(super) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if !self.unknown.is_empty() && self.screen != Screen::Unknown {
            rows.push(Row::text(
                "unknown-status",
                format!(
                    "{} original outcome(s) unknown · Actions → Original unknown outcomes",
                    self.unknown.len()
                ),
            ));
        }
        match &self.screen {
            Screen::Unknown => rows.extend(self.unknown.iter().enumerate().map(|(i, target)| {
                Row::text(
                    format!("unknown-{i}"),
                    format!("Original outcome unknown: {target}"),
                )
            })),
            Screen::Rooms => {
                rows.extend(self.rooms.iter().map(|(id, name)| {
                    Row::choice(id.as_str(), format!("{name} · {}", id.as_str()))
                }));
                rows.push(Row::choice("refresh", "Refresh room choices"));
            }
            Screen::List => {
                if let Some(current) = self.current() {
                    let empty = if current.checklist_id.is_none() {
                        Some("No checklist yet · Actions → Create")
                    } else if current.items.is_empty() {
                        Some("Checklist is empty · Actions → Create")
                    } else if !current.items.iter().any(|v| self.matches(v)) {
                        Some("No items match these filters · Filter to change scope")
                    } else {
                        None
                    };
                    if let Some(message) = empty {
                        rows.push(Row::text("empty", message));
                    }
                    rows.extend(current.items.iter().filter(|v| self.matches(v)).map(|v| {
                        Row::choice(
                            v.item.id.as_str(),
                            format!(
                                "{} {} · {}{}",
                                if v.item.completion == Completion::Complete {
                                    "✓"
                                } else {
                                    "○"
                                },
                                v.item.content.title,
                                assignee(v),
                                if v.item.archived { " · archived" } else { "" }
                            ),
                        )
                    }));
                }
            }
            Screen::Actions => {
                rows.push(Row::choice("refresh", "Refresh"));
                if !self.unknown.is_empty() {
                    rows.push(Row::choice("unknown", "Original unknown outcomes"));
                }
                if self.draft.is_some() {
                    rows.push(Row::choice("resume", "Resume retained draft"));
                }
                if self.submit.is_some() {
                    rows.push(Row::choice("resume-operation", "Review retained update"));
                }
                if self.draft.is_none() && self.submit.is_none() && self.writable() {
                    rows.push(Row::choice("create", "Create item (unassigned)"));
                    if self.manager() && self.current().is_some_and(|c| !c.items.is_empty()) {
                        rows.push(Row::choice("reorder", "Reorder full inventory"));
                    }
                }
            }
            Screen::Filters => rows.extend(
                [
                    ("open", "Open"),
                    ("complete", "Complete"),
                    ("all", "All completion"),
                    ("squad", "Squad-wide"),
                    ("unassigned", "Unassigned"),
                    ("member", "Choose member filter"),
                    ("archive", "Toggle archived inclusion"),
                ]
                .map(|(id, label)| Row::choice(id, label)),
            ),
            Screen::Members(_) => {
                if let Some(preview) = &self.preview {
                    rows.extend(preview.members.iter().map(|(id, name)| {
                        Row::choice(id.as_str(), format!("{name} · {}", id.as_str()))
                    }));
                }
            }
            Screen::Details(id) => {
                if let Some(v) = self.item(id) {
                    let item = &v.item;
                    rows.push(Row::text("detail-title", item.content.title.clone()));
                    rows.push(Row::text(
                        "detail-state",
                        format!(
                            "{} · {}{}",
                            if item.completion == Completion::Open {
                                "Open"
                            } else {
                                "Complete"
                            },
                            assignee(v),
                            if item.archived { " · archived" } else { "" }
                        ),
                    ));
                    rows.push(Row::context(
                        "detail-item",
                        format!("Item {} · revision {}", id.as_str(), item.revision),
                    ));
                    if let Some(current) = self.current() {
                        rows.push(Row::context(
                            "detail-context",
                            format!(
                                "Room {}\nChecklist {} · inventory revision {}",
                                current.room.id.as_str(),
                                current.checklist_id.as_ref().map_or("absent", Id::as_str),
                                current.inventory_revision.unwrap_or(0)
                            ),
                        ));
                    }
                    if let Some(body) = &item.content.body {
                        rows.push(Row::text("detail-body", body.clone()));
                    }
                    if let Some(link) = &item.content.reference {
                        rows.push(Row::text("detail-reference", link.clone()));
                        rows.push(Row::choice("reference", "Open reference"));
                    }
                    if self.writable() && self.draft.is_none() && self.submit.is_none() {
                        if !item.archived {
                            rows.push(Row::choice("edit", "Edit title, body and reference"));
                            rows.push(Row::choice(
                                if item.completion == Completion::Open {
                                    "complete"
                                } else {
                                    "reopen"
                                },
                                if item.completion == Completion::Open {
                                    "Complete"
                                } else {
                                    "Reopen"
                                },
                            ));
                        }
                        if self.manager() {
                            if !item.archived {
                                rows.push(Row::choice("assign", "Assign to member"));
                                if item.assignee.is_some() {
                                    rows.push(Row::choice("unassign", "Unassign"));
                                }
                            }
                            rows.push(Row::choice(
                                if item.archived { "restore" } else { "archive" },
                                if item.archived { "Restore" } else { "Archive" },
                            ));
                            rows.push(Row::choice("delete", "Delete permanently…"));
                        }
                    }
                } else {
                    rows.push(Row::text(
                        "missing",
                        "The exact item is no longer available.",
                    ));
                }
                rows.push(Row::choice("refresh", "Refresh"));
            }
            Screen::Form => {
                if let Some(draft) = &self.draft {
                    rows.extend(
                        [
                            ("title", format!("Title: {}", draft.title)),
                            ("body", format!("Body: {}", body_preview(&draft.body))),
                            ("reference", format!("Reference: {}", draft.reference)),
                        ]
                        .map(|(id, label)| Row::choice(id, label)),
                    );
                    rows.extend(
                        [
                            ("cancel", "Cancel and discard draft"),
                            ("refresh", "Refresh current state"),
                            ("review", "Review current expectations"),
                            ("apply", "Preview update…"),
                        ]
                        .map(|(id, label)| Row::choice(id, label)),
                    );
                }
            }
            Screen::Confirm => {
                rows.push(Row::choice("cancel", "Cancel"));
                if let Some(title) = &self.target_label {
                    rows.push(Row::text("target-title", title.clone()));
                }
                rows.extend(
                    self.target_text()
                        .lines()
                        .skip(1)
                        .filter(|line| !line.starts_with("Title: "))
                        .enumerate()
                        .map(|(i, line)| Row::context(format!("target-{i}"), line)),
                );
                let content = match self.submit.as_ref().map(|r| &r.action) {
                    Some(Action::Create { content, .. }) => Some(format!(
                        "Title: {}\nBody: {}\nReference: {}",
                        content.title,
                        content.body.as_deref().unwrap_or(""),
                        content.reference.as_deref().unwrap_or("")
                    )),
                    Some(Action::Item {
                        mutation: Mutation::Edit(edit),
                        ..
                    }) => Some(format!(
                        "Title: {}\nBody: {}\nReference: {}",
                        edit.title.as_deref().unwrap_or(""),
                        edit.body.as_ref().and_then(|x| x.as_deref()).unwrap_or(""),
                        edit.reference
                            .as_ref()
                            .and_then(|x| x.as_deref())
                            .unwrap_or("")
                    )),
                    _ => None,
                };
                if let Some(content) = content {
                    rows.push(Row::text("authored", content));
                }
                rows.extend(
                    [
                        (
                            "confirm",
                            if self.reviewed {
                                if self.deleting() {
                                    "Confirm delete"
                                } else {
                                    "Confirm update"
                                }
                            } else {
                                "Confirm disabled until fresh review"
                            },
                        ),
                        ("refresh", "Refresh current state"),
                        ("review", "Review current expectations"),
                    ]
                    .map(|(id, label)| Row::choice(id, label)),
                );
            }
            Screen::Reorder => {
                rows.push(Row::choice("cancel", "Cancel reorder"));
                if let Some(Request {
                    action: Action::Reorder { order, .. },
                    ..
                }) = &self.submit
                {
                    rows.extend(order.iter().map(|id| {
                        Row::choice(
                            id.as_str(),
                            format!(
                                "{} · {}",
                                self.item(id)
                                    .map_or("Unavailable item", |v| &v.item.content.title),
                                id.as_str()
                            ),
                        )
                    }));
                }
                rows.push(Row::choice("apply", "Preview full order…"));
            }
            Screen::OrderActions(_) => rows.extend([
                Row::choice("up", "Move up"),
                Row::choice("down", "Move down"),
            ]),
        }
        rows
    }
}
fn assignee(view: &crate::checklist::ItemView) -> String {
    match &view.item.assignee {
        None => "Unassigned".into(),
        Some(a) => format!(
            "{}{}",
            view.assignee_label.as_deref().unwrap_or(a.id.as_str()),
            if view.assignee_available {
                ""
            } else {
                " (unavailable)"
            }
        ),
    }
}

fn body_preview(body: &str) -> String {
    let (first, more) = body
        .split_once('\n')
        .map_or((body, false), |(first, _)| (first, true));
    format!(
        "{}{}",
        first.trim_end_matches('\r'),
        if more { "…" } else { "" }
    )
}
fn schema() -> tmt_tui::binding::Schema {
    use tmt_tui::binding::Schema;
    let mut schema = picker_surface::schema(&["cursor", "label", "role"]);
    if let Schema::Object(fields) = &mut schema
        && let Some(Schema::Collection(notes)) = fields.get_mut("notes")
        && let Schema::Object(fields) = notes.as_mut()
    {
        fields.insert("role".into(), Schema::Scalar);
    }
    schema
}
