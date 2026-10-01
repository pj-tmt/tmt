//! How a squad's rows are laid out: one shared grid of columns, and the
//! lines each row takes, where a cell may span several columns. The board
//! sizes the grid with tmt-cli-style's one solver; `ls` takes only the field
//! selection and order, since a list stays complete.
//!
//! `[squad.<name>.rows]` is the full form; the older `[squad.<name>.columns]`
//! (`show` plus a `title`/`width` per field) reads as the same model.

use crate::{
    core::SquadError,
    source::{ColumnSource, Format, PATHS},
};
use serde_json::{Value, json};
use tmt_cli_style::grid::{Align, Track, Truncate};
use toml_edit::{Item, TableLike};

const MAX_COLUMNS: usize = 12;
const MAX_LINES: usize = 4;
const MAX_WIDTH: i64 = 200;
/// Fields kept as Squad's own data (the name, free-text role, a state's
/// order and color, a decision owed, the note). A bound value replaces the
/// field of its column's name, so these cannot be bound.
pub(crate) const OWN_FIELDS: &[&str] = &["member", "role", "state", "pending", "note"];
/// Where a growing column starts, and the least an unsized one keeps.
const NARROWEST: usize = 4;

/// One grid column: the field it shows by default, its header and sizing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub field: String,
    pub title: String,
    /// An exact width; with `min` it may shrink that far.
    pub width: Option<u16>,
    pub min: Option<u16>,
    pub max: Option<u16>,
    pub grow: u16,
    pub align: Align,
    pub truncate: Truncate,
    /// Steps aside on a narrow board, highest first; None never does.
    pub priority: Option<u16>,
    /// Where the value comes from, when not the squad field of its name.
    pub from: Option<ColumnSource>,
    pub format: Format,
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
            priority: None,
            from: None,
            format: Format::Text,
        }
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

    fn sized(field: &str, title: &str, width: Option<u16>) -> Self {
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
        let width = self.width.map(usize::from);
        let basis = match (width, self.grow) {
            (Some(width), _) => width,
            (None, 0) => natural,
            (None, _) => 0,
        };
        let min = self.min.map_or_else(
            || match (width, self.grow) {
                (Some(width), _) => width,
                (None, 0) => NARROWEST.min(basis),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub columns: Vec<Column>,
    /// At least one line; each line's spans cover at most every column.
    pub lines: Vec<Vec<Cell>>,
}

impl Rows {
    /// Every preset's rows: the name, state, task and pull request.
    pub fn preset() -> Self {
        // On a narrow board the link steps aside before anything is cut off.
        let mut link = Column::sized("pr_link", "PR", Some(12));
        link.priority = Some(1);
        Self::with_one_line(vec![
            Column::sized("member", "MEMBER", Some(14)),
            Column::sized("state", "STATE", Some(10)),
            Column::sized("task", "TASK", None),
            link,
        ])
    }

    /// The built-in leads tab (#507): each squad's lead on one line.
    pub fn leads() -> Self {
        Self::with_one_line(vec![
            Column::sized("squad", "SQUAD", Some(14)),
            Column::sized("member", "LEAD", Some(14)),
            Column::sized("state", "STATE", Some(10)),
            Column::sized("task", "TASK", None),
        ])
    }

    /// The built-in `all` tab (#507): one squad per line.
    pub fn overview() -> Self {
        Self::with_one_line(vec![
            Column::sized("squad", "SQUAD", Some(14)),
            Column::sized("lead", "LEAD", Some(14)),
            Column::sized("members", "MEMBERS", Some(8)),
            Column::sized("waiting", "WAITING", Some(8)),
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
                })
                .collect(),
        ];
        Self { columns, lines }
    }

    /// The fields `ls` shows, in order: the first line's, then any that only
    /// later lines show.
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
        json!({
            "columns": self.columns.iter().map(|column| json!({
                "field": column.field, "title": column.title, "width": column.width,
                "min": column.min, "max": column.max, "grow": column.grow,
                "align": align(column.align), "truncate": truncate(column.truncate),
                "priority": column.priority,
                "from": column.from.as_ref().map(|from| from.path.as_str()),
                "format": column.format.as_str(),
            })).collect::<Vec<_>>(),
            "lines": self.lines.iter().map(|line| line.iter().map(|cell| json!({
                "field": cell.field, "span": cell.span,
            })).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
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

/// `[squad.<name>.rows]` when present, else the older `columns` table, else
/// the preset. Setting both is refused rather than guessed.
pub fn read(squad: Option<&dyn TableLike>, name: &str) -> Result<Rows, SquadError> {
    let rows = squad.and_then(|table| table.get("rows"));
    let columns = squad.and_then(|table| table.get("columns"));
    match (rows, columns) {
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
    }
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
    Ok(Rows { columns, lines })
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
            "width" => column.width = Some(number(value, &place, 1..=MAX_WIDTH)?),
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
            // Kept for value colors, so adding them reshapes nothing.
            "color" => {
                return Err(invalid(format!(
                    "`{place}` is not available yet; it arrives with theme tokens (#503, #514)."
                )));
            }
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
        && column.width.is_none_or(|width| {
            low.is_none_or(|low| low <= width) && high.is_none_or(|high| width <= high)
        });
    if !consistent {
        return Err(invalid(format!("`{here}` needs min <= width <= max.")));
    }
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

/// `"field"`, `""` (an empty cell), or `{ field = "…", span = n }`.
fn read_cell(cell: &toml_edit::Value, here: &str, columns: usize) -> Result<Cell, SquadError> {
    if let Some(field) = cell.as_str() {
        return match field {
            "" => Ok(Cell {
                field: None,
                span: 1,
            }),
            field if field_name(field) => Ok(Cell {
                field: Some(field.to_owned()),
                span: 1,
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
                        "width" => {
                            column = Column::sized(
                                &field,
                                &column.title,
                                Some(number(value, &here, 1..=MAX_WIDTH)?),
                            );
                            column.priority = priority;
                        }
                        other => {
                            return Err(invalid(format!(
                                "`{place}.{field}.{other}` is not a column setting."
                            )));
                        }
                    }
                }
            }
            Ok(column)
        })
        .collect::<Result<Vec<_>, SquadError>>()?;
    Ok(Rows::with_one_line(columns))
}

#[cfg(test)]
mod tests;
