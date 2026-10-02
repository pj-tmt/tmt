//! Integer cell utilities. No CSS cascade, geometry engine or palette lives here.
use std::collections::BTreeMap;
use tmt_cli_style::theme::Role;

use crate::Kind;

pub const MAX_CELLS: u16 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    Auto,
    Cells(u16),
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFlow {
    Clip,
    Wrap,
    Truncate,
}

/// Static style intent. Full means the parent's available axis, not a cell
/// count. Gap and padding pairs are [horizontal, vertical]; padding is symmetric.
/// None for token preserves inheritance for the later materialization stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStyle {
    pub direction: Direction,
    pub width: Extent,
    pub height: Extent,
    pub gap: [u16; 2],
    pub padding: [u16; 2],
    pub grow: u16,
    pub shrink: u16,
    pub text_flow: TextFlow,
    pub token: Option<Role>,
}

fn cells(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|n| *n <= MAX_CELLS && value.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("requires an integer from 0 to {MAX_CELLS}"))
}

pub(crate) fn admit(kind: Kind, attrs: &BTreeMap<String, String>) -> Result<CellStyle, String> {
    let mut style = CellStyle {
        direction: if matches!(kind, Kind::View | Kind::Col) {
            Direction::Column
        } else {
            Direction::Row
        },
        width: Extent::Auto,
        height: Extent::Auto,
        gap: [0; 2],
        padding: [0; 2],
        grow: 0,
        shrink: 1,
        text_flow: TextFlow::Clip,
        token: None,
    };
    let mut seen = BTreeMap::new();
    let mut claim = |properties: &[&str], value: &str| -> Result<(), String> {
        for &property in properties {
            if let Some(previous) = seen.insert(property.to_owned(), value.to_owned()) {
                return Err(format!("{value} conflicts with {previous} for {property}"));
            }
        }
        Ok(())
    };
    for class in attrs
        .get("class")
        .map(String::as_str)
        .unwrap_or("")
        .split_whitespace()
    {
        let label = format!("class={class:?}");
        let (key, value) = match class {
            "flex" => ("display", None),
            "flex-row" | "flex-col" => ("direction", None),
            "truncate" => ("text-flow", None),
            "w-full" => ("w", None),
            "h-full" => ("h", None),
            "grow" => ("grow", Some(1)),
            "shrink" => ("shrink", Some(1)),
            _ => {
                let (key, raw) = class
                    .rsplit_once('-')
                    .ok_or_else(|| format!("unsupported {label}"))?;
                if ![
                    "w", "h", "gap", "gap-x", "gap-y", "p", "px", "py", "grow", "shrink",
                ]
                .contains(&key)
                {
                    return Err(format!("unsupported {label}"));
                }
                (
                    key,
                    Some(cells(raw).map_err(|why| format!("{label} {why}"))?),
                )
            }
        };
        let properties: &[&str] = match key {
            "gap" => &["gap-x", "gap-y"],
            "p" => &["px", "py"],
            _ => &[key],
        };
        claim(properties, &label)?;
        match (key, value) {
            ("direction", _) => {
                style.direction = if class == "flex-row" {
                    Direction::Row
                } else {
                    Direction::Column
                }
            }
            ("w", Some(n)) => style.width = Extent::Cells(n),
            ("h", Some(n)) => style.height = Extent::Cells(n),
            ("w", None) => style.width = Extent::Full,
            ("h", None) => style.height = Extent::Full,
            ("gap", Some(n)) => style.gap = [n; 2],
            ("gap-x", Some(n)) => style.gap[0] = n,
            ("gap-y", Some(n)) => style.gap[1] = n,
            ("p", Some(n)) => style.padding = [n; 2],
            ("px", Some(n)) => style.padding[0] = n,
            ("py", Some(n)) => style.padding[1] = n,
            ("grow", Some(n)) => style.grow = n,
            ("shrink", Some(n)) => style.shrink = n,
            ("text-flow", _) => style.text_flow = TextFlow::Truncate,
            _ => {}
        }
    }
    if let Some(value) = attrs.get("wrap") {
        let label = format!("wrap={value:?}");
        claim(&["text-flow"], &label)?;
        style.text_flow = match value.as_str() {
            "true" => TextFlow::Wrap,
            "false" => TextFlow::Clip,
            _ => return Err(format!("{label} must be true or false")),
        };
    }
    if let Some(value) = attrs.get("token") {
        style.token = Some(Role::parse(value).ok_or_else(|| {
            format!(
                "unknown token={value:?}; expected {}",
                Role::ALL.map(Role::name).join(", ")
            )
        })?);
    }
    Ok(style)
}

#[cfg(test)]
mod tests;
