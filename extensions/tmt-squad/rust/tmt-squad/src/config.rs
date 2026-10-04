//! `squad.toml`: the user's file, beside TMT's global configuration.
//! Named edits pass through one format-preserving writer.

use crate::core::{Core, SquadError};
use crate::split::{Size, Split};
use crate::{
    action::{Bindings, parse_bindings, preset},
    filter::{Filter, Row},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

mod edit;
mod settings;
mod sourced;
mod states;
pub use states::{Rank, States};

const FILE_LIMIT: u64 = 1024 * 1024;
const MAX_SECTIONS: usize = 16;

fn split_value(split: &Split) -> Value {
    match split {
        Split::Pane(pane) => json!(pane.title()),
        Split::Group {
            direction,
            children,
        } => json!({
            "direction": match direction { Direction::LeftRight => "left-right", Direction::TopBottom => "top-bottom" },
            "sizes": children.iter().map(|(size, _)| match size {
                Size::Percent(n) => json!(n), Size::Grow(n) => json!(format!("{n}fr")),
            }).collect::<Vec<_>>(),
            "panes": children.iter().map(|(_, child)| split_value(child)).collect::<Vec<_>>(),
        }),
    }
}

/// Per-squad observation/reminder policy; enabling never installs hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminders {
    pub enabled: bool,
    pub stale_after: Duration,
}

impl Default for Reminders {
    fn default() -> Self {
        Self {
            enabled: false,
            stale_after: Duration::from_secs(1800),
        }
    }
}

/// Completed-request meter policy; independent of the ordinary board reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenWindow {
    Five,
    #[default]
    Minute,
    HalfHour,
    Hour,
}

impl TokenWindow {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "5s" => Some(Self::Five),
            "1m" => Some(Self::Minute),
            "30m" => Some(Self::HalfHour),
            "1h" => Some(Self::Hour),
            _ => None,
        }
    }
    pub fn milliseconds(self) -> u64 {
        match self {
            Self::Five => 5_000,
            Self::Minute => 60_000,
            Self::HalfHour => 1_800_000,
            Self::Hour => 3_600_000,
        }
    }
    pub fn available(self, every: Duration) -> Self {
        if self == Self::Five && every != Duration::from_secs(5) {
            Self::Minute
        } else {
            self
        }
    }
    pub fn next(self, every: Duration) -> Self {
        match self {
            Self::Five => Self::Minute,
            Self::Minute => Self::HalfHour,
            Self::HalfHour => Self::Hour,
            Self::Hour => Self::Five.available(every),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Five => "5s",
            Self::Minute => "1m",
            Self::HalfHour => "30m",
            Self::Hour => "1h",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRate {
    pub enabled: bool,
    pub every: Duration,
    pub reduced_motion: bool,
    pub window: TokenWindow,
}

impl Default for TokenRate {
    fn default() -> Self {
        Self {
            enabled: false,
            every: Duration::from_secs(5),
            reduced_motion: false,
            window: TokenWindow::Minute,
        }
    }
}

/// One sort key; `-field` sorts descending. `state` follows the layout's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// None shows every row.
    pub filter: Option<Filter>,
    pub sort: Vec<SortKey>,
    /// This section's own bindings, over `[bind]` and the host preset.
    pub bind: Bindings,
}

impl Section {
    fn read(table: &Table, place: &str) -> Result<Self, SquadError> {
        let text = |key: &str| -> Result<Option<&str>, SquadError> {
            table
                .get(key)
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| invalid(format!("`{place}.{key}` must be a string.")))
                })
                .transpose()
        };
        if let Some(key) = table
            .iter()
            .map(|(key, _)| key)
            .find(|key| !["title", "filter", "sort", "bind"].contains(key))
        {
            return Err(invalid(format!(
                "`{place}.{key}` is not a section setting."
            )));
        }
        let title = text("title")?
            .filter(|title| {
                !title.trim().is_empty()
                    && title.len() <= 80
                    && !title.chars().any(char::is_control)
            })
            .ok_or_else(|| invalid(format!("`{place}.title` must be one line of 1-80 bytes.")))?;
        let filter = text("filter")?
            .map(|filter| {
                Filter::parse(filter)
                    .map_err(|error| invalid(format!("`{place}.filter`: {error}.")))
            })
            .transpose()?;
        let sort = match table.get("sort") {
            None => Vec::new(),
            Some(item) => item
                .as_array()
                .ok_or_else(|| invalid(format!("`{place}.sort` must be an array of field names.")))?
                .iter()
                .map(|value| {
                    let key = value.as_str().unwrap_or_default();
                    let (descending, field) = key
                        .strip_prefix('-')
                        .map_or((false, key), |field| (true, field));
                    (!field.is_empty()
                        && field.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                        }))
                    .then(|| SortKey {
                        field: field.into(),
                        descending,
                    })
                    .ok_or_else(|| {
                        invalid(format!(
                            "`{place}.sort` entries are field names, optionally prefixed with '-'."
                        ))
                    })
                })
                .collect::<Result<_, _>>()?,
        };
        let bind = match table.get("bind") {
            None => Bindings::new(),
            Some(bind) => bindings_table(bind, &format!("{place}.bind"))?,
        };
        Ok(Self {
            title: title.into(),
            filter,
            sort,
            bind,
        })
    }

    pub fn includes(&self, row: &impl Row) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.matches(row))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Crew,
    PrQueue,
    Minimal,
    Team,
}

impl Layout {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "crew" => Some(Self::Crew),
            "pr-queue" => Some(Self::PrQueue),
            "minimal" => Some(Self::Minimal),
            "team" => Some(Self::Team),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::PrQueue => "pr-queue",
            Self::Minimal => "minimal",
            Self::Team => "team",
        }
    }

    /// Ordered state vocabulary; `add` starts members in the first state.
    pub fn states(self) -> &'static [&'static str] {
        match self {
            Self::Crew | Self::Team => &["working", "idle", "blocked", "review", "testing", "hold"],
            Self::PrQueue => &["preparing", "ready", "sent", "merged"],
            Self::Minimal => &[],
        }
    }

    /// Default state colors; `squad.<name>.states` overrides them.
    fn state_colors(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Crew | Self::Team => &[
                ("working", "working"),
                ("idle", "dim"),
                ("blocked", "blocked"),
                ("review", "review"),
                ("testing", "accent"),
                ("hold", "dim"),
            ],
            Self::PrQueue => &[
                ("preparing", "dim"),
                ("ready", "working"),
                ("sent", "review"),
                ("merged", "dim"),
            ],
            Self::Minimal => &[],
        }
    }

    /// Crew and team sort rows that owe the user a decision (`pending`) first.
    pub fn pending_first(self) -> bool {
        matches!(self, Self::Crew | Self::Team)
    }
}

/// Default team settings, expressed in the same configuration grammar as overrides.
const TEAM: &str = r#"
[team.board]
token_rate = { enabled = true }
[team.rows]
columns = [
    { name = "member", width = "22%", min = 12, max = 24 },
    { name = "state", width = "14%", min = 9, max = 10 },
    { name = "task", grow = 1, min = 18 },
    { name = "pr", width = "24%", min = 12, max = 28, priority = 2 },
    { name = "model", from = "session.model", width = "16%", min = 18, max = 18, priority = 3 },
]
lines = [["member", "state", "task", "pr", "model"], ["", "", { field = "pending", span = 3, token = "waiting" }]]
[team.fields.pr]
preset = "github-pr"
every = "60s"
[team.reminders]
enabled = true
stale_after = "30m"
"#;

fn team() -> &'static DocumentMut {
    static PRESET: std::sync::OnceLock<DocumentMut> = std::sync::OnceLock::new();
    PRESET.get_or_init(|| TEAM.parse().expect("the team preset is valid TOML"))
}

/// `[tabs]`: see [`Config::tabs`]. Entries are tab keys: a squad name, or
/// [`crate::board::LEADS`] for the built-in tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabs {
    pub order: Vec<String>,
    /// Tabs that come first and stay in view when the tab line scrolls.
    pub pin: Vec<String>,
    pub hide: Vec<String>,
    pub colors: TabColors,
    /// `[tabs.leads.bind]`, over `[bind]` and the host preset.
    pub leads: Bindings,
    /// `[tabs.all.bind]`, over the `all` tab's own preset.
    pub all: Bindings,
    pub user: Vec<UserTab>,
}

impl Default for Tabs {
    fn default() -> Self {
        Self {
            order: vec![crate::tabs::ALL.into(), crate::tabs::LEADS.into()],
            pin: vec![crate::tabs::ALL.into()],
            hide: Vec::new(),
            colors: TabColors::default(),
            leads: Bindings::new(),
            all: Bindings::new(),
            user: Vec::new(),
        }
    }
}

/// A configured member view; selection applies before section shaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTab {
    pub name: String,
    pub selection: Section,
    pub sections: Vec<Section>,
}

impl UserTab {
    fn read(name: &str, item: &Item) -> Result<Self, SquadError> {
        let place = format!("tabs.{name}");
        if !crate::squad::valid_name(name) {
            return Err(invalid(format!("`{place}` must use a squad-style name.")));
        }
        let mut table = item
            .as_table()
            .cloned()
            .ok_or_else(|| invalid(format!("`{place}` must be a [tabs.{name}] table.")))?;
        if let Some(key) = table
            .iter()
            .map(|(key, _)| key)
            .find(|key| !["filter", "sort", "section", "bind"].contains(key))
        {
            return Err(invalid(format!(
                "`{place}.{key}` is not a tab setting; use filter, sort, section or bind."
            )));
        }
        let sections = read_sections(
            table.remove("section").as_ref(),
            &format!("{place}.section"),
        )?;
        table.insert("title", value(name));
        let selection = Section::read(&table, &place)?;
        Ok(Self {
            name: name.into(),
            selection,
            sections,
        })
    }
}

fn read_sections(item: Option<&Item>, place: &str) -> Result<Vec<Section>, SquadError> {
    let Some(item) = item else {
        return Ok(Vec::new());
    };
    let tables = item
        .as_array_of_tables()
        .ok_or_else(|| invalid(format!("`{place}` must be [[{place}]] tables.")))?;
    if tables.len() > MAX_SECTIONS {
        return Err(invalid(format!(
            "`{place}` allows at most {MAX_SECTIONS} sections."
        )));
    }
    tables
        .iter()
        .enumerate()
        .map(|(index, table)| Section::read(table, &format!("{place}[{index}]")))
        .collect()
}

/// A built-in tab's table: only `bind`.
fn tab_bindings(item: &Item, place: &str) -> Result<Bindings, SquadError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
    let mut bindings = Bindings::new();
    for (key, item) in table.iter() {
        match key {
            "bind" => bindings = bindings_table(item, &format!("{place}.bind"))?,
            other => {
                return Err(invalid(format!(
                    "`{place}.{other}` is not a tab setting; use bind."
                )));
            }
        }
    }
    Ok(bindings)
}

/// A list of tab names as tab keys, each at most once.
fn tab_list(item: &Item, place: &str) -> Result<Vec<String>, SquadError> {
    let array = item
        .as_array()
        .ok_or_else(|| invalid(format!("`{place}` must be a list of tab names.")))?;
    let mut keys: Vec<String> = Vec::new();
    for entry in array.iter() {
        let name = entry
            .as_str()
            .ok_or_else(|| invalid(format!("`{place}` must be a list of tab names.")))?;
        let key = match name {
            "leads" => crate::board::LEADS.to_owned(),
            "all" => crate::board::ALL.to_owned(),
            _ if name.starts_with("tab:") => {
                let tab = &name[4..];
                if !crate::squad::valid_name(tab) || crate::tabs::reserved(tab) {
                    return Err(invalid(format!(
                        "`{place}` names invalid user tab `{name}`."
                    )));
                }
                crate::tabs::user_key(tab)
            }
            _ => {
                let squad = name.strip_prefix("squad:").unwrap_or(name);
                if !crate::squad::valid_name(squad) {
                    return Err(invalid(format!(
                        "`{place}` names `{name}`, which is not `leads`, `all` or a squad name."
                    )));
                }
                squad.to_owned()
            }
        };
        if keys.contains(&key) {
            return Err(invalid(format!("`{place}` names `{name}` twice.")));
        }
        keys.push(key);
    }
    Ok(keys)
}

/// `[tabs.colors]`: the `waiting` and `blocked` tokens by default; any
/// token, or an older color name, may replace them.
fn tab_colors(item: &Item) -> Result<TabColors, SquadError> {
    let mut colors = TabColors::default();
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid("`tabs.colors` must be a table."))?;
    for (key, value) in table.iter() {
        let slot = match key {
            "waiting" => &mut colors.waiting,
            "blocked" => &mut colors.blocked,
            other => {
                return Err(invalid(format!(
                    "`tabs.colors.{other}` is not a tab state; use waiting or blocked."
                )));
            }
        };
        *slot = value
            .as_str()
            .filter(|color| crate::look::known(color))
            .ok_or_else(|| {
                invalid(format!(
                    "`tabs.colors.{key}` must be {}.",
                    crate::look::names()
                ))
            })?
            .into();
    }
    Ok(colors)
}

/// Tab colors by attention state; a normal tab keeps the board's own style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabColors {
    pub waiting: String,
    pub blocked: String,
}

impl Default for TabColors {
    fn default() -> Self {
        Self {
            waiting: "waiting".into(),
            blocked: "blocked".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pane {
    Rows,
    Notes,
    Detail,
    Replies,
}

impl Pane {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "rows" => Some(Self::Rows),
            "notes" => Some(Self::Notes),
            "detail" => Some(Self::Detail),
            "replies" => Some(Self::Replies),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Notes => "notes",
            Self::Detail => "detail",
            Self::Replies => "replies",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardMode {
    Split,
    Tabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftRight,
    TopBottom,
}

/// Width-based presentation defaults; toggles remain session-local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldBelow {
    pub width: u16,
    pub panes: std::collections::BTreeSet<Pane>,
}

/// `[squad.<name>.board]`: which panes the board shows and how they sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub mode: BoardMode,
    /// Every pane in focus order (split) or tab order (tabs).
    pub panes: Vec<Pane>,
    /// Split mode: how the panes sit, possibly nested.
    pub split: crate::split::Split,
    /// Initial presentation state; runtime toggles never write configuration.
    pub collapsed: std::collections::BTreeSet<Pane>,
    pub fold_below: Option<FoldBelow>,
}

impl Board {
    fn read_collapsed(&mut self, item: Option<&Item>, place: &str) -> Result<(), SquadError> {
        let Some(item) = item else { return Ok(()) };
        let place = format!("{place}.collapsed");
        if self.mode != BoardMode::Split {
            return Err(invalid(format!("`{place}` applies to split mode only.")));
        }
        self.collapsed = self.read_panes(Some(item), &place)?;
        Ok(())
    }

    fn read_panes(
        &self,
        item: Option<&Item>,
        place: &str,
    ) -> Result<std::collections::BTreeSet<Pane>, SquadError> {
        let names = item
            .and_then(Item::as_array)
            .ok_or_else(|| invalid(format!("`{place}` must list panes.")))?;
        let mut panes = std::collections::BTreeSet::new();
        for (index, name) in names.iter().enumerate() {
            let pane = name.as_str().and_then(Pane::parse).ok_or_else(|| {
                invalid(format!(
                    "`{place}[{index}]` must be rows, notes, detail or replies."
                ))
            })?;
            if !self.panes.contains(&pane) {
                return Err(invalid(format!(
                    "`{place}[{index}]` names {} which is not on this board.",
                    pane.title()
                )));
            }
            if !panes.insert(pane) {
                return Err(invalid(format!("`{place}` lists {} twice.", pane.title())));
            }
        }
        Ok(panes)
    }

    fn read_fold_below(&mut self, item: Option<&Item>, place: &str) -> Result<(), SquadError> {
        let Some(item) = item else { return Ok(()) };
        let place = format!("{place}.fold_below");
        if self.mode != BoardMode::Split {
            return Err(invalid(format!("`{place}` applies to split mode only.")));
        }
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        for (key, _) in table.iter() {
            if !["width", "panes"].contains(&key) {
                return Err(invalid(format!("`{place}.{key}` is not a fold setting.")));
            }
        }
        let width = table
            .get("width")
            .and_then(Item::as_integer)
            .filter(|width| (1..=1000).contains(width))
            .ok_or_else(|| {
                invalid(format!(
                    "`{place}.width` must be an integer from 1 to 1000."
                ))
            })? as u16;
        let panes = self.read_panes(table.get("panes"), &format!("{place}.panes"))?;
        self.fold_below = Some(FoldBelow { width, panes });
        Ok(())
    }

    fn from_settings(table: &dyn TableLike, place: &str) -> Result<Self, SquadError> {
        let split = crate::split::read(
            table.get("layout").expect("preset layout"),
            &format!("{place}.layout"),
        )?;
        let mut board = Self {
            mode: BoardMode::Split,
            panes: split.panes(),
            split,
            collapsed: Default::default(),
            fold_below: None,
        };
        board.read_collapsed(table.get("collapsed"), place)?;
        board.read_fold_below(table.get("fold_below"), place)?;
        Ok(board)
    }

    fn factory(view: crate::view::ViewName) -> Self {
        Self::from_settings(view.settings(), view.name()).expect("factory view is valid")
    }

    /// Crew keeps rows and the lead's notes side by side; pr-queue pairs rows
    /// with the selected row's detail; minimal shows rows only. Team nests all
    /// four panes, with full-width lead notes below the rows/detail/replies.
    fn preset(layout: Layout) -> Self {
        if layout == Layout::Team {
            return Self::factory(crate::view::ViewName::Team);
        }
        let (direction, panes, sizes) = match layout {
            Layout::Crew => (
                Direction::LeftRight,
                vec![Pane::Rows, Pane::Notes],
                vec![60, 40],
            ),
            Layout::PrQueue => (
                Direction::TopBottom,
                vec![Pane::Rows, Pane::Detail],
                vec![70, 30],
            ),
            Layout::Minimal => (Direction::LeftRight, vec![Pane::Rows], vec![100]),
            Layout::Team => unreachable!("team uses its nested split"),
        };
        Self::simple(BoardMode::Split, direction, panes, &sizes)
    }

    /// The one-level form: `panes` side by side or stacked at `sizes`.
    pub fn simple(mode: BoardMode, direction: Direction, panes: Vec<Pane>, sizes: &[u16]) -> Self {
        Self {
            mode,
            split: crate::split::Split::simple(direction, &panes, sizes),
            panes,
            collapsed: Default::default(),
            fold_below: None,
        }
    }
}

/// How the notes pane shows the lead's notebook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesRender {
    Markdown,
    Plain,
}

fn field_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn bindings_table(item: &Item, place: &str) -> Result<Bindings, SquadError> {
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table of bindings.")))?;
    parse_bindings(
        table.iter().map(|(event, action)| (event, action.as_str())),
        place,
    )
    .map_err(invalid)
}

/// A program as argv: a non-empty array of strings, the first a bare name on
/// PATH or an absolute path. It never passes through a shell.
/// The board's reload interval when nothing sets one.
pub const DEFAULT_REFRESH: Duration = Duration::from_secs(5);

/// Convert whole-unit durations; callers retain their units, ranges and errors.
pub(crate) fn duration(text: &str, units: &[char]) -> Option<Duration> {
    let (number, multiplier) = [('s', 1), ('m', 60), ('h', 3600)]
        .into_iter()
        .filter(|(unit, _)| units.contains(unit))
        .find_map(|(unit, multiplier)| {
            text.strip_suffix(unit).map(|number| (number, multiplier))
        })?;
    let seconds = number.parse::<u64>().ok()?.saturating_mul(multiplier);
    Some(Duration::from_secs(seconds))
}

/// `"off"`, or whole seconds or minutes such as `"2s"` or `"1m"`, from 1 s to
/// 1 h: often enough to be useful, never a busy loop.
fn refresh(item: &Item, place: &str) -> Result<Option<Duration>, SquadError> {
    let wrong = || {
        invalid(format!(
            "`{place}` must be \"off\" or 1s-60m, such as \"5s\" or \"1m\"."
        ))
    };
    let text = item.as_str().ok_or_else(wrong)?;
    if text == "off" {
        return Ok(None);
    }
    duration(text, &['s', 'm'])
        .filter(|duration| (1..=3600).contains(&duration.as_secs()))
        .map(Some)
        .ok_or_else(wrong)
}

fn program(item: &Item, place: &str) -> Result<Vec<String>, SquadError> {
    let malformed = || {
        invalid(format!(
            "`{place}` must be a program argv, for example [\"pbcopy\"]."
        ))
    };
    let argv: Vec<String> = item
        .as_array()
        .ok_or_else(malformed)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or_else(malformed)?;
    let runnable = argv
        .first()
        .is_some_and(|name| !name.is_empty() && (name.starts_with('/') || !name.contains('/')));
    if !runnable || argv.iter().any(|arg| arg.chars().any(char::is_control)) {
        return Err(malformed());
    }
    Ok(argv)
}

/// Prefix keys for the board and `back`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxKeys {
    pub popup: String,
    pub pane: String,
    pub back: Option<String>,
    pub lead: Option<String>,
}

/// A tmux key that needs no quoting: one printable character other than
/// quotes, `;`, `#`, `$`, `\\`, `{`, `}` or `~`; `C-` or `M-` with a letter or
/// digit; or F1 through F12.
fn tmux_key(key: &str) -> bool {
    let plain = |c: char| c.is_ascii_graphic() && !"\"';#\\{}~$".contains(c);
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => plain(c),
        _ => {
            let modified = key
                .strip_prefix("C-")
                .or_else(|| key.strip_prefix("M-"))
                .is_some_and(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_alphanumeric());
            let function = key
                .strip_prefix('F')
                .and_then(|number| number.parse::<u8>().ok())
                .is_some_and(|number| (1..=12).contains(&number) && !key.starts_with("F0"));
            modified || function
        }
    }
}

fn theme_settings(item: Option<&Item>, place: &str) -> Result<Vec<(String, String)>, SquadError> {
    let Some(item) = item else {
        return Ok(Vec::new());
    };
    let table = item
        .as_table_like()
        .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
    table
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|text| (key.to_owned(), text.to_owned()))
                .ok_or_else(|| invalid(format!("`{place}.{key}` must be a string.")))
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

#[derive(Clone)]
pub struct Config {
    path: PathBuf,
    original: Option<Vec<u8>>,
    document: DocumentMut,
    /// The global `theme` as `tmt config show` reports it, as written;
    /// empty when it names none, or for a file read on its own.
    global_theme: Vec<(String, String)>,
    /// Why the global theme is not used: `config show`'s `themeError`.
    theme_error: Option<String>,
}

impl Config {
    /// The file lives next to the global config that `tmt config show` reports,
    /// so TMT alone owns path discovery. A missing file is an empty document.
    pub fn load(core: &Core) -> Result<Self, SquadError> {
        let shown = core.json(&["config", "show"])?;
        let mut config = Self::read(Self::squad_file(&shown)?)?;
        config.global_theme(&shown);
        Ok(config)
    }

    /// Takes the global theme, and why it is not used, from `config show`.
    fn global_theme(&mut self, shown: &serde_json::Value) {
        self.global_theme = shown["resolved"]["theme"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect();
        self.theme_error = shown["themeError"].as_object().map(|problem| {
            format!(
                "{} {}",
                problem
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("theme"),
                problem
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .trim_end_matches('.')
            )
        });
    }

    /// Where squad.toml lives, without reading it.
    pub fn locate(core: &Core) -> Result<PathBuf, SquadError> {
        Self::squad_file(&core.json(&["config", "show"])?)
    }

    /// squad.toml beside the global config `config show` reports.
    fn squad_file(shown: &serde_json::Value) -> Result<PathBuf, SquadError> {
        let global = shown["paths"]["global"]
            .as_str()
            .ok_or_else(|| invalid("tmt config show did not report the global config path."))?;
        Ok(Path::new(global)
            .parent()
            .ok_or_else(|| invalid("The global config path has no directory."))?
            .join("squad.toml"))
    }

    pub fn read(path: PathBuf) -> Result<Self, SquadError> {
        let original = read_bounded(&path)
            .map_err(|error| invalid(format!("Could not read {}: {error}", path.display())))?;
        let text = std::str::from_utf8(original.as_deref().unwrap_or_default())
            .map_err(|_| invalid(format!("{} is not UTF-8.", path.display())))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let config = Self {
            path,
            original,
            document,
            global_theme: Vec::new(),
            theme_error: None,
        };
        config.me()?;
        config.me_id()?;
        config.validate_views()?;
        config.tabs()?;
        config.links()?;
        Ok(config)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The host preset's bindings, overridden by top-level `[bind]`.
    pub fn bindings(&self, tmux: bool, panes: &[Pane]) -> Result<Bindings, SquadError> {
        let mut bindings = preset(tmux, panes);
        bindings.extend(self.configured_bindings()?);
        Ok(bindings)
    }

    /// Shared tab-binding assembly, also used by the board's load path.
    pub fn bindings_for_tab(
        &self,
        key: &str,
        tmux: bool,
        panes: &[Pane],
    ) -> Result<Bindings, SquadError> {
        let settings = self.tabs()?;
        if key == crate::tabs::ALL {
            let mut bindings = crate::action::all_preset();
            bindings.extend(settings.all);
            return Ok(bindings);
        }
        let mut bindings = self.bindings(tmux, panes)?;
        if key == crate::tabs::LEADS {
            bindings.extend(settings.leads);
        }
        if let Some(tab) = settings
            .user
            .iter()
            .find(|tab| Some(tab.name.as_str()) == crate::tabs::user_name(key))
        {
            bindings.extend(tab.selection.bind.clone());
        }
        Ok(bindings)
    }

    pub fn configured_bindings(&self) -> Result<Bindings, SquadError> {
        self.document
            .get("bind")
            .map(|item| bindings_table(item, "bind"))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// Top-level `opener` and `clipboard`: programs that replace the system
    /// opener and the built-in clipboard route.
    pub fn program(&self, key: &str) -> Result<Option<Vec<String>>, SquadError> {
        self.document
            .get(key)
            .map(|item| program(item, key))
            .transpose()
    }

    /// Programs authorized only by the user's `[links]` table.
    pub fn links(&self) -> Result<crate::links::Handlers, SquadError> {
        let mut handlers = crate::links::Handlers::new();
        if let Some(item) = self.document.get("links") {
            let table = item
                .as_table_like()
                .ok_or_else(|| invalid("`links` must be a table."))?;
            if table.len() > 32 {
                return Err(invalid("`links` allows at most 32 handlers."));
            }
            for (name, value) in table.iter() {
                if !crate::links::scheme(name) || ["http", "https", "file", "tmt"].contains(&name) {
                    return Err(invalid(format!("`links.{name}` is not a custom scheme.")));
                }
                let action = value
                    .as_str()
                    .ok_or_else(|| invalid(format!("`links.{name}` must be a run binding.")))?;
                let action = crate::action::Action::parse(action).map_err(invalid)?;
                if action.verb != crate::action::Verb::Run {
                    return Err(invalid(format!("`links.{name}` must use run.")));
                }
                // Only {path} is available; reject misspelled/row placeholders at load.
                action
                    .argv(&serde_json::json!({"fields":{"path":"example"}}))
                    .map_err(invalid)?;
                handlers.insert(name.to_owned(), action);
            }
        }
        Ok(handlers)
    }

    /// `[tmux]`: the prefix keys that open the board as a popup (default `S`)
    /// or a pane (default `B`), and optional keys for `tmt squad back` and
    /// `tmt squad jump --lead`.
    pub fn tmux_keys(&self) -> Result<TmuxKeys, SquadError> {
        let table = match self.document.get("tmux") {
            None => None,
            Some(item) => Some(
                item.as_table_like()
                    .ok_or_else(|| invalid("`tmux` must be a table."))?,
            ),
        };
        if let Some(unknown) = table
            .into_iter()
            .flat_map(|table| table.iter().map(|(key, _)| key))
            .find(|key| !["popup", "pane", "back", "lead"].contains(key))
        {
            return Err(invalid(format!(
                "`tmux.{unknown}` is not a setting; use popup, pane, back or lead."
            )));
        }
        let key = |name: &str| -> Result<Option<String>, SquadError> {
            match table.and_then(|table| table.get(name)) {
                None => Ok(None),
                Some(item) => item
                    .as_str()
                    .filter(|key| tmux_key(key))
                    .map(|key| Some(key.to_owned()))
                    .ok_or_else(|| {
                        invalid(format!(
                            "`tmux.{name}` must be a tmux key such as \"S\", \"C-s\" or \"F5\"."
                        ))
                    }),
            }
        };
        let keys = TmuxKeys {
            popup: key("popup")?.unwrap_or_else(|| "S".into()),
            pane: key("pane")?.unwrap_or_else(|| "B".into()),
            back: key("back")?,
            lead: key("lead")?,
        };
        let mut chosen = vec![&keys.popup, &keys.pane];
        chosen.extend(keys.back.as_ref());
        chosen.extend(keys.lead.as_ref());
        if (1..chosen.len()).any(|index| chosen[..index].contains(&chosen[index])) {
            return Err(invalid("`tmux` keys must differ from each other."));
        }
        Ok(keys)
    }

    /// Top-level `me`: the saved identity that is the user. Never guessed.
    pub fn me(&self) -> Result<Option<&str>, SquadError> {
        match self.document.get("me") {
            None => Ok(None),
            Some(item) => item
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .map(Some)
                .ok_or_else(|| invalid("`me` must be a non-empty identity name.")),
        }
    }

    /// Top-level `me_id`: the UUID `me` named when it was recorded, so `me`
    /// follows an identity rename. Written by squad, never needed by hand.
    pub fn me_id(&self) -> Result<Option<&str>, SquadError> {
        match self.document.get("me_id") {
            None => Ok(None),
            Some(item) => item
                .as_str()
                .filter(|id| uuid_like(id))
                .map(Some)
                .ok_or_else(|| invalid("`me_id` must be the UUID of a saved identity.")),
        }
    }

    /// The `[squad.<name>]` table, when the user configured one.
    fn squad_table(&self, squad: &str) -> Result<Option<&dyn TableLike>, SquadError> {
        let Some(section) = self.document.get("squad") else {
            return Ok(None);
        };
        let table = section
            .as_table_like()
            .ok_or_else(|| invalid("`squad` must be a table of squads."))?;
        table
            .get(squad)
            .map(|entry| {
                entry
                    .as_table_like()
                    .ok_or_else(|| invalid(format!("`squad.{squad}` must be a table.")))
            })
            .transpose()
    }

    /// Team's defaults enter the ordinary row/provider/reminder readers.
    /// Whole row grids are replaced; provider fields and reminder keys override
    /// their matching defaults. No other layout's settings are changed.
    fn preset_settings(&self, squad: &str) -> Result<Option<Table>, SquadError> {
        let own = self.squad_table(squad)?;
        if self.layout(squad)? != Layout::Team {
            return Ok(own.map(|table| {
                table
                    .iter()
                    .map(|(key, value)| (key, value.clone()))
                    .collect()
            }));
        }
        let mut settings = team()["team"].as_table().expect("team table").clone();
        // Board::preset owns the pane layout, not this settings projection.
        settings.remove("board");
        if let Some(own) = own {
            if own.get("rows").is_some() || own.get("columns").is_some() {
                settings.remove("rows");
            }
            for (key, item) in own.iter() {
                if matches!(key, "fields" | "reminders")
                    && let Some(overrides) = item.as_table_like()
                {
                    let defaults = settings[key].as_table_mut().expect("preset table");
                    for (name, value) in overrides.iter() {
                        defaults.insert(name, value.clone());
                    }
                    continue;
                }
                settings.insert(key, item.clone());
            }
        }
        Ok(Some(settings))
    }

    /// `[squad.<name>.reminders]`, off except team. No global enable switch.
    pub fn reminders(&self, squad: &str) -> Result<Reminders, SquadError> {
        let place = format!("squad.{squad}.reminders");
        let settings = self.preset_settings(squad)?;
        let Some(item) = settings.as_ref().and_then(|table| table.get("reminders")) else {
            return Ok(Reminders::default());
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        let mut reminders = Reminders::default();
        for (key, item) in table.iter() {
            match key {
                "enabled" => {
                    reminders.enabled = item.as_bool().ok_or_else(|| {
                        invalid(format!("`{place}.enabled` must be true or false."))
                    })?
                }
                "stale_after" => {
                    let wrong = || {
                        invalid(format!(
                            "`{place}.stale_after` must be 1m-24h in whole s/m/h units, such as \"30m\"."
                        ))
                    };
                    let text = item.as_str().ok_or_else(wrong)?;
                    reminders.stale_after = duration(text, &['s', 'm', 'h'])
                        // Reminders historically require digits, unlike the older timings.
                        .filter(|_| text.as_bytes().first().is_some_and(u8::is_ascii_digit))
                        .filter(|duration| (60..=86400).contains(&duration.as_secs()))
                        .ok_or_else(wrong)?;
                }
                _ => {
                    return Err(invalid(format!(
                        "`{place}.{key}` is not a reminder setting; use enabled and stale_after."
                    )));
                }
            }
        }
        Ok(reminders)
    }

    /// Resolve the explicit preset or the compatible default in one place.
    fn resolve_layout(&self, squad: &str) -> Result<Layout, SquadError> {
        self.layout_setting(squad).map(|resolved| resolved.0)
    }

    /// Squads without a layout key use team unless the simple board form keeps crew.
    pub fn layout(&self, squad: &str) -> Result<Layout, SquadError> {
        self.resolve_layout(squad)
    }

    /// User-defined `[[squad.<name>.section]]` entries, in order. None means
    /// the single default list. `bind` is shape-checked here and used by the
    /// board's actions; it never comes from row data.
    pub fn sections(&self, squad: &str) -> Result<Vec<Section>, SquadError> {
        read_sections(
            self.squad_table(squad)?
                .and_then(|table| table.get("section")),
            &format!("squad.{squad}.section"),
        )
    }

    /// How rows are laid out: `[squad.<name>.rows]`, the older `columns`
    /// table, or the preset.
    /// The board's theme: core, then `[board.theme]`, then the squad's
    /// `[squad.<name>.theme]`. Invalid core appearance falls back to the
    /// built-in base before Squad overrides, with a notice. Invalid Squad
    /// layers remain configuration errors.
    pub fn theme(&self, squad: &str) -> Result<(tmt_cli_style::Theme, Option<String>), SquadError> {
        let place = format!("squad.{squad}.theme");
        let own = theme_settings(
            self.squad_table(squad)?
                .and_then(|table| table.get("theme")),
            &place,
        )?;
        let board = self.board_theme()?;
        use crate::look::Problem;
        // A broken global theme is TMT's config, not this file's: the board
        // keeps the squad's own theme over the default and says why.
        let (global, notice) = match &self.theme_error {
            Some(problem) => (&[][..], Some(problem.clone())),
            None => (&self.global_theme[..], None),
        };
        let (theme, notice) = match crate::look::board_theme(global, &board, &own, &place) {
            Ok(theme) => (theme, notice),
            Err(Problem::Global(problem)) => (
                crate::look::board_theme(&[], &board, &own, &place).map_err(
                    |problem| match problem {
                        Problem::Global(message)
                        | Problem::Board(message)
                        | Problem::Squad(message) => invalid(message),
                    },
                )?,
                Some(problem),
            ),
            Err(Problem::Board(message) | Problem::Squad(message)) => return Err(invalid(message)),
        };
        Ok((
            theme,
            notice.map(|notice| format!("{notice}; the board ignores the invalid CLI theme")),
        ))
    }

    fn board_theme(&self) -> Result<Vec<(String, String)>, SquadError> {
        let board = self
            .document
            .get("board")
            .map(|item| {
                item.as_table_like()
                    .ok_or_else(|| invalid("`board` must be a table."))
            })
            .transpose()?;
        theme_settings(board.and_then(|table| table.get("theme")), "board.theme")
    }

    /// The layer supplying the effective base, separately from token overrides.
    pub fn theme_source(&self, squad: &str) -> Result<&'static str, SquadError> {
        Ok(self.theme_setting(squad, "base")?.1)
    }

    /// Authored token and provenance through the same validated theme layers.
    fn theme_setting(
        &self,
        squad: &str,
        name: &str,
    ) -> Result<(Option<String>, &'static str), SquadError> {
        let (_, notice) = self.theme(squad)?;
        let own = theme_settings(
            self.squad_table(squad)?
                .and_then(|table| table.get("theme")),
            &format!("squad.{squad}.theme"),
        )?;
        let board = self.board_theme()?;
        for (source, settings) in [
            ("squad", &own[..]),
            ("board", &board[..]),
            ("cli", &self.global_theme[..]),
        ] {
            if source == "cli" && notice.is_some() {
                continue;
            }
            if let Some((_, value)) = settings.iter().find(|(key, _)| key == name) {
                return Ok((Some(value.clone()), source));
            }
        }
        Ok((None, "default"))
    }

    /// Edit only a base; all token overrides and unrelated TOML stay intact.
    pub fn set_theme_base(
        &mut self,
        scope: &crate::theme::ThemeScope,
        base: tmt_cli_style::Base,
    ) -> Result<bool, SquadError> {
        self.edit_theme_base(scope, Some(base))
    }

    /// Remove only the selected layer's base, retaining token overrides.
    pub fn remove_theme_base(
        &mut self,
        scope: &crate::theme::ThemeScope,
    ) -> Result<bool, SquadError> {
        self.edit_theme_base(scope, None)
    }

    fn edit_theme_base(
        &mut self,
        scope: &crate::theme::ThemeScope,
        base: Option<tmt_cli_style::Base>,
    ) -> Result<bool, SquadError> {
        let draft = self.theme_draft(scope, base)?;
        draft.theme(scope.squad().unwrap_or(""))?;
        draft.refresh(scope.squad().unwrap_or(""))?;
        let changed = draft.document.to_string() != self.document.to_string();
        self.write(|document| *document = draft.document)?;
        Ok(changed)
    }

    /// Preview a base through exactly the same layer edits without writing.
    pub fn preview_theme_base(
        &self,
        scope: &crate::theme::ThemeScope,
        base: tmt_cli_style::Base,
        squad: &str,
    ) -> Result<tmt_cli_style::Theme, SquadError> {
        self.theme_draft(scope, Some(base))?
            .theme(squad)
            .map(|(theme, _)| theme)
    }

    fn theme_draft(
        &self,
        scope: &crate::theme::ThemeScope,
        base: Option<tmt_cli_style::Base>,
    ) -> Result<Self, SquadError> {
        let mut draft = self.clone();
        let path: Vec<&str> = match scope {
            crate::theme::ThemeScope::Board => vec!["board", "theme"],
            crate::theme::ThemeScope::Squad(name) => vec!["squad", name, "theme"],
        };
        let mut table: &mut dyn toml_edit::TableLike = draft.document.as_table_mut();
        for (index, key) in path.iter().enumerate() {
            if !table.contains_key(key) {
                if base.is_none() {
                    return Ok(draft);
                }
                let mut child = Table::new();
                child.set_implicit(index + 1 < path.len());
                table.insert(key, Item::Table(child));
            }
            table = table
                .get_mut(key)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| {
                    invalid(format!(
                        "`{}` must be a table to edit its base.",
                        path[..=index].join(".")
                    ))
                })?;
        }
        match base {
            Some(base) => {
                if table.get("base").and_then(Item::as_str) == Some(base.name()) {
                    return Ok(draft);
                }
                // Retain an existing base's decoration as well as token overrides.
                let decor = table
                    .get("base")
                    .and_then(Item::as_value)
                    .map(|value| value.decor().clone());
                let mut item = value(base.name());
                if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
                    *value.decor_mut() = decor;
                }
                table.insert("base", item);
            }
            None => {
                table.remove("base");
            }
        }
        Ok(draft)
    }

    /// `[squad.<name>.fields]`: the squad's field providers.
    pub fn providers(&self, squad: &str) -> Result<Vec<crate::provider::Provider>, SquadError> {
        crate::provider::read(
            self.preset_settings(squad)?
                .as_ref()
                .map(|table| table as &dyn TableLike),
            squad,
            crate::rows::field_name,
            |field| crate::rows::OWN_FIELDS.contains(&field),
        )
    }

    pub fn rows(&self, squad: &str) -> Result<crate::rows::Rows, SquadError> {
        self.rows_setting(squad).map(|resolved| resolved.0)
    }
    fn read_rows(&self, squad: &str) -> Result<crate::rows::Rows, SquadError> {
        crate::rows::read(
            self.preset_settings(squad)?
                .as_ref()
                .map(|table| table as &dyn TableLike),
            squad,
        )
    }

    fn view_setting(
        &self,
        squad: Option<&str>,
    ) -> Result<Option<crate::view::ViewName>, SquadError> {
        let (item, place) = match squad {
            Some(name) => (
                self.squad_table(name)?.and_then(|table| table.get("board")),
                format!("squad.{name}.board"),
            ),
            None => (self.document.get("board"), "board".into()),
        };
        let Some(item) = item else { return Ok(None) };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        table
            .get("view")
            .map(|item| {
                item.as_str()
                    .and_then(crate::view::ViewName::parse)
                    .ok_or_else(|| {
                        invalid(format!(
                            "`{place}.view` must be team, focus, notes, detail or wide."
                        ))
                    })
            })
            .transpose()
    }

    fn validate_views(&self) -> Result<(), SquadError> {
        self.view_setting(None)?;
        if let Some(table) = self.document.get("squad").and_then(Item::as_table_like) {
            for (name, _) in table.iter() {
                self.view_setting(Some(name))?;
            }
        }
        Ok(())
    }

    pub fn custom_board(&self, squad: &str) -> Result<bool, SquadError> {
        Ok(self
            .squad_table(squad)?
            .and_then(|table| table.get("board"))
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.get("layout").is_some() || table.get("panes").is_some()))
    }

    /// Arrangement precedence stays separate from the workflow layout decision.
    pub fn view_source(
        &self,
        squad: &str,
    ) -> Result<(Option<crate::view::ViewName>, &'static str), SquadError> {
        let global = self.view_setting(None)?;
        let own = self.view_setting(Some(squad))?;
        Ok(if self.custom_board(squad)? {
            (None, "custom")
        } else if let Some(view) = own {
            (Some(view), "squad")
        } else if let Some(view) = global {
            (Some(view), "board")
        } else {
            (
                (self.resolve_layout(squad)? == Layout::Team)
                    .then_some(crate::view::ViewName::Team),
                "layout",
            )
        })
    }

    pub fn set_view(
        &mut self,
        scope: &crate::view::ViewScope,
        view: crate::view::ViewName,
    ) -> Result<bool, SquadError> {
        if let Some(name) = scope.squad() {
            self.refuse_custom_view(name)?;
        }
        self.edit_view(scope, Some(view))
    }

    fn refuse_custom_view(&self, squad: &str) -> Result<(), SquadError> {
        if self.custom_board(squad)? {
            return Err(SquadError::hinted(
                "SQUAD_VIEW_CUSTOM",
                &format!("squad {squad} has a hand-written board layout"),
                "; ",
                &format!(
                    "remove squad.{squad}.board.layout or panes from squad.toml manually before saving a view"
                ),
            ));
        }
        Ok(())
    }

    pub fn remove_view(&mut self, scope: &crate::view::ViewScope) -> Result<bool, SquadError> {
        self.edit_view(scope, None)
    }

    fn edit_view(
        &mut self,
        scope: &crate::view::ViewScope,
        view: Option<crate::view::ViewName>,
    ) -> Result<bool, SquadError> {
        let draft = self.view_draft(scope, view)?;
        draft.board(scope.squad().unwrap_or(""))?;
        draft.refresh(scope.squad().unwrap_or(""))?;
        if scope.squad().is_none()
            && let Some(squads) = draft.document.get("squad").and_then(Item::as_table_like)
        {
            for (name, _) in squads.iter() {
                draft.board(name)?;
            }
        }
        let changed = draft.document.to_string() != self.document.to_string();
        self.write(|document| *document = draft.document)?;
        Ok(changed)
    }

    fn view_draft(
        &self,
        scope: &crate::view::ViewScope,
        view: Option<crate::view::ViewName>,
    ) -> Result<Self, SquadError> {
        let mut draft = self.clone();
        let path = match scope {
            crate::view::ViewScope::Board => vec!["board"],
            crate::view::ViewScope::Squad(name) => vec!["squad", name, "board"],
        };
        let mut table: &mut dyn TableLike = draft.document.as_table_mut();
        for (index, key) in path.iter().enumerate() {
            if !table.contains_key(key) {
                if view.is_none() {
                    return Ok(draft);
                }
                let mut child = Table::new();
                child.set_implicit(index + 1 < path.len());
                table.insert(key, Item::Table(child));
            }
            table = table
                .get_mut(key)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| {
                    invalid(format!(
                        "`{}` must be a table to edit its view.",
                        path[..=index].join(".")
                    ))
                })?;
        }
        let had_view = table.contains_key("view");
        match view {
            Some(view) if table.get("view").and_then(Item::as_str) != Some(view.name()) => {
                let decor = table
                    .get("view")
                    .and_then(Item::as_value)
                    .map(|value| value.decor().clone());
                let mut item = value(view.name());
                if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
                    *value.decor_mut() = decor;
                }
                table.insert("view", item);
            }
            None => {
                table.remove("view");
            }
            _ => {}
        }
        if view.is_none() && had_view && table.is_empty() {
            let mut parent: &mut dyn TableLike = draft.document.as_table_mut();
            for key in &path[..path.len() - 1] {
                parent = parent
                    .get_mut(key)
                    .and_then(Item::as_table_like_mut)
                    .expect("validated view parent");
            }
            let key = path.last().unwrap();
            if parent
                .get(key)
                .and_then(Item::as_table)
                .is_some_and(|table| {
                    [table.decor().prefix(), table.decor().suffix()]
                        .into_iter()
                        .all(|raw| {
                            raw.and_then(|raw| raw.as_str())
                                .is_none_or(|text| text.trim().is_empty())
                        })
                })
            {
                parent.remove(key);
            }
        }
        Ok(draft)
    }

    /// Custom preview suppresses arrangement keys only on a disposable copy.
    /// Save still checks the untouched opening configuration and refuses custom.
    pub fn preview_view(
        &self,
        scope: &crate::view::ViewScope,
        view: Option<crate::view::ViewName>,
        squad: &str,
    ) -> Result<Self, SquadError> {
        let mut draft = self.view_draft(scope, view)?;
        if view.is_some() && self.custom_board(squad)? {
            let table = draft.document["squad"][squad]["board"]
                .as_table_like_mut()
                .expect("custom board table");
            for key in [
                "layout",
                "panes",
                "direction",
                "sizes",
                "mode",
                "collapsed",
                "fold_below",
            ] {
                table.remove(key);
            }
        }
        Ok(draft)
    }

    /// `[squad.<name>.board]` over the layout's preset. Validated before the
    /// terminal changes mode, so a mistake never leaves a half-drawn screen.
    pub fn board(&self, squad: &str) -> Result<Board, SquadError> {
        self.board_setting(squad).map(|resolved| resolved.0)
    }
    fn read_board(&self, squad: &str) -> Result<Board, SquadError> {
        let layout = self.resolve_layout(squad)?;
        let (view, _) = self.view_source(squad)?;
        let preset = view.map_or_else(|| Board::preset(layout), Board::factory);
        let place = format!("squad.{squad}.board");
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("board"))
        else {
            return Ok(preset);
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        let text = |key: &str| table.get(key).map(|value| value.as_str());
        for (key, _) in table.iter() {
            if ![
                "mode",
                "direction",
                "panes",
                "sizes",
                "layout",
                "refresh",
                "collapsed",
                "fold_below",
                "token_rate",
                "view",
                "hidden_columns",
            ]
            .contains(&key)
            {
                return Err(invalid(format!("`{place}.{key}` is not a board setting.")));
            }
        }
        let mode = match text("mode") {
            None => preset.mode,
            Some(Some("split")) => BoardMode::Split,
            Some(Some("tabs")) => BoardMode::Tabs,
            Some(_) => return Err(invalid(format!("`{place}.mode` must be split or tabs."))),
        };
        // The full form: a nested split. The one-level keys are its simple
        // form, so the two are never mixed.
        if let Some(layout) = table.get("layout") {
            if let Some(key) = ["direction", "panes", "sizes"]
                .into_iter()
                .find(|key| table.get(key).is_some())
            {
                return Err(invalid(format!(
                    "`{place}` sets both `layout` and `{key}`; keep `layout`."
                )));
            }
            if mode == BoardMode::Tabs {
                return Err(invalid(format!(
                    "`{place}.layout` applies to split mode only."
                )));
            }
            let split = crate::split::read(layout, &format!("{place}.layout"))?;
            let mut board = Board {
                mode,
                panes: split.panes(),
                split,
                collapsed: Default::default(),
                fold_below: None,
            };
            board.read_collapsed(table.get("collapsed"), &place)?;
            board.read_fold_below(table.get("fold_below"), &place)?;
            return Ok(board);
        }
        if (layout == Layout::Team || view.is_some()) && table.get("panes").is_none() {
            if let Some(key) = ["direction", "sizes"]
                .into_iter()
                .find(|key| table.get(key).is_some())
            {
                return Err(invalid(format!(
                    "`{place}.{key}` cannot partially override the resolved arrangement; set `{place}.layout` or `{place}.panes`."
                )));
            }
            let mut board = Board { mode, ..preset };
            if mode == BoardMode::Tabs {
                board.fold_below = None;
                board.collapsed.clear();
            }
            board.read_collapsed(table.get("collapsed"), &place)?;
            board.read_fold_below(table.get("fold_below"), &place)?;
            return Ok(board);
        }
        let (mut direction, mut panes, mut sizes) = match &preset.split {
            crate::split::Split::Group {
                direction,
                children,
            } => (
                *direction,
                preset.panes.clone(),
                children
                    .iter()
                    .map(|(size, _)| match size {
                        crate::split::Size::Percent(percent) => *percent,
                        crate::split::Size::Grow(_) => 0,
                    })
                    .collect::<Vec<u16>>(),
            ),
            crate::split::Split::Pane(_) => (Direction::LeftRight, preset.panes.clone(), vec![100]),
        };
        match text("direction") {
            None => {}
            Some(Some("left-right")) => direction = Direction::LeftRight,
            Some(Some("top-bottom")) => direction = Direction::TopBottom,
            Some(_) => {
                return Err(invalid(format!(
                    "`{place}.direction` must be left-right or top-bottom."
                )));
            }
        }
        let panes_set = table.get("panes").is_some();
        if let Some(names) = table.get("panes") {
            let names = names
                .as_array()
                .ok_or_else(|| invalid(format!("`{place}.panes` must list panes.")))?;
            let mut chosen = Vec::new();
            for name in names.iter() {
                let pane = name.as_str().and_then(Pane::parse).ok_or_else(|| {
                    invalid(format!(
                        "`{place}.panes` entries are rows, notes, detail or replies."
                    ))
                })?;
                if chosen.contains(&pane) {
                    return Err(invalid(format!(
                        "`{place}.panes` lists {} twice.",
                        pane.title()
                    )));
                }
                chosen.push(pane);
            }
            if !chosen.contains(&Pane::Rows) {
                return Err(invalid(format!("`{place}.panes` must include rows.")));
            }
            panes = chosen;
        }
        match (table.get("sizes"), mode) {
            (Some(_), BoardMode::Tabs) => {
                return Err(invalid(format!(
                    "`{place}.sizes` applies to split mode only."
                )));
            }
            (Some(given), BoardMode::Split) => {
                sizes = given
                    .as_array()
                    .ok_or_else(|| invalid(format!("`{place}.sizes` must list percentages.")))?
                    .iter()
                    .map(|size| {
                        size.as_integer()
                            .and_then(|size| u16::try_from(size).ok())
                            .filter(|size| (10..=100).contains(size))
                            .ok_or_else(|| {
                                invalid(format!("`{place}.sizes` entries must be 10-100."))
                            })
                    })
                    .collect::<Result<_, _>>()?;
            }
            // Changed panes without sizes share the width equally.
            (None, _) if panes_set => {
                let share = 100 / panes.len() as u16;
                sizes = vec![share; panes.len()];
                if let Some(last) = sizes.last_mut() {
                    *last += 100 - share * panes.len() as u16;
                }
            }
            (None, _) => {}
        }
        if mode == BoardMode::Split
            && (sizes.len() != panes.len() || sizes.iter().sum::<u16>() != 100)
        {
            return Err(invalid(format!(
                "`{place}.sizes` needs one percentage per pane, summing to 100."
            )));
        }
        // In tabs mode the lead's full notes always get their own tab.
        if mode == BoardMode::Tabs && !panes.contains(&Pane::Notes) {
            panes.push(Pane::Notes);
            sizes.push(0);
        }
        let mut board = Board::simple(mode, direction, panes, &sizes);
        board.read_collapsed(table.get("collapsed"), &place)?;
        board.read_fold_below(table.get("fold_below"), &place)?;
        Ok(board)
    }

    /// How often the board reloads everything: `[squad.<name>.board] refresh`,
    /// then top-level `[board] refresh`, then [`DEFAULT_REFRESH`]. `None` is
    /// "off": only ctrl-r and the board's own actions reload.
    pub fn refresh(&self, squad: &str) -> Result<Option<Duration>, SquadError> {
        self.refresh_setting(squad).map(|resolved| resolved.0)
    }

    /// Preset, then global, then per-squad keys; no implicit second team path.
    pub fn token_rate(&self, squad: &str) -> Result<TokenRate, SquadError> {
        let mut settings = TokenRate::default();
        let own = self
            .squad_table(squad)?
            .and_then(|table| table.get("board"))
            .and_then(Item::as_table_like)
            .and_then(|table| table.get("token_rate"));
        let preset = (self.resolve_layout(squad)? == Layout::Team)
            .then(|| &team()["team"]["board"]["token_rate"]);
        let global = self
            .document
            .get("board")
            .and_then(Item::as_table_like)
            .and_then(|table| table.get("token_rate"));
        for (item, place) in [
            (preset, "team.board.token_rate".to_owned()),
            (global, "board.token_rate".to_owned()),
            (own, format!("squad.{squad}.board.token_rate")),
        ] {
            let Some(item) = item else { continue };
            let table = item
                .as_table_like()
                .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
            for (key, item) in table.iter() {
                match key {
                    "enabled" | "reduced_motion" => {
                        let value = item.as_bool().ok_or_else(|| {
                            invalid(format!("`{place}.{key}` must be a boolean."))
                        })?;
                        if key == "enabled" {
                            settings.enabled = value;
                        } else {
                            settings.reduced_motion = value;
                        }
                    }
                    "window" => {
                        settings.window =
                            item.as_str().and_then(TokenWindow::parse).ok_or_else(|| {
                                invalid(format!("`{place}.window` must be 5s, 1m, 30m or 1h."))
                            })?;
                    }
                    "every" => {
                        settings.every = refresh(item, &format!("{place}.every"))?
                            .filter(|every| {
                                (Duration::from_secs(5)..=Duration::from_secs(10)).contains(every)
                            })
                            .ok_or_else(|| {
                                invalid(format!("`{place}.every` must be 5s through 10s."))
                            })?;
                    }
                    _ => {
                        return Err(invalid(format!(
                            "`{place}.{key}` is not a token-rate setting."
                        )));
                    }
                }
            }
        }
        settings.window = settings.window.available(settings.every);
        Ok(settings)
    }

    /// `[squad.<name>.notes] render = "markdown" | "plain"`; markdown by default.
    pub fn notes_render(&self, squad: &str) -> Result<NotesRender, SquadError> {
        self.notes_setting(squad).map(|resolved| resolved.0)
    }

    /// `[tabs]` (#507): the tab order, hidden tabs, the colors by attention
    /// and the built-in tabs' own bindings. In `order` and `hide`, `leads`
    /// and `all` are the built-in tabs and `squad:<name>` names a squad whose
    /// name is taken by a built-in; any other entry is a squad name.
    pub fn tabs(&self) -> Result<Tabs, SquadError> {
        let mut tabs = Tabs::default();
        let Some(item) = self.document.get("tabs") else {
            return Ok(tabs);
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid("`tabs` must be a table."))?;
        // Any explicit ordering policy, including an empty array, keeps the
        // user's existing arrangement. Hide alone still uses the new defaults.
        if table.contains_key("order") || table.contains_key("pin") {
            tabs.order.clear();
            tabs.pin.clear();
        }
        for (key, item) in table.iter() {
            match key {
                "order" => tabs.order = tab_list(item, "tabs.order")?,
                "hide" => tabs.hide = tab_list(item, "tabs.hide")?,
                "pin" => tabs.pin = tab_list(item, "tabs.pin")?,
                "colors" => tabs.colors = tab_colors(item)?,
                "leads" => tabs.leads = tab_bindings(item, "tabs.leads")?,
                "all" => tabs.all = tab_bindings(item, "tabs.all")?,
                other => tabs.user.push(UserTab::read(other, item)?),
            }
        }
        if tabs.user.len() > MAX_SECTIONS {
            return Err(invalid(format!(
                "`tabs` allows at most {MAX_SECTIONS} user tabs."
            )));
        }
        for (place, list) in [
            ("order", &tabs.order),
            ("hide", &tabs.hide),
            ("pin", &tabs.pin),
        ] {
            for key in list {
                if let Some(name) = crate::tabs::user_name(key)
                    && !tabs.user.iter().any(|tab| tab.name == name)
                {
                    return Err(invalid(format!(
                        "`tabs.{place}` references undefined `tabs.{name}`."
                    )));
                }
            }
        }
        Ok(tabs)
    }

    /// Resolve the layout's exact states and per-squad ordered glob patterns.
    pub fn states(&self, squad: &str, layout: Layout) -> Result<States, SquadError> {
        States::read(self.squad_table(squad)?, squad, layout)
    }

    /// Writes `me` and its UUID `me_id` together by replacing the file
    /// atomically. Refuses if another editor changed the file since it was
    /// read, rather than overwriting their edit.
    pub fn set_me(&mut self, name: &str, id: &str) -> Result<(), SquadError> {
        self.write_me(Some((name, id)))
    }

    /// Removes `me` and `me_id` the same way; the rest of the file is kept.
    pub fn clear_me(&mut self) -> Result<(), SquadError> {
        self.write_me(None)
    }

    fn write_me(&mut self, me: Option<(&str, &str)>) -> Result<(), SquadError> {
        self.write(|document| match me {
            Some((name, id)) => {
                document.insert(
                    "me",
                    Item::Value(value(name).into_value().expect("string value")),
                );
                document.insert(
                    "me_id",
                    Item::Value(value(id).into_value().expect("string value")),
                );
            }
            None => {
                document.remove("me");
                document.remove("me_id");
            }
        })
    }

    /// Writes `[tabs] order` (#507), keeping the rest of the file as it is.
    /// `keys` are tab keys; each is written as `order` reads it back.
    pub fn set_tab_order(&mut self, keys: &[String]) -> Result<(), SquadError> {
        let names: toml_edit::Array = keys
            .iter()
            .map(|key| match key.as_str() {
                crate::board::LEADS => "leads".to_owned(),
                crate::board::ALL => "all".to_owned(),
                "leads" | "all" => format!("squad:{key}"),
                key if crate::tabs::user_name(key).is_some() => {
                    format!("tab:{}", crate::tabs::user_name(key).unwrap())
                }
                squad => squad.to_owned(),
            })
            .collect();
        if !matches!(self.document.get("tabs"), None | Some(Item::Table(_))) {
            return Err(invalid(
                "`tabs` is not a [tabs] table, so the order cannot be saved; edit squad.toml.",
            ));
        }
        self.write(|document| {
            let tabs = document
                .entry("tabs")
                .or_insert_with(|| Item::Table(Table::new()));
            tabs["order"] = toml_edit::value(names);
        })
    }

    /// Replaces the file atomically with one edit applied. Refuses if another
    /// editor changed the file since it was read, rather than overwriting
    /// their edit.
    fn write(&mut self, edit: impl FnOnce(&mut DocumentMut)) -> Result<(), SquadError> {
        let current = read_bounded(&self.path).map_err(|error| write_failed(&self.path, error))?;
        if current != self.original {
            return Err(SquadError::new(
                "SQUAD_CONFIG_CHANGED",
                format!(
                    "{} changed while squad was running; retry.",
                    self.path.display()
                ),
            ));
        }
        let mut document = self.document.clone();
        edit(&mut document);
        let bytes = document.to_string().into_bytes();
        if self.original.as_deref().unwrap_or_default() == bytes {
            return Ok(());
        }
        publish(&self.path, &bytes).map_err(|error| write_failed(&self.path, error))?;
        self.document = document;
        self.original = Some(bytes);
        Ok(())
    }
}

/// The shape core gives identity UUIDs; anything else is a hand edit.
pub fn uuid_like(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn write_failed(path: &Path, error: io::Error) -> SquadError {
    SquadError::new(
        "SQUAD_CONFIG_WRITE_FAILED",
        format!("Could not write {}: {error}", path.display()),
    )
}

fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(io::Error::other("larger than 1 MiB"));
    }
    Ok(Some(bytes))
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("no parent directory"))?;
    fs::create_dir_all(directory)?;
    let staged = directory.join(format!(".squad.toml.{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&staged, path));
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written?;
    fs::File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::Split;

    #[test]
    fn link_handlers_are_explicit_validated_run_bindings() {
        let path = temp("links");
        fs::write(&path, "[links]\ngh='run gh issue view {path}'").unwrap();
        assert!(
            Config::read(path.clone())
                .unwrap()
                .links()
                .unwrap()
                .contains_key("gh")
        );
        for value in [
            "gh='open {path}'",
            "tmt='run tmt {path}'",
            "gh='run {path}'",
            "gh='run gh {name}'",
            "gh=42",
        ] {
            fs::write(&path, format!("[links]\n{value}")).unwrap();
            assert!(Config::read(path.clone()).is_err(), "{value}");
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reminders_are_per_squad_team_enabled_by_default_and_bounded() {
        let path = temp("reminders-valid");
        fs::write(
            &path,
            "[squad.product.reminders]\nenabled = true\nstale_after = \"30m\"\n",
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(
            config.reminders("product").unwrap(),
            Reminders {
                enabled: true,
                stale_after: Duration::from_secs(1800)
            }
        );
        assert_eq!(
            config.reminders("other").unwrap(),
            Reminders {
                enabled: true,
                ..Reminders::default()
            }
        );
        for (value, seconds) in [
            ("60s", 60),
            ("1m", 60),
            ("0005m", 300),
            ("1h", 3600),
            ("24h", 86400),
            ("1440m", 86400),
            ("86400s", 86400),
        ] {
            fs::write(
                &path,
                format!("[squad.product.reminders]\nstale_after = {value:?}\n"),
            )
            .unwrap();
            let config = Config::read(path.clone()).unwrap();
            assert_eq!(
                config.reminders("product").unwrap().stale_after,
                Duration::from_secs(seconds)
            );
            assert!(config.reminders("product").unwrap().enabled);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_reminders_never_mutate_config() {
        let path = temp("reminders-invalid");
        for setting in [
            "enabled = 1",
            "enabled = \"true\"",
            "stale_after = 60",
            "extra = true",
            "stale_after = \"59s\"",
            "stale_after = \"25h\"",
            "stale_after = \"1.5m\"",
            "stale_after = \"+1m\"",
            "stale_after = \"-1m\"",
            // Non-ASCII input deliberately exercises the parser boundary.
            "stale_after = \"1分钟\"",
            "stale_after = \"5分\"",
            "stale_after = \"5秒\"",
            "stale_after = \"18446744073709551615h\"",
        ] {
            let original = format!("[squad.product.reminders]\n{setting}\n");
            fs::write(&path, &original).unwrap();
            let config = Config::read(path.clone()).unwrap();
            let error = config.reminders("product").unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(error.message.contains("squad.product.reminders"), "{error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
        fs::write(&path, "[squad.product]\nreminders = true\n").unwrap();
        assert!(
            Config::read(path.clone())
                .unwrap()
                .reminders("product")
                .is_err()
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn theme_base_edits_keep_the_exact_surrounding_document_and_tokens() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::{Base, Depth, Role};
        let path = temp("theme-base-preservation");
        let original = "# personal board\nopaque = { future = 42 }\n\n[board]\nrefresh = \"off\" # manual\n\n[board.theme] # colors\nbase = \"tmt\" # dark\naccent = \"blue\"\n\n[squad.product.theme]\nwaiting = \"red\" # attention\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        assert!(
            config
                .set_theme_base(&ThemeScope::Board, Base::TmtLight)
                .unwrap()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original.replacen("base = \"tmt\"", "base = \"tmt-light\"", 1)
        );
        let (theme, _) = config.theme("product").unwrap();
        assert_eq!(theme.base, Base::TmtLight);
        assert_eq!(
            theme.style(Role::Waiting, Depth::TrueColor),
            tmt_cli_style::Theme::parse("expected", [("waiting", "red")])
                .unwrap()
                .style(Role::Waiting, Depth::TrueColor)
        );
        assert!(config.remove_theme_base(&ThemeScope::Board).unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original.replace("base = \"tmt\" # dark\n", "")
        );
        assert_eq!(config.theme_source("product").unwrap(), "default");
        assert!(!config.remove_theme_base(&ThemeScope::Board).unwrap());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn theme_layers_resolve_each_token_and_report_the_base_source() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::{Base, Depth, Role, Theme};
        let path = temp("theme-three-layers");
        fs::write(&path, "[board.theme]\nbase = \"terminal\"\naccent = \"green\"\n[squad.product.theme]\nbase = \"mono\"\nwaiting = \"blue\"\n").unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        config.global_theme(&serde_json::json!({"resolved":{"theme":{"base":"tmt-light", "accent":"red", "blocked":"yellow"}}}));
        let (actual, _) = config.theme("product").unwrap();
        let expected = Theme::parse(
            "expected",
            [
                ("base", "mono"),
                ("accent", "green"),
                ("waiting", "blue"),
                ("blocked", "yellow"),
            ],
        )
        .unwrap();
        for role in Role::ALL {
            assert_eq!(
                actual.style(role, Depth::TrueColor),
                expected.style(role, Depth::TrueColor)
            );
        }
        assert_eq!(config.theme_source("product").unwrap(), "squad");
        config
            .remove_theme_base(&ThemeScope::Squad("product".into()))
            .unwrap();
        assert_eq!(config.theme("product").unwrap().0.base, Base::Terminal);
        assert_eq!(config.theme_source("product").unwrap(), "board");
        config.remove_theme_base(&ThemeScope::Board).unwrap();
        assert_eq!(config.theme("product").unwrap().0.base, Base::TmtLight);
        assert_eq!(config.theme_source("product").unwrap(), "cli");
        config.global_theme(&serde_json::json!({"resolved":{"theme":{"base":"invalid"}}}));
        assert_eq!(config.theme_source("product").unwrap(), "default");
        assert!(config.theme("product").unwrap().1.is_some());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn theme_writes_refuse_changed_files_even_for_an_identical_base() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::Base;
        let path = temp("theme-stale-write");
        fs::write(&path, "[board.theme]\nbase = \"mono\"\n").unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let original_document = config.document.to_string();
        fs::write(
            &path,
            "# edited elsewhere\n[board.theme]\nbase = \"mono\"\n",
        )
        .unwrap();
        let changed_bytes = fs::read(&path).unwrap();
        assert_eq!(
            config
                .set_theme_base(&ThemeScope::Board, Base::Mono)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(
            config
                .remove_theme_base(&ThemeScope::Board)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read(&path).unwrap(), changed_bytes);
        assert_eq!(config.document.to_string(), original_document);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_publication_keeps_config_and_does_not_leak_the_draft_into_me_write() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::Base;
        let path = temp("theme-failed-publish");
        let original = "[board.theme]\nbase = \"tmt\"\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let staged = path
            .parent()
            .unwrap()
            .join(format!(".squad.toml.{}", std::process::id()));
        fs::write(&staged, "occupied stage").unwrap();
        assert_eq!(
            config
                .set_theme_base(&ThemeScope::Board, Base::Mono)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_WRITE_FAILED"
        );
        assert_eq!(config.document.to_string(), original);
        assert_eq!(config.original.as_deref(), Some(original.as_bytes()));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_file(staged).unwrap();
        config
            .set_me("ada", "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f")
            .unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains(original));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn theme_noop_preserves_quoted_bytes_and_does_not_publish() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::Base;
        let path = temp("theme-noop");
        let original = "[board.theme]\nbase = 'mono' # untouched\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let staged = path
            .parent()
            .unwrap()
            .join(format!(".squad.toml.{}", std::process::id()));
        fs::write(&staged, "publish would fail").unwrap();
        assert!(
            !config
                .set_theme_base(&ThemeScope::Board, Base::Mono)
                .unwrap()
        );
        assert!(
            !config
                .remove_theme_base(&ThemeScope::Squad("other".into()))
                .unwrap()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn theme_edits_create_valid_tables_and_preserve_inline_overrides() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::Base;
        let path = temp("theme-create-inline");
        let mut config = Config::read(path.clone()).unwrap();
        assert!(!config.remove_theme_base(&ThemeScope::Board).unwrap());
        assert!(!path.exists());
        config
            .set_theme_base(&ThemeScope::Squad("product".into()), Base::Mono)
            .unwrap();
        let reread = Config::read(path.clone()).unwrap();
        assert_eq!(reread.theme("product").unwrap().0.base, Base::Mono);
        fs::write(&path, "board = { theme = { base = \"tmt\", accent = \"blue\" }, refresh = \"off\" } # inline\n").unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        config
            .set_theme_base(&ThemeScope::Board, Base::Terminal)
            .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "board = { theme = { base = \"terminal\", accent = \"blue\" }, refresh = \"off\" } # inline\n"
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_board_theme_is_not_hidden_by_a_squad_override_or_written() {
        use crate::theme::ThemeScope;
        use tmt_cli_style::Base;
        let path = temp("theme-invalid-board");
        for bad in [
            "waiting = \"orange\"",
            "waiting = 42",
            "unexpected = \"red\"",
        ] {
            let original = format!(
                "[board.theme]\n{bad}\n[squad.product.theme]\nbase = \"mono\"\nwaiting = \"blue\"\n"
            );
            fs::write(&path, &original).unwrap();
            let mut config = Config::read(path.clone()).unwrap();
            assert!(
                config
                    .theme("product")
                    .unwrap_err()
                    .message
                    .contains("board.theme")
            );
            assert!(
                config
                    .set_theme_base(&ThemeScope::Squad("product".into()), Base::Tmt)
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("tmt-squad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory.join("squad.toml")
    }

    #[test]
    fn settings_edits_use_existing_scopes_and_keep_custom_structure() {
        let path = temp("setting-scopes");
        fs::write(&path, "[squad.x.board]\ndirection='left-right'\n[squad.x.states.working]\nsort=7 # preserve rank\n[squad.x.columns]\nshow=['member','state','task','pr_link']\npr_link={width=12}\n").unwrap();
        let config = Config::read(path.clone()).unwrap();
        let draft = config
            .preview_setting(Some("x"), "board.direction", "top-bottom")
            .unwrap();
        let board = draft.board("x").unwrap();
        assert_eq!(
            board.split,
            Board::simple(
                BoardMode::Split,
                Direction::TopBottom,
                vec![Pane::Rows, Pane::Notes],
                &[60, 40]
            )
            .split
        );
        assert_eq!(draft.layout("x").unwrap(), Layout::Crew);
        let mut writer = config.clone();
        writer
            .set_setting(Some("x"), "board.direction", "top-bottom")
            .unwrap();
        let pinned = Config::read(path.clone()).unwrap();
        assert_eq!(pinned.board("x").unwrap().split, board.split);
        assert_eq!(
            pinned.document["squad"]["x"]["layout"].as_str(),
            Some("crew")
        );
        for key in ["direction", "panes", "sizes"] {
            assert!(pinned.document["squad"]["x"]["board"].get(key).is_some());
        }
        let resized = draft
            .preview_setting(Some("x"), "board.sizes", "[70,30]")
            .unwrap()
            .settings(Some("x"), false, None)
            .unwrap();
        assert_eq!(
            resized
                .entries
                .iter()
                .find(|entry| entry.key == "board.sizes")
                .unwrap()
                .value,
            serde_json::json!([70, 30])
        );
        let draft = config
            .preview_setting(Some("x"), "states.working.color", "blue")
            .unwrap();
        assert!(draft.document["squad"]["x"].get("layout").is_none());
        assert!(
            draft
                .document
                .to_string()
                .contains("sort=7 # preserve rank")
        );
        assert_eq!(
            draft
                .states("x", Layout::Crew)
                .unwrap()
                .color(Some("working")),
            Some("blue")
        );
        assert!(
            config
                .preview_setting(None, "notes.render", "plain")
                .is_err()
        );
        assert!(
            config
                .preview_setting(Some("x"), "states.working.sort", "1")
                .is_err()
        );
        let hidden = config
            .preview_setting(Some("x"), "board.hidden_columns", "[\"pr_link\"]")
            .unwrap();
        assert_eq!(
            hidden.rows("x").unwrap().columns,
            config.rows("x").unwrap().columns
        );
        assert!(hidden.document.to_string().contains("pr_link={width=12}"));
        let tabs = config
            .preview_setting(Some("x"), "tabs.hide", "[\"leads\"]")
            .unwrap();
        assert_eq!(tabs.tabs().unwrap().hide, [crate::tabs::LEADS]);
        assert!(tabs.document.get("tabs").is_some());
        assert!(tabs.document["squad"]["x"].get("tabs").is_none());
        fs::write(&path, "[squad.x.board]\nlayout={direction='left-right',sizes=[60,40],panes=['rows',{direction='top-bottom',sizes=[50,50],panes=['notes','detail']}]}\n").unwrap();
        let config = Config::read(path.clone()).unwrap();
        let before = config.document.to_string();
        assert!(!config.can_edit_setting("board.panes", Some("x")));
        assert!(
            config
                .preview_setting(Some("x"), "board.panes", "[\"rows\"]")
                .err()
                .unwrap()
                .message
                .contains("Nested")
        );
        assert_eq!(
            config
                .preview_setting(Some("x"), "notes.render", "plain")
                .unwrap()
                .board("x")
                .unwrap(),
            config.board("x").unwrap()
        );
        assert_eq!(config.document.to_string(), before);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn setting_drafts_preserve_grids_and_refuse_invalid_or_changed_files() {
        let path = temp("setting-edits");
        let original = "# kept\nopaque='keep me'\n[squad.x.rows]\ncolumns=[{name='member'},{name='state'},{name='task'},{name='pr_link'}]\nlines=[['member','state',{field='task',span=2}],[{field='member',span=4}]]\n[squad.x.board]\nrefresh='5s' # retain\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let opening = config.rows("x").unwrap();
        let draft = config
            .preview_setting(Some("x"), "board.hidden_columns", "[\"pr_link\"]")
            .unwrap();
        assert_eq!(draft.rows("x").unwrap().lines, opening.lines);
        assert_eq!(draft.rows("x").unwrap().columns, opening.columns);
        assert_eq!(draft.rows("x").unwrap().hidden_columns, ["pr_link"]);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        for value in [
            "[\"unknown\"]",
            "[\"state\",\"state\"]",
            "[\"member\",\"state\",\"task\",\"pr_link\"]",
        ] {
            let error = config
                .set_setting(Some("x"), "board.hidden_columns", value)
                .unwrap_err();
            assert!(error.message.contains("squad.x.board.hidden_columns"));
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
        assert!(
            config
                .set_setting(Some("x"), "bind.o", "run touch /never")
                .is_err()
        );
        assert!(
            config
                .set_setting(Some("x"), "board.refresh", "0s")
                .is_err()
        );
        config
            .set_setting(Some("x"), "board.refresh", "10s")
            .unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("# kept")
                && saved.contains("# retain")
                && saved.contains("opaque='keep me'")
        );
        config
            .set_setting(Some("x"), "board.hidden_columns", "[\"pr_link\"]")
            .unwrap();
        config
            .set_setting(Some("x"), "board.hidden_columns", "[]")
            .unwrap();
        assert_eq!(config.rows("x").unwrap(), opening);
        fs::write(&path, "# external editor\n").unwrap();
        assert_eq!(
            config
                .set_setting(Some("x"), "notes.render", "plain")
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "# external editor\n");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn me_is_written_without_disturbing_the_users_document() {
        let path = temp("me");
        let original = "# my board\n[squad.product]\nlayout = \"pr-queue\" # queue\n\n[bind]\no = \"open {pr_link}\"\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        assert_eq!(config.me().unwrap(), None);
        assert_eq!(config.layout("product").unwrap(), Layout::PrQueue);
        assert_eq!(config.layout("other").unwrap(), Layout::Team);
        config
            .set_me("ada", "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f")
            .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.contains(original),
            "user bytes are preserved: {written}"
        );
        let reread = Config::read(path.clone()).unwrap();
        assert_eq!(reread.me().unwrap(), Some("ada"));
        assert_eq!(
            reread.me_id().unwrap(),
            Some("7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f")
        );
        fs::write(&path, "me = \"Someone\"\n").unwrap();
        assert_eq!(
            config
                .set_me("ada", "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f")
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "me = \"Someone\"\n");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn clearing_me_removes_only_me_and_its_id() {
        let path = temp("clear-me");
        let user = "# my board\n[squad.product]\nlayout = \"pr-queue\" # queue\n";
        fs::write(
            &path,
            format!("me = \"ada\"\nme_id = \"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f\"\n{user}"),
        )
        .unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        config.clear_me().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), user);
        let reread = Config::read(path.clone()).unwrap();
        assert_eq!(
            (reread.me().unwrap(), reread.me_id().unwrap()),
            (None, None)
        );
        assert_eq!(reread.layout("product").unwrap(), Layout::PrQueue);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_documents_and_values_are_rejected_before_use() {
        let path = temp("invalid");
        assert_eq!(Config::read(path.clone()).unwrap().me().unwrap(), None);
        for text in [
            "me = 3\n",
            "me = \"\"\n",
            "me_id = \"not-a-uuid\"\n",
            "[squad\n",
            "squad = 1\n",
        ] {
            fs::write(&path, text).unwrap();
            let result = Config::read(path.clone()).and_then(|config| config.layout("x"));
            assert_eq!(result.unwrap_err().code, "SQUAD_CONFIG_INVALID", "{text}");
        }
        fs::write(&path, "[squad.x]\nlayout = \"kanban\"\n").unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(config.layout("x").unwrap_err().code, "SQUAD_CONFIG_INVALID");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn sections_are_optional_ordered_and_strictly_validated() {
        let path = temp("sections");
        fs::write(
            &path,
            r#"[squad.product]
layout = "crew"
[[squad.product.section]]
title = "Needs me"
filter = "pending or state = blocked"
[squad.product.section.bind]
enter = "reply"
[[squad.product.section]]
title = "Everyone"
sort = ["state", "-name"]
"#,
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert!(config.sections("other").unwrap().is_empty());
        let sections = config.sections("product").unwrap();
        assert_eq!(
            sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Needs me", "Everyone"]
        );
        assert!(sections[0].filter.is_some() && sections[1].filter.is_none());
        assert_eq!(sections[0].bind["enter"].verb, crate::action::Verb::Reply);
        assert!(sections[1].bind.is_empty());
        assert_eq!(
            sections[1].sort,
            [
                SortKey {
                    field: "state".into(),
                    descending: false
                },
                SortKey {
                    field: "name".into(),
                    descending: true
                }
            ]
        );
        for body in [
            "[squad.x]\nsection = 1\n",
            "[[squad.x.section]]\nfilter = \"a\"\n",
            "[[squad.x.section]]\ntitle = \"\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\nfilter = \"a and\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\nsort = [\"Bad\"]\n",
            "[[squad.x.section]]\ntitle = \"T\"\ncolour = \"red\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\n[squad.x.section.bind]\no = 3\n",
        ] {
            fs::write(&path, body).unwrap();
            let code = Config::read(path.clone())
                .and_then(|config| config.sections("x"))
                .err()
                .map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn programs_are_argv_arrays_that_never_need_a_shell() {
        let path = temp("programs");
        fs::write(
            &path,
            "opener = [\"firefox\", \"--new-tab\"]\nclipboard = [\"/usr/bin/xclip\", \"-selection\", \"clipboard\"]\n",
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(
            config.program("opener").unwrap().unwrap(),
            ["firefox", "--new-tab"]
        );
        assert_eq!(
            config.program("clipboard").unwrap().unwrap()[0],
            "/usr/bin/xclip"
        );
        fs::write(&path, "").unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(config.program("opener").unwrap(), None);
        for body in [
            "opener = \"open\"\n",
            "opener = []\n",
            "opener = [\"bin/open\"]\n",
            "opener = [\"open\", 1]\n",
            "opener = [\"\"]\n",
            "opener = [\"open\", \"a\\u001bb\"]\n",
        ] {
            fs::write(&path, body).unwrap();
            let error = Config::read(path.clone())
                .and_then(|config| config.program("opener"))
                .unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn tmux_keys_default_to_s_and_b_and_back_and_lead_are_opt_in() {
        let path = temp("tmux-keys");
        fs::write(&path, "").unwrap();
        let keys = Config::read(path.clone()).unwrap().tmux_keys().unwrap();
        assert_eq!(
            keys,
            TmuxKeys {
                popup: "S".into(),
                pane: "B".into(),
                back: None,
                lead: None,
            }
        );
        fs::write(
            &path,
            "[tmux]\npopup = \"C-s\"\npane = \"F5\"\nback = \"b\"\nlead = \"J\"\n",
        )
        .unwrap();
        let keys = Config::read(path.clone()).unwrap().tmux_keys().unwrap();
        assert_eq!((keys.popup.as_str(), keys.pane.as_str()), ("C-s", "F5"));
        assert_eq!(keys.back.as_deref(), Some("b"));
        assert_eq!(keys.lead.as_deref(), Some("J"));
        for body in [
            "[tmux]\npopup = \"\"\n",
            "[tmux]\npopup = \"SS\"\n",
            "[tmux]\npopup = \";\"\n",
            "[tmux]\npopup = \"'\"\n",
            "[tmux]\npopup = \"#\"\n",
            "[tmux]\npane = \"F13\"\n",
            "[tmux]\npane = \"C-\"\n",
            "[tmux]\npane = \"C-ab\"\n",
            "[tmux]\npane = \"S\"\n",
            "[tmux]\nback = \"B\"\n",
            "[tmux]\nlead = \"S\"\n",
            "[tmux]\nback = \"J\"\nlead = \"J\"\n",
            "[tmux]\nlead = \"#\"\n",
            "[tmux]\nhotkey = \"S\"\n",
            "tmux = \"S\"\n",
            "[tmux]\npopup = 1\n",
        ] {
            fs::write(&path, body).unwrap();
            let error = Config::read(path.clone())
                .and_then(|config| config.tmux_keys())
                .unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn bind_overrides_the_host_preset_and_rejects_bad_actions_at_load() {
        let path = temp("bind");
        fs::write(
            &path,
            "[bind]\nenter = \"open {pr_link}\"\nf5 = \"refresh\"\n",
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        for tmux in [true, false] {
            let bindings = config.bindings(tmux, &[]).unwrap();
            assert_eq!(bindings["enter"].verb, crate::action::Verb::Open);
            assert_eq!(bindings["f5"].verb, crate::action::Verb::Refresh);
            assert_eq!(bindings["y"].verb, crate::action::Verb::Copy, "preset kept");
        }
        fs::write(&path, "").unwrap();
        let config = Config::read(path.clone()).unwrap();
        for tmux in [true, false] {
            assert!(!config.bindings(tmux, &[]).unwrap().contains_key("f5"));
            assert_eq!(
                config.bindings(tmux, &[]).unwrap()["ctrl-r"].verb,
                crate::action::Verb::Refresh
            );
        }
        fs::write(&path, "[bind]\nf5 = \"copy\"\n").unwrap();
        let rebound = Config::read(path.clone()).unwrap();
        for tmux in [true, false] {
            assert_eq!(
                rebound.bindings(tmux, &[]).unwrap()["f5"].verb,
                crate::action::Verb::Copy
            );
        }
        assert_eq!(
            config.bindings(false, &[]).unwrap()["double-click"].verb,
            crate::action::Verb::Menu
        );
        for body in [
            "[bind]\nq = \"refresh\"\n",
            "[bind]\nhold = \"refresh\"\n",
            "[bind]\no = \"launch\"\n",
            "[bind]\no = \"run ./script {name}\"\n",
            "[bind]\no = \"open {pr link}\"\n",
            "[bind]\no = 1\n",
            "bind = \"o\"\n",
        ] {
            fs::write(&path, body).unwrap();
            let error = Config::read(path.clone())
                .and_then(|config| config.bindings(true, &[]))
                .unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn configured_group_toggle_preserves_overrides_and_rejects_duplicate_or_unknown_panes() {
        let path = temp("toggle-group");
        let panes = [Pane::Detail, Pane::Replies];
        for line in ["toggle detail replies", "toggle notes"] {
            fs::write(&path, format!("[bind]\nd = \"{line}\"\n")).unwrap();
            let config = Config::read(path.clone()).unwrap();
            for tmux in [false, true] {
                assert_eq!(config.bindings(tmux, &panes).unwrap()["d"].text, line);
            }
        }
        for line in [
            "toggle",
            "toggle detail detail",
            "toggle detail unknown",
            "toggle {pane}",
        ] {
            fs::write(&path, format!("[bind]\nd = \"{line}\"\n")).unwrap();
            let error = Config::read(path.clone())
                .unwrap()
                .bindings(true, &panes)
                .unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(error.message.contains("bind.d"));
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_board_theme_layers_the_squad_over_the_global_one() {
        use tmt_cli_style::{Base, Role};
        let path = temp("theme");
        fs::write(
            &path,
            "[squad.product.theme]\nwaiting = \"#010203\"\n[squad.bad.theme]\nwaiting = \"orange\"\n\
             [squad.odd]\ntheme = \"mono\"\n",
        )
        .unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        config.global_theme(&serde_json::json!({
            "resolved": {"theme": {"base": "mono", "accent": "blue"}},
            "themeError": null
        }));
        let (theme, notice) = config.theme("product").unwrap();
        assert_eq!((theme.base, notice), (Base::Mono, None));
        let (other, _) = config.theme("other").unwrap();
        assert_ne!(
            theme.style(Role::Waiting, tmt_cli_style::Depth::TrueColor),
            other.style(Role::Waiting, tmt_cli_style::Depth::TrueColor),
            "the squad's override applies to its own board only"
        );
        let error = |config: &Config, squad| config.theme(squad).unwrap_err().to_string();
        assert!(
            error(&config, "bad").contains("`squad.bad.theme.waiting`"),
            "{}",
            error(&config, "bad")
        );
        assert!(error(&config, "odd").contains("`squad.odd.theme` must be a table"));

        // A broken global theme is TMT's config, not this file's: the board
        // draws with the default theme and says why.
        config.global_theme(&serde_json::json!({
            "resolved": {"theme": {}},
            "themeError": {"key": "theme.base", "message": "unknown base dark."}
        }));
        let (theme, notice) = config.theme("product").unwrap();
        assert_eq!(theme.base, Base::Auto);
        assert_eq!(
            notice.as_deref(),
            Some("theme.base unknown base dark; the board ignores the invalid CLI theme")
        );
        // The same when only the theme's meaning is wrong and core reported
        // no themeError: the global layer fails, the squad's still applies.
        config.global_theme(&serde_json::json!({
            "resolved": {"theme": {"base": "dark"}},
            "themeError": null
        }));
        let (fallback, notice) = config.theme("product").unwrap();
        assert_eq!(fallback.base, Base::Auto);
        assert_eq!(
            fallback.style(Role::Waiting, tmt_cli_style::Depth::TrueColor),
            theme.style(Role::Waiting, tmt_cli_style::Depth::TrueColor),
            "the squad's override survives a bad global theme"
        );
        assert!(
            notice
                .as_deref()
                .is_some_and(|notice| notice.starts_with("`theme.base`")
                    && notice.ends_with("; the board ignores the invalid CLI theme")),
            "{notice:?}"
        );
        // A bad squad theme stays this file's error under either global one.
        assert!(error(&config, "bad").contains("`squad.bad.theme.waiting`"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn columns_and_state_colors_default_by_layout_and_override_strictly() {
        let path = temp("board");
        fs::write(
            &path,
            "[squad.product.columns]\nshow = [\"member\", \"state\", \"note\"]\nnote = { title = \"WHY\", width = 30 }\n\
             [squad.product.states]\nblocked = { color = \"red\" }\nparked = { color = \"dim\" }\n",
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        let defaults = config.rows("other").unwrap().columns;
        assert_eq!(
            defaults
                .iter()
                .map(|c| c.field.as_str())
                .collect::<Vec<_>>(),
            ["member", "state", "task", "pr", "model"]
        );
        let columns = config.rows("product").unwrap().columns;
        assert_eq!(
            (columns[2].field.as_str(), columns[2].title.as_str()),
            ("note", "WHY")
        );
        assert_eq!(
            (columns[2].width, columns[2].grow),
            (Some(tmt_cli_style::grid::Basis::Cells(30)), 0)
        );
        assert_eq!(columns[0].title, "MEMBER");
        let states = config.states("product", Layout::Crew).unwrap();
        assert_eq!(states.color(Some("blocked")).unwrap(), "red");
        assert_eq!(states.color(Some("parked")).unwrap(), "dim");
        assert_eq!(
            states.color(Some("working")).unwrap(),
            "working",
            "layout defaults remain"
        );
        assert!(
            config
                .states("other", Layout::Minimal)
                .unwrap()
                .color(Some("working"))
                .is_none()
        );
        for body in [
            "[squad.x.columns]\nshow = []\n",
            "[squad.x.columns]\nshow = [\"Bad\"]\n",
            "[squad.x.columns]\ntask = { width = 5 }\nshow = [\"member\"]\n",
            "[squad.x.columns]\nmember = { width = 0 }\n",
            "[squad.x.columns]\nmember = { align = \"left\" }\n",
            "[squad.x.states]\nworking = { color = \"teal\" }\n",
            "[squad.x.states]\nworking = { sort = 1000 }\n",
            "[squad.x.states]\nworking = { sort = \"first\" }\n",
            "[squad.x.states]\nworking = { order = 1 }\n",
        ] {
            fs::write(&path, body).unwrap();
            let config = Config::read(path.clone()).unwrap();
            let code = config
                .rows("x")
                .and_then(|_| config.states("x", Layout::Crew))
                .err()
                .map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn refresh_preserves_accepted_numbers_units_and_off() {
        for (text, seconds) in [
            ("1s", Some(1)),
            ("+5s", Some(5)),
            ("005s", Some(5)),
            ("5m", Some(300)),
            ("+5m", Some(300)),
            ("60m", Some(3600)),
            ("3600s", Some(3600)),
            ("off", None),
        ] {
            assert_eq!(
                refresh(&value(text), "board.refresh").unwrap(),
                seconds.map(Duration::from_secs),
                "{text}"
            );
        }
    }

    #[test]
    fn refresh_non_ascii_duration_reports_the_setting_without_panicking() {
        // Multi-byte suffixes reproduce the old byte-index split panic.
        for place in ["board.refresh", "squad.x.board.refresh"] {
            for text in ["5分", "5秒"] {
                let error = refresh(&value(text), place).unwrap_err();
                assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
                assert!(error.message.contains(place), "{error}");
            }
        }
    }

    #[test]
    fn refresh_is_per_squad_then_global_then_five_seconds() {
        let path = temp("refresh");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap()
        };
        let secs = |seconds| Ok(Some(Duration::from_secs(seconds)));
        assert_eq!(read("").refresh("x"), secs(5));
        let config = read("[board]\nrefresh = \"2m\"\n[squad.x.board]\nrefresh = \"2s\"\n");
        assert_eq!(config.refresh("x"), secs(2));
        assert_eq!(config.refresh("y"), secs(120), "the global value");
        assert_eq!(read("[board]\nrefresh = \"off\"\n").refresh("x"), Ok(None));
        assert_eq!(
            read("[board]\nrefresh = \"60m\"\n").refresh("x"),
            secs(3600)
        );
        assert_eq!(
            read("[squad.x]\nlayout = \"crew\"\n[squad.x.board]\nrefresh = \"1s\"\n")
                .board("x")
                .map(|board| board.panes),
            Ok(vec![Pane::Rows, Pane::Notes]),
            "refresh is a board setting beside the panes"
        );
        for body in [
            "[board]\nrefresh = \"0s\"\n",
            "[board]\nrefresh = \"61m\"\n",
            "[board]\nrefresh = 5\n",
            "[board]\nrefresh = \"5\"\n",
            "[board]\nrefresh = \"5h\"\n",
            "[board]\nrefresh = \"1h\"\n",
            "[board]\nrefresh = \"fast\"\n",
            "[board]\npanes = [\"rows\"]\n",
            "board = 5\n",
            "[squad.x.board]\nrefresh = \"500ms\"\n",
        ] {
            fs::write(&path, body).unwrap();
            let code = Config::read(path.clone())
                .and_then(|config| config.refresh("x"))
                .err()
                .map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn team_is_default_and_existing_presets_keep_their_overrides() {
        let path = temp("team");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap()
        };
        let default = read("");
        assert_eq!(default.layout("x").unwrap(), Layout::Team);
        let previous_rows = read("[squad.x]\nlayout = \"crew\"\n").rows("x").unwrap();
        for layout in ["crew", "pr-queue", "minimal"] {
            let config = read(&format!("[squad.x]\nlayout = \"{layout}\"\n"));
            assert_eq!(config.rows("x").unwrap(), previous_rows);
            assert!(config.providers("x").unwrap().is_empty());
            assert_eq!(config.reminders("x").unwrap(), Reminders::default());
        }
        for layout in [Layout::Crew, Layout::PrQueue, Layout::Minimal] {
            let expected = Board::preset(layout);
            for body in ["[squad.x.board]\n", "[squad.x.board]\nrefresh = \"10s\"\n"] {
                assert_eq!(
                    read(&format!(
                        "[squad.x]\nlayout = {:?}\n{body}",
                        layout.as_str()
                    ))
                    .board("x")
                    .unwrap(),
                    expected
                );
            }
        }
        let config = read("[squad.x]\nlayout = \"team\"\n");
        let layout = config.layout("x").unwrap();
        assert_eq!(layout, Layout::Team);
        assert_eq!(layout.states(), Layout::Crew.states());
        assert_eq!(
            config.states("x", layout).unwrap(),
            default.states("x", Layout::Crew).unwrap()
        );
        assert!(layout.pending_first());
        let rows = config.rows("x").unwrap();
        assert_eq!(
            rows.fields(),
            ["member", "state", "task", "pr", "model", "pending"]
        );
        assert_eq!(rows.columns[4].from.as_ref().unwrap().path, "session.model");
        assert!(
            !rows.reads_metadata(),
            "model uses the existing presence projection"
        );
        assert_eq!(rows.lines[1][2].field.as_deref(), Some("pending"));
        assert_eq!(rows.lines[1][2].span, 3);
        assert_eq!(rows.lines[1][2].token, Some(tmt_cli_style::Role::Waiting));
        assert_eq!(config.providers("x").unwrap()[0].name, "pr");
        assert_eq!(
            config.providers("x").unwrap()[0].every(),
            Duration::from_secs(60)
        );
        assert_eq!(
            config.reminders("x").unwrap(),
            Reminders {
                enabled: true,
                ..Reminders::default()
            }
        );
        let preset = config.board("x").unwrap();
        assert_eq!(
            preset.panes,
            [Pane::Rows, Pane::Detail, Pane::Replies, Pane::Notes]
        );
        assert_eq!(
            preset.split,
            crate::split::read(
                crate::view::ViewName::Team
                    .settings()
                    .get("layout")
                    .unwrap(),
                "team"
            )
            .unwrap()
        );
        let refresh = read("[squad.x]\nlayout = \"team\"\n[squad.x.board]\nrefresh = \"10s\"\n");
        assert_eq!(refresh.board("x").unwrap(), preset);
        for setting in ["direction = \"left-right\"", "sizes = [50, 50]"] {
            let partial = read(&format!(
                "[squad.x]\nlayout = \"team\"\n[squad.x.board]\n{setting}\n"
            ));
            let error = partial.board("x").unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(error.message.contains("squad.x.board"));
            assert!(
                error
                    .message
                    .contains("partially override the resolved arrangement")
            );
        }
        let added = read(
            "[squad.x]\nlayout = \"team\"\n[squad.x.fields.ci]\nrun = [\"echo\", \"ready\"]\n",
        );
        let added_providers = added.providers("x").unwrap();
        let names: std::collections::BTreeSet<_> = added_providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect();
        assert_eq!(names, std::collections::BTreeSet::from(["ci", "pr"]));
        let overrides = read(
            r#"
[squad.x]
layout = "team"
[squad.x.rows]
columns = [{ name = "member" }, { name = "pr", from = "fields.pr" }]
[squad.x.fields.pr]
run = ["echo", "custom"]
every = "2m"
[squad.x.reminders]
enabled = false
[squad.x.board]
panes = ["rows", "notes"]
"#,
        );
        assert_eq!(overrides.rows("x").unwrap().fields(), ["member", "pr"]);
        assert_eq!(
            overrides.providers("x").unwrap()[0].every(),
            Duration::from_secs(120)
        );
        assert!(!overrides.reminders("x").unwrap().enabled);
        assert_eq!(
            overrides.board("x").unwrap().panes,
            [Pane::Rows, Pane::Notes]
        );
        let tabs = read("[squad.x]\nlayout = \"team\"\n[squad.x.board]\nmode = \"tabs\"\n");
        assert_eq!(tabs.board("x").unwrap().mode, BoardMode::Tabs);
        let legacy = read("[squad.x]\nlayout = \"team\"\n[squad.x.columns]\nshow = [\"member\"]\n");
        assert_eq!(legacy.rows("x").unwrap().fields(), ["member"]);
        let invalid = read("[squad.x]\nlayout = \"team\"\nreminders = false\n");
        assert_eq!(
            invalid.reminders("x").unwrap_err().code,
            "SQUAD_CONFIG_INVALID"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn board_presets_overrides_and_validation() {
        let path = temp("panes");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap()
        };
        let crew = read("[squad.x]\nlayout = \"crew\"\n").board("x").unwrap();
        assert_eq!(
            crew.split,
            Split::simple(Direction::LeftRight, &[Pane::Rows, Pane::Notes], &[60, 40])
        );
        let queue = read("[squad.x]\nlayout = \"pr-queue\"\n")
            .board("x")
            .unwrap();
        assert_eq!(
            queue.split,
            Split::simple(Direction::TopBottom, &[Pane::Rows, Pane::Detail], &[70, 30])
        );
        assert_eq!(
            read("[squad.x]\nlayout = \"minimal\"\n")
                .board("x")
                .unwrap()
                .panes,
            [Pane::Rows]
        );

        let custom = read("[squad.x.board]\ndirection = \"top-bottom\"\npanes = [\"detail\", \"rows\", \"replies\"]\n")
            .board("x")
            .unwrap();
        assert_eq!(custom.panes, [Pane::Detail, Pane::Rows, Pane::Replies]);
        assert_eq!(
            custom.split,
            Split::simple(
                Direction::TopBottom,
                &[Pane::Detail, Pane::Rows, Pane::Replies],
                &[33, 33, 34]
            ),
            "unsized panes share the space"
        );
        // The handbook's nested layout: rows beside detail over notes.
        let nested = read(
            "[squad.x.board]\nlayout = { direction = \"left-right\", sizes = [60, 40], panes = [\n  \"rows\",\n  { direction = \"top-bottom\", sizes = [40, 60], panes = [\"detail\", \"notes\"] },\n] }\n",
        )
        .board("x")
        .unwrap();
        assert_eq!(nested.panes, [Pane::Rows, Pane::Detail, Pane::Notes]);
        assert_eq!(
            nested.split,
            Split::Group {
                direction: Direction::LeftRight,
                children: vec![
                    (crate::split::Size::Percent(60), Split::Pane(Pane::Rows)),
                    (
                        crate::split::Size::Percent(40),
                        Split::simple(
                            Direction::TopBottom,
                            &[Pane::Detail, Pane::Notes],
                            &[40, 60]
                        )
                    ),
                ],
            }
        );
        let tabs = read("[squad.x.board]\nmode = \"tabs\"\npanes = [\"rows\", \"detail\"]\n")
            .board("x")
            .unwrap();
        assert_eq!(
            tabs.panes,
            [Pane::Rows, Pane::Detail, Pane::Notes],
            "notes always get a tab"
        );

        for body in [
            "[squad.x.board]\nmode = \"grid\"\n",
            "[squad.x.board]\ndirection = \"diagonal\"\n",
            "[squad.x.board]\npanes = [\"notes\"]\n",
            "[squad.x.board]\npanes = [\"rows\", \"rows\"]\n",
            "[squad.x.board]\npanes = [\"rows\", \"chat\"]\n",
            "[squad.x.board]\nsizes = [50, 40]\n",
            "[squad.x.board]\nsizes = [95, 5]\n",
            "[squad.x.board]\nsizes = [100]\n",
            "[squad.x.board]\nmode = \"tabs\"\nsizes = [60, 40]\n",
            "[squad.x.board]\ncolumns = 2\n",
            "[squad.x.board]\ndirection = \"left-right\"\nlayout = { direction = \"left-right\", panes = [\"rows\"] }\n",
            "[squad.x.board]\nmode = \"tabs\"\nlayout = { direction = \"left-right\", panes = [\"rows\"] }\n",
        ] {
            let code = read(body).board("x").err().map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn state_sort_overrides_reorder_known_and_new_states() {
        let path = temp("order");
        fs::write(
            &path,
            "[squad.x.states]\nblocked = { color = \"amber\", sort = 0 }\nparked = { sort = 3 }\n",
        )
        .unwrap();
        let states = Config::read(path.clone())
            .unwrap()
            .states("x", Layout::Crew)
            .unwrap();
        let mut names = [
            "working", "idle", "blocked", "review", "testing", "hold", "parked",
        ];
        names.sort_by_key(|state| (states.rank(Some(state)), *state));
        assert_eq!(
            names,
            [
                "blocked", "working", "idle", "parked", "review", "testing", "hold"
            ]
        );
        assert!(states.rank(Some("blocked")) < states.rank(Some("working")));
        assert!(states.rank(Some("unknown")) > states.rank(Some("hold")));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn notes_render_defaults_to_markdown_and_accepts_plain() {
        let path = temp("notes");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap().notes_render("x")
        };
        assert_eq!(read("").unwrap(), NotesRender::Markdown);
        assert_eq!(
            read("[squad.x.notes]\nrender = \"plain\"\n").unwrap(),
            NotesRender::Plain
        );
        for body in [
            "[squad.x.notes]\nrender = \"html\"\n",
            "[squad.x.notes]\nwrap = false\n",
            "[squad.x]\nnotes = \"plain\"\n",
        ] {
            assert_eq!(
                read(body).err().map(|e| e.code).as_deref(),
                Some("SQUAD_CONFIG_INVALID"),
                "{body}"
            );
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn saving_the_tab_order_keeps_the_rest_of_the_file_and_reads_back() {
        let path = temp("tab-order");
        let original = "# my board\n[tabs] # arranged by hand\nhide = [\"quiet\"]\n\n[bind]\no = \"open {pr_link}\"\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let keys: Vec<String> = [crate::board::ALL, "product", "leads", crate::board::LEADS]
            .map(String::from)
            .to_vec();
        config.set_tab_order(&keys).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "# my board\n[tabs] # arranged by hand\nhide = [\"quiet\"]\norder = [\"all\", \"product\", \"squad:leads\", \"leads\"]\n\n[bind]\no = \"open {pr_link}\"\n"
        );
        assert_eq!(
            Config::read(path.clone())
                .and_then(|config| config.tabs())
                .unwrap()
                .order,
            keys
        );

        // A file with no [tabs] gains one; an edit made meanwhile is kept.
        fs::write(&path, "me = \"Ben\"\n").unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        fs::write(&path, "me = \"Ben\"\n# edited meanwhile\n").unwrap();
        assert_eq!(
            config.set_tab_order(&keys).unwrap_err().code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("edited meanwhile")
        );
        let mut config = Config::read(path.clone()).unwrap();
        config.set_tab_order(&keys[..1]).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        // The file's trailing comment stays last, after the new table.
        assert_eq!(
            written,
            "me = \"Ben\"\n\n[tabs]\norder = [\"all\"]\n# edited meanwhile\n"
        );
        fs::write(&path, "tabs = { order = [] }\n").unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        assert_eq!(
            config.set_tab_order(&keys).unwrap_err().code,
            "SQUAD_CONFIG_INVALID"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn tabs_read_order_hide_colors_and_the_leads_bindings() {
        let path = temp("tabs");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).and_then(|config| config.tabs())
        };
        assert_eq!(read("").unwrap(), Tabs::default());
        let tabs = read(
            "[tabs]\norder = [\"leads\", \"infra\", \"squad:leads\", \"all\"]\nhide = [\"quiet\"]\npin = [\"all\"]\n\
             [tabs.colors]\nblocked = \"magenta\"\n[tabs.leads.bind]\nenter = \"run herdr agent focus {pane}\"\n",
        )
        .unwrap();
        // `squad:leads` is the squad named leads, not the built-in tab.
        assert_eq!(tabs.order, ["@leads", "infra", "leads", "@all"]);
        assert_eq!(tabs.hide, ["quiet"]);
        assert_eq!(tabs.pin, ["@all"]);
        assert_eq!(
            tabs.colors,
            TabColors {
                waiting: "waiting".into(),
                blocked: "magenta".into()
            }
        );
        assert_eq!(tabs.leads["enter"].verb, crate::action::Verb::Run);
        for body in [
            "tabs = 1\n",
            "[tabs]\nsort = []\n",
            "[tabs]\norder = \"leads\"\n",
            "[tabs]\norder = [1]\n",
            "[tabs]\norder = [\"Infra\"]\n",
            "[tabs]\norder = [\"everyone\", \"squad:x y\"]\n",
            "[tabs.all]\nbind = 1\n",
            "[tabs]\nhide = [\"infra\", \"infra\"]\n",
            "[tabs]\npin = \"infra\"\n",
            "[tabs.colors]\nwaiting = \"pink\"\n",
            "[tabs.colors]\nnormal = \"dim\"\n",
            "[tabs]\ncolors = \"amber\"\n",
            "[tabs.leads]\nrows = 1\n",
            "[tabs.leads.bind]\nenter = \"launch\"\n",
        ] {
            assert_eq!(
                read(body).err().map(|e| e.code).as_deref(),
                Some("SQUAD_CONFIG_INVALID"),
                "{body}"
            );
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn home_defaults_respect_explicit_empty_order_pin_and_hide() {
        let path = temp("home-defaults");
        let squads = ["product".into(), "infra".into()];
        for (body, expected, pinned) in [
            ("", vec!["@all", "@leads", "product", "infra"], 1),
            (
                "[tabs]\nhide = [\"all\"]",
                vec!["@leads", "product", "infra"],
                0,
            ),
            (
                "[tabs]\norder = []",
                vec!["product", "infra", "@leads", "@all"],
                0,
            ),
            (
                "[tabs]\npin = []",
                vec!["product", "infra", "@leads", "@all"],
                0,
            ),
            (
                "[tabs]\norder = [\"infra\", \"all\"]",
                vec!["infra", "@all", "product", "@leads"],
                0,
            ),
            (
                "[tabs]\npin = [\"infra\"]",
                vec!["infra", "product", "@leads", "@all"],
                1,
            ),
        ] {
            fs::write(&path, body).unwrap();
            let config = Config::read(path.clone()).unwrap();
            assert_eq!(
                crate::tabs::arrange(&squads, &config.tabs().unwrap()),
                (expected.into_iter().map(String::from).collect(), pinned),
                "{body}"
            );
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                body,
                "reading never writes"
            );
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn user_tabs_validate_eagerly_and_keep_distinct_squad_keys() {
        let path = temp("user-tabs");
        let body = r#"[tabs]
order = ["tab:needs-me", "needs-me", "all"]
pin = ["tab:needs-me"]
hide = ["tab:quiet"]
[tabs.needs-me]
filter = "pending or waiting_on_you"
sort = ["squad", "-name"]
[tabs.needs-me.bind]
enter = "jump"
[[tabs.needs-me.section]]
title = "Blocked"
filter = "state = blocked"
[tabs.needs-me.section.bind]
o = "tab"
[tabs.quiet]
filter = "not pending"
"#;
        fs::write(&path, body).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        let tabs = config.tabs().unwrap();
        assert_eq!(tabs.user[0].name, "needs-me");
        assert_eq!(
            tabs.user[0].sections[0].bind["o"].verb,
            crate::action::Verb::Tab
        );
        let (keys, pinned) = crate::tabs::arrange(&["needs-me".into()], &tabs);
        assert_eq!(keys, ["@tab:needs-me", "needs-me", "@all", "@leads"]);
        assert_eq!(pinned, 1);
        config.set_tab_order(&keys).unwrap();
        assert_eq!(
            Config::read(path.clone()).unwrap().tabs().unwrap().order,
            keys
        );
        for (body, place) in [
            (
                "[tabs.hidden]\nfilter = 'pending and'",
                "tabs.hidden.filter",
            ),
            ("[tabs.hidden]\nsort = [1]", "tabs.hidden.sort"),
            ("[tabs.hidden]\nrows = {}", "tabs.hidden.rows"),
            ("[tabs.hidden.bind]\nx = 'launch'", "tabs.hidden.bind"),
            (
                "[[tabs.hidden.section]]\ntitle = 'X'\nfilter = '('",
                "tabs.hidden.section[0].filter",
            ),
            ("[tabs]\nhide = ['tab:missing']", "tabs.hide"),
            ("[tabs.leads]\nfilter = 'pending'", "tabs.leads.filter"),
            ("[tabs.Bad]", "tabs.Bad"),
        ] {
            fs::write(&path, body).unwrap();
            let error = Config::read(path.clone()).err().unwrap();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(error.message.contains(place), "{error:?}");
        }
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn implicit_team_preserves_simple_board_configs_and_explicit_team_is_strict() {
        let path = temp("layout-resolution");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap()
        };
        let team = read("");
        assert_eq!(team.layout("x").unwrap(), Layout::Team);
        assert_eq!(team.board("x").unwrap(), Board::preset(Layout::Team));
        for key in [
            "direction = \"top-bottom\"",
            "panes = [\"rows\", \"notes\"]",
            "sizes = [60, 40]",
        ] {
            let body = format!("[squad.x.board]\n{key}\n");
            let implicit = read(&body);
            assert_eq!(implicit.layout("x").unwrap(), Layout::Crew);
            let board = implicit.board("x").unwrap();
            assert!(board.fold_below.is_none());
            let explicit = read(&format!("[squad.x]\nlayout = \"crew\"\n{body}"));
            assert_eq!(board, explicit.board("x").unwrap());
            assert_eq!(implicit.rows("x").unwrap(), explicit.rows("x").unwrap());
            assert_eq!(
                implicit.reminders("x").unwrap(),
                explicit.reminders("x").unwrap()
            );
        }
        for key in ["direction = \"top-bottom\"", "sizes = [60, 40]"] {
            let explicit = read(&format!(
                "[squad.x]\nlayout = \"team\"\n[squad.x.board]\n{key}\n"
            ));
            assert_eq!(explicit.layout("x").unwrap(), Layout::Team);
            assert!(
                explicit
                    .board("x")
                    .unwrap_err()
                    .message
                    .contains("partially override the resolved arrangement")
            );
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn fold_below_validates_width_panes_and_mode_with_placed_errors() {
        let path = temp("fold-below");
        for (value, suffix) in [
            ("{ width = 0, panes = [] }", "width"),
            ("{ width = 1001, panes = [] }", "width"),
            ("{ width = 100.5, panes = [] }", "width"),
            ("{ panes = [] }", "width"),
            ("{ width = 100 }", "panes"),
            ("{ width = 100, panes = [\"missing\"] }", "panes[0]"),
            ("{ width = 100, panes = [\"notes\"] }", "panes[0]"),
            ("{ width = 100, panes = [\"detail\", \"detail\"] }", "panes"),
            ("{ width = 100, panes = [], extra = true }", "extra"),
        ] {
            fs::write(
                &path,
                format!("[squad.x.board]\npanes = [\"rows\", \"detail\"]\nfold_below = {value}\n"),
            )
            .unwrap();
            let error = Config::read(path.clone()).unwrap().board("x").unwrap_err();
            assert!(
                error
                    .message
                    .contains(&format!("squad.x.board.fold_below.{suffix}")),
                "{error}"
            );
        }
        fs::write(
            &path,
            "[squad.x.board]\nmode = \"tabs\"\nfold_below = { width = 100, panes = [] }\n",
        )
        .unwrap();
        assert!(
            Config::read(path.clone())
                .unwrap()
                .board("x")
                .unwrap_err()
                .message
                .contains("fold_below")
        );
        fs::write(&path, "[squad.x.board]\nfold_below = { width = 80, panes = [\"detail\"] }\ncollapsed = [\"notes\"]\n").unwrap();
        let board = Config::read(path.clone()).unwrap().board("x").unwrap();
        assert_eq!(
            board.fold_below.unwrap(),
            FoldBelow {
                width: 80,
                panes: [Pane::Detail].into()
            }
        );
        assert_eq!(board.collapsed, [Pane::Notes].into());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn collapsed_is_strict_and_validates_the_resolved_board_before_use() {
        let path = temp("collapsed");
        for layout in [
            "panes = [\"rows\", \"detail\"]\n",
            "layout = { direction = \"left-right\", panes = [\"rows\", { direction = \"top-bottom\", panes = [\"detail\", \"notes\"] }] }\n",
        ] {
            fs::write(
                &path,
                format!("[squad.x.board]\n{layout}collapsed = [\"detail\"]\n"),
            )
            .unwrap();
            let board = Config::read(path.clone()).unwrap().board("x").unwrap();
            assert_eq!(
                board.collapsed,
                std::collections::BTreeSet::from([Pane::Detail])
            );
        }
        for value in [
            "3",
            "\"detail\"",
            "[1]",
            "[\"other\"]",
            "[\"detail\",\"detail\"]",
            "[\"notes\"]",
        ] {
            let bytes =
                format!("[squad.x.board]\npanes = [\"rows\",\"detail\"]\ncollapsed = {value}\n");
            fs::write(&path, &bytes).unwrap();
            let error = Config::read(path.clone()).unwrap().board("x").unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID");
            assert!(
                error.message.contains("squad.x.board.collapsed"),
                "{error:?}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
        }
        fs::write(&path, "[squad.x.board]\nmode = \"tabs\"\ncollapsed = []\n").unwrap();
        assert!(
            Config::read(path.clone())
                .unwrap()
                .board("x")
                .unwrap_err()
                .message
                .contains("squad.x.board.collapsed")
        );
        fs::write(&path, "[squad.x.board]\ncollapsed = []\n").unwrap();
        assert!(
            Config::read(path.clone())
                .unwrap()
                .board("x")
                .unwrap()
                .collapsed
                .is_empty()
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn token_rate_defaults_and_layered_overrides_are_strict() {
        let read = |text: &str, name: &str| {
            let path = temp("token-rate");
            fs::write(&path, text).unwrap();
            let config = Config::read(path.clone()).unwrap();
            fs::remove_file(path).unwrap();
            config.token_rate(name)
        };
        assert!(read("", "p").unwrap().enabled);
        for layout in ["crew", "minimal", "pr-queue"] {
            assert!(
                !read(&format!("[squad.p]\nlayout='{layout}'\n"), "p")
                    .unwrap()
                    .enabled
            );
        }
        let config=read("[board.token_rate]\nenabled=false\nevery='10s'\nreduced_motion=true\n[squad.p.board.token_rate]\nenabled=true\nevery='5s'\n","p").unwrap();
        assert!(config.enabled && config.reduced_motion);
        assert_eq!(config.every, Duration::from_secs(5));
        assert_eq!(
            read("[board.token_rate]\nwindow='5s'\nevery='10s'", "p")
                .unwrap()
                .window,
            TokenWindow::Minute
        );
        assert_eq!(
            read("[board.token_rate]\nwindow='30m'", "p")
                .unwrap()
                .window,
            TokenWindow::HalfHour
        );
        for value in ["off", "4s", "11s"] {
            assert!(read(&format!("[board.token_rate]\nevery='{value}'"), "p").is_err());
        }
        for setting in [
            "enabled=1",
            "reduced_motion='yes'",
            "window='2m'",
            "every=5",
        ] {
            assert!(read(&format!("[squad.p.board.token_rate]\n{setting}"), "p").is_err());
        }
    }
}
