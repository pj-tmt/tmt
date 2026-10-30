//! The conversation checklist overlay on the shared modal and list surface.
use super::{
    Summary,
    draft::Draft,
    overlay::Controller,
    rows::{Kind, LINK_MARK, OPEN_MARK, Row},
};
use crate::{
    board::{
        app::{App, Overlay},
        chip_group, picker_surface,
    },
    look::Look,
};
use ratatui::{Frame, layout::Rect, style::Modifier};
use serde_json::json;

const FILE: &str = "ops.conversation-checklist.xml";
/// The hit id of a row's second line.
pub(super) const LINK_HIT: &str = "link";
/// What parts the link from `who · age` on the second line.
const META_GAP: &str = "   ";

/// Each item takes two lines: the cursor, its mark and the whole title, then under the title
/// `↗` and its whole link followed by `who · age`, or for a todo only who added it when. The
/// second line is one wrapped text so the byline follows the link wherever the link ends.
fn markup(title: &str, width: u16, form_only: bool) -> String {
    let lines = "grid grid-cols-[1_1_1fr] gap-x-1 shrink-0";
    let choices = if form_only {
        String::new()
    } else {
        format!(
            r#"<tmt-list id="choices" bind="$.rows" empty="No items yet · a adds one"><tmt-row class="flex-col shrink-0"><tmt-row class="{lines}"><tmt-text bind="row.cursor"/><tmt-text bind="row.mark" token-bind="row.role"/><tmt-text bind="row.title" token="text" wrap="true"/></tmt-row><tmt-row class="{lines}"><tmt-text/><tmt-text/><tmt-text id="{LINK_HIT}" bind="row.link" token-bind="row.link_role" wrap="true"/></tmt-row></tmt-row></tmt-list>"#
        )
    };
    format!(
        r#"<tmt-view version="1"><tmt-modal id="notes" title="{title}" placement="center" class="w-{width}"><tmt-scroll id="body"><tmt-repeat each="$.notes" as="note"><tmt-text id-bind="note.id" bind="note.text" token-bind="note.role" class="truncate"/></tmt-repeat>{choices}</tmt-scroll><tmt-text slot="status" bind="$.status" token="waiting" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted" class="truncate"/></tmt-modal></tmt-view>"#
    )
}

fn schema() -> tmt_tui::binding::Schema {
    use tmt_tui::binding::Schema;
    let mut schema =
        picker_surface::schema(&["cursor", "mark", "link", "link_role", "title", "role"]);
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

/// A link row's `↗` and whole address, the part of its second line that opens the link.
fn link_text(row: &Row) -> String {
    format!("{LINK_MARK} {}", safe(&row.address))
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
        // The sheet asks for the whole board; the modal keeps its usual margin around it.
        let width = body.width.max(1);
        let key = format!("{title}/{width}/{form_only}");
        let mut cached = self.template.borrow_mut();
        if cached.as_ref().is_none_or(|(old, _)| old != &key) {
            let escaped = tmt_cli_style::table::escape(&title)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('"', "&quot;");
            *cached = Some((
                key,
                picker_surface::compile(FILE, &markup(&escaped, width, form_only), schema()),
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
                // A todo has no link, so who added it when takes the link's place.
                let link = match row.kind {
                    Kind::Link if row.byline.is_empty() => link_text(row),
                    Kind::Link => format!("{}{META_GAP}{}", link_text(row), safe(&row.byline)),
                    Kind::Todo => safe(&row.byline),
                };
                json!({
                    "id": row.id.as_str(), "disabled": false,
                    "cursor": if cursor { "›" } else { "" },
                    "mark": OPEN_MARK,
                    "role": "waiting",
                    "link": link,
                    "link_role": if row.kind == Kind::Link { "link" } else { "dim" },
                    "title": safe(&row.title),
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
            self.paint_links(frame, look, &items, selected.as_deref(), &ui);
        }
    }

    /// Find the link on each link row's painted second line: its first glyphs, in reading
    /// order, are `↗` and the address, the rest is the byline, which takes the `dim` colour.
    /// The cursor row's link and the link under the pointer are highlighted as other board
    /// links are: the selection colour, underlined.
    fn paint_links(
        &self,
        frame: &mut Frame,
        look: Look,
        items: &[Row],
        selected: Option<&str>,
        ui: &picker_surface::State,
    ) {
        let hovered = self.hovered.borrow();
        let mut hits = Vec::new();
        for hit in ui.frame.iter().flat_map(|map| &map.hits) {
            let Some(row) = hit
                .row_id
                .as_deref()
                .filter(|_| hit.id.last().is_some_and(|id| id == LINK_HIT))
            else {
                continue;
            };
            let Some(item) = items
                .iter()
                .find(|item| item.id.as_str() == row && item.kind == Kind::Link)
            else {
                continue;
            };
            let glyphs = link_text(item)
                .chars()
                .filter(|glyph| !glyph.is_whitespace())
                .count();
            let lit = selected == Some(row) || hovered.as_deref() == Some(row);
            let dim = look.role(tmt_cli_style::Role::Dim);
            let buffer = frame.buffer_mut();
            let painted = (hit.rect.top()..hit.rect.bottom())
                .flat_map(|y| (hit.rect.left()..hit.rect.right()).map(move |x| (x, y)))
                .filter(|at| !buffer[*at].symbol().trim().is_empty())
                .collect::<Vec<_>>();
            for (index, (x, y)) in painted.into_iter().enumerate() {
                let cell = &mut buffer[(x, y)];
                if index >= glyphs {
                    if let Some(fg) = dim.fg {
                        cell.set_fg(fg);
                    }
                    continue;
                }
                hits.push((Rect::new(x, y, 1, 1), row.to_owned()));
                if lit {
                    cell.set_style(look.selection().add_modifier(Modifier::UNDERLINED));
                }
            }
        }
        *self.link_hits.borrow_mut() = hits;
    }
}
