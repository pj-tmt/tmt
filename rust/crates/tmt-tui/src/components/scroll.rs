//! Visual-line scrolling: Keys, Lists and selection, and Narrow widths in the
//! Full-screen interaction guideline. No application pane names or frame owner.
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    layout::Rect,
};
use std::ops::Range;

pub const WHEEL_LINES: isize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Lines(isize),
    Pages(isize),
    Top,
    Bottom,
}

#[derive(Debug, Default)]
pub struct ScrollState {
    offset: usize,
    content: usize,
    area: Rect,
}

impl ScrollState {
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn content(&self) -> usize {
        self.content
    }
    pub fn viewport(&self) -> Rect {
        self.area
    }
    pub fn limit(&self) -> usize {
        self.content.saturating_sub(usize::from(self.area.height))
    }

    /// Publish current geometry before routing input. Resize and content shrink
    /// clamp the offset; an optional selected visual range remains visible.
    pub fn update(&mut self, area: Rect, content: usize, selected: Option<Range<usize>>) {
        self.area = area;
        self.content = content;
        self.offset = self.offset.min(self.limit());
        if let Some(range) = selected {
            self.reveal(range);
        }
    }

    pub fn reveal(&mut self, range: Range<usize>) {
        let height = usize::from(self.area.height);
        if height == 0 || range.is_empty() {
            return;
        }
        if range.len() > height {
            // An oversized selected row can be read by scrolling within it.
            // Keep any existing window inside the row, rather than snapping to
            // its first line on every repaint or wrapped-width change.
            self.offset = self
                .offset
                .max(range.start)
                .min(range.end.saturating_sub(height));
        } else if range.start < self.offset {
            self.offset = range.start;
        } else if range.end > self.offset.saturating_add(height) {
            self.offset = range.end.saturating_sub(height);
        }
        self.offset = self.offset.min(self.limit());
    }

    pub fn step(&mut self, step: Step) {
        let page = usize::from(self.area.height).saturating_sub(1).max(1) as isize;
        self.offset = match step {
            Step::Lines(lines) => self.offset.saturating_add_signed(lines),
            Step::Pages(pages) => self
                .offset
                .saturating_add_signed(pages.saturating_mul(page)),
            Step::Top => 0,
            Step::Bottom => self.limit(),
        }
        .min(self.limit());
    }

    /// Scroll keys apply only outside editable fields. A field's handler runs
    /// first through `app::route`; modifiers do not accidentally become scroll.
    pub fn input(&mut self, event: &Event) -> bool {
        let step = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => Step::Lines(-1),
                    KeyCode::Down | KeyCode::Char('j') => Step::Lines(1),
                    KeyCode::PageUp => Step::Pages(-1),
                    KeyCode::PageDown => Step::Pages(1),
                    KeyCode::Home | KeyCode::Char('g') => Step::Top,
                    KeyCode::End | KeyCode::Char('G') => Step::Bottom,
                    _ => return false,
                }
            }
            Event::Mouse(mouse) if self.area.contains((mouse.column, mouse.row).into()) => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => Step::Lines(-WHEEL_LINES),
                    MouseEventKind::ScrollDown => Step::Lines(WHEEL_LINES),
                    _ => return false,
                }
            }
            _ => return false,
        };
        self.step(step);
        true
    }

    pub fn position(&self) -> String {
        if self.content == 0 || self.area.height == 0 {
            return String::new();
        }
        let end = self
            .offset
            .saturating_add(usize::from(self.area.height))
            .min(self.content);
        let below = self.content.saturating_sub(end);
        if below > 0 {
            format!(
                "{}–{end} of {} · {below} more ↓",
                self.offset + 1,
                self.content
            )
        } else {
            format!("{}–{end} of {}", self.offset + 1, self.content)
        }
    }
}
