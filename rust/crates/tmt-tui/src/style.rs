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
    Percent(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFlow {
    Clip,
    Wrap,
    Truncate,
    Middle,
    Clamp(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Flex,
    Grid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breadth {
    Auto,
    Cells(u16),
    Percent(u8),
    Fraction(u16),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridTrack {
    pub min: Breadth,
    pub max: Breadth,
}

/// Static style intent. Full means the parent's available axis, not a cell
/// count. Gap and padding pairs are [horizontal, vertical]; padding is symmetric.
/// None for token preserves inheritance for the later materialization stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStyle {
    pub display: Display,
    pub direction: Direction,
    pub basis: Extent,
    pub min_width: Option<u16>,
    pub max_width: Option<u16>,
    pub columns: std::sync::Arc<[GridTrack]>,
    pub col_span: u16,
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

fn percent(raw: &str) -> Result<u8, String> {
    cells(raw.strip_suffix('%').unwrap_or(raw))?
        .try_into()
        .ok()
        .filter(|n| *n <= 100)
        .ok_or_else(|| "requires an integer percentage from 0 to 100".into())
}
fn breadth(raw: &str) -> Result<Breadth, String> {
    if raw == "auto" {
        Ok(Breadth::Auto)
    } else if raw.ends_with('%') {
        Ok(Breadth::Percent(percent(raw)?))
    } else if let Some(n) = raw.strip_suffix("fr") {
        Ok(Breadth::Fraction(cells(n)?))
    } else {
        Ok(Breadth::Cells(cells(raw)?))
    }
}
fn tracks(raw: &str) -> Result<Vec<GridTrack>, String> {
    raw.split('_')
        .map(|raw| {
            if let Some(inner) = raw
                .strip_prefix("minmax(")
                .and_then(|s| s.strip_suffix(')'))
            {
                let (a, b) = inner
                    .split_once(',')
                    .ok_or("minmax requires two endpoints")?;
                let min = breadth(a)?;
                let max = breadth(b)?;
                if max == Breadth::Auto {
                    return Err(
                        "auto is a bare track or a minmax minimum; use auto or minmax(auto,N)"
                            .into(),
                    );
                }
                if matches!(min, Breadth::Fraction(_)) {
                    return Err("minmax minimum cannot be fr".into());
                }
                Ok(GridTrack { min, max })
            } else {
                let max = breadth(raw)?;
                Ok(GridTrack {
                    min: if matches!(max, Breadth::Fraction(_)) {
                        Breadth::Auto
                    } else {
                        max
                    },
                    max,
                })
            }
        })
        .collect()
}

pub(crate) fn admit(kind: Kind, attrs: &BTreeMap<String, String>) -> Result<CellStyle, String> {
    let mut style = CellStyle {
        display: Display::Flex,
        basis: Extent::Auto,
        min_width: None,
        max_width: None,
        columns: Default::default(),
        col_span: 1,
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
        let mut percentage = None;
        let (key, value) = match class {
            "flex" | "grid" => ("display", None),
            "flex-row" | "flex-col" => ("direction", None),
            "truncate" | "truncate-middle" => ("text-flow", None),
            "text-ellipsis-middle" => {
                return Err(format!("unsupported {label}; hint: use truncate-middle"));
            }
            _ if class.starts_with("grid-cols-[") => ("columns", None),
            "w-full" => ("w", None),
            "h-full" => ("h", None),
            "grow" => ("grow", Some(1)),
            "shrink" => ("shrink", Some(1)),
            _ => {
                let (key, raw) = [
                    "min-w",
                    "max-w",
                    "col-span",
                    "line-clamp",
                    "gap-x",
                    "gap-y",
                    "basis",
                    "grow",
                    "shrink",
                    "gap",
                    "px",
                    "py",
                    "p",
                    "w",
                    "h",
                ]
                .iter()
                .find_map(|key| {
                    class
                        .strip_prefix(&format!("{key}-"))
                        .map(|raw| (*key, raw))
                })
                .ok_or_else(|| {
                    format!("unsupported {label}; hint: use cell utilities or grid-cols-[tracks]")
                })?;
                let bracketed = raw.starts_with('[');
                let raw = if bracketed {
                    raw.strip_prefix('[')
                        .and_then(|s| s.strip_suffix(']'))
                        .ok_or_else(|| format!("malformed {label}"))?
                } else {
                    raw
                };
                let is_percent = raw.ends_with('%');
                let percent_key = ["w", "h", "basis"].contains(&key);
                if bracketed != is_percent || (is_percent && !percent_key) {
                    let hint = if is_percent && percent_key {
                        format!("{key}-[n%]")
                    } else {
                        format!("{key}-N (cells or counts)")
                    };
                    return Err(format!("unsupported {label}; hint: use {hint}"));
                }
                if is_percent {
                    let n = percent(raw).map_err(|why| format!("{label} {why}"))?;
                    percentage = Some(n);
                    (key, None)
                } else {
                    (
                        key,
                        Some(cells(raw).map_err(|why| format!("{label} {why}"))?),
                    )
                }
            }
        };
        let properties: &[&str] = match key {
            "gap" => &["gap-x", "gap-y"],
            "line-clamp" => &["text-flow"],
            "p" => &["px", "py"],
            _ => &[key],
        };
        claim(properties, &label)?;
        if let Some(n) = percentage {
            match key {
                "w" => style.width = Extent::Percent(n),
                "h" => style.height = Extent::Percent(n),
                _ => style.basis = Extent::Percent(n),
            }
            continue;
        }
        match (key, value) {
            ("display", _) => {
                style.display = if class == "grid" {
                    Display::Grid
                } else {
                    Display::Flex
                }
            }
            ("columns", _) => {
                style.columns = tracks(
                    class
                        .strip_prefix("grid-cols-[")
                        .and_then(|s| s.strip_suffix(']'))
                        .ok_or_else(|| format!("malformed {label}"))?,
                )
                .map_err(|why| format!("{label} {why}"))?
                .into()
            }
            ("basis", Some(n)) => style.basis = Extent::Cells(n),
            ("min-w", Some(n)) => style.min_width = Some(n),
            ("max-w", Some(n)) => style.max_width = Some(n),
            ("col-span", Some(n)) if n > 0 => style.col_span = n,
            ("line-clamp", Some(n)) if n > 0 => style.text_flow = TextFlow::Clamp(n),
            ("col-span" | "line-clamp", _) => {
                return Err(format!("{label} requires a positive integer"));
            }
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
            ("text-flow", _) => {
                style.text_flow = if class == "truncate" {
                    TextFlow::Truncate
                } else {
                    TextFlow::Middle
                }
            }
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
    if !style.columns.is_empty() && style.display != Display::Grid {
        return Err(format!(
            "{} requires grid",
            seen.get("columns").expect("admitted columns")
        ));
    }
    if style.display == Display::Grid && seen.contains_key("direction") {
        return Err(format!("{} conflicts with grid", seen["direction"]));
    }
    Ok(style)
}

#[cfg(test)]
mod tests;
