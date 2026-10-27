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
/// The widest the LINK column grows before the title gives way.
const LINK_MAX: usize = 28;
/// The widest the right-hand `who · age` column grows; it is painted on the cursor row only.
const META_MAX: usize = 24;
/// The title keeps at least this many cells; the link gives way first, then the byline.
const TITLE_MIN: usize = 24;
/// The STATUS column: a mark, a space and the word (`↗ link`, `○ todo`).
const STATUS_CELLS: usize = 6;
/// The modal's width (`w-100`) and what its border, padding, cursor, status and gaps take.
const MODAL_CELLS: u16 = 100;
const CHROME_CELLS: usize = 4 + 1 + STATUS_CELLS + 4;

/// The column widths of the list: the link and the byline, as admitted by the room left.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Columns {
    link: usize,
    meta: usize,
}

fn markup(title: &str, columns: Columns, header: bool, form_only: bool) -> String {
    let Columns { link, meta } = columns;
    let grid = format!("grid grid-cols-[1_{STATUS_CELLS}_{link}_1fr_{meta}] gap-x-1 shrink-0");
    let choices = if form_only {
        String::new()
    } else {
        let header = if header {
            format!(
                r#"<tmt-row class="{grid}"><tmt-text/><tmt-text token="muted">STATUS</tmt-text><tmt-text token="muted">LINK</tmt-text><tmt-text token="muted">TITLE</tmt-text><tmt-text/></tmt-row>"#
            )
        } else {
            String::new()
        };
        format!(
            r#"{header}<tmt-list id="choices" bind="$.rows" empty="No items yet · a adds one"><tmt-row class="{grid}"><tmt-text bind="row.cursor"/><tmt-text bind="row.status" token-bind="row.role"/><tmt-text bind="row.link" token-bind="row.link_role" class="truncate"/><tmt-text bind="row.title" token="text" class="truncate"/><tmt-text bind="row.meta" token="dim" class="truncate"/></tmt-row></tmt-list>"#
        )
    };
    format!(
        r#"<tmt-view version="1"><tmt-modal id="notes" title="{title}" placement="center" class="w-100"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" class="truncate"/></tmt-repeat>{choices}</tmt-scroll><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#
    )
}

fn schema() -> tmt_tui::binding::Schema {
    use tmt_tui::binding::Schema;
    let mut schema = picker_surface::schema(&[
        "cursor",
        "status",
        "link",
        "link_role",
        "title",
        "meta",
        "role",
    ]);
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
        let widest = |cell: fn(&Row) -> usize, most: usize| {
            items.iter().map(cell).max().unwrap_or(0).min(most)
        };
        // The title has priority: the link gives way first, then the byline.
        let spare = room.saturating_sub(TITLE_MIN);
        let link = widest(|row| row.link.width(), LINK_MAX)
            .max("LINK".len())
            .min(spare.max(1));
        let meta = widest(|row| row.byline.width(), META_MAX).min(spare.saturating_sub(link));
        let columns = Columns {
            link,
            meta: meta.max(1),
        };
        let header = !items.is_empty();
        let key = format!("{title}/{link}/{meta}/{header}/{form_only}");
        let mut cached = self.template.borrow_mut();
        if cached.as_ref().is_none_or(|(old, _)| old != &key) {
            let escaped = tmt_cli_style::table::escape(&title)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;");
            *cached = Some((
                key,
                picker_surface::compile(
                    FILE,
                    &markup(&escaped, columns, header, form_only),
                    schema(),
                ),
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
        let rows: Vec<_> = items
            .iter()
            .map(|row| {
                let cursor = selected.as_deref() == Some(row.id.as_str());
                json!({
                    "id": row.id.as_str(), "disabled": false,
                    "cursor": if cursor { "›" } else { "" },
                    "status": row.kind.status(),
                    "role": if row.kind == Kind::Link { "link" } else { "waiting" },
                    "link": safe(&row.link),
                    "link_role": if row.kind == Kind::Link { "link" } else { "dim" },
                    "title": safe(&row.title),
                    "meta": if cursor { safe(&row.byline) } else { String::new() },
                })
            })
            .collect();
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
        let enter_ticks = app.view.as_ref().map(|view| view.checklist_enter)
            == Some(crate::config::ChecklistEnter::Tick);
        let footer = if form_only {
            "Tab title/link · ←→ kind · Enter save · Esc keep draft"
        } else if draft.is_some() {
            "Tab title/link · Enter save · Esc close, keeping the draft"
        } else if deleting.is_some() {
            "Enter delete · Esc keep"
        } else if enter_ticks {
            "↑↓ choose · Enter tick · o open · a add · e edit · d delete · Shift+↑↓ move · Esc close"
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
