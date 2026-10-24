//! The conversation checklist overlay on the shared modal and list surface.
use super::{
    Summary,
    draft::Draft,
    overlay::Controller,
    rows::{Kind, Row},
};
use crate::{
    board::{
        app::{App, Overlay},
        chip_group, picker_surface,
    },
    look::Look,
};
use ratatui::{Frame, layout::Rect};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

const FILE: &str = "ops.conversation-checklist.xml";
/// The widest the right-hand `host  who · age` column grows before the title gives way.
const META_MAX: usize = 36;
/// A host is cut to this many cells so that it never crowds the title.
const HOST_CELLS: usize = 16;
/// The title keeps at least this many cells; below it the host is dropped, the byline stays.
const TITLE_MIN: usize = 24;
/// The modal's width (`w-100`) and what its border, padding, cursor, mark and gaps take.
const MODAL_CELLS: u16 = 100;
const CHROME_CELLS: usize = 4 + 1 + 1 + 3;

fn markup(title: &str, meta: usize, form_only: bool) -> String {
    let choices = if form_only {
        String::new()
    } else {
        format!(
            r#"<tmt-list id="choices" bind="$.rows" empty="No items yet · a adds one"><tmt-row class="grid grid-cols-[1_1_1fr_{meta}] gap-x-1 shrink-0"><tmt-text bind="row.cursor"/><tmt-text bind="row.mark" token-bind="row.role"/><tmt-text bind="row.title" token-bind="row.role" class="truncate"/><tmt-text bind="row.meta" token="dim" class="truncate"/></tmt-row></tmt-list>"#
        )
    };
    format!(
        r#"<tmt-view version="1"><tmt-modal id="notes" title="{title}" placement="center" class="w-100"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" class="truncate"/></tmt-repeat>{choices}</tmt-scroll><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#
    )
}

fn schema() -> tmt_tui::binding::Schema {
    use tmt_tui::binding::Schema;
    let mut schema = picker_surface::schema(&["cursor", "mark", "title", "meta", "role"]);
    if let Schema::Object(fields) = &mut schema
        && let Some(Schema::Collection(notes)) = fields.get_mut("notes")
        && let Schema::Object(fields) = notes.as_mut()
    {
        fields.insert("role".into(), Schema::Scalar);
    }
    schema
}

fn safe(text: &str) -> String {
    crate::board::notes::single_line(text)
}

/// The field with a caret where the next character goes.
fn with_caret(draft: &Draft, field: usize) -> String {
    let text = draft.text(field);
    let at = draft.fields[field].cursor().min(text.len());
    let at = (0..=at)
        .rev()
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(0);
    format!("{}▏{}", &text[..at], &text[at..])
}

impl Controller {
    pub fn render(&self, frame: &mut Frame, app: &App, look: Look, body: Rect) {
        let items = self.items(&app.notes);
        let draft = app.notes.draft(&self.key);
        let summary = app.notes.summary(&self.key);
        let form_only = draft.is_some_and(|draft| draft.seed.is_some());
        // Resolve the held recipient by the controller's frozen UUID, never the current cursor.
        let member = app
            .notes
            .preview(&self.key)
            .and_then(|preview| preview.current.member.as_ref())
            .and_then(|member| member.name.clone())
            .or_else(|| {
                let target = crate::board::app::RowTarget::Workspace(
                    crate::board::workspace::OpsTarget::Member {
                        room_id: Some(self.key.room.clone()),
                        identity_id: self.key.member()?.into(),
                    },
                );
                app.target_row(&target)
                    .and_then(|row| row["name"].as_str())
                    .map(str::to_owned)
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                self.key.member().map_or_else(
                    || "unavailable".into(),
                    |id| format!("member {}", id.chars().take(8).collect::<String>()),
                )
            });
        let count = items.len();
        let title = if form_only {
            format!("Add to checklist · {}", safe(&member))
        } else if matches!(summary, Some(Summary::Counts(_))) {
            format!(
                "Checklist · {} · {count} {}",
                safe(&member),
                if count == 1 { "item" } else { "items" }
            )
        } else {
            format!("Checklist · {}", safe(&member))
        };
        let room = usize::from(body.width.min(MODAL_CELLS)).saturating_sub(CHROME_CELLS);
        let widest = |host_cells| {
            items
                .iter()
                .map(|row| row.meta(host_cells).width())
                .max()
                .unwrap_or(0)
                .min(META_MAX)
        };
        // The title has priority: a host that would leave it too little gives way entirely.
        let host_cells = if room.saturating_sub(widest(HOST_CELLS)) < TITLE_MIN {
            0
        } else {
            HOST_CELLS
        };
        let meta = widest(host_cells).min(room.saturating_sub(TITLE_MIN).max(1));
        let key = format!("{title}/{meta}/{form_only}");
        let mut cached = self.template.borrow_mut();
        if cached.as_ref().is_none_or(|(old, _)| old != &key) {
            let escaped = tmt_cli_style::table::escape(&title)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;");
            *cached = Some((
                key,
                picker_surface::compile(FILE, &markup(&escaped, meta.max(1), form_only), schema()),
            ));
            self.ui.borrow_mut().template_changed();
        }
        let template = &cached.as_ref().unwrap().1;
        let list = Self::list_rows(&items);
        let mut ui = self.ui.borrow_mut();
        ui.reconcile(list.clone());
        if let Some(id) = self.reveal.borrow_mut().take()
            && items.iter().any(|row| row.id == id)
        {
            ui.select(id.as_str());
        }
        if ui.picker.list.selected().is_none()
            && let Some(first) = list.iter().find(|row| !row.disabled)
        {
            ui.select(&first.id);
        }
        let selected = ui.picker.list.selected().map(str::to_owned);
        let deleting = self.deleting();
        let mut rows = Vec::new();
        for (heading, label, links) in [
            ("group:links", "Links", true),
            ("group:todo", "Todo", false),
        ] {
            let members: Vec<&Row> = items
                .iter()
                .filter(|row| (row.kind == Kind::Link) == links)
                .collect();
            if members.is_empty() {
                continue;
            }
            rows.push(json!({
                "id": heading, "disabled": true, "cursor": "", "mark": "",
                "title": format!("{label} · {}", members.len()), "meta": "", "role": "dim",
            }));
            rows.extend(members.into_iter().map(|row| {
                let role = match row.kind {
                    Kind::Done => "muted",
                    _ => "text",
                };
                json!({
                    "id": row.id.as_str(), "disabled": false,
                    "cursor": if selected.as_deref() == Some(row.id.as_str()) { "›" } else { "" },
                    "mark": row.kind.mark(), "title": safe(&row.title),
                    "meta": safe(&row.meta(host_cells)), "role": role,
                })
            }));
        }
        let mut form: Vec<_> = draft
            .map(|draft| {
                [("Title", 0), ("Link", 1)]
                    .into_iter()
                    .map(|(label, field)| {
                        let focused = draft.field() == Some(field);
                        let text = if focused {
                            with_caret(draft, field)
                        } else {
                            draft.text(field).to_owned()
                        };
                        json!({
                            "id": format!("field:{field}"),
                            "text": safe(&format!("{label} › {text}")),
                            "role": if focused { "accent" } else { "muted" },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(seed) = draft.and_then(|draft| draft.seed.as_ref()) {
            form.insert(0, json!({ "id":"kind", "text":"Kind", "role":"muted" }));
            form.push(json!({"id":"source", "text":safe(&format!("From   {}", seed.source.label())), "role":"muted"}));
        }
        let status = if let Some(notice) = &self.notice {
            safe(notice)
        } else if app.notes.writing(&self.key) {
            "Saving…".into()
        } else if let Some(row) = deleting.and_then(|id| items.iter().find(|row| row.id == *id)) {
            format!(
                "Delete “{}”? Enter deletes · Esc keeps it",
                safe(&row.title)
            )
        } else {
            match summary {
                Some(Summary::Unavailable(words)) => safe(words),
                Some(Summary::Counts(_)) => String::new(),
                _ => "Reading the checklist…".into(),
            }
        };
        let footer = if form_only {
            "Tab title/link · ←→ kind · Enter save · Esc keep draft"
        } else if draft.is_some() {
            "Tab title/link · Enter save · Esc close, keeping the draft"
        } else if deleting.is_some() {
            "Enter delete · Esc keep"
        } else {
            "↑↓ choose · Enter open/tick · a add · e edit · d delete · Shift+↑↓ move · Esc close"
        };
        let data = json!({
            "rows": rows, "notes": form, "query": "", "status": status, "footer": footer,
        });
        if form_only {
            ui.render_modal(FILE, template, data, frame, look, body);
            self.kind_hits.borrow_mut().clear();
            if let Some(draft) = draft
                && let Some(seed) = &draft.seed
                && let Some(map) = &ui.frame
                && let Some(kind) = map
                    .hits
                    .iter()
                    .find(|hit| hit.id.last().is_some_and(|id| id == "kind"))
            {
                let groups = [chip_group::ChipGroup {
                    label: "Kind  ".into(),
                    choices: [
                        ("Todo", super::NoteKind::Todo),
                        ("Link", super::NoteKind::Link),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, kind))| chip_group::Choice {
                        id: format!("kind-{index}"),
                        actionable: true,
                        text: format!(" {label} "),
                        gap: " ".into(),
                        selected: seed.kind == kind,
                        disabled: draft.sent(),
                        cursor: draft.focus == 0 && seed.kind == kind,
                    })
                    .collect(),
                }];
                let area = Rect::new(
                    map.areas.content.x,
                    kind.rect.y,
                    map.areas.content.width,
                    kind.rect.height,
                );
                self.kind_hits.borrow_mut().extend(
                    chip_group::paint(frame.buffer_mut(), area, &groups, app, Some(Overlay::Notes))
                        .into_iter()
                        .map(|hit| {
                            (
                                hit.area,
                                if hit.id == "kind-0" {
                                    super::NoteKind::Todo
                                } else {
                                    super::NoteKind::Link
                                },
                            )
                        }),
                );
            }
        } else {
            ui.render(FILE, template, data, frame, look, body);
        }
    }
}
