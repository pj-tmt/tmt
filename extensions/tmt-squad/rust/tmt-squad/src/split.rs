//! How the board's panes sit: a split is a row or a column of panes, each
//! with a size or a grow share, and splits nest. `[squad.<name>.board]`'s
//! `mode`/`direction`/`panes`/`sizes` keys are the one-level form of it;
//! `layout = { direction, sizes, panes }` is the full form.

use crate::{
    config::{Direction, Pane},
    core::SquadError,
};
use ratatui::layout::Constraint;
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
