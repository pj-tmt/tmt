//! Pane ratios and current-frame boundaries. Paint publishes geometry; App owns input.
use super::{
    app::{App, Effect},
    conversation::layout::{
        CONVERSATION_MIN_COLUMNS, CONVERSATION_MIN_ROWS, LIST_MIN_COLUMNS, LIST_MIN_ROWS,
        OVERVIEW_MIN_COLUMNS, Placement,
    },
};
use crate::action::ResizeBoundary as Id;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::{Position, Rect},
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tmt_cli_style::Role;

const SCALE: u32 = 10_000;
pub(super) type Ratios = BTreeMap<Id, u16>;
pub(super) fn ratios_json(ratios: &Ratios) -> Value {
    Value::Object(
        ratios
            .iter()
            .map(|(id, ratio)| (id.name().into(), json!(ratio)))
            .collect(),
    )
}
pub(super) fn ratios_decode(value: Option<&Value>) -> Option<Ratios> {
    let Some(value) = value else {
        return Some(Ratios::new());
    };
    value
        .as_object()?
        .iter()
        .map(|(name, value)| {
            let ratio = u16::try_from(value.as_u64()?).ok()?;
            (u32::from(ratio) <= SCALE).then_some((Id::parse(name)?, ratio))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Boundary {
    pub id: Id,
    pub hit: Rect,
    pub total: u16,
    pub before: u16,
    pub min: u16,
    pub max: u16,
}
impl Boundary {
    fn coordinate(self, point: Position) -> u16 {
        if self.id.columns() { point.x } else { point.y }
    }
}
#[derive(Clone, Copy)]
struct Drag {
    boundary: Boundary,
    start: u16,
    moved: bool,
}

#[derive(Default)]
pub(super) struct State {
    ratios: RefCell<Ratios>,
    frame: RefCell<Vec<Boundary>>,
    hovered: Cell<Option<Id>>,
    drag: Cell<Option<Drag>>,
    last_click: Cell<Option<(Id, Instant)>>,
}
impl State {
    pub fn dragging(&self) -> bool {
        self.drag.get().is_some()
    }
    pub fn adjusted(&self, id: Id) -> bool {
        self.ratios.borrow().contains_key(&id)
    }
    pub fn ratios(&self) -> Ratios {
        self.ratios.borrow().clone()
    }
    pub fn restore(&self, ratios: Ratios) {
        *self.ratios.borrow_mut() = ratios;
        self.invalidate();
    }
    pub fn begin_frame(&self) {
        self.frame.borrow_mut().clear();
    }
    pub fn invalidate(&self) {
        self.begin_frame();
        self.hovered.set(None);
        self.drag.set(None);
        self.last_click.set(None);
    }
    /// The unchanged default is exact; only an explicitly adjusted split uses a ratio.
    pub fn size(&self, id: Id, total: u16, default: u16, min: u16, after_min: u16) -> u16 {
        let ratios = self.ratios.borrow();
        let Some(ratio) = ratios.get(&id) else {
            return default;
        };
        if total < min.saturating_add(after_min) {
            return default;
        }
        (((u32::from(total) * u32::from(*ratio) + SCALE / 2) / SCALE) as u16)
            .clamp(min, total - after_min)
    }

    pub fn record(&self, boundary: Boundary, clip: Rect) {
        if boundary.total > 0
            && boundary.min < boundary.max
            && boundary.before >= boundary.min
            && boundary.before <= boundary.max
            && !boundary.hit.is_empty()
            && boundary.hit.intersection(clip) == boundary.hit
        {
            let mut frame = self.frame.borrow_mut();
            // A split has one live authority, even when a painter projects it more than once.
            frame.retain(|item| item.id != boundary.id);
            frame.push(boundary);
        }
    }
    /// A rule's title, count and links retain their original pointer behavior.
    /// Only actual painted separator strokes accept a resize press.
    pub fn finish_frame(&self, buffer: &Buffer) {
        let recorded = std::mem::take(&mut *self.frame.borrow_mut());
        let mut frame = self.frame.borrow_mut();
        for boundary in recorded {
            if boundary.id.columns() {
                frame.push(boundary);
                continue;
            }
            let area = boundary.hit.intersection(buffer.area);
            let mut start = None;
            for x in area.x..=area.right() {
                let stroke =
                    x < area.right() && matches!(buffer[(x, area.y)].symbol(), "─" | "━" | "┬");
                if stroke {
                    start.get_or_insert(x);
                } else if let Some(begin) = start.take() {
                    frame.push(Boundary {
                        hit: Rect::new(begin, area.y, x - begin, 1),
                        ..boundary
                    });
                }
            }
        }
    }
    pub fn move_pointer(&self, point: Position, active: bool) -> bool {
        let next = active
            .then(|| {
                self.frame
                    .borrow()
                    .iter()
                    .rev()
                    .find(|item| item.hit.contains(point))
                    .map(|item| item.id)
            })
            .flatten();
        self.hovered.replace(next) != next
    }
    fn save(&self, boundary: Boundary, before: u16) -> bool {
        let before = before.clamp(boundary.min, boundary.max);
        let ratio = ((u32::from(before) * SCALE + u32::from(boundary.total) / 2)
            / u32::from(boundary.total)) as u16;
        self.ratios.borrow_mut().insert(boundary.id, ratio) != Some(ratio)
    }
    fn reset(&self, id: Id) -> bool {
        self.ratios.borrow_mut().remove(&id).is_some()
    }
    fn mouse(&self, event: MouseEvent, now: Instant) -> Option<bool> {
        let point = Position::new(event.column, event.row);
        if self.hovered.get().is_some() {
            self.move_pointer(point, true);
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let boundary = *self
                    .frame
                    .borrow()
                    .iter()
                    .rev()
                    .find(|item| item.hit.contains(point))?;
                let double = self.last_click.get().is_some_and(|(id, at)| {
                    id == boundary.id
                        && now.saturating_duration_since(at) <= Duration::from_millis(400)
                });
                if double {
                    self.last_click.set(None);
                    self.drag.set(None);
                    Some(self.reset(boundary.id))
                } else {
                    self.last_click.set(Some((boundary.id, now)));
                    self.drag.set(Some(Drag {
                        boundary,
                        start: boundary.coordinate(point),
                        moved: false,
                    }));
                    Some(false)
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                let mut drag = self.drag.get()?;
                let delta = i32::from(drag.boundary.coordinate(point)) - i32::from(drag.start);
                let before = (i32::from(drag.boundary.before) + delta)
                    .clamp(i32::from(drag.boundary.min), i32::from(drag.boundary.max))
                    as u16;
                let changed = (delta != 0 || drag.moved) && self.save(drag.boundary, before);
                drag.moved |= delta != 0;
                self.drag.set(Some(drag));
                if delta != 0 {
                    self.last_click.set(None);
                }
                if matches!(event.kind, MouseEventKind::Up(_)) {
                    self.drag.set(None);
                    if drag.moved {
                        self.move_pointer(point, true);
                    }
                }
                Some(changed)
            }
            _ => None,
        }
    }
    pub fn hint(&self) -> Option<&'static str> {
        let id = self
            .drag
            .get()
            .map(|drag| drag.boundary.id)
            .or(self.hovered.get())?;
        self.frame
            .borrow()
            .iter()
            .any(|boundary| boundary.id == id)
            .then_some(if id.columns() { "↔" } else { "↕" })
    }
    pub fn highlight(&self, buffer: &mut Buffer, look: crate::look::Look) {
        let id = self
            .drag
            .get()
            .map(|drag| drag.boundary.id)
            .or(self.hovered.get());
        let frame = self.frame.borrow();
        for boundary in frame.iter().filter(|boundary| Some(boundary.id) == id) {
            let area = boundary.hit.intersection(buffer.area);
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if matches!(buffer[(x, y)].symbol(), "│" | "─" | "┬" | "━") {
                        buffer[(x, y)].set_style(
                            look.role(Role::Text)
                                .add_modifier(ratatui::style::Modifier::BOLD),
                        );
                    }
                }
            }
        }
    }
    /// Apply and register only an admitted pair. One-pane responsive layouts have no boundary.
    pub fn panes(&self, mut placement: Placement, columns: Id, rows: Id) -> Placement {
        let (Some(mut before), Some(mut after)) = (placement.list, placement.conversation) else {
            return placement;
        };
        if before.y == after.y {
            let total = before.width + after.width;
            let gap = after.x.saturating_sub(before.right());
            let width = self.size(
                columns,
                total,
                before.width,
                if columns == Id::OverviewColumns {
                    OVERVIEW_MIN_COLUMNS
                } else {
                    LIST_MIN_COLUMNS
                },
                CONVERSATION_MIN_COLUMNS,
            );
            after.x = before.x + width + gap;
            after.width = total - width;
            before.width = width;
        } else {
            let total = before.height + after.height;
            let gap = after.y.saturating_sub(before.bottom());
            let height = self.size(
                rows,
                total,
                before.height,
                LIST_MIN_ROWS,
                CONVERSATION_MIN_ROWS,
            );
            after.y = before.y + height + gap;
            after.height = total - height;
            before.height = height;
        }
        placement.list = Some(before);
        placement.conversation = Some(after);
        placement
    }
    pub fn record_panes(&self, placement: Placement, columns: Id, rows: Id, clip: Rect) {
        let (Some(before), Some(after)) = (placement.list, placement.conversation) else {
            return;
        };
        let boundary = if before.y == after.y {
            Boundary {
                id: columns,
                hit: Rect::new(before.right(), before.y, 1, before.height.min(after.height)),
                total: before.width + after.width,
                before: before.width,
                min: if columns == Id::OverviewColumns {
                    OVERVIEW_MIN_COLUMNS
                } else {
                    LIST_MIN_COLUMNS
                },
                max: (before.width + after.width).saturating_sub(CONVERSATION_MIN_COLUMNS),
            }
        } else {
            Boundary {
                id: rows,
                hit: Rect::new(after.x, after.y, after.width, 1),
                total: before.height + after.height,
                before: before.height,
                min: LIST_MIN_ROWS,
                max: (before.height + after.height).saturating_sub(CONVERSATION_MIN_ROWS),
            }
        };
        self.record(boundary, clip);
    }
}
impl App {
    fn invalidate_after_resize(&self) {
        let drag = self.pane_resize.drag.get();
        self.invalidate_frame_geometry();
        self.pane_resize.drag.set(drag);
    }
    pub(super) fn resize_mouse(&mut self, event: MouseEvent, now: Instant) -> bool {
        if self.modal_open() || self.input.is_some() || self.searching || self.loading() {
            self.pane_resize.invalidate();
            return false;
        }
        let Some(changed) = self.pane_resize.mouse(event, now) else {
            return false;
        };
        if changed {
            self.invalidate_after_resize();
        }
        // Never let a divider press become a row's click/double-click.
        true
    }
    pub(super) fn resize_action(&mut self, action: &crate::action::Action) -> Effect {
        if self.modal_open() || self.input.is_some() || self.searching || self.loading() {
            return Effect::None;
        }
        let id = action
            .args
            .first()
            .and_then(|arg| arg.literal())
            .and_then(Id::parse);
        let direction = action
            .args
            .get(1)
            .and_then(|arg| arg.literal())
            .unwrap_or("");
        let boundary = self
            .pane_resize
            .frame
            .borrow()
            .iter()
            .find(|item| Some(item.id) == id)
            .copied();
        let Some(boundary) = boundary.filter(|boundary| boundary.id.accepts(direction)) else {
            return self.say("That pane boundary is not visible or resizable.");
        };
        let changed = if direction == "reset" {
            self.pane_resize.reset(boundary.id)
        } else {
            let step = (boundary.total / 20).max(1);
            let before = if matches!(direction, "left" | "up") {
                boundary.before.saturating_sub(step)
            } else {
                boundary.before.saturating_add(step)
            };
            self.pane_resize.save(boundary, before)
        };
        if changed {
            self.invalidate_frame_geometry();
        }
        Effect::None
    }
}

#[cfg(test)]
mod tests;
