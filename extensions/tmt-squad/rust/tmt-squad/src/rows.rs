//! How a squad's rows are laid out: one shared grid of columns, and the
//! lines each row takes, where a cell may span several columns. The board
//! compiles its shared grid through tmt-tui; `ls` keeps natural list
//! sizing unless a shown column opts into percentage width or overflow.
//!
//! `[squad.<name>.rows]` is the full form; the older `[squad.<name>.columns]`
//! (`show` plus a `title`/`width` per field) reads as the same model.

use crate::{
    core::SquadError,
    source::{ColumnSource, Format, PATHS},
};
use serde_json::{Value, json};
#[cfg(test)]
use tmt_cli_style::grid;
use tmt_cli_style::{
    Role,
    grid::{Align, Basis, Overflow, Track, Truncate},
};
use toml_edit::{Item, TableLike};

const MAX_COLUMNS: usize = 12;
const MAX_LINES: usize = 4;
const MAX_WIDTH: i64 = 200;
/// Fields kept as Squad's own data (the name, free-text role, a state's
/// order and color, a decision owed). A bound value replaces the field of its
/// column's name, so these cannot be bound. Retired `note` remains reserved.
pub(crate) const OWN_FIELDS: &[&str] = &["member", "role", "state", "pending", "note"];
/// Where a growing column starts, and the least an unsized one keeps.
pub(super) const NARROWEST: usize = 4;

/// One grid column: the field it shows by default, its header and sizing.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub field: String,
    pub title: String,
    /// A cell width or percentage basis; `min`/`max` stay in cells.
    pub width: Option<Basis>,
    pub min: Option<u16>,
    pub max: Option<u16>,
    pub grow: u16,
    pub align: Align,
    pub truncate: Truncate,
    pub overflow: Option<Overflow>,
    /// Steps aside on a narrow board, highest first; None never does.
    pub priority: Option<u16>,
    /// Where the value comes from, when not the squad field of its name.
    pub from: Option<ColumnSource>,
    pub format: Format,
    /// Numeric thresholds, strictly increasing: a value at or above `at`
    /// shows that token, the highest reached winning.
    pub color: Vec<Threshold>,
}

/// Existing lists keep natural sizing unless a shown column opts into the
/// new configured fitting contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSizing {
    Natural,
    Configured,
}

impl ListSizing {
    pub fn for_columns(columns: &[Column]) -> Self {
        if columns.iter().any(|column| {
            matches!(column.width, Some(Basis::Percent(_))) || column.overflow.is_some()
        }) {
            Self::Configured
        } else {
            Self::Natural
        }
    }
}

/// From `at` upward, a cell's value shows `token` (a theme token).
#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    pub at: f64,
    pub token: String,
}

impl Column {
    fn new(field: &str, title: Option<&str>) -> Self {
        Self {
            title: title.map_or_else(|| field.to_uppercase(), str::to_owned),
            field: field.to_owned(),
            width: None,
            min: None,
            max: None,
            grow: 0,
            align: Align::Left,
            truncate: Truncate::End,
            overflow: None,
            priority: None,
            from: None,
            format: Format::Text,
            color: Vec::new(),
        }
    }

    /// Display settings published by `Rows::value`; values and sources have
    /// already been projected into the document's full row fields.
    pub fn display(field: &str, value: &Value) -> Self {
        let mut column = Self::new(field, value["title"].as_str());
        if value["valueOnly"] == true {
            return column;
        }
        column.width = value["width"]
            .as_u64()
            .map(|width| Basis::Cells(width as usize))
            .or_else(|| {
                value["width"]
                    .as_str()?
                    .strip_suffix('%')?
                    .parse()
                    .ok()
                    .map(Basis::Percent)
            });
        column.min = value["min"].as_u64().map(|value| value as u16);
        column.max = value["max"].as_u64().map(|value| value as u16);
        column.grow = value["grow"].as_u64().unwrap_or(0) as u16;
        column.priority = value["priority"].as_u64().map(|value| value as u16);
        column.align = match value["align"].as_str() {
            Some("right") => Align::Right,
            Some("center") => Align::Center,
            _ => Align::Left,
        };
        column.truncate = if value["truncate"] == "middle" {
            Truncate::Middle
        } else {
            Truncate::End
        };
        column.overflow = match value["overflow"].as_str() {
            Some("wrap") => Some(Overflow::Wrap {
                max_lines: value["max_lines"].as_u64().unwrap_or(2).clamp(1, 8) as u8,
            }),
            Some("ellipsis") => Some(Overflow::Ellipsis),
            _ => None,
        };
        column
    }

    /// The token of the highest threshold `number` reaches, if any.
    pub fn threshold(&self, number: f64) -> Option<&str> {
        self.color
            .iter()
            .rev()
            .find(|threshold| number >= threshold.at)
            .map(|threshold| threshold.token.as_str())
    }

    /// Where the column's value comes from, when it is not shown as the
    /// member's own squad field; None for a plain column.
    pub fn source(&self) -> Option<ColumnSource> {
        match (&self.from, self.format) {
            (Some(from), _) => Some(from.clone()),
            (None, Format::Text) => None,
            (None, _) => {
                ColumnSource::parse(&format!("meta.squad.{}", self.field), field_name, |_| false)
            }
        }
    }

    fn sized(field: &str, title: &str, width: Option<Basis>) -> Self {
        let mut column = Self::new(field, Some(title));
        column.width = width;
        if width.is_none() {
            // A column without a width shares what is left.
            column.grow = 1;
        }
        column
    }

    /// Its solver track; `natural` is its widest value, used when it has
    /// neither a width nor a grow share.
    pub fn track(&self, natural: usize) -> Track {
        let width = self.width;
        let basis = match (width, self.grow) {
            (Some(width), _) => width,
            (None, 0) => Basis::Cells(natural),
            (None, _) => Basis::Cells(0),
        };
        let min = self.min.map_or_else(
            || match (width, self.grow) {
                (Some(Basis::Cells(width)), _) => width,
                (Some(Basis::Percent(_)), _) => {
                    NARROWEST.min(self.max.map_or(NARROWEST, usize::from))
                }
                (None, 0) => NARROWEST.min(natural),
                (None, _) => NARROWEST,
            },
            usize::from,
        );
        Track {
            basis,
            min,
            max: self.max.map(usize::from),
            grow: self.grow,
            shrink: 0,
            priority: self.priority,
        }
    }
}

/// One cell of a line: a field (None for an empty cell) across `span`
/// columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub field: Option<String>,
    pub span: usize,
    /// Declarative semantic style; omission keeps the projected field color.
    pub token: Option<Role>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rows {
    pub columns: Vec<Column>,
    /// Explicitly hidden original tracks; field values and spans remain intact.
    pub hidden_columns: Vec<String>,
    /// At least one line; each line's spans cover at most every column.
    pub lines: Vec<Vec<Cell>>,
}

impl Rows {
    /// Every preset's rows: the name, state, task and pull request.
    pub fn preset() -> Self {
        // On a narrow board the link steps aside before anything is cut off.
        let mut link = Column::sized("pr_link", "PR", Some(Basis::Cells(12)));
        link.priority = Some(1);
        Self::with_one_line(vec![
            Column::sized("member", "MEMBER", Some(Basis::Cells(14))),
            Column::sized("state", "STATE", Some(Basis::Cells(10))),
            Column::sized("task", "TASK", None),
            link,
        ])
    }

    /// Board-only observation columns; public ls grids and custom grids stay intact.
    pub fn with_usage(mut self, windows: [crate::config::TokenWindow; 3]) -> Self {
        if !self.columns.iter().any(|c| c.field == "model") {
            let mut model = Column::sized("model", "MODEL", Some(Basis::Cells(10)));
            model.from = ColumnSource::parse("session.model", field_name, |_| false);
            model.priority = Some(3);
            self.lines[0].push(Cell {
                field: Some("model".into()),
                span: 1,
            });
            self.columns.push(model);
        }
        for (i, window) in windows.into_iter().enumerate() {
            let field = format!("tok_{}", i + 1);
            let mut column = Column::sized(&field, &window.label(), Some(Basis::Cells(7)));
            column.min = Some(5);
            column.align = Align::Right;
            column.priority = Some((5 - i) as u16);
            self.lines[0].push(Cell {
                field: Some(field),
                span: 1,
            });
            self.columns.push(column);
        }
        self
    }

    /// The built-in leads tab (#507): each squad's lead on one line.
    pub fn leads() -> Self {
        Self::with_one_line(vec![
            Column::sized("squad", "SQUAD", Some(Basis::Cells(14))),
            Column::sized("member", "LEAD", Some(Basis::Cells(14))),
            Column::sized("state", "STATE", Some(Basis::Cells(10))),
            Column::sized("task", "TASK", None),
        ])
    }

    /// The built-in `all` tab (#507): one squad per line.
    pub fn overview() -> Self {
        Self::with_one_line(vec![
            Column::sized("squad", "SQUAD", Some(Basis::Cells(14))),
            Column::sized("lead", "LEAD", Some(Basis::Cells(14))),
            Column::sized("members", "MEMBERS", Some(Basis::Cells(8))),
            Column::sized("waiting", "WAITING", Some(Basis::Cells(8))),
            Column::sized("blocked", "BLOCKED", None),
        ])
    }

    fn with_one_line(columns: Vec<Column>) -> Self {
        let lines = vec![
            columns
                .iter()
                .map(|column| Cell {
                    field: Some(column.field.clone()),
                    span: 1,
                    token: None,
                })
                .collect(),
        ];
        Self {
            columns,
            lines,
            hidden_columns: Vec::new(),
        }
    }

    /// Cells cover consecutive tracks from zero, including empty cells.
    /// Columns beyond the longest line provide values but reserve no width.
    pub fn covered_tracks(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.iter().map(|cell| cell.span).sum::<usize>())
            .max()
            .unwrap_or(0)
    }

    /// Legacy CLI-policy oracle for coverage tests; production boards use Taffy.
    #[cfg(test)]
    pub fn solve(
        &self,
        natural: impl Fn(usize) -> usize,
        available: usize,
        gap: usize,
    ) -> Vec<Option<usize>> {
        let tracks: Vec<_> = self.columns[..self.covered_tracks()]
            .iter()
            .enumerate()
            .map(|(index, column)| column.track(natural(index)))
            .collect();
        let mut widths = grid::solve(&tracks, Some(available), gap);
        widths.resize(self.columns.len(), None);
        widths
    }

    /// Inspect the configured row lines in tests, in first-occurrence order.
    #[cfg(test)]
    pub fn fields(&self) -> Vec<&str> {
        let mut fields: Vec<&str> = Vec::new();
        for cell in self.lines.iter().flatten() {
            if let Some(field) = cell.field.as_deref()
                && !fields.contains(&field)
            {
                fields.push(field);
            }
        }
        fields
    }

    /// Whether a column reads identity metadata beyond the squad's fields.
    pub fn reads_metadata(&self) -> bool {
        self.columns.iter().any(|column| {
            column
                .from
                .as_ref()
                .is_some_and(ColumnSource::reads_metadata)
        })
    }

    /// `{columns, lines}` as `ls --json` reports them.
    pub fn value(&self) -> Value {
        let align = |align: Align| match align {
            Align::Left => "left",
            Align::Right => "right",
            Align::Center => "center",
        };
        let truncate = |truncate: Truncate| match truncate {
            Truncate::End => "end",
            Truncate::Middle => "middle",
        };
        let mut value = json!({
            "columns": self.columns.iter().enumerate().map(|(index, column)| {
                let mut value = json!({
                "field": column.field, "title": column.title, "width": width_value(column.width),
                "min": column.min, "max": column.max, "grow": column.grow,
                "align": align(column.align), "truncate": truncate(column.truncate),
                "priority": column.priority,
                "from": column.from.as_ref().map(|from| from.path.as_str()),
                "format": column.format.as_str(),
                });
                if index >= self.covered_tracks() {
                    value["valueOnly"] = json!(true);
                }
                if let Some(mode) = column.overflow {
                    value["overflow"] = json!(if matches!(mode, Overflow::Wrap { .. }) { "wrap" } else { "ellipsis" });
                    if let Overflow::Wrap { max_lines } = mode { value["max_lines"] = json!(max_lines); }
                }
                value
            }).collect::<Vec<_>>(),
            "lines": self.lines.iter().map(|line| line.iter().map(|cell| {
                let mut value = json!({"field": cell.field, "span": cell.span});
                if let Some(token) = cell.token {
                    value["token"] = json!(token.name());
                }
                value
            }).collect::<Vec<_>>()).collect::<Vec<_>>(),
        });
        if !self.hidden_columns.is_empty() {
            value["hidden_columns"] = json!(self.hidden_columns);
        }
        value
    }
}

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

pub(crate) fn field_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn title(value: &Item, place: &str) -> Result<String, SquadError> {
    value
        .as_str()
        .filter(|title| title.len() <= 40 && !title.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("`{place}` must be one short line.")))
}

/// `[{ at = <number>, token = "<theme token>" }, …]`, strictly increasing.
fn thresholds(value: &Item, place: &str) -> Result<Vec<Threshold>, SquadError> {
    let shape = || {
        invalid(format!(
            "`{place}` must be a list of {{ at = <number>, token = \"<theme token>\" }}, at strictly increasing."
        ))
    };
    let entries = value.as_array().ok_or_else(shape)?;
    let mut thresholds: Vec<Threshold> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let here = format!("{place}[{index}]");
        let table = entry.as_inline_table().ok_or_else(shape)?;
        if let Some((key, _)) = table.iter().find(|(key, _)| !["at", "token"].contains(key)) {
            return Err(invalid(format!(
                "`{here}.{key}` is not a threshold setting; use at and token."
            )));
        }
        let at = table
            .get("at")
            .and_then(|at| {
                at.as_float()
                    .or_else(|| at.as_integer().map(|at| at as f64))
            })
            .filter(|at| at.is_finite())
            .ok_or_else(|| invalid(format!("`{here}.at` must be a number.")))?;
        let token = table
            .get("token")
            .and_then(|token| token.as_str())
            .filter(|token| crate::look::role(token).is_some())
            .ok_or_else(|| {
                invalid(format!(
                    "`{here}.token` must be a theme token: {}.",
                    crate::look::names()
                ))
            })?;
        if thresholds.last().is_some_and(|last| at <= last.at) {
            return Err(invalid(format!(
                "`{here}.at` must be greater than the threshold before it."
            )));
        }
        thresholds.push(Threshold {
            at,
            token: token.to_owned(),
        });
    }
    Ok(thresholds)
}

fn number(
    value: &Item,
    place: &str,
    range: std::ops::RangeInclusive<i64>,
) -> Result<u16, SquadError> {
    value
        .as_integer()
        .filter(|number| range.contains(number))
        .and_then(|number| u16::try_from(number).ok())
        .ok_or_else(|| {
            invalid(format!(
                "`{place}` must be {}-{}.",
                range.start(),
                range.end()
            ))
        })
}

fn width(value: &Item, place: &str) -> Result<Basis, SquadError> {
    if value.is_integer() {
        return number(value, place, 1..=MAX_WIDTH).map(|width| Basis::Cells(usize::from(width)));
    }
    value
        .as_str()
        .and_then(|text| text.strip_suffix('%'))
        .filter(|text| {
            !text.is_empty()
                && !text.starts_with('0')
                && text.bytes().all(|byte| byte.is_ascii_digit())
        })
        .and_then(|text| text.parse::<u8>().ok())
        .filter(|percent| (1..=100).contains(percent))
        .map(Basis::Percent)
        .ok_or_else(|| {
            invalid(format!(
                "`{place}` must be 1-{MAX_WIDTH} cells or a percentage 1%-100%."
            ))
        })
}

fn overflow(settings: &dyn TableLike, place: &str) -> Result<Option<Overflow>, SquadError> {
    let mode = settings
        .get("overflow")
        .map(|item| {
            item.as_str()
                .ok_or_else(|| invalid(format!("`{place}.overflow` must be ellipsis or wrap.")))
        })
        .transpose()?;
    let max_lines = settings
        .get("max_lines")
        .map(|item| number(item, &format!("{place}.max_lines"), 1..=8))
        .transpose()?;
    match (mode, max_lines) {
        (None, None) => Ok(None),
        (Some("wrap"), max_lines) => Ok(Some(Overflow::Wrap {
            max_lines: max_lines.unwrap_or(2) as u8,
        })),
        (None | Some("ellipsis"), Some(_)) => Err(invalid(format!(
            "`{place}.max_lines` requires overflow = wrap."
        ))),
        (Some("ellipsis"), None) => Ok(Some(Overflow::Ellipsis)),
        _ => Err(invalid(format!(
            "`{place}.overflow` must be ellipsis or wrap."
        ))),
    }
}

fn width_value(width: Option<Basis>) -> Value {
    match width {
        None => Value::Null,
        Some(Basis::Cells(width)) => json!(width),
        Some(Basis::Percent(percent)) => json!(format!("{percent}%")),
    }
}

/// `[squad.<name>.rows]` when present, else the older `columns` table, else
/// the preset. Setting both is refused rather than guessed.
pub fn read(squad: Option<&dyn TableLike>, name: &str) -> Result<Rows, SquadError> {
    let rows = squad.and_then(|table| table.get("rows"));
    let columns = squad.and_then(|table| table.get("columns"));
    let mut result = match (rows, columns) {
        (Some(_), Some(_)) => Err(invalid(format!(
            "`squad.{name}` sets both `rows` and `columns`; keep `rows`."
        ))),
        (Some(rows), None) => {
            // `from = "fields.<name>"` needs a provider of that name.
            let provided = |field: &str| {
                squad
                    .and_then(|table| table.get("fields"))
                    .and_then(Item::as_table_like)
                    .is_some_and(|fields| fields.contains_key(field))
            };
            read_rows(rows, &format!("squad.{name}.rows"), &provided)
        }
        (None, Some(columns)) => read_legacy(columns, &format!("squad.{name}.columns")),
        (None, None) => Ok(Rows::preset()),
    }?;
    let percent: u16 = result
        .columns
        .iter()
        .take(result.covered_tracks())
        .map(|column| match column.width {
            Some(Basis::Percent(percent)) => u16::from(percent),
            _ => 0,
        })
        .sum();
    if percent > 100 {
        return Err(invalid(format!(
            "`squad.{name}` configured column percentages must total at most 100%."
        )));
    }
    if let Some(item) = squad
        .and_then(|s| s.get("board"))
        .and_then(|b| b.get("hidden_columns"))
    {
        let place = format!("squad.{name}.board.hidden_columns");
        let list = item
            .as_array()
            .ok_or_else(|| invalid(format!("`{place}` must list column names.")))?;
        for (index, item) in list.iter().enumerate() {
            let field = item
                .as_str()
                .filter(|field| {
                    result.columns[..result.covered_tracks()]
                        .iter()
                        .any(|c| c.field == *field)
                })
                .ok_or_else(|| {
                    invalid(format!("`{place}[{index}]` must name a covered column."))
                })?;
            if result.hidden_columns.iter().any(|known| known == field) {
                return Err(invalid(format!("`{place}[{index}]` repeats `{field}`.")));
            }
            result.hidden_columns.push(field.into());
        }
        if result.hidden_columns.len() == result.covered_tracks() {
            return Err(invalid(format!(
                "`{place}` must leave at least one covered track visible."
            )));
        }
    }
    Ok(result)
}

fn read_rows(
    item: &Item,
    place: &str,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Rows, SquadError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
    for (key, _) in table.iter() {
        if !matches!(key, "columns" | "lines") {
            return Err(invalid(format!("`{place}.{key}` is not a rows setting.")));
        }
    }
    // Inline (`columns = [{ … }]`) or `[[…rows.columns]]` tables.
    let list: Option<Vec<Option<&dyn TableLike>>> = match table.get("columns") {
        Some(Item::Value(toml_edit::Value::Array(array))) => Some(
            array
                .iter()
                .map(|entry| entry.as_inline_table().map(|t| t as &dyn TableLike))
                .collect(),
        ),
        Some(Item::ArrayOfTables(array)) => Some(
            array
                .iter()
                .map(|entry| Some(entry as &dyn TableLike))
                .collect(),
        ),
        _ => None,
    };
    let list = list
        .filter(|columns| (1..=MAX_COLUMNS).contains(&columns.len()))
        .ok_or_else(|| {
            invalid(format!(
                "`{place}.columns` must list 1-{MAX_COLUMNS} columns."
            ))
        })?;
    let mut columns: Vec<Column> = Vec::new();
    for (index, entry) in list.into_iter().enumerate() {
        let here = format!("{place}.columns[{index}]");
        let settings = entry.ok_or_else(|| invalid(format!("`{here}` must be a table.")))?;
        let column = read_column(settings, &here, provided)?;
        if columns.iter().any(|known| known.field == column.field) {
            return Err(invalid(format!(
                "`{here}.name` repeats the column `{}`.",
                column.field
            )));
        }
        columns.push(column);
    }
    let lines = match table.get("lines") {
        None => Rows::with_one_line(columns.clone()).lines,
        Some(lines) => read_lines(lines, &format!("{place}.lines"), columns.len())?,
    };
    Ok(Rows {
        columns,
        lines,
        hidden_columns: Vec::new(),
    })
}

fn read_column(
    settings: &dyn TableLike,
    here: &str,
    provided: &dyn Fn(&str) -> bool,
) -> Result<Column, SquadError> {
    let name = settings
        .get("name")
        .and_then(Item::as_str)
        .filter(|name| field_name(name))
        .ok_or_else(|| invalid(format!("`{here}.name` must be a field name.")))?;
    let mut column = Column::new(name, None);
    for (key, value) in settings.iter() {
        let place = format!("{here}.{key}");
        match key {
            "name" => {}
            "title" => column.title = title(value, &place)?,
            "width" => column.width = Some(width(value, &place)?),
            "overflow" | "max_lines" => {}
            "min" => column.min = Some(number(value, &place, 0..=MAX_WIDTH)?),
            "max" => column.max = Some(number(value, &place, 1..=MAX_WIDTH)?),
            "grow" => column.grow = number(value, &place, 0..=100)?,
            "priority" => column.priority = Some(number(value, &place, 1..=100)?),
            "align" => {
                column.align = match value.as_str() {
                    Some("left") => Align::Left,
                    Some("right") => Align::Right,
                    Some("center") => Align::Center,
                    _ => {
                        return Err(invalid(format!("`{place}` must be left, right or center.")));
                    }
                }
            }
            "truncate" => {
                column.truncate = match value.as_str() {
                    Some("end") => Truncate::End,
                    Some("middle") => Truncate::Middle,
                    _ => return Err(invalid(format!("`{place}` must be end or middle."))),
                }
            }
            "from" => {
                column.from = Some(
                    value
                        .as_str()
                        .and_then(|path| ColumnSource::parse(path, field_name, provided))
                        .ok_or_else(|| invalid(format!("`{place}` must be one of: {PATHS}.")))?,
                );
            }
            "format" => {
                column.format = value.as_str().and_then(Format::parse).ok_or_else(|| {
                    invalid(format!("`{place}` must be text, tokens, age or count."))
                })?;
            }
            "color" => column.color = thresholds(value, &place)?,
            other => {
                return Err(invalid(format!(
                    "`{here}.{other}` is not a column setting."
                )));
            }
        }
    }
    if column.source().is_some() && OWN_FIELDS.contains(&column.field.as_str()) {
        return Err(invalid(format!(
            "`{here}.name` `{}` is a field Squad reads itself; give a column with `from` or `format` another name.",
            column.field
        )));
    }
    let low = column.min;
    let high = column.max;
    let consistent = low.zip(high).is_none_or(|(low, high)| low <= high)
        && column.width.is_none_or(|width| match width {
            Basis::Percent(_) => true,
            Basis::Cells(width) => {
                low.is_none_or(|low| usize::from(low) <= width)
                    && high.is_none_or(|high| width <= usize::from(high))
            }
        });
    if !consistent {
        return Err(invalid(format!("`{here}` needs min <= width <= max.")));
    }
    column.overflow = overflow(settings, here)?;
    Ok(column)
}

fn read_lines(item: &Item, place: &str, columns: usize) -> Result<Vec<Vec<Cell>>, SquadError> {
    let lines = item
        .as_array()
        .filter(|lines| (1..=MAX_LINES).contains(&lines.len()))
        .ok_or_else(|| invalid(format!("`{place}` must list 1-{MAX_LINES} lines.")))?;
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let here = format!("{place}[{index}]");
            let cells = line
                .as_array()
                .filter(|cells| !cells.is_empty())
                .ok_or_else(|| invalid(format!("`{here}` must list at least one cell.")))?;
            let cells = cells
                .iter()
                .enumerate()
                .map(|(position, cell)| read_cell(cell, &format!("{here}[{position}]"), columns))
                .collect::<Result<Vec<_>, _>>()?;
            if cells.iter().map(|cell| cell.span).sum::<usize>() > columns {
                return Err(invalid(format!(
                    "`{here}` spans more than the {columns} columns."
                )));
            }
            Ok(cells)
        })
        .collect()
}

/// `"field"`, `""`, or `{ field = "…", span = n, token = "waiting" }`.
fn read_cell(cell: &toml_edit::Value, here: &str, columns: usize) -> Result<Cell, SquadError> {
    if let Some(field) = cell.as_str() {
        return match field {
            "" => Ok(Cell {
                field: None,
                span: 1,
                token: None,
            }),
            field if field_name(field) => Ok(Cell {
                field: Some(field.to_owned()),
                span: 1,
                token: None,
            }),
            _ => Err(invalid(format!("`{here}` must be a field name or \"\"."))),
        };
    }
    let table = cell
        .as_inline_table()
        .ok_or_else(|| invalid(format!("`{here}` must be a field name or a table.")))?;
    let mut parsed = Cell {
        field: None,
        span: 1,
        token: None,
    };
    for (key, value) in table.iter() {
        match key {
            "field" => {
                parsed.field = Some(
                    value
                        .as_str()
                        .filter(|field| field_name(field))
                        .ok_or_else(|| invalid(format!("`{here}.field` must be a field name.")))?
                        .to_owned(),
                );
            }
            "span" => {
                parsed.span = value
                    .as_integer()
                    .and_then(|span| usize::try_from(span).ok())
                    .filter(|span| (1..=columns).contains(span))
                    .ok_or_else(|| invalid(format!("`{here}.span` must be 1-{columns}.")))?;
            }
            "token" => {
                parsed.token = Some(value.as_str().and_then(Role::parse).ok_or_else(|| {
                    invalid(format!(
                        "`{here}.token` must be a semantic theme token: {}.",
                        Role::ALL.map(Role::name).join(", ")
                    ))
                })?);
            }
            other => {
                return Err(invalid(format!("`{here}.{other}` is not a cell setting.")));
            }
        }
    }
    Ok(parsed)
}

/// `[squad.<name>.columns]`: `show` lists fields in order; a table named
/// after a field sets its `title` and `width`. A column without a width
/// grows into what is left, as before.
fn read_legacy(item: &Item, place: &str) -> Result<Rows, SquadError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
    let defaults = Rows::preset().columns;
    let show: Vec<String> = match table.get("show") {
        None => defaults.iter().map(|column| column.field.clone()).collect(),
        Some(show) => show
            .as_array()
            .filter(|fields| (1..=MAX_COLUMNS).contains(&fields.len()))
            .ok_or_else(|| invalid(format!("`{place}.show` must list 1-12 fields.")))?
            .iter()
            .map(|field| {
                field
                    .as_str()
                    .filter(|field| field_name(field))
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(format!("`{place}.show` entries are field names.")))
            })
            .collect::<Result<_, _>>()?,
    };
    for (key, _) in table.iter() {
        if key != "show" && !show.iter().any(|field| field == key) {
            return Err(invalid(format!(
                "`{place}.{key}` configures a column that is not shown."
            )));
        }
    }
    let columns = show
        .into_iter()
        .map(|field| {
            let default = defaults.iter().find(|column| column.field == field);
            let mut column = Column::sized(
                &field,
                &default.map_or_else(|| field.to_uppercase(), |column| column.title.clone()),
                default.and_then(|column| column.width),
            );
            // A preset column keeps how it steps aside (pr_link drops first).
            let priority = default.and_then(|column| column.priority);
            column.priority = priority;
            if let Some(settings) = table.get(&field) {
                let settings = settings
                    .as_table_like()
                    .ok_or_else(|| invalid(format!("`{place}.{field}` must be a table.")))?;
                for (key, value) in settings.iter() {
                    let here = format!("{place}.{field}.{key}");
                    match key {
                        "title" => column.title = title(value, &here)?,
                        "overflow" | "max_lines" => {}
                        "width" => {
                            column.width = Some(width(value, &here)?);
                            column.grow = 0;
                        }
                        other => {
                            return Err(invalid(format!(
                                "`{place}.{field}.{other}` is not a column setting."
                            )));
                        }
                    }
                }
                column.overflow = overflow(settings, &format!("{place}.{field}"))?;
            }
            Ok(column)
        })
        .collect::<Result<Vec<_>, SquadError>>()?;
    Ok(Rows::with_one_line(columns))
}

#[cfg(test)]
mod tests;
