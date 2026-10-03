//! Stable row selection and visual-range navigation, without application actions.
use super::{ScrollState, Step};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    layout::Rect,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub id: String,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListEvent {
    Changed(String),
    Confirm(String),
}

#[derive(Debug, Clone)]
pub struct RowGeometry {
    pub id: String,
    pub lines: Range<usize>,
    pub visible: Rect,
}

/// Geometry is a snapshot of the rows and viewport used for painting. A changed
/// model invalidates it; the caller also discards it on resize before routing.
#[derive(Debug, Clone)]
pub struct ListFrame {
    pub rows: Vec<ListRow>,
    pub geometry: Vec<RowGeometry>,
    pub viewport: Rect,
    pub offset: usize,
}

#[derive(Debug, Default)]
pub struct ListState {
    rows: Vec<ListRow>,
    selected: Option<String>,
    pub scroll: ScrollState,
}
impl ListState {
    pub fn rows(&self) -> &[ListRow] {
        &self.rows
    }
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Validate before changing state. Reorder follows identity. Removal prefers
    /// the closest enabled survivor in the old order; new rows are the fallback.
    pub fn reconcile(&mut self, rows: Vec<ListRow>) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for row in &rows {
            if !crate::binding::stable_id(&row.id) || !ids.insert(&row.id) {
                return Err("list rows require unique stable IDs".into());
            }
        }
        let enabled: BTreeSet<_> = rows
            .iter()
            .filter(|row| !row.disabled)
            .map(|row| row.id.as_str())
            .collect();
        let old = self
            .rows
            .iter()
            .position(|row| Some(&row.id) == self.selected.as_ref());
        let survives = self
            .selected
            .as_ref()
            .is_some_and(|id| enabled.contains(id.as_str()));
        if !survives {
            let nearest = old.and_then(|position| {
                self.rows
                    .iter()
                    .enumerate()
                    .filter(|(_, old)| enabled.contains(old.id.as_str()))
                    .min_by_key(|(index, _)| (index.abs_diff(position), *index))
                    .map(|(_, row)| row.id.clone())
            });
            self.selected = nearest.or_else(|| {
                rows.iter()
                    .enumerate()
                    .filter(|(_, row)| !row.disabled)
                    .min_by_key(|(index, _)| index.abs_diff(old.unwrap_or(0)))
                    .map(|(_, row)| row.id.clone())
            });
        }
        self.rows = rows;
        Ok(())
    }

    pub fn select(&mut self, id: &str) -> Option<ListEvent> {
        if self.selected() == Some(id) || !self.rows.iter().any(|row| row.id == id && !row.disabled)
        {
            return None;
        }
        self.selected = Some(id.into());
        Some(ListEvent::Changed(id.into()))
    }
    pub fn confirm(&self) -> Option<ListEvent> {
        self.selected()
            .filter(|id| self.rows.iter().any(|row| row.id == *id && !row.disabled))
            .map(|id| ListEvent::Confirm(id.into()))
    }

    pub fn input(&mut self, event: &Event, frame: &ListFrame) -> Option<ListEvent> {
        if frame.rows != self.rows || frame.viewport != self.scroll.viewport() {
            return None;
        }
        let step = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                match key.code {
                    KeyCode::Enter => return self.confirm(),
                    KeyCode::Up | KeyCode::Char('k') => Step::Lines(-1),
                    KeyCode::Down | KeyCode::Char('j') => Step::Lines(1),
                    KeyCode::PageUp => Step::Pages(-1),
                    KeyCode::PageDown => Step::Pages(1),
                    KeyCode::Home | KeyCode::Char('g') => Step::Top,
                    KeyCode::End | KeyCode::Char('G') => Step::Bottom,
                    _ => return None,
                }
            }
            Event::Mouse(mouse)
                if frame.offset == self.scroll.offset()
                    && frame.viewport.contains((mouse.column, mouse.row).into()) =>
            {
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) && frame
                    .geometry
                    .iter()
                    .find(|row| Some(row.id.as_str()) == self.selected())
                    .is_some_and(|row| row.lines.len() > usize::from(frame.viewport.height))
                {
                    self.scroll.input(event);
                    return None;
                }
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        let row =
                            frame.geometry.iter().rev().find(|row| {
                                row.visible.contains((mouse.column, mouse.row).into())
                            })?;
                        return self.select(&row.id);
                    }
                    MouseEventKind::ScrollUp => Step::Lines(-super::scroll::WHEEL_LINES),
                    MouseEventKind::ScrollDown => Step::Lines(super::scroll::WHEEL_LINES),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let enabled: Vec<_> = self.rows.iter().filter(|row| !row.disabled).collect();
        if enabled.is_empty() {
            return None;
        }
        let current = enabled
            .iter()
            .position(|row| Some(row.id.as_str()) == self.selected())
            .unwrap_or(0);
        let index = match step {
            Step::Lines(lines) => current.saturating_add_signed(lines).min(enabled.len() - 1),
            Step::Top => 0,
            Step::Bottom => enabled.len() - 1,
            Step::Pages(pages) => {
                let start = frame
                    .geometry
                    .iter()
                    .find(|row| Some(row.id.as_str()) == self.selected())
                    .map_or(0, |row| row.lines.start);
                let page = usize::from(frame.viewport.height).saturating_sub(1).max(1);
                let target = start.saturating_add_signed(pages.saturating_mul(page as isize));
                let ranges: BTreeMap<_, _> = frame
                    .geometry
                    .iter()
                    .map(|row| (row.id.as_str(), row.lines.start))
                    .collect();
                let nearest = enabled
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        ranges
                            .get(row.id.as_str())
                            .map(|start| (index, start.abs_diff(target)))
                    })
                    .min_by_key(|(_, distance)| *distance)
                    .map_or(current, |(index, _)| index);
                if nearest == current {
                    current
                        .saturating_add_signed(pages.signum())
                        .min(enabled.len() - 1)
                } else {
                    nearest
                }
            }
        };
        let id = enabled[index].id.clone();
        let effect = self.select(&id);
        if let Some(row) = frame.geometry.iter().find(|row| row.id == id) {
            self.scroll.reveal(row.lines.clone());
        }
        effect
    }
}

#[cfg(test)]
mod tests;
