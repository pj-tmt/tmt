//! Paste chips project Editor folds; the editor remains the only source of send bytes.
use super::{App, DraftKey, Projection};
use crate::board::app::Effect;
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::Modifier,
    text::Line,
};
use std::{
    cell::{Ref, RefCell},
    ops::Range,
};
use tmt_cli_style::{Depth, Role};
use tmt_tui::components::{Editor, Modal, Outline, Placement, ScrollState, strip};
use unicode_width::UnicodeWidthStr;

/// Both limits are exclusive. Source bytes are never normalized for counting.
const PASTE_THRESHOLD: (usize, usize) = (500, 10);

fn counts(text: &str) -> (usize, usize) {
    let mut lines = usize::from(!text.is_empty());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            lines += 1;
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else if ch == '\n' {
            lines += 1;
        }
    }
    (text.chars().count(), lines)
}

fn number(value: usize) -> String {
    let raw = value.to_string();
    let mut result = String::new();
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

pub(in crate::board) fn label(text: &str) -> Option<String> {
    let (chars, lines) = counts(text);
    (chars > PASTE_THRESHOLD.0 || lines > PASTE_THRESHOLD.1).then(|| {
        let (count, unit) = if lines > PASTE_THRESHOLD.1 {
            (lines, "lines")
        } else {
            (chars, "chars")
        };
        format!("⧉ Pasted · {} {unit} ×", number(count))
    })
}

#[derive(Clone)]
pub(in crate::board) struct Hit {
    area: Rect,
    remove: Rect,
    key: DraftKey,
    id: u64,
    source: Range<usize>,
    label: String,
}

pub(in crate::board) struct Preview {
    key: DraftKey,
    id: u64,
    text: Editor,
    scroll: RefCell<ScrollState>,
}

impl Preview {
    pub(super) fn invalidate(&self) {
        let count = self.scroll.borrow().content();
        self.scroll
            .borrow_mut()
            .update(Rect::default(), count, None);
    }
}

/// Keep the editor's admitted cell width, including when ASCII needs fewer glyphs.
fn display(label: &str, width: usize, ascii: bool) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let content = label.strip_suffix(" ×").unwrap_or(label);
    let content = if ascii {
        format!(
            "[{}",
            content
                .strip_prefix("⧉ ")
                .unwrap_or(content)
                .replace(" · ", " ")
        )
    } else {
        content.to_owned()
    };
    let suffix = if ascii { " x]" } else { " ×" };
    if width < suffix.width() {
        return (
            format!("{}{}", if ascii { "x" } else { "×" }, " ".repeat(width - 1)),
            0,
        );
    }
    let mut visible = if content.width() + suffix.width() <= width {
        content + suffix
    } else {
        let head = crate::board::view::fit(&content, width - suffix.width());
        head + suffix
    };
    let remove = visible.width() - if ascii { 2 } else { 1 };
    if ascii {
        visible = visible.replace('…', ".");
    }
    visible.push_str(&" ".repeat(width.saturating_sub(visible.width())));
    (visible, remove)
}

fn preview_area(bounds: Rect, anchor: Rect, lines: usize) -> Option<Rect> {
    let width = bounds.right().saturating_sub(anchor.x).min(64);
    if width < 5 || bounds.height < 4 || !bounds.contains((anchor.x, anchor.y).into()) {
        return None;
    }
    let wanted =
        (lines.saturating_add(3).min(usize::from(u16::MAX)) as u16).min((bounds.height / 2).max(4));
    let above = anchor.y.saturating_sub(bounds.y);
    let below = bounds.bottom().saturating_sub(anchor.bottom());
    let (y, height) = if above >= wanted {
        (anchor.y - wanted, wanted)
    } else if below >= 4 {
        (anchor.bottom(), wanted.min(below))
    } else if above >= 4 {
        (bounds.y, wanted.min(above))
    } else {
        return None;
    };
    Some(Rect::new(anchor.x, y, width, height))
}

impl App {
    pub(in crate::board) fn paint_paste_chips(
        &self,
        frame: &mut Frame,
        projection: &Projection<'_>,
        area: Rect,
    ) {
        let (Some(text), Some(key)) = (projection.text, projection.key.as_ref()) else {
            return;
        };
        if area.is_empty() || text.folds().is_empty() {
            return;
        }
        let layout = text.layout(area.width);
        let skip = self.input_viewport.get().map_or(0, |(_, skip)| skip);
        let look = self.look();
        let ascii = look.depth == Depth::None
            || matches!(
                self.cron_glyphs,
                crate::board::overview::cron_counts::GlyphSet::Ascii
            );
        let base = look.role(Role::PasteText).patch(look.role(Role::PasteChip));
        let selected = text.selection();
        for (row, line) in layout
            .lines
            .iter()
            .skip(skip)
            .take(usize::from(area.height))
            .enumerate()
        {
            for cell in &line.cells {
                let Some(fold) = text
                    .folds()
                    .iter()
                    .find(|fold| !fold.expanded && fold.source == cell.source)
                else {
                    continue;
                };
                let rect = Rect::new(
                    area.x + cell.column as u16,
                    area.y + row as u16,
                    cell.width as u16,
                    1,
                )
                .intersection(area);
                if rect.is_empty() {
                    continue;
                }
                let (label, remove) = display(&fold.label, usize::from(rect.width), ascii);
                let style = if fold.source.start < selected.end && fold.source.end > selected.start
                {
                    base.add_modifier(Modifier::REVERSED)
                } else {
                    base
                };
                strip::paint_left(frame.buffer_mut(), rect, Line::styled(label, style));
                self.composer.paste_hits.borrow_mut().push(Hit {
                    area: rect,
                    remove: Rect::new(rect.x + remove as u16, rect.y, 1, 1),
                    key: key.clone(),
                    id: fold.id,
                    source: fold.source.clone(),
                    label: fold.label.clone(),
                });
            }
        }
    }

    fn paste_hit_is_current(&self, hit: &Hit) -> bool {
        self.composer_projection().is_some_and(|projection| {
            projection.key.as_ref() == Some(&hit.key)
                && projection.text.is_some_and(|text| {
                    text.folds().iter().any(|fold| {
                        fold.id == hit.id
                            && fold.source == hit.source
                            && fold.label == hit.label
                            && !fold.expanded
                    })
                })
        })
    }

    fn open_paste_preview(&mut self, hit: Hit) -> Effect {
        if !self.paste_hit_is_current(&hit) {
            return Effect::None;
        }
        let fits = self
            .conversation
            .borrow()
            .pane
            .is_some_and(|bounds| preview_area(bounds, hit.area, 1).is_some());
        if !fits {
            return self.say("Not enough room for paste preview; enlarge the conversation pane.");
        }
        if self.input.is_none() {
            self.focus_composer();
        }
        let Some(input) = self
            .input
            .as_ref()
            .filter(|input| input.draft_key.as_ref() == Some(&hit.key))
        else {
            return Effect::None;
        };
        let Some(source) = input.text.as_str().get(hit.source.clone()) else {
            return Effect::None;
        };
        *self.composer.paste_preview.get_mut() = Some(Preview {
            key: hit.key,
            id: hit.id,
            text: source.into(),
            scroll: RefCell::default(),
        });
        Effect::None
    }

    pub(in crate::board) fn preview_paste_at_cursor(&mut self) -> Effect {
        let hit = self.input.as_ref().and_then(|input| {
            let cursor = input.text.cursor();
            self.composer
                .paste_hits
                .borrow()
                .iter()
                .rev()
                .find(|hit| hit.source.contains(&cursor) || hit.source.end == cursor)
                .cloned()
        });
        hit.map_or(Effect::None, |hit| self.open_paste_preview(hit))
    }

    pub(in crate::board) fn paste_chip_mouse(&mut self, event: MouseEvent) -> Option<Effect> {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let hit = self
            .composer
            .paste_hits
            .borrow()
            .iter()
            .find(|hit| hit.area.contains((event.column, event.row).into()))
            .cloned()?;
        if !self.paste_hit_is_current(&hit) {
            return Some(Effect::None);
        }
        if !hit.remove.contains((event.column, event.row).into()) {
            return Some(self.open_paste_preview(hit));
        }
        if self.input.is_none() {
            self.focus_composer();
        }
        if let Some(input) = &mut self.input
            && input.draft_key.as_ref() == Some(&hit.key)
        {
            input
                .text
                .replace_range(hit.source, "", crate::board::app::MESSAGE_LIMIT);
            self.remember_draft();
            self.update_mentions();
            self.invalidate_frame_geometry();
        }
        Some(Effect::None)
    }

    fn current_paste_preview(&self) -> Option<Ref<'_, Preview>> {
        let preview = Ref::filter_map(self.composer.paste_preview.borrow(), |preview| {
            preview.as_ref()
        })
        .ok()?;
        let projection = self.composer_projection()?;
        (projection.key.as_ref() == Some(&preview.key)
            && projection.text.is_some_and(|text| {
                text.folds().iter().any(|fold| {
                    fold.id == preview.id
                        && text.as_str().get(fold.source.clone()) == Some(preview.text.as_str())
                })
            }))
        .then_some(preview)
    }

    pub(in crate::board) fn paste_preview_shown(&self) -> bool {
        self.current_paste_preview().is_some()
    }

    pub(in crate::board) fn paste_preview_event(&mut self, event: &Event) -> Effect {
        if matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release && matches!(key.code, KeyCode::Esc | KeyCode::Char('q')))
        {
            self.composer.paste_preview.get_mut().take();
        } else if let Some(preview) = self.current_paste_preview() {
            preview.scroll.borrow_mut().input(event);
        }
        Effect::None
    }

    pub(in crate::board) fn render_paste_preview(&self, frame: &mut Frame, body: Rect) {
        let Some(preview) = self.current_paste_preview() else {
            return;
        };
        let anchor = self
            .composer
            .paste_hits
            .borrow()
            .iter()
            .find(|hit| hit.key == preview.key && hit.id == preview.id)
            .map(|hit| hit.area);
        let Some(anchor) = anchor else {
            drop(preview);
            self.composer.paste_preview.borrow_mut().take();
            return;
        };
        let bounds = self
            .conversation
            .borrow()
            .pane
            .unwrap_or(body)
            .intersection(body);
        let width = bounds
            .right()
            .saturating_sub(anchor.x)
            .min(64)
            .saturating_sub(4);
        let layout = preview.text.layout(width);
        let Some(outer) = preview_area(bounds, anchor, layout.lines.len()) else {
            drop(preview);
            self.composer.paste_preview.borrow_mut().take();
            return;
        };
        let (chars, lines) = counts(preview.text.as_str());
        let title = format!("Pasted · {} chars · {} lines", number(chars), number(lines));
        let modal = Modal {
            title: String::new(),
            placement: Placement::Body,
        };
        let areas = modal.areas(outer, [outer.width, outer.height], false, false);
        let look = self.look();
        let base = look.role(Role::Text).patch(look.role(Role::PastePreview));
        let border = look
            .role(Role::PasteFrame)
            .patch(look.role(Role::PastePreview));
        // Modal owns the opaque clearing; Outline applies this attachment's
        // named chrome without introducing another drawing primitive.
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        frame.buffer_mut().set_style(outer, base);
        Outline {
            title: Line::from(format!(" {title} ")),
            border,
            title_style: border,
        }
        .paint(outer, frame.buffer_mut());
        let mut scroll = preview.scroll.borrow_mut();
        scroll.update(areas.content, layout.lines.len(), None);
        layout.paint(
            frame.buffer_mut(),
            areas.content,
            scroll.offset(),
            0..0,
            base,
            base,
        );
        let detailed = format!("{} · ↑↓ scroll · Esc close", scroll.position());
        let hint = if detailed.width() <= usize::from(areas.position.width) {
            detailed.as_str()
        } else if areas.position.width >= 23 {
            "↑↓ scroll · Esc close"
        } else {
            "Esc close"
        };
        strip::paint_left(
            frame.buffer_mut(),
            areas.position,
            Line::styled(hint, border),
        );
        self.peek_areas.set(crate::board::peek::Peek::Paste, outer);
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
