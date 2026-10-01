//! How the board's panes sit: a split is a row or a column of panes, each
//! with a size or a grow share, and splits nest. `[squad.<name>.board]`'s
//! `mode`/`direction`/`panes`/`sizes` keys are the one-level form of it;
//! `layout = { direction, sizes, panes }` is the full form.

use crate::{
    config::{Direction, Pane},
    core::SquadError,
};
use ratatui::layout::{Constraint, Layout, Rect};
use std::collections::BTreeSet;
use toml_edit::{InlineTable, Item, TableLike, Value};

/// Deepest nesting a layout may have.
const MAX_DEPTH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// A percentage of the split.
    Percent(u16),
    /// A share of what the percentages leave.
    Grow(u16),
}

impl Size {
    pub fn constraint(self) -> Constraint {
        match self {
            Self::Percent(percent) => Constraint::Percentage(percent),
            Self::Grow(share) => Constraint::Fill(share),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Split {
    Pane(Pane),
    Group {
        direction: Direction,
        children: Vec<(Size, Split)>,
    },
}

impl Split {
    /// One level: `panes` side by side (or stacked) at `sizes` percent.
    pub fn simple(direction: Direction, panes: &[Pane], sizes: &[u16]) -> Self {
        Self::Group {
            direction,
            children: panes
                .iter()
                .zip(sizes)
                .map(|(pane, size)| (Size::Percent(*size), Self::Pane(*pane)))
                .collect(),
        }
    }

    /// Title footprint when this entire subtree is folded.
    fn footprint(&self, collapsed: &BTreeSet<Pane>) -> Option<(u16, u16)> {
        match self {
            Self::Pane(pane) => collapsed
                .contains(pane)
                .then(|| (2 + pane.title().len() as u16, 1)),
            Self::Group {
                direction,
                children,
            } => {
                let sizes = children
                    .iter()
                    .map(|(_, child)| child.footprint(collapsed))
                    .collect::<Option<Vec<_>>>()?;
                Some(match direction {
                    Direction::LeftRight => (
                        sizes.iter().map(|(w, _)| *w).sum(),
                        sizes.iter().map(|(_, h)| *h).max().unwrap_or(0),
                    ),
                    Direction::TopBottom => (
                        sizes.iter().map(|(w, _)| *w).max().unwrap_or(0),
                        sizes.iter().map(|(_, h)| *h).sum(),
                    ),
                })
            }
        }
    }

    /// The existing split tree is the sole placement owner. Folded children
    /// reserve their title footprint; expanded siblings share the remainder.
    /// With no folded children the original ratatui constraints are unchanged.
    pub fn solve(&self, mut area: Rect, collapsed: &BTreeSet<Pane>) -> Vec<(Pane, Rect)> {
        if let Self::Pane(pane) = self {
            if collapsed.contains(pane) {
                area.height = area.height.min(1);
            }
            return vec![(*pane, area)];
        }
        if let Some((width, height)) = self.footprint(collapsed) {
            area.width = area.width.min(width);
            area.height = area.height.min(height);
        }
        let Self::Group {
            direction,
            children,
        } = self
        else {
            unreachable!("pane handled above")
        };
        let horizontal = *direction == Direction::LeftRight;
        let layout = |constraints: Vec<Constraint>, area| {
            if horizontal {
                Layout::horizontal(constraints).split(area)
            } else {
                Layout::vertical(constraints).split(area)
            }
        };
        let folded: Vec<_> = children
            .iter()
            .map(|(_, child)| child.footprint(collapsed))
            .collect();
        let areas = if folded.iter().all(Option::is_none) {
            layout(
                children.iter().map(|(size, _)| size.constraint()).collect(),
                area,
            )
            .to_vec()
        } else {
            let length = if horizontal { area.width } else { area.height };
            // Reserve titles in reading order, clipping before the solver so
            // even an undersized terminal has disjoint, bounded hit regions.
            let mut remaining = length;
            let fixed: Vec<_> = folded
                .iter()
                .map(|size| {
                    let wanted = size.map_or(0, |(w, h)| if horizontal { w } else { h });
                    let given = wanted.min(remaining);
                    remaining -= given;
                    given
                })
                .collect();
            let percent: u32 = children
                .iter()
                .map(|(size, _)| match size {
                    Size::Percent(p) => u32::from(*p),
                    Size::Grow(_) => 0,
                })
                .sum();
            let grow: u32 = children
                .iter()
                .map(|(size, _)| match size {
                    Size::Grow(g) => u32::from(*g),
                    Size::Percent(_) => 0,
                })
                .sum();
            let weights: Vec<_> = children
                .iter()
                .zip(&folded)
                .map(|((size, _), folded)| {
                    if folded.is_some() {
                        return 0;
                    }
                    match size {
                        Size::Percent(p) => u32::from(*p) * grow.max(1),
                        Size::Grow(g) => (100 - percent) * u32::from(*g),
                    }
                })
                .collect();
            let total: u32 = weights.iter().sum();
            let count = folded.iter().filter(|size| size.is_none()).count() as u32;
            let constraints = weights
                .iter()
                .zip(&folded)
                .filter_map(|(weight, folded)| {
                    folded.is_none().then_some(if total == 0 {
                        Constraint::Ratio(1, count.max(1))
                    } else {
                        Constraint::Ratio(*weight, total)
                    })
                })
                .collect();
            let free = if horizontal {
                Rect {
                    width: remaining,
                    ..area
                }
            } else {
                Rect {
                    height: remaining,
                    ..area
                }
            };
            let expanded = layout(constraints, free);
            let mut expanded = expanded.iter();
            let mut offset = 0;
            fixed
                .iter()
                .zip(&folded)
                .map(|(fixed, folded)| {
                    let length = if folded.is_some() {
                        *fixed
                    } else {
                        expanded
                            .next()
                            .map_or(0, |rect| if horizontal { rect.width } else { rect.height })
                    };
                    let rect = if horizontal {
                        Rect {
                            x: area.x + offset,
                            width: length,
                            ..area
                        }
                    } else {
                        Rect {
                            y: area.y + offset,
                            height: length,
                            ..area
                        }
                    };
                    offset += length;
                    rect
                })
                .collect()
        };
        children
            .iter()
            .zip(areas)
            .flat_map(|((_, child), rect)| child.solve(rect, collapsed))
            .collect()
    }

    /// Every pane in reading order: the order Tab moves focus.
    pub fn panes(&self) -> Vec<Pane> {
        match self {
            Self::Pane(pane) => vec![*pane],
            Self::Group { children, .. } => children
                .iter()
                .flat_map(|(_, child)| child.panes())
                .collect(),
        }
    }
}

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

/// `layout = { direction, sizes?, panes }`, validated whole: rows is
/// present, no pane appears twice, nesting stays within three levels.
pub fn read(item: &Item, place: &str) -> Result<Split, SquadError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
    let split = group(table, place, 1)?;
    let panes = split.panes();
    if !panes.contains(&Pane::Rows) {
        return Err(invalid(format!("`{place}` must include rows.")));
    }
    for (index, pane) in panes.iter().enumerate() {
        if panes[..index].contains(pane) {
            return Err(invalid(format!("`{place}` lists {} twice.", pane.title())));
        }
    }
    Ok(split)
}

fn group(table: &dyn TableLike, place: &str, depth: usize) -> Result<Split, SquadError> {
    if depth > MAX_DEPTH {
        return Err(invalid(format!(
            "`{place}` nests deeper than {MAX_DEPTH} levels."
        )));
    }
    for (key, _) in table.iter() {
        if !matches!(key, "direction" | "sizes" | "panes") {
            return Err(invalid(format!("`{place}.{key}` is not a layout setting.")));
        }
    }
    let direction = match table.get("direction").and_then(Item::as_str) {
        Some("left-right") => Direction::LeftRight,
        Some("top-bottom") => Direction::TopBottom,
        _ => {
            return Err(invalid(format!(
                "`{place}.direction` must be left-right or top-bottom."
            )));
        }
    };
    let panes = table
        .get("panes")
        .and_then(Item::as_array)
        .filter(|panes| !panes.is_empty())
        .ok_or_else(|| invalid(format!("`{place}.panes` must list at least one pane.")))?;
    let children = panes
        .iter()
        .enumerate()
        .map(|(index, entry)| child(entry, &format!("{place}.panes[{index}]"), depth))
        .collect::<Result<Vec<_>, _>>()?;
    let sizes = match table.get("sizes") {
        // Unsized panes share the split equally.
        None => vec![Size::Grow(1); children.len()],
        Some(sizes) => {
            let sizes = sizes
                .as_array()
                .filter(|sizes| sizes.len() == children.len())
                .ok_or_else(|| invalid(format!("`{place}.sizes` needs one size per pane.")))?;
            sizes
                .iter()
                .enumerate()
                .map(|(index, size)| read_size(size, &format!("{place}.sizes[{index}]")))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let percent: u16 = sizes
        .iter()
        .map(|size| match size {
            Size::Percent(percent) => *percent,
            Size::Grow(_) => 0,
        })
        .sum();
    let all_percent = sizes.iter().all(|size| matches!(size, Size::Percent(_)));
    if percent > 100 || (all_percent && percent != 100) {
        return Err(invalid(format!(
            "`{place}.sizes` percentages must total 100, or at most 100 with grow shares."
        )));
    }
    Ok(Split::Group {
        direction,
        children: sizes.into_iter().zip(children).collect(),
    })
}

fn child(entry: &Value, place: &str, depth: usize) -> Result<Split, SquadError> {
    if let Some(name) = entry.as_str() {
        return Pane::parse(name).map(Split::Pane).ok_or_else(|| {
            invalid(format!(
                "`{place}` must be rows, notes, detail, replies or a split."
            ))
        });
    }
    let table: &InlineTable = entry.as_inline_table().ok_or_else(|| {
        invalid(format!(
            "`{place}` must be rows, notes, detail, replies or a split."
        ))
    })?;
    group(table, place, depth + 1)
}

/// A percentage (10-100) or `{ grow = n }` (1-100).
fn read_size(size: &Value, place: &str) -> Result<Size, SquadError> {
    if let Some(percent) = size.as_integer() {
        return u16::try_from(percent)
            .ok()
            .filter(|percent| (10..=100).contains(percent))
            .map(Size::Percent)
            .ok_or_else(|| invalid(format!("`{place}` must be 10-100 percent.")));
    }
    let table = size
        .as_inline_table()
        .ok_or_else(|| invalid(format!("`{place}` must be a percentage or {{ grow = n }}.")))?;
    let mut share = None;
    for (key, value) in table.iter() {
        match key {
            "grow" => {
                share = value
                    .as_integer()
                    .and_then(|share| u16::try_from(share).ok())
                    .filter(|share| (1..=100).contains(share));
                if share.is_none() {
                    return Err(invalid(format!("`{place}.grow` must be 1-100.")));
                }
            }
            other => return Err(invalid(format!("`{place}.{other}` is not a size setting."))),
        }
    }
    share
        .map(Size::Grow)
        .ok_or_else(|| invalid(format!("`{place}` must be a percentage or {{ grow = n }}.")))
}

#[cfg(test)]
mod tests;
