//! `squad.toml`: the user's file, beside TMT's global configuration. Squad reads
//! it and writes only `me`, preserving every other byte of the document.

use crate::core::{Core, SquadError};
use crate::{
    action::{Bindings, parse_bindings, preset},
    filter::{Filter, Row},
};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

const FILE_LIMIT: u64 = 1024 * 1024;
const MAX_SECTIONS: usize = 16;

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
}

impl Layout {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "crew" => Some(Self::Crew),
            "pr-queue" => Some(Self::PrQueue),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::PrQueue => "pr-queue",
            Self::Minimal => "minimal",
        }
    }

    /// Ordered state vocabulary; `add` starts members in the first state.
    pub fn states(self) -> &'static [&'static str] {
        match self {
            Self::Crew => &["working", "idle", "blocked", "review", "testing", "hold"],
            Self::PrQueue => &["preparing", "ready", "sent", "merged"],
            Self::Minimal => &[],
        }
    }

    /// Default state colors; `squad.<name>.states` overrides them.
    fn state_colors(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Crew => &[
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

    /// Crew sorts rows that owe the user a decision (`pending`) first.
    pub fn pending_first(self) -> bool {
        self == Self::Crew
    }
}

/// `[tabs]`: see [`Config::tabs`]. Entries are tab keys: a squad name, or
/// [`crate::board::LEADS`] for the built-in tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

/// `[squad.<name>.board]`: which panes the board shows and how they sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub mode: BoardMode,
    /// Every pane in focus order (split) or tab order (tabs).
    pub panes: Vec<Pane>,
    /// Split mode: how the panes sit, possibly nested.
    pub split: crate::split::Split,
}

impl Board {
    /// Crew keeps rows and the lead's notes side by side; pr-queue pairs rows
    /// with the selected row's detail; minimal shows rows only.
    fn preset(layout: Layout) -> Self {
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
        };
        Self::simple(BoardMode::Split, direction, panes, &sizes)
    }

    /// The one-level form: `panes` side by side or stacked at `sizes`.
    pub fn simple(mode: BoardMode, direction: Direction, panes: Vec<Pane>, sizes: &[u16]) -> Self {
        Self {
            mode,
            split: crate::split::Split::simple(direction, &panes, sizes),
            panes,
        }
    }
}

/// How the notes pane shows the lead's notebook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesRender {
    Markdown,
    Plain,
}

/// A squad's state vocabulary after overrides: display order and colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct States {
    /// Known states in sort order; unknown states sort after all of them.
    pub order: Vec<String>,
    pub colors: BTreeMap<String, String>,
}

impl States {
    pub fn rank(&self, state: Option<&str>) -> usize {
        state
            .and_then(|state| self.order.iter().position(|known| known == state))
            .unwrap_or(self.order.len())
    }
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

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

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
        Ok(config)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The host preset's bindings, overridden by top-level `[bind]`.
    pub fn bindings(&self, tmux: bool) -> Result<Bindings, SquadError> {
        let mut bindings = preset(tmux);
        if let Some(item) = self.document.get("bind") {
            bindings.extend(bindings_table(item, "bind")?);
        }
        Ok(bindings)
    }

    /// Top-level `opener` and `clipboard`: programs that replace the system
    /// opener and the built-in clipboard route.
    pub fn program(&self, key: &str) -> Result<Option<Vec<String>>, SquadError> {
        self.document
            .get(key)
            .map(|item| program(item, key))
            .transpose()
    }

    /// `[tmux]`: the prefix keys that open the board as a popup (default `S`)
    /// or a pane (default `B`), and an optional key for `tmt squad back`.
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
            .find(|key| !["popup", "pane", "back"].contains(key))
        {
            return Err(invalid(format!(
                "`tmux.{unknown}` is not a setting; use popup, pane or back."
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
        };
        let mut chosen = vec![&keys.popup, &keys.pane];
        chosen.extend(keys.back.as_ref());
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

    /// `[squad.<name>.reminders]`, off by default. No global enable switch.
    pub fn reminders(&self, squad: &str) -> Result<Reminders, SquadError> {
        let place = format!("squad.{squad}.reminders");
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("reminders"))
        else {
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

    /// `[squad.<name>] layout` selects the preset; crew is the default.
    pub fn layout(&self, squad: &str) -> Result<Layout, SquadError> {
        match self
            .squad_table(squad)?
            .and_then(|table| table.get("layout"))
        {
            None => Ok(Layout::Crew),
            Some(item) => item.as_str().and_then(Layout::parse).ok_or_else(|| {
                invalid(format!(
                    "`squad.{squad}.layout` must be crew, pr-queue or minimal."
                ))
            }),
        }
    }

    /// User-defined `[[squad.<name>.section]]` entries, in order. None means
    /// the single default list. `bind` is shape-checked here and used by the
    /// board's actions; it never comes from row data.
    pub fn sections(&self, squad: &str) -> Result<Vec<Section>, SquadError> {
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("section"))
        else {
            return Ok(Vec::new());
        };
        let place = format!("squad.{squad}.section");
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

    /// How rows are laid out: `[squad.<name>.rows]`, the older `columns`
    /// table, or the preset.
    /// The board's theme for `squad`: TMT's global theme with the squad's
    /// `[squad.<name>.theme]` over it (`look::theme`). A bad global theme is
    /// not this file's mistake: the board uses the default and says why,
    /// returned as the notice. A bad squad theme is a configuration error.
    pub fn theme(&self, squad: &str) -> Result<(tmt_cli_style::Theme, Option<String>), SquadError> {
        let place = format!("squad.{squad}.theme");
        let own: Vec<(String, String)> = match self
            .squad_table(squad)?
            .and_then(|table| table.get("theme"))
        {
            None => Vec::new(),
            Some(item) => {
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
                    .collect::<Result<_, _>>()?
            }
        };
        let fallback = |notice: String| {
            crate::look::theme(&[], &own, &place)
                .map(|theme| {
                    (
                        theme,
                        Some(format!("{notice}; the board uses the default theme")),
                    )
                })
                .map_err(invalid)
        };
        if let Some(problem) = &self.theme_error {
            return fallback(problem.clone());
        }
        match crate::look::theme(&self.global_theme, &own, &place) {
            Ok(theme) => Ok((theme, None)),
            // A core that reports no themeError: the global theme is still
            // not this file's mistake.
            Err(error) if error.starts_with("`theme.") => fallback(error),
            Err(error) => Err(invalid(error)),
        }
    }

    /// `[squad.<name>.fields]`: the squad's field providers.
    pub fn providers(&self, squad: &str) -> Result<Vec<crate::provider::Provider>, SquadError> {
        crate::provider::read(
            self.squad_table(squad)?,
            squad,
            crate::rows::field_name,
            |field| crate::rows::OWN_FIELDS.contains(&field),
        )
    }

    pub fn rows(&self, squad: &str) -> Result<crate::rows::Rows, SquadError> {
        crate::rows::read(self.squad_table(squad)?, squad)
    }

    /// `[squad.<name>.board]` over the layout's preset. Validated before the
    /// terminal changes mode, so a mistake never leaves a half-drawn screen.
    pub fn board(&self, squad: &str, layout: Layout) -> Result<Board, SquadError> {
        let preset = Board::preset(layout);
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
            if !["mode", "direction", "panes", "sizes", "layout", "refresh"].contains(&key) {
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
            return Ok(Board {
                mode,
                panes: split.panes(),
                split,
            });
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
        Ok(Board::simple(mode, direction, panes, &sizes))
    }

    /// How often the board reloads everything: `[squad.<name>.board] refresh`,
    /// then top-level `[board] refresh`, then [`DEFAULT_REFRESH`]. `None` is
    /// "off": only F5 and the board's own actions reload.
    pub fn refresh(&self, squad: &str) -> Result<Option<Duration>, SquadError> {
        let global = match self.document.get("board") {
            None => None,
            Some(item) => {
                let table = item
                    .as_table_like()
                    .ok_or_else(|| invalid("`board` must be a table."))?;
                if let Some((key, _)) = table.iter().find(|(key, _)| *key != "refresh") {
                    return Err(invalid(format!("`board.{key}` is not a board setting.")));
                }
                table
                    .get("refresh")
                    .map(|item| (item, "board.refresh".to_owned()))
            }
        };
        let own = self
            .squad_table(squad)?
            .and_then(|table| table.get("board"))
            .and_then(Item::as_table_like)
            .and_then(|table| table.get("refresh"))
            .map(|item| (item, format!("squad.{squad}.board.refresh")));
        match own.or(global) {
            None => Ok(Some(DEFAULT_REFRESH)),
            Some((item, place)) => refresh(item, &place),
        }
    }

    /// `[squad.<name>.notes] render = "markdown" | "plain"`; markdown by default.
    pub fn notes_render(&self, squad: &str) -> Result<NotesRender, SquadError> {
        let place = format!("squad.{squad}.notes");
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("notes"))
        else {
            return Ok(NotesRender::Markdown);
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid(format!("`{place}` must be a table.")))?;
        if let Some((key, _)) = table.iter().find(|(key, _)| *key != "render") {
            return Err(invalid(format!("`{place}.{key}` is not a notes setting.")));
        }
        match table.get("render").map(|value| value.as_str()) {
            None | Some(Some("markdown")) => Ok(NotesRender::Markdown),
            Some(Some("plain")) => Ok(NotesRender::Plain),
            Some(_) => Err(invalid(format!(
                "`{place}.render` must be markdown or plain."
            ))),
        }
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
        for (key, item) in table.iter() {
            match key {
                "order" => tabs.order = tab_list(item, "tabs.order")?,
                "hide" => tabs.hide = tab_list(item, "tabs.hide")?,
                "pin" => tabs.pin = tab_list(item, "tabs.pin")?,
                "colors" => tabs.colors = tab_colors(item)?,
                "leads" => tabs.leads = tab_bindings(item, "tabs.leads")?,
                "all" => tabs.all = tab_bindings(item, "tabs.all")?,
                other => {
                    return Err(invalid(format!(
                        "`tabs.{other}` is not a tabs setting; use order, pin, hide, colors, leads or all."
                    )));
                }
            }
        }
        Ok(tabs)
    }

    /// The state vocabulary: the layout's order and colors, overridden by
    /// `[squad.<name>.states] <state> = { color = "...", sort = N }`. An explicit
    /// `sort` ranks before a layout default with the same number.
    pub fn states(&self, squad: &str, layout: Layout) -> Result<States, SquadError> {
        // (explicit sort, implicit tie-break, layout position, name)
        let mut ranks: BTreeMap<String, (u16, bool, usize)> = layout
            .states()
            .iter()
            .enumerate()
            .map(|(index, state)| ((*state).into(), (index as u16, true, index)))
            .collect();
        let mut colors: BTreeMap<String, String> = layout
            .state_colors()
            .iter()
            .map(|(state, color)| ((*state).into(), (*color).into()))
            .collect();
        let place = format!("squad.{squad}.states");
        if let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("states"))
        {
            let table = item
                .as_table_like()
                .ok_or_else(|| invalid(format!("`{place}` must be a table of states.")))?;
            for (state, settings) in table.iter() {
                let settings = settings
                    .as_table_like()
                    .filter(|_| field_name(state))
                    .ok_or_else(|| invalid(format!("`{place}.{state}` must be a table.")))?;
                for (key, value) in settings.iter() {
                    match key {
                        "color" => {
                            let color = value
                                .as_str()
                                .filter(|color| crate::look::known(color))
                                .ok_or_else(|| {
                                    invalid(format!(
                                        "`{place}.{state}.color` must be {}.",
                                        crate::look::names()
                                    ))
                                })?;
                            colors.insert(state.into(), color.into());
                        }
                        "sort" => {
                            let sort = value
                                .as_integer()
                                .and_then(|sort| u16::try_from(sort).ok())
                                .filter(|sort| *sort <= 999)
                                .ok_or_else(|| {
                                    invalid(format!("`{place}.{state}.sort` must be 0-999."))
                                })?;
                            let position = ranks.get(state).map_or(usize::MAX, |rank| rank.2);
                            ranks.insert(state.into(), (sort, false, position));
                        }
                        other => {
                            return Err(invalid(format!(
                                "`{place}.{state}.{other}` is not a state setting; use color or sort."
                            )));
                        }
                    }
                }
            }
        }
        let mut order: Vec<(String, (u16, bool, usize))> = ranks.into_iter().collect();
        order.sort_by(|(a, left), (b, right)| left.cmp(right).then_with(|| a.cmp(b)));
        Ok(States {
            order: order.into_iter().map(|(state, _)| state).collect(),
            colors,
        })
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
        edit(&mut self.document);
        let bytes = self.document.to_string().into_bytes();
        publish(&self.path, &bytes).map_err(|error| write_failed(&self.path, error))?;
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
    fn reminders_are_per_squad_off_by_default_and_bounded() {
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
        assert_eq!(config.reminders("other").unwrap(), Reminders::default());
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
            assert!(!config.reminders("product").unwrap().enabled);
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

    fn temp(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("tmt-squad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory.join("squad.toml")
    }

    #[test]
    fn me_is_written_without_disturbing_the_users_document() {
        let path = temp("me");
        let original = "# my board\n[squad.product]\nlayout = \"pr-queue\" # queue\n\n[bind]\no = \"open {pr_link}\"\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        assert_eq!(config.me().unwrap(), None);
        assert_eq!(config.layout("product").unwrap(), Layout::PrQueue);
        assert_eq!(config.layout("other").unwrap(), Layout::Crew);
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
    fn tmux_keys_default_to_s_and_b_and_back_is_opt_in() {
        let path = temp("tmux-keys");
        fs::write(&path, "").unwrap();
        let keys = Config::read(path.clone()).unwrap().tmux_keys().unwrap();
        assert_eq!(
            keys,
            TmuxKeys {
                popup: "S".into(),
                pane: "B".into(),
                back: None
            }
        );
        fs::write(
            &path,
            "[tmux]\npopup = \"C-s\"\npane = \"F5\"\nback = \"b\"\n",
        )
        .unwrap();
        let keys = Config::read(path.clone()).unwrap().tmux_keys().unwrap();
        assert_eq!((keys.popup.as_str(), keys.pane.as_str()), ("C-s", "F5"));
        assert_eq!(keys.back.as_deref(), Some("b"));
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
            let bindings = config.bindings(tmux).unwrap();
            assert_eq!(bindings["enter"].verb, crate::action::Verb::Open);
            assert_eq!(bindings["f5"].verb, crate::action::Verb::Refresh);
            assert_eq!(bindings["y"].verb, crate::action::Verb::Copy, "preset kept");
        }
        fs::write(&path, "").unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(
            config.bindings(false).unwrap()["double-click"].verb,
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
                .and_then(|config| config.bindings(true))
                .unwrap_err();
            assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{body}");
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
        let error = |squad| config.theme(squad).unwrap_err().to_string();
        assert!(
            error("bad").contains("`squad.bad.theme.waiting`"),
            "{}",
            error("bad")
        );
        assert!(error("odd").contains("`squad.odd.theme` must be a table"));

        // A broken global theme is TMT's config, not this file's: the board
        // draws with the default theme and says why.
        config.global_theme(&serde_json::json!({
            "resolved": {"theme": {}},
            "themeError": {"key": "theme.base", "message": "unknown base dark."}
        }));
        let (theme, notice) = config.theme("product").unwrap();
        assert_eq!(theme.base, Base::Tmt);
        assert_eq!(
            notice.as_deref(),
            Some("theme.base unknown base dark; the board uses the default theme")
        );
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
            ["member", "state", "task", "pr_link"]
        );
        let columns = config.rows("product").unwrap().columns;
        assert_eq!(
            (columns[2].field.as_str(), columns[2].title.as_str()),
            ("note", "WHY")
        );
        assert_eq!((columns[2].width, columns[2].grow), (Some(30), 0));
        assert_eq!(columns[0].title, "MEMBER");
        let colors = config.states("product", Layout::Crew).unwrap().colors;
        assert_eq!(colors["blocked"], "red");
        assert_eq!(colors["parked"], "dim");
        assert_eq!(colors["working"], "working", "layout defaults remain");
        assert!(
            config
                .states("other", Layout::Minimal)
                .unwrap()
                .colors
                .is_empty()
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
            read("[squad.x.board]\nrefresh = \"1s\"\n")
                .board("x", Layout::Crew)
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
            let code = read(body).refresh("x").err().map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn board_presets_overrides_and_validation() {
        let path = temp("panes");
        let read = |body: &str| {
            fs::write(&path, body).unwrap();
            Config::read(path.clone()).unwrap()
        };
        let config = read("");
        let crew = config.board("x", Layout::Crew).unwrap();
        assert_eq!(
            crew.split,
            Split::simple(Direction::LeftRight, &[Pane::Rows, Pane::Notes], &[60, 40])
        );
        let queue = config.board("x", Layout::PrQueue).unwrap();
        assert_eq!(
            queue.split,
            Split::simple(Direction::TopBottom, &[Pane::Rows, Pane::Detail], &[70, 30])
        );
        assert_eq!(
            config.board("x", Layout::Minimal).unwrap().panes,
            [Pane::Rows]
        );

        let custom = read("[squad.x.board]\ndirection = \"top-bottom\"\npanes = [\"detail\", \"rows\", \"replies\"]\n")
            .board("x", Layout::Crew)
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
        .board("x", Layout::Crew)
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
            .board("x", Layout::Crew)
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
            let code = read(body)
                .board("x", Layout::Crew)
                .err()
                .map(|error| error.code);
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
        assert_eq!(
            states.order,
            [
                "blocked", "working", "idle", "parked", "review", "testing", "hold"
            ]
        );
        assert_eq!(states.rank(Some("blocked")), 0);
        assert_eq!(states.rank(Some("unknown")), states.order.len());
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
            Config::read(path.clone()).unwrap().tabs().unwrap().order,
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
            Config::read(path.clone()).unwrap().tabs()
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
}
