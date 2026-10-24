//! The Notes overlay on the shared modal and list surface.
use super::{
    Summary,
    draft::Draft,
    overlay::Controller,
    rows::{Kind, Row},
};
use crate::{
    board::{app::App, picker_surface},
    look::Look,
};
use ratatui::{Frame, layout::Rect};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

const FILE: &str = "ops.conversation-notes.xml";
/// The widest the right-hand `host  who · age` column grows before the title gives way.
const META_MAX: usize = 36;

fn markup(title: &str, meta: usize) -> String {
    format!(
        r#"<tmt-view version="1"><tmt-modal id="notes" title="{title}" placement="center" class="w-100"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" class="truncate"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty="No notes yet · a adds one"><tmt-row class="grid grid-cols-[1_1_1fr_{meta}] gap-x-1 shrink-0"><tmt-text bind="row.cursor"/><tmt-text bind="row.mark" token-bind="row.role"/><tmt-text bind="row.title" token-bind="row.role" class="truncate"/><tmt-text bind="row.meta" token="dim" class="truncate"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#
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
        let member = app
            .notes
            .preview(&self.key)
            .and_then(|preview| preview.current.member.as_ref())
            .map(|member| {
                member
                    .name
                    .clone()
                    .unwrap_or_else(|| member.id.as_str().chars().take(8).collect())
            });
        let count = items.len();
        let title = match (&member, summary) {
            (Some(member), Some(Summary::Counts(_))) => format!(
                "Notes · {} · {count} {}",
                safe(member),
                if count == 1 { "item" } else { "items" }
            ),
            (Some(member), _) => format!("Notes · {}", safe(member)),
            _ => "Notes".into(),
        };
        let meta = items
            .iter()
            .map(|row| row.meta.width())
            .max()
            .unwrap_or(0)
            .min(META_MAX);
        let key = format!("{title}/{meta}");
        let mut cached = self.template.borrow_mut();
        if cached.as_ref().is_none_or(|(old, _)| old != &key) {
            let escaped = tmt_cli_style::table::escape(&title)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;");
            *cached = Some((
                key,
                picker_surface::compile(FILE, &markup(&escaped, meta.max(1)), schema()),
            ));
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
                    "meta": safe(&row.meta), "role": role,
                })
            }));
        }
        let form: Vec<_> = draft
            .map(|draft| {
                [("Note", 0), ("Link", 1)]
                    .into_iter()
                    .map(|(label, field)| {
                        let focused = draft.focus == field;
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
                _ => "Reading the Notes…".into(),
            }
        };
        let footer = if draft.is_some() {
            "Tab note/link · Enter save · Esc close, keeping the draft"
        } else if deleting.is_some() {
            "Enter delete · Esc keep"
        } else {
            "↑↓ choose · Enter open/tick · a add · e edit · d delete · Shift+↑↓ move · Esc close"
        };
        ui.render(
            FILE,
            template,
            json!({
                "rows": rows, "notes": form, "query": "", "status": status, "footer": footer,
            }),
            frame,
            look,
            body,
        );
    }
}
