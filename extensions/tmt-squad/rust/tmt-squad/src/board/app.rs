//! Board state and key handling, independent of the terminal. Every keypress
//! works on what is already loaded; loading happens in the refresh worker,
//! and actions leave as fully resolved requests.

use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::display_rows::{Item, RowOrigin};
use crate::links::Kind;
use crate::{
    action::{Action, Bindings, Verb},
    attention::Attention,
    config::{Board, BoardMode, Config, NotesRender, Pane},
    effects,
};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

/// What the refresh worker loaded for one squad.
pub struct RateView {
    pub settings: crate::config::TokenRate,
    pub input: super::rate::Input,
    pub history: Option<super::rate::history::Seeds>,
}

/// Raw observations for tiles and headers; formatting belongs to the caller.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Consumed by #1293 tiles and #1295 HOME header painting."
    )
)]
pub(super) struct HomeUsage<'a> {
    pub lead_model: Option<&'a str>,
    pub windows: [crate::config::TokenWindow; 3],
    pub lead: [Option<super::rate::Reading>; 3],
    pub squad: [Option<super::rate::Reading>; 3],
    pub share: Option<UsageShare>,
}

#[derive(Debug, PartialEq)]
pub(super) struct UsageShare {
    pub fraction: f64,
    pub partial: bool,
}

/// Raw UUID-deduplicated observations for the HOME header; painting only formats.
#[derive(Debug)]
pub(super) struct HomeHeaderUsage<'a> {
    pub windows: [crate::config::TokenWindow; 3],
    pub totals: [Option<super::rate::Reading>; 3],
    pub top: Option<UsageTop<'a>>,
    pub models: Vec<UsageModel<'a>>,
    pub unreported: usize,
}

#[derive(Debug)]
pub(super) struct UsageTop<'a> {
    pub member: &'a str,
    pub share: UsageShare,
}

#[derive(Debug)]
pub(super) struct UsageModel<'a> {
    pub model: Option<&'a str>,
    pub share: UsageShare,
}

#[derive(Clone, Copy)]
struct HeaderObservation<'a> {
    reading: super::rate::Reading,
    covered_slots: u64,
    model: Option<&'a str>,
}

impl HeaderObservation<'_> {
    fn evidence(&self) -> (u64, bool, u64) {
        (self.covered_slots, !self.reading.partial, self.reading.span)
    }
}

struct HeaderMember<'a> {
    name: &'a str,
    readings: [Option<HeaderObservation<'a>>; 3],
}

pub struct View {
    pub ask_lead: String,
    pub home_replies: bool,
    pub token_rate: Option<RateView>,
    /// HOME sampling templates, separate from the painter/controller model.
    pub home_rate: BTreeMap<String, RateView>,
    /// The `status --json` document, so the board and `status` never differ.
    pub document: Value,
    /// Retained home composition; only the aggregate board view owns it.
    pub home: Option<super::home::Home>,
    pub(super) derived: RefCell<super::derived::Derived>,
    pub rows: crate::rows::Rows,
    pub board: Board,
    /// The full-reload interval; `None` reloads only on ctrl-r and actions.
    pub refresh: Option<std::time::Duration>,
    pub notes: Notes,
    pub render: NotesRender,
    /// The host preset overridden by `[bind]`.
    pub bindings: Bindings,
    /// Each document section's own bindings, in document order.
    pub section_bindings: Vec<Bindings>,
    pub configured_bindings: Bindings,
    pub opener: Option<Vec<String>>,
    pub clipboard: Option<Vec<String>>,
    pub links: crate::links::Handlers,
    /// `[tabs.colors]`, re-read with every load like the rest of squad.toml.
    pub tab_colors: crate::config::TabColors,
    /// The user's saved identity, the sender of talk, reply and annotate.
    pub me: Option<String>,
    pub me_id: Option<String>,
    /// Finals to the user's squad requests, newest first (replies pane).
    pub replies: Vec<Value>,
    /// The squad's theme at the terminal's depth: every color the board draws.
    pub look: crate::look::Look,
    /// Why the board uses the default theme, when the global one is wrong.
    pub theme_notice: Option<String>,
}

/// The lead's notebook, already sanitized for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notes {
    /// The notes pane is not shown, so nothing was read.
    NotShown,
    NoLead,
    /// The lead has no notebook yet; the board never creates one.
    Missing,
    Text(String),
    Failed(String),
}

pub struct Snapshot {
    /// Squad names in core list order, before tab policy.
    pub squad_keys: Vec<String>,
    /// Tab keys in order: squad names and built-in tabs (`board::tabs`).
    pub tabs: Vec<String>,
    /// Hidden tabs, still reachable through the switcher.
    pub hidden: Vec<String>,
    /// How many tabs at the front are pinned.
    pub pinned: usize,
    /// Every listed squad's attention, for its tab's color and counts.
    pub attention: BTreeMap<String, Attention>,
    pub squad: Option<String>,
    pub view: Result<View, String>,
}

/// An action with every value already taken from the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show this member's pane in the invoking client.
    Jump(String),
    /// Return the invoker's client to where its last jump came from.
    Back,
    Open {
        link: String,
        opener: Option<Vec<String>>,
    },
    Copy {
        text: String,
        program: Option<Vec<String>>,
    },
    Run(Vec<String>),
    RevealFile {
        path: String,
        opener: Option<Vec<String>>,
    },
    Talk {
        me: String,
        squad: String,
        to: String,
        text: String,
    },
    Annotate {
        me: String,
        squad: String,
        to: String,
        row: String,
        text: String,
    },
    Reply {
        me: String,
        request: String,
        from: String,
        text: String,
    },
    Leads {
        sender: String,
        recipients: Vec<crate::send::LeadRecipient>,
        all: bool,
        text: String,
    },
    /// Save this tab order to `[tabs] order` (tab keys, in order).
    Reorder(Vec<String>),
    /// A cron job control, with its actor, job and viewed revision resolved.
    Cron(super::cronboard::CronRequest),
}

impl Request {
    /// Sending changes request state, and a saved order changes the tabs, so
    /// the view reloads afterwards.
    pub fn sends(&self) -> bool {
        matches!(
            self,
            Self::Talk { .. }
                | Self::Annotate { .. }
                | Self::Reply { .. }
                | Self::Reorder(_)
                | Self::Cron(_)
                | Self::Leads { .. }
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    /// Load this squad now (a squad switch).
    Load(String),
    Refresh,
    Settings,
    SaveSetting,
    HomeReplies(bool),
    CancelSettings,
    PickTheme,
    SaveTheme,
    PickView,
    SaveView,
    CancelView,
    Act(Request),
}

/// What a menu entry does: run a binding, or reply to one open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Action(Action),
    /// Confirms a cron control picked from a menu.
    Cron(super::cronboard::CronRequest),
    /// Leaves the menu without doing anything.
    Dismiss,
    Leads {
        recipients: Vec<crate::send::LeadRecipient>,
        all: bool,
    },
    Reply {
        request: String,
        from: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// The key that chooses it directly.
    pub key: String,
    pub label: String,
    pub choice: Choice,
}

/// Opening sender/member retained until a link composer is submitted.
#[derive(Clone)]
pub struct LinkSend {
    member: String,
    sender: String,
}

/// The row's action menu, or the choice among a member's open requests.
pub struct Menu {
    pub(super) row_send: Option<RowSend>,
    pub link: Option<LinkSend>,
    pub prefill: String,
    pub title: String,
    pub entries: Vec<MenuEntry>,
    pub selected: usize,
    pub(super) surface: RefCell<Option<super::menu_surface::MenuSurface>>,
}

/// Where composed text goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compose {
    Leads {
        sender: String,
        recipients: Vec<crate::send::LeadRecipient>,
        all: bool,
    },
    /// Read-only mode of the existing anchored band; it never submits text.
    ReadLead {
        key: super::home_leads::MessageKey,
        offset: usize,
    },
    AskLead {
        to: String,
        sender: String,
    },
    Talk {
        to: String,
    },
    Annotate {
        to: String,
        row: String,
    },
    Reply {
        request: String,
        from: String,
    },
    /// One step of a cron form; its draft lives in `App::cron_draft`.
    Cron,
}

/// A row occurrence, independent of its position after refresh or sorting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowTarget {
    Home(super::home::Target),
    Member {
        tab: String,
        section: usize,
        squad: String,
        id: String,
    },
    /// A notebook link may address the squad lead outside the member rows.
    Lead {
        tab: String,
        squad: String,
        id: String,
    },
}

#[derive(Clone)]
pub(super) struct RowSend {
    pub target: RowTarget,
    pub sender: String,
    pub name: String,
    pub note: Option<Compose>,
    pub note_member: bool,
}

/// A successful send keeps its Home row visible through the ensuing refresh,
/// until the next key clears the confirmation. This is display evidence only.
pub(super) struct RowFeedback {
    pub target: RowTarget,
    pub home: Option<HomeFeedback>,
}

pub(super) struct HomeFeedback {
    pub row: Value,
    pub lead: Option<String>,
    pub index: usize,
}

impl RowSend {
    fn valid(&self, app: &App, input: &Input) -> bool {
        if matches!(input.compose, Compose::Leads { .. }) {
            return app.leads_valid(input);
        }
        if app.loading()
            || app.view.as_ref().and_then(|view| view.me.as_ref()) != Some(&self.sender)
        {
            return false;
        }
        let squad = match &self.target {
            RowTarget::Home(target) => &target.squad,
            RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => squad,
        };
        let Some(row) = app.target_row(&self.target) else {
            return false;
        };
        if squad != &input.squad || row["name"].as_str() != Some(&self.name) {
            return false;
        }
        match &input.compose {
            Compose::Talk { to } => to == &self.name,
            Compose::Reply { request, from } => {
                from == &self.name
                    && row["waitingOnYou"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["requestId"].as_str() == Some(request))
                    })
            }
            Compose::Annotate { to, .. } => {
                self.note.as_ref() == Some(&input.compose)
                    && (self.note_member || app.note_recipient(&self.target).as_ref() == Some(to))
            }
            Compose::AskLead { .. }
            | Compose::ReadLead { .. }
            | Compose::Leads { .. }
            | Compose::Cron => false,
        }
    }
}

/// The one-line composer: Enter sends, Esc cancels, empty sends nothing.
pub struct Input {
    pub(super) row_send: Option<RowSend>,
    pub(super) alternative: Option<Compose>,
    pub(super) quote: Option<String>,
    pub link: Option<LinkSend>,
    pub prompt: String,
    pub text: String,
    pub compose: Compose,
    /// The squad the text is sent in: the row's own on the leads tab.
    pub squad: String,
    /// A line after the text: the accepted forms, or why the last entry failed.
    pub hint: Option<Hint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub text: String,
    pub error: bool,
}

impl Input {
    pub(super) fn header(&self) -> String {
        match &self.compose {
            Compose::ReadLead { .. } | Compose::Leads { .. } => self.prompt.clone(),
            Compose::Talk { to } => format!("→ {to} ({})", self.squad),
            Compose::Reply { from, .. } => format!("→ {from} ({})", self.squad),
            Compose::Annotate { to, row } if to != row => {
                format!("✎ note → {to} · about {row}")
            }
            Compose::Annotate { to, .. } => format!("✎ note → {to}"),
            Compose::AskLead { to, .. } => format!("→ lead {to}"),
            Compose::Cron => self.prompt.clone(),
        }
    }
}

/// Longest text the composer accepts, in characters.
const INPUT_LIMIT: usize = 4000;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Shared routing identifies the active surface; its controller owns effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Overlay {
    Help,
    Settings,
    View,
    Theme,
    Switcher,
    CronList,
}
impl Overlay {
    fn id(self) -> tmt_tui::app::ComponentId {
        vec![
            match self {
                Self::Help => "help",
                Self::Settings => "settings",
                Self::View => "view-picker",
                Self::Theme => "theme-picker",
                Self::Switcher => "switcher",
                Self::CronList => "cron-list",
            }
            .into(),
        ]
    }
}

/// The quick switcher: a filter over every tab, hidden ones included.
pub struct Switcher {
    pub(super) unpicked_only: bool,
    pub(super) surface: RefCell<super::picker_surface::State>,
}
impl Default for Switcher {
    fn default() -> Self {
        Self::new(String::new())
    }
}
impl Switcher {
    pub fn new(query: String) -> Self {
        Self {
            unpicked_only: false,
            surface: RefCell::new(super::picker_surface::State::new(Some(query), vec![], None)),
        }
    }
    pub fn query(&self) -> String {
        self.surface.borrow().picker.query().unwrap_or("").into()
    }
}

/// Where one tab was drawn on the tab line, for clicks and drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabHit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub tab: usize,
}

/// A screen line of the rows pane and the visible row it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub row: usize,
}

/// Config is immutable; only this session presentation set changes on a toggle.
struct FoldState {
    board: Board,
    overrides: BTreeMap<Pane, bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct TitleHit {
    pub pane: Pane,
    pub area: ratatui::layout::Rect,
}

#[derive(Default)]
pub struct App {
    pub(super) note_link: Option<(String, usize)>,
    pub(super) link_hits: RefCell<Vec<(ratatui::layout::Rect, usize)>>,
    pub(super) note_cursors: RefCell<BTreeMap<String, super::notes::NotesCursor>>,
    pub(super) note_hits: RefCell<Vec<(ratatui::layout::Rect, usize)>>,
    pub(super) notebooks: RefCell<super::notes::Notebooks>,
    pub(super) cron: super::cronboard::State,
    pub(super) home_leads: super::home_leads::State,
    /// The cron form being filled in on the input line, if any.
    pub(super) cron_draft: Option<super::cronboard::Draft>,
    /// The squad tab's jobs half has focus (Tab moves in after the last pane).
    pub(super) jobs_focus: bool,
    /// One list state per squad room; selection survives tab switches.
    pub(super) jobs: RefCell<BTreeMap<String, super::cronboard::JobsPane>>,
    /// Where the jobs half was last drawn, for the pointer; empty when absent.
    pub(super) jobs_area: std::cell::Cell<ratatui::layout::Rect>,
    pub(super) meter: Option<super::meter::Meter>,
    meters: BTreeMap<String, super::meter::Meter>,
    usage_document: Option<Value>,
    pub(super) token_window: crate::config::TokenWindow,
    pub(super) excluded_counters: Vec<String>,
    window_changed: bool,
    squad_keys: Vec<String>,
    pub(super) picks: super::pick::Picks,
    pub tabs: Vec<String>,
    pub hidden: Vec<String>,
    pub pinned: usize,
    /// The quick switcher (`s`), while open.
    pub switcher: Option<Switcher>,
    /// The `c` list of every squad's jobs, while open.
    pub(super) cron_list: Option<super::cronboard::List>,
    pub attention: BTreeMap<String, Attention>,
    pub current: Option<String>,
    pub view: Option<View>,
    pub error: Option<String>,
    pub search: String,
    pub searching: bool,
    /// Index among visible rows (headers excluded).
    pub selected: usize,
    pub(super) home_target: Option<super::home::Target>,
    /// Where the cursor was when the user left HOME; the next visit restores it.
    home_left: Option<super::home::Target>,
    /// The cursor still sits where HOME's opening rule put it. The first lead
    /// read can reorder the leads, so it places the cursor once more.
    pub(super) home_start: bool,
    pub notice: Option<String>,
    pub help: bool,
    pub(super) help_state: RefCell<super::help::Help>,
    overlay_focus: tmt_tui::app::FocusStack,
    pub menu: Option<Menu>,
    pub(super) view_picker: Option<super::view_picker::Picker>,
    pub(super) theme_picker: Option<super::theme_picker::Picker>,
    pub(super) settings: Option<super::settings::Overlay>,
    pub input: Option<Input>,
    /// Last successful row send; cleared on the next user action.
    pub(super) sent: Option<RowFeedback>,
    pending_send: Option<RowFeedback>,
    /// Current-frame inline band, reserved by the row painter.
    pub(super) input_band: std::cell::Cell<Option<ratatui::layout::Rect>>,
    /// Index of the focused pane (split) or visible tab (tabs).
    pub focus: usize,
    /// Every pane's scroll position, from one owner.
    pub scrolls: Scrolls,
    /// The rows pane keeps the selection on screen until the wheel moves it.
    pub follow: bool,
    /// Opened as a tmux popup: a successful jump closes the board.
    pub popup: bool,
    /// Views of squads already visited, so switching back is instant while
    /// the worker refreshes them.
    cache: BTreeMap<String, View>,
    /// Set from a squad switch until that squad's data arrives.
    pub loading_since: Option<Instant>,
    /// The squad `view` belongs to.
    shown: Option<String>,
    last_click: Option<(usize, Instant)>,
    /// Where rows were last drawn, for mouse events.
    pub hits: RefCell<Vec<Hit>>,
    /// Each record's first visual line in the last rows draw, for paging.
    pub row_starts: RefCell<Vec<usize>>,
    /// Where tabs were last drawn.
    pub tab_hits: RefCell<Vec<TabHit>>,
    pub(super) unpicked_hit: std::cell::Cell<Option<ratatui::layout::Rect>>,
    /// Only titles actually painted in the last frame can toggle.
    pub title_hits: RefCell<Vec<TitleHit>>,
    folds: BTreeMap<String, FoldState>,
    body_width: u16,
    /// The first tab the tab line showed, so it scrolls only as needed.
    pub tab_start: std::cell::Cell<usize>,
    /// The tab a left button went down on, until it is released.
    dragging: Option<usize>,
}

fn page_step(code: KeyCode) -> Step {
    match code {
        KeyCode::PageUp => Step::Pages(-1),
        KeyCode::PageDown => Step::Pages(1),
        KeyCode::Home => Step::Top,
        _ => Step::Bottom,
    }
}

pub(super) fn matches(row: &Value, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    let contains = |value: &Value| {
        value
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains(&needle))
    };
    contains(&row["name"])
        || row["fields"]
            .as_object()
            .is_some_and(|fields| fields.values().any(contains))
}

/// The binding name of a key, as `[bind]` spells it.
pub fn event_name(key: KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char(character) if ctrl => format!("ctrl-{}", character.to_ascii_lowercase()),
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(character) if character.is_ascii_graphic() => character.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::F(number) => format!("f{number}"),
        _ => return None,
    })
}

impl App {
    pub fn new(squad: Option<String>) -> Self {
        Self {
            current: squad,
            follow: true,
            ..Self::default()
        }
    }

    /// RowOrigin and row for every displayed row matching the search, in board order.
    pub(super) fn rows(&self) -> Vec<(RowOrigin, &Value)> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        if view.home.is_some() {
            return self
                .home_entries()
                .into_iter()
                .map(|entry| (RowOrigin::Section(0), entry.row))
                .collect();
        }
        self.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(slot, row) => Some((slot, row)),
                _ => None,
            })
            .collect()
    }

    /// The lead, the members rule, section headers and the rows matching the
    /// search, in board order. The single default section has no header.
    pub fn items(&self) -> Vec<Item<'_>> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        crate::display_rows::project(
            self.usage_document.as_ref().unwrap_or(&view.document),
            |row| matches(row, &self.search),
        )
    }

    /// Replace only the board display projection. Sampling never changes public ls JSON.
    pub(super) fn project_usage(&mut self, now: Instant) {
        if self.loading() {
            return;
        } // Keep the retained view's values while switching.
        let Some(view) = self.view.as_mut() else {
            self.usage_document = None;
            return;
        };
        let Some(meter) = self.meter.as_ref().filter(|m| m.settings.enabled) else {
            self.usage_document = None;
            return;
        };
        if view.rows.select_window(
            meter
                .settings
                .windows
                .iter()
                .position(|window| *window == self.token_window)
                .unwrap_or(0),
        ) {
            view.derived.borrow_mut().grid = None;
        }
        let mut document = view.document.clone();
        crate::display_rows::each_row(&mut document, |row| {
            if let Some(id) = row["id"].as_str().map(str::to_owned) {
                for column in &view.rows.columns {
                    let Some(source) = &column.from else { continue };
                    if let Some(index) = source.window() {
                        let reading = meter.member(&id, index, now);
                        let value = reading
                            .and_then(|r| {
                                crate::source::render_value(
                                    &serde_json::json!(r.tokens.to_string()),
                                    column.format,
                                    0,
                                )
                                .map(|value| format!("{}{value}", if r.partial { "~" } else { "" }))
                            })
                            .unwrap_or_else(|| "–".into());
                        row["fields"][&column.field] = value.into();
                        if let Some(token) = reading.and_then(|r| column.threshold(r.tokens as f64))
                        {
                            row["colors"][&column.field] = token.into();
                        }
                    } else if source.path == "session.model" {
                        row["fields"][&column.field] = meter.model(&id).unwrap_or("–").into();
                    }
                }
            }
        });
        if self.usage_document.as_ref() != Some(&document) {
            view.derived.borrow_mut().grid = None;
            self.usage_document = Some(document);
        }
    }

    /// Session model display does not require token sampling or a usable counter.
    pub(super) fn home_lead_model(&self, squad: &str) -> Option<&str> {
        let view = self.view.as_ref()?;
        let id = view
            .home
            .as_ref()?
            .squads
            .iter()
            .find(|line| line.squad == squad)?
            .lead
            .as_ref()?["id"]
            .as_str()?;
        view.home_rate.get(squad)?.input.resumes.get(id)?["model"]
            .as_str()
            .filter(|model| !model.is_empty())
    }

    fn home_meter(&self, squad: &str) -> Option<&super::meter::Meter> {
        let rate = self.view.as_ref()?.home_rate.get(squad)?;
        self.meters
            .get(squad)
            .or_else(|| {
                self.meter
                    .as_ref()
                    .filter(|_| self.current.as_deref() == Some(squad))
            })
            .filter(|meter| meter.room == rate.input.room && meter.settings == rate.settings)
    }

    pub(super) fn home_usage(&self, squad: &str, now: Instant) -> Option<HomeUsage<'_>> {
        let view = self.view.as_ref()?;
        let home = view.home.as_ref()?;
        let rate = view
            .home_rate
            .get(squad)
            .filter(|rate| rate.settings.enabled)?;
        let meter = self.home_meter(squad);
        let id = home
            .squads
            .iter()
            .find(|line| line.squad == squad)?
            .lead
            .as_ref()
            .and_then(|lead| lead["id"].as_str());
        let lead = std::array::from_fn(|i| {
            meter.and_then(|meter| id.and_then(|id| meter.member(id, i, now)))
        });
        let squad = std::array::from_fn(|i| meter.and_then(|meter| meter.total(i, now)));
        let share = lead[2]
            .zip(squad[2])
            .filter(|(_, total)| total.tokens > 0)
            .map(|(lead, total)| UsageShare {
                fraction: lead.tokens as f64 / total.tokens as f64,
                partial: lead.partial || total.partial,
            });
        Some(HomeUsage {
            lead_model: meter.and_then(|meter| id.and_then(|id| meter.model(id))),
            windows: rate.settings.windows,
            lead,
            squad,
            share,
        })
    }

    pub(super) fn home_header_usage(&self, now: Instant) -> Option<HomeHeaderUsage<'_>> {
        let view = self.view.as_ref()?;
        let windows = view.home.as_ref()?.windows;
        // Stable displayed squad order decides equal-evidence UUID collisions.
        let shown = self.home_entries();
        let mut enabled = false;
        let mut identities = BTreeMap::new();
        let mut members: Vec<HeaderMember<'_>> = Vec::new();
        for entry in shown
            .iter()
            .filter(|entry| entry.target.section == "squads")
        {
            let squad = &entry.target.squad;
            let Some(rate) = view
                .home_rate
                .get(squad)
                .filter(|rate| rate.settings.enabled)
            else {
                continue;
            };
            enabled = true;
            let meter = self.home_meter(squad);
            for id in rate.input.resumes.keys() {
                let index = *identities.entry(id).or_insert_with(|| {
                    members.push(HeaderMember {
                        name: rate
                            .input
                            .names
                            .get(id)
                            .map_or("unknown member", String::as_str),
                        readings: [None; 3],
                    });
                    members.len() - 1
                });
                for (i, window) in windows.iter().enumerate() {
                    let candidate = meter.and_then(|meter| {
                        meter
                            .observation(id, *window, now)
                            .map(|(reading, covered_slots)| HeaderObservation {
                                reading,
                                covered_slots,
                                model: meter.model(id),
                            })
                    });
                    if let Some(candidate) = candidate
                        && members[index].readings[i]
                            .is_none_or(|old| candidate.evidence() > old.evidence())
                    {
                        members[index].readings[i] = Some(candidate);
                    }
                }
            }
        }
        if !enabled {
            return None;
        }
        let totals = std::array::from_fn(|i| {
            let mut sum = super::rate::Reading {
                tokens: 0,
                partial: false,
                span: windows[i].milliseconds(),
            };
            let mut measured = false;
            for member in &members {
                if let Some(HeaderObservation { reading, .. }) = member.readings[i] {
                    sum.tokens += reading.tokens;
                    sum.partial |= reading.partial;
                    sum.span = sum.span.min(reading.span);
                    measured = true;
                } else {
                    sum.partial = true;
                }
            }
            measured.then_some(sum)
        });
        let unreported = members
            .iter()
            .filter(|member| member.readings[2].is_none())
            .count();
        let mut top = None;
        let mut by_model = BTreeMap::<Option<&str>, u128>::new();
        if let Some(total) = totals[2].filter(|total| total.tokens > 0) {
            let share = |tokens| UsageShare {
                fraction: tokens as f64 / total.tokens as f64,
                partial: total.partial,
            };
            let mut highest = 0;
            for member in &members {
                if let Some(HeaderObservation { reading, model, .. }) = member.readings[2] {
                    *by_model.entry(model).or_default() += reading.tokens;
                    if reading.tokens > highest {
                        highest = reading.tokens;
                        top = Some(UsageTop {
                            member: member.name,
                            share: share(reading.tokens),
                        });
                    }
                }
            }
            let mut models = by_model.into_iter().collect::<Vec<_>>();
            models.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
            return Some(HomeHeaderUsage {
                windows,
                totals,
                top,
                unreported,
                models: models
                    .into_iter()
                    .map(|(model, tokens)| UsageModel {
                        model,
                        share: share(tokens),
                    })
                    .collect(),
            });
        }
        Some(HomeHeaderUsage {
            windows,
            totals,
            top,
            models: Vec::new(),
            unreported,
        })
    }

    /// The existing worker's one HOME receipt updates the same retained meters.
    pub(super) fn sample_home(
        &mut self,
        resumes: Result<&BTreeMap<String, Value>, ()>,
        now: Instant,
    ) -> bool {
        if self.loading() || self.current.as_deref() != Some(super::ALL) {
            return false;
        }
        let Some(view) = self.view.as_mut().filter(|view| view.home.is_some()) else {
            return false;
        };
        let mut sampled = false;
        for (name, rate) in &mut view.home_rate {
            if let Ok(rows) = resumes {
                rate.input = rate.input.joined(rows);
            }
            if let Some(meter) = self
                .meters
                .get_mut(name)
                .filter(|meter| rate.settings.enabled && meter.due(now))
            {
                let input = resumes.map(|rows| rate.input.joined(rows));
                meter.sample(input.as_ref().map_err(|_| ()), now);
                sampled = true;
            }
        }
        sampled
    }

    fn clamp(&mut self) {
        if !self.message_valid() {
            self.input = None;
        }
        let anchor = self
            .input
            .as_ref()
            .and_then(|input| input.row_send.as_ref())
            .map(|send| &send.target)
            .or(self.sent.as_ref().map(|feedback| &feedback.target));
        if let Some(target) = anchor
            && let Some(index) = (0..self.rows().len())
                .find(|&index| self.row_target(index).as_ref() == Some(target))
        {
            self.selected = index;
            if let RowTarget::Home(target) = target {
                self.home_target = Some(target.clone());
            }
        }
        if self.view.as_ref().is_some_and(|view| view.home.is_some()) {
            let entries = self.home_entries();
            let index = self
                .home_target
                .as_ref()
                .and_then(|target| entries.iter().position(|entry| &entry.target == target))
                .unwrap_or(self.selected.min(entries.len().saturating_sub(1)));
            let target = entries.get(index).map(|entry| entry.target.clone());
            self.selected = index;
            self.home_target = target;
        } else {
            self.home_target = None;
            self.selected = self.selected.min(self.rows().len().saturating_sub(1));
        }
    }

    /// Another squad is now on screen: its selection and scrolling start over.
    /// HOME alone returns to the row the user left.
    fn shown_changed(&mut self) {
        self.note_link = None;
        self.home_target = None;
        self.home_start = false;
        self.selected = 0;
        self.scrolls = Scrolls::default();
        self.follow = true;
        self.reconcile_folds();
        if self.view.as_ref().is_some_and(|view| view.home.is_some()) {
            let left = self.home_left.take();
            let index = left.and_then(|target| {
                self.home_entries()
                    .iter()
                    .position(|entry| entry.target == target)
            });
            match index {
                Some(index) => self.selected = index,
                None => return self.place_home_start(),
            }
        }
        self.clamp();
    }

    /// HOME opens on its first row from the top: a needs-you row, else a
    /// blocked row, else the first lead, else the first squad. The cron line
    /// is never the opening row. Squad tabs have no home entries and keep the
    /// lead they start on.
    fn place_home_start(&mut self) {
        let entries = self.home_entries();
        let first = |section: &str| {
            entries
                .iter()
                .position(|entry| entry.target.section == section)
        };
        let index = ["needs-you", "blocked", super::home::LEADS, "squads"]
            .into_iter()
            .find_map(first);
        self.home_start =
            index.is_some_and(|index| entries[index].target.section == super::home::LEADS);
        self.selected = index.unwrap_or(0);
        self.home_target = None;
        self.clamp();
    }

    /// Tab owning the retained view, even while another tab loads.
    pub(super) fn shown_tab(&self) -> Option<&str> {
        self.shown.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn set_tab_focus_for_test(&mut self, key: &str) {
        self.current = Some(key.to_owned());
        self.shown = self.current.clone();
        self.loading_since = None;
    }

    /// The view on screen belongs to another squad while a switch loads.
    pub fn loading(&self) -> bool {
        self.view.is_some() && self.shown != self.current
    }

    pub(super) fn apply_home_leads(&mut self, read: super::home_leads::Read) {
        self.home_leads.replace(read);
        if std::mem::take(&mut self.home_start) {
            self.place_home_start();
        }
        self.clamp();
    }

    /// Swaps in a loaded squad in one step. A result for a squad the user
    /// already left is kept for switching back, never shown.
    pub fn apply(&mut self, snapshot: Snapshot) {
        debug_assert!(
            !snapshot.view.as_ref().is_ok_and(|view| view.home.is_some())
                || snapshot.squad.as_deref() == Some(super::ALL),
            "home data belongs to the aggregate snapshot"
        );
        if self.loading()
            && snapshot.squad == self.current
            && let Err(error) = &snapshot.view
        {
            // Keep the retained view and its tab selected after a failed switch.
            // The footer shows the failure instead of leaving a pending target.
            let now = Instant::now();
            let shown = self.shown.clone();
            if let Some(shown) = shown.as_deref() {
                self.handoff_meters(shown, now);
            }
            self.current = shown;
            self.loading_since = None;
            self.notice = None;
            self.error = Some(error.clone());
            self.project_usage(now);
            return;
        }
        if let Some(overlay) = &mut self.settings {
            overlay.squad_keys = snapshot.squad_keys.clone();
        }
        self.squad_keys = snapshot.squad_keys;

        self.invalidate_overlay_frames();
        self.tabs = snapshot.tabs;
        self.hidden = snapshot.hidden;
        // A failed inventory read has no keys, including no built-ins. Keep
        // this board's policy until an authoritative inventory returns.
        if !self.tabs.is_empty() || !self.hidden.is_empty() {
            self.picks.reconcile(&self.switchable());
        }
        self.pinned = snapshot.pinned;
        self.reconcile_switcher(false);
        self.note_cursors
            .borrow_mut()
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
        self.meters
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
        self.folds
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
        if let (Some(key), Ok(view)) = (&snapshot.squad, &snapshot.view) {
            self.remember_folds(key.clone(), &view.board);
        }
        self.attention
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
        self.attention.extend(snapshot.attention);
        if self.current.is_some() && snapshot.squad != self.current {
            if let (Some(name), Ok(view)) = (snapshot.squad, snapshot.view) {
                self.cache.insert(name, view);
            }
            self.prune_views();
            return;
        }
        self.current = snapshot.squad;
        self.loading_since = None;
        match snapshot.view {
            Ok(mut view) => {
                let now = Instant::now();
                if let Some(home) = &view.home {
                    self.home_leads.reconcile(home, view.me_id.as_deref());
                    for (name, meter) in &mut self.meters {
                        if view
                            .home_rate
                            .get(name)
                            .is_none_or(|rate| !rate.settings.enabled)
                        {
                            meter.suspend(now);
                        }
                    }
                    for (name, rate) in view
                        .home_rate
                        .iter_mut()
                        .filter(|(_, rate)| rate.settings.enabled)
                    {
                        if self.meters.get(name).is_none_or(|meter| {
                            meter.room != rate.input.room || meter.settings != rate.settings
                        }) {
                            self.meters.insert(
                                name.clone(),
                                super::meter::Meter::new(rate.settings, &rate.input, now),
                            );
                        } else if let Some(meter) = self.meters.get_mut(name) {
                            meter.retain(&rate.input);
                        }
                        if let Some(seeds) = rate.history.take() {
                            self.meters.get_mut(name).unwrap().seed(
                                &rate.input,
                                &seeds,
                                now,
                                false,
                            );
                        }
                    }
                }
                match &mut view.token_rate {
                    Some(rate) if rate.settings.enabled => {
                        if !self.window_changed {
                            self.token_window = rate.settings.window;
                        }
                        self.token_window = self.token_window.available(rate.settings.windows);
                        if let Some(meter) = self.meter.as_mut().filter(|meter| {
                            meter.room == rate.input.room && meter.settings == rate.settings
                        }) {
                            if meter.due(now) {
                                meter.sample(Ok(&rate.input), now);
                            }
                        } else {
                            self.meter =
                                Some(super::meter::Meter::new(rate.settings, &rate.input, now));
                        }
                        if let Some(seeds) = rate.history.take() {
                            let meter = self.meter.as_mut().unwrap();
                            meter.seed(&rate.input, &seeds, now, true);
                        }
                    }
                    _ => self.meter = None,
                }
                if let Some(meter) = self.meter.as_mut() {
                    meter.select(self.token_window, now);
                }
                // Help belongs to the ordinary immutable roster snapshot, not meter ticks.
                self.excluded_counters = view
                    .token_rate
                    .as_ref()
                    .zip(self.meter.as_ref())
                    .map(|(rate, meter)| {
                        meter
                            .excluded(&rate.input)
                            .into_iter()
                            .filter(|id| !rate.input.resumes[*id]["consumption"].is_object())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let panes = self
                    .view_picker
                    .as_ref()
                    .map_or(view.board.panes.len(), |picker| picker.board().panes.len());
                self.focus = self.focus.min(panes.saturating_sub(1));
                let changed = self.loading() || self.view.is_none();
                if let Some(overlay) = &mut self.settings {
                    overlay.staleness =
                        Some(crate::staleness::Snapshot::for_preview(&view.document));
                }
                self.usage_document = None;
                let previous = self.view.replace(view);
                let previous_squad = std::mem::replace(&mut self.shown, self.current.clone());
                // A result that arrived for it meanwhile is newer: keep that.
                if let (Some(previous), Some(name)) = (previous, previous_squad)
                    && Some(&name) != self.current.as_ref()
                {
                    self.cache.entry(name).or_insert(previous);
                }
                if changed {
                    self.shown_changed();
                } else {
                    self.reconcile_folds();
                }
                self.error = None;
            }
            Err(error) => {
                if self.current.as_deref() == Some(super::ALL) {
                    for meter in self.meters.values_mut() {
                        meter.suspend(Instant::now());
                    }
                }
                self.error = Some(error);
            }
        }
        self.project_usage(Instant::now());
        self.prune_views();
        if self
            .settings
            .as_ref()
            .is_some_and(|overlay| overlay.settings.context != self.shown)
        {
            self.settings = None;
            self.notice = Some("Shown tab changed; reopen settings.".into());
        }
        self.settings_preview();
        self.clamp();
    }

    fn prune_views(&mut self) {
        self.cache
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
    }

    /// Canonical indices survive selection, so hit maps and saved order share one identity.
    pub(super) fn picked_indices(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(index, key)| self.picks.contains(key).then_some(index))
            .collect()
    }

    pub(super) fn unpicked_squads(&self) -> Vec<&String> {
        self.tabs
            .iter()
            .filter(|key| !super::tabs::aggregate(key) && !self.picks.contains(key))
            .collect()
    }

    fn switch(&mut self, step: isize) -> Effect {
        let picked = self.picked_indices();
        let Some(position) = picked
            .iter()
            .position(|&index| self.current.as_ref() == self.tabs.get(index))
        else {
            return Effect::None;
        };
        let next = picked[(position as isize + step).rem_euclid(picked.len() as isize) as usize];
        self.go(self.tabs[next].clone())
    }

    /// An inventory removal cannot leave navigation on an excluded or vanished tab.
    pub(super) fn reconcile_pick_current(&mut self) -> Effect {
        let inventory = self.switchable();
        if self
            .current
            .as_ref()
            .is_some_and(|key| inventory.contains(key) && self.picks.contains(key))
            || inventory.is_empty()
        {
            return Effect::None;
        }
        let next = self
            .tabs
            .iter()
            .find(|key| self.picks.contains(key))
            .cloned()
            .or_else(|| {
                inventory
                    .iter()
                    .find(|key| key.as_str() == super::ALL)
                    .cloned()
            });
        next.map_or(Effect::None, |key| self.go(key))
    }

    fn toggle_tab_pick(&mut self, key: String) -> Effect {
        let inventory = self.switchable();
        if !inventory.contains(&key) {
            return Effect::None;
        }
        self.picks.toggle(&key, &inventory);
        self.tab_hits.borrow_mut().clear();
        self.unpicked_hit.set(None);
        self.dragging = None;
        self.invalidate_overlay_frames();
        let effect = if self.current.as_ref() == Some(&key) && !self.picks.contains(&key) {
            let position = self.tabs.iter().position(|tab| tab == &key).unwrap_or(0);
            let next = self
                .tabs
                .iter()
                .skip(position + 1)
                .chain(self.tabs.iter().take(position + 1))
                .find(|tab| self.picks.contains(tab))
                .cloned()
                .unwrap_or_else(|| super::ALL.to_owned());
            self.go(next)
        } else {
            Effect::None
        };
        self.reconcile_switcher(false);
        effect
    }

    /// Default Space is local to the switcher; an explicit pick binding replaces it.
    pub(super) fn pick_keys(&self) -> Vec<String> {
        let bindings = self.bindings();
        let keys = bindings
            .iter()
            .filter(|(_, action)| action.verb == Verb::PickTab)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        if keys.is_empty() && !bindings.contains_key("space") {
            vec!["space".into()]
        } else {
            keys
        }
    }

    pub(super) fn switcher_keys(&self) -> Vec<String> {
        if self
            .switcher
            .as_ref()
            .is_some_and(|switcher| switcher.unpicked_only)
        {
            self.unpicked_squads().into_iter().cloned().collect()
        } else {
            self.switchable()
        }
    }

    /// Shift+←/→: the current tab trades places with its neighbor, and the
    /// new order is saved. The ends do not wrap.
    fn move_current(&mut self, step: isize) -> Effect {
        let Some(from) = self
            .current
            .as_ref()
            .and_then(|current| self.tabs.iter().position(|tab| tab == current))
        else {
            return Effect::None;
        };
        let picked = self.picked_indices();
        let Some(position) = picked.iter().position(|&index| index == from) else {
            return Effect::None;
        };
        match position
            .checked_add_signed(step)
            .and_then(|index| picked.get(index))
        {
            Some(&to) => self.move_tab(from, to),
            None => Effect::None,
        }
    }

    /// Moves one tab to another position and saves the order. Pinned tabs
    /// keep `[tabs] pin`'s order, so only the others move, and never among
    /// them: a saved `order` could not change where a pin is drawn.
    fn move_tab(&mut self, from: usize, to: usize) -> Effect {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return Effect::None;
        }
        if from < self.pinned || to < self.pinned {
            return self.say("Pinned tabs keep the order in [tabs] pin.");
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        Effect::Act(Request::Reorder(self.tabs.clone()))
    }

    /// Close the current tab's meter continuity and resume the target's cached
    /// meter, including the enabled HOME meters when its view is retained.
    fn handoff_meters(&mut self, next: &str, now: Instant) {
        if self.current.as_deref() == Some(super::ALL) {
            for meter in self.meters.values_mut() {
                meter.suspend(now);
            }
        }
        if let (Some(key), Some(mut meter)) = (self.current.as_ref(), self.meter.take()) {
            meter.suspend(now);
            if self.tabs.contains(key) || self.hidden.contains(key) {
                self.meters.insert(key.clone(), meter);
            }
        }
        // A cached view is not a fresh counter receipt for another squad.
        self.meter = self.meters.remove(next);
        if let Some(meter) = self.meter.as_mut() {
            meter.resume(self.token_window, now);
        }
        if next == super::ALL {
            let home = if self.shown.as_deref() == Some(next) {
                self.view.as_ref()
            } else {
                self.cache.get(next)
            };
            if let Some(home) = home {
                for (name, meter) in &mut self.meters {
                    if home
                        .home_rate
                        .get(name)
                        .is_some_and(|rate| rate.settings.enabled)
                    {
                        meter.resume(self.token_window, now);
                    }
                }
            }
        }
    }

    /// Shows the tab `next`, from the cache at once when it was visited.
    pub(super) fn go(&mut self, next: String) -> Effect {
        self.picks.include(&next);
        if Some(&next) == self.current.as_ref() {
            return Effect::None;
        }
        self.handoff_meters(&next, Instant::now());
        if self.shown.as_deref() == Some(super::ALL) && self.home_target.is_some() {
            self.home_left = self.home_target.clone();
        }
        self.current = Some(next.clone());
        self.error = None;
        self.jobs_focus = false;
        self.menu = None;
        self.loading_since = Some(Instant::now());
        // Never blank the screen: a visited squad shows from the cache at
        // once; otherwise the current frame stays until the new one arrives.
        if let Some(cached) = self.cache.remove(&next) {
            self.loading_since = None;
            let previous = self.view.replace(cached);
            if let (Some(previous), Some(name)) = (previous, self.shown.replace(next.clone())) {
                self.cache.insert(name, previous);
            }
            self.usage_document = None;
            self.project_usage(Instant::now());
            self.shown_changed();
        }
        Effect::Load(next)
    }

    fn remember_folds(&mut self, key: String, board: &Board) {
        if self.view_picker.is_some() || (!self.tabs.contains(&key) && !self.hidden.contains(&key))
        {
            return;
        }
        if self
            .folds
            .get(&key)
            .is_none_or(|state| state.board != *board)
        {
            self.folds.insert(
                key,
                FoldState {
                    board: board.clone(),
                    overrides: BTreeMap::new(),
                },
            );
        }
    }

    fn reconcile_folds(&mut self) {
        if let (Some(key), Some(view)) = (&self.shown, &self.view) {
            let key = key.clone();
            let board = view.board.clone();
            self.remember_folds(key, &board);
        }
        self.restore_focus();
    }

    pub(super) fn open_settings(&mut self, config: Config) -> Result<(), crate::core::SquadError> {
        let mut overlay =
            super::settings::Overlay::open(config, self.shown_tab(), self.selected_section())?;
        overlay.opening_focus = self.focus;
        overlay.squad_keys = self.squad_keys.clone();
        overlay.staleness = self
            .view
            .as_ref()
            .map(|view| crate::staleness::Snapshot::for_preview(&view.document));
        self.settings = Some(overlay);
        self.help = false;
        Ok(())
    }

    /// Apply disposable presentation to the newest acquired data; the opening Config restores it on cancel.
    pub(super) fn settings_preview(&mut self) {
        let Some(overlay) = &self.settings else {
            return;
        };
        let Some(config) = overlay.config() else {
            return;
        };
        let key = overlay.settings.context.as_deref().unwrap_or("");
        let policy = config.tabs().expect("validated settings draft");
        (self.tabs, self.pinned) = if overlay.squad_keys.is_empty() && policy.user.is_empty() {
            (Vec::new(), 0)
        } else {
            crate::tabs::arrange(&overlay.squad_keys, &policy)
        };
        self.hidden = if overlay.squad_keys.is_empty() && policy.user.is_empty() {
            Vec::new()
        } else {
            overlay
                .squad_keys
                .iter()
                .cloned()
                .chain([crate::tabs::LEADS.to_owned(), crate::tabs::ALL.to_owned()])
                .chain(
                    policy
                        .user
                        .iter()
                        .map(|tab| crate::tabs::user_key(&tab.name)),
                )
                .filter(|key| !self.tabs.contains(key))
                .collect()
        };
        if let Some(view) = &mut self.view {
            view.refresh = config.refresh(key).expect("validated settings draft");
            view.home_replies = config.home_replies().expect("validated settings draft");
            if !crate::tabs::aggregate(key) {
                view.board = config.board(key).expect("validated settings draft");
                view.bindings = config
                    .bindings_for_tab(key, overlay.settings.host == "tmux", &view.board.panes)
                    .expect("validated settings draft");
                view.rows = config.rows(key).expect("validated settings draft");
                view.render = config.notes_render(key).expect("validated settings draft");
                let states = config
                    .states(key, config.layout(key).unwrap())
                    .expect("validated settings draft");
                crate::display_rows::each_row(&mut view.document, |row| {
                    let color = states.color(row["state"].as_str()).map(str::to_owned);
                    if let Some(colors) = row["colors"].as_object_mut() {
                        colors.remove("state");
                    }
                    if let Some(color) = color {
                        if !row["colors"].is_object() {
                            row["colors"] = serde_json::json!({});
                        }
                        row["colors"]["state"] = color.into();
                    }
                });
                if let Some(snapshot) = &overlay.staleness {
                    snapshot.apply_preview(
                        &mut view.document,
                        config.reminders(key).expect("validated settings draft"),
                    );
                }
                *view.derived.borrow_mut() = Default::default();
            }
        }
        if overlay.draft.is_none() {
            self.focus = overlay.opening_focus;
        }
        self.restore_focus();
    }

    /// All geometry consumers see the in-memory draft while the picker is open.
    pub fn effective_board(&self) -> Option<&Board> {
        self.view_picker
            .as_ref()
            .map(|picker| picker.board())
            .or_else(|| self.view.as_ref().map(|view| &view.board))
    }

    pub fn open_view_picker(&mut self, config: Config) -> Result<(), crate::core::SquadError> {
        let Some(board) = self.effective_board().cloned() else {
            return Ok(());
        };
        let squad = self
            .current
            .clone()
            .filter(|name| !super::tabs::aggregate(name));
        self.view_picker = Some(super::view_picker::Picker::open(
            config, squad, board, self.focus,
        )?);
        self.help = false;
        Ok(())
    }

    pub fn close_view_picker(&mut self, saved: bool) {
        if let Some(picker) = self.view_picker.take() {
            let board = if saved {
                picker.board().clone()
            } else {
                picker.opening.clone()
            };
            if let Some(view) = &mut self.view {
                view.board = board;
            }
            if saved {
                self.reconcile_folds();
            } else {
                self.focus = picker.opening_focus;
                self.restore_focus();
            }
        }
    }

    /// Resolve immutable defaults and user overrides for the board body width.
    pub fn folds_at(&self, width: u16) -> BTreeSet<Pane> {
        let Some(board) = self.effective_board() else {
            return BTreeSet::new();
        };
        let mut collapsed = board.collapsed.clone();
        if let Some(rule) = &board.fold_below
            && width < rule.width
        {
            collapsed.extend(&rule.panes);
        }
        if let Some(state) = self.shown.as_ref().and_then(|key| self.folds.get(key)) {
            for (pane, folded) in &state.overrides {
                if *folded {
                    collapsed.insert(*pane);
                } else {
                    collapsed.remove(pane);
                }
            }
        }
        collapsed
    }

    pub fn collapsed_panes(&self) -> BTreeSet<Pane> {
        self.folds_at(self.body_width)
    }

    pub fn set_body_width(&mut self, width: u16) {
        self.body_width = width;
        self.restore_focus();
    }

    pub fn focused_pane(&self) -> Option<Pane> {
        self.effective_board()
            .and_then(|board| board.panes.get(self.focus).copied())
            .filter(|pane| !self.collapsed_panes().contains(pane) && !self.jobs_focus)
    }

    pub fn focused(&self) -> Pane {
        self.effective_board()
            .and_then(|board| board.panes.get(self.focus).copied())
            .unwrap_or(Pane::Rows)
    }

    /// Focuses the pane drawn under the pointer, if any.
    fn focus_at(&mut self, column: u16, row: u16) {
        let Some(pane) = self
            .scrolls
            .pane_at(column, row)
            .filter(|pane| !self.collapsed_panes().contains(pane))
        else {
            return;
        };
        if let Some(position) = self
            .effective_board()
            .and_then(|board| board.panes.iter().position(|p| *p == pane))
        {
            self.focus = position;
        }
    }

    /// A hidden focus returns to rows; boards without visible rows use the next pane.
    fn restore_focus(&mut self) {
        if self.jobs_focus || self.focused_pane().is_some() {
            return;
        }
        if let Some(position) = self
            .effective_board()
            .and_then(|board| board.panes.iter().position(|pane| *pane == Pane::Rows))
            && !self.collapsed_panes().contains(&Pane::Rows)
        {
            self.focus = position;
        } else {
            self.next_pane();
        }
    }

    fn next_pane(&mut self) {
        if self.jobs_focus {
            self.jobs_leave(false);
            return;
        }
        if let Some(board) = self.effective_board() {
            let collapsed = self.collapsed_panes();
            let count = board.panes.len();
            // After the last visible pane, Tab enters the jobs half when it is drawn.
            if self.jobs_painted()
                && !(self.focus + 1..count).any(|index| !collapsed.contains(&board.panes[index]))
            {
                self.jobs_focus = true;
                return;
            }
            if let Some(next) = (1..=count)
                .map(|step| (self.focus + step) % count)
                .find(|index| !collapsed.contains(&board.panes[*index]))
            {
                self.focus = next;
            }
        }
    }

    /// Leaves the jobs half for the first (or, backwards, last) visible pane.
    pub(super) fn jobs_leave(&mut self, backwards: bool) {
        self.jobs_focus = false;
        let collapsed = self.collapsed_panes();
        let visible = |pane: &Pane| !collapsed.contains(pane);
        let target = self.effective_board().and_then(|board| {
            if backwards {
                board.panes.iter().rposition(visible)
            } else {
                board.panes.iter().position(visible)
            }
        });
        if let Some(target) = target {
            self.focus = target;
        }
    }

    fn toggle_panes(&mut self, panes: &[Pane]) -> Effect {
        self.jobs_focus = false;
        if self.loading() {
            return self.say(format!(
                "Loading {}…",
                self.current.clone().unwrap_or_default()
            ));
        }
        let Some(board) = self.effective_board().cloned() else {
            return Effect::None;
        };
        if board.mode != BoardMode::Split {
            return self.say("toggle applies to split mode only.");
        }
        let panes: Vec<_> = panes
            .iter()
            .filter(|pane| board.panes.contains(pane))
            .copied()
            .collect();
        if panes.is_empty() {
            return Effect::None;
        }
        let position = board
            .panes
            .iter()
            .position(|pane| Some(pane) == panes.first())
            .expect("validated toggle panes");
        let had_focus = self.focused_pane().is_some();
        let Some(key) = self.shown.clone() else {
            return Effect::None;
        };
        self.remember_folds(key.clone(), &board);
        let collapsed = self.collapsed_panes();
        let expanding = panes.iter().all(|pane| collapsed.contains(pane));
        let Some(state) = self.folds.get_mut(&key) else {
            return Effect::None;
        };
        for pane in &panes {
            state.overrides.insert(*pane, !expanding);
        }
        if expanding && !had_focus {
            self.focus = position;
        } else {
            self.restore_focus();
        }
        self.last_click = None;
        Effect::None
    }

    pub(super) fn selected_section(&self) -> Option<usize> {
        self.rows()
            .get(self.selected)
            .and_then(|(slot, _)| slot.section())
    }

    /// The selected row's section overrides its tab/global/host bindings.
    pub fn bindings(&self) -> Bindings {
        let Some(view) = &self.view else {
            return Bindings::new();
        };
        let section = self
            .rows()
            .get(self.selected)
            .and_then(|(slot, _)| view.section_bindings.get(slot.section()?));
        let enabled = view
            .token_rate
            .as_ref()
            .is_some_and(|rate| rate.settings.enabled)
            || self
                .meter
                .as_ref()
                .is_some_and(|meter| meter.settings.enabled);
        crate::action::effective_bindings(view.bindings.clone(), section, enabled)
    }

    pub(super) fn say(&mut self, notice: impl Into<String>) -> Effect {
        self.notice = Some(notice.into());
        Effect::None
    }

    /// Resolves an action against the selected row. Missing values refuse
    /// the action with a notice; nothing runs half-filled. While a switch
    /// loads, the rows on screen are another squad's, so nothing acts on them.
    pub fn perform(&mut self, action: &Action) -> Effect {
        if action.verb == Verb::PickTab {
            self.switcher = Some(Switcher::default());
            self.reconcile_switcher(false);
            return Effect::None;
        }
        if self.loading() && !matches!(action.verb, Verb::NextPane | Verb::Refresh | Verb::Notes) {
            let loading = self.current.clone().unwrap_or_default();
            return self.say(format!("Loading {loading}…"));
        }
        if self.view.as_ref().is_some_and(|view| view.home.is_some()) {
            match action.verb {
                Verb::Tab | Verb::Jump
                    if action.args.first().and_then(|arg| arg.literal()) != Some("lead") =>
                {
                    return self.home_enter();
                }
                Verb::Annotate => return self.home_answer(),
                _ => {}
            }
        }
        if action.verb == Verb::Annotate && self.focused_pane() == Some(Pane::Notes) {
            return self.annotate_note();
        }
        match action.verb {
            Verb::Toggle => {
                let panes: Vec<_> = action
                    .args
                    .iter()
                    .map(|arg| {
                        arg.literal()
                            .and_then(Pane::parse)
                            .expect("validated toggle pane")
                    })
                    .collect();
                return self.toggle_panes(&panes);
            }
            Verb::TokenWindow => {
                if let Some(meter) = self.meter.as_mut() {
                    self.token_window = self.token_window.next(meter.settings.windows);
                    self.window_changed = true;
                    let now = Instant::now();
                    meter.select(self.token_window, now);
                    self.project_usage(now);
                    self.notice = Some(format!("Token window: {}", self.token_window.label()));
                }
                return Effect::None;
            }
            Verb::HomeReplies => {
                return self
                    .view
                    .as_ref()
                    .filter(|view| view.home.is_some())
                    .map_or(Effect::None, |view| Effect::HomeReplies(!view.home_replies));
            }
            Verb::HomeMessage => return self.home_expand(),
            Verb::HomeWrite => return self.home_write(),
            Verb::HomePick => return self.home_pick(),
            Verb::AskLead => return self.ask_lead(),
            Verb::Settings => return Effect::Settings,
            Verb::Theme => return Effect::PickTheme,
            Verb::View => return Effect::PickView,
            Verb::NextPane => {
                self.next_pane();
                return Effect::None;
            }
            Verb::Refresh => {
                self.notice = Some("Refreshing…".into());
                return Effect::Refresh;
            }
            Verb::Notes => {
                let position = self
                    .effective_board()
                    .and_then(|board| board.panes.iter().position(|p| *p == Pane::Notes));
                return match position {
                    Some(position) => {
                        if self.loading() && self.collapsed_panes().contains(&Pane::Notes) {
                            return self.say(format!(
                                "Loading {}…",
                                self.current.clone().unwrap_or_default()
                            ));
                        }
                        if self.collapsed_panes().contains(&Pane::Notes) {
                            self.toggle_panes(&[Pane::Notes]);
                        }
                        self.jobs_focus = false;
                        self.focus = position;
                        Effect::None
                    }
                    None => self.say("The notes pane is not on this board; add it to panes."),
                };
            }
            Verb::Back => return Effect::Act(Request::Back),
            Verb::Jump if action.args.first().and_then(|arg| arg.literal()) == Some("lead") => {
                return match self.lead() {
                    Ok(lead) => Effect::Act(Request::Jump(lead)),
                    Err(reason) => self.say(format!("jump lead: {reason}.")),
                };
            }
            _ => {}
        }
        if self.jobs_focus && action.verb.acts_on_member() {
            return self.say("Tab returns to the members; this key acts on a member row.");
        }
        let Some(row) = self.selected_row().cloned() else {
            return self.say("No row is selected.");
        };
        let view = self.view.as_ref().expect("a selected row has a view");
        let request = match action.verb {
            Verb::Talk | Verb::Annotate | Verb::Reply => return self.compose(action, &row),
            Verb::Menu => {
                let mut entries: Vec<MenuEntry> = self
                    .bindings()
                    .into_iter()
                    .filter(|(event, action)| {
                        !matches!(event.as_str(), "click" | "double-click")
                            && !matches!(
                                action.verb,
                                Verb::Menu | Verb::NextPane | Verb::TokenWindow
                            )
                    })
                    .map(|(key, action)| MenuEntry {
                        key,
                        label: action.text.clone(),
                        choice: Choice::Action(action),
                    })
                    .collect();
                // Stable: bindings of one verb keep their key order.
                entries.sort_by_key(|entry| match &entry.choice {
                    Choice::Action(action) => action.order(),
                    _ => u8::MAX,
                });
                self.menu = Some(Menu {
                    row_send: None,
                    link: None,
                    prefill: String::new(),
                    title: row["name"].as_str().unwrap_or_default().to_owned(),
                    entries,
                    selected: 0,
                    surface: Default::default(),
                });
                return Effect::None;
            }
            Verb::Tab => {
                return match row["squad"].as_str() {
                    Some(squad) => self.go(squad.to_owned()),
                    None => self.say("tab: this row has no squad."),
                };
            }
            Verb::Jump => match row["name"].as_str() {
                Some(name) => Ok(Request::Jump(name.to_owned())),
                None => Err("This row has no member name.".to_owned()),
            },
            Verb::Open => match action.args.first() {
                Some(template) => template.fill(&row),
                None => effects::default_link(&row)
                    .map(str::to_owned)
                    .ok_or_else(|| "This row has no link field.".to_owned()),
            }
            .and_then(|link| effects::web_link(&link).map(str::to_owned))
            .map(|link| Request::Open {
                link,
                opener: view.opener.clone(),
            }),
            Verb::Copy => action.args[0].fill(&row).map(|text| Request::Copy {
                text,
                program: view.clipboard.clone(),
            }),
            Verb::Run => action.argv(&row).map(Request::Run),
            _ => unreachable!("handled above"),
        };
        match request {
            Ok(request) => Effect::Act(request),
            Err(reason) => self.say(format!("{}: {reason}.", action.verb.name())),
        }
    }

    /// The lead `jump lead` goes to: the squad's own lead on its tab; on the
    /// leads and all tabs, which list several squads, the selected row's
    /// squad's lead.
    fn lead(&self) -> Result<String, String> {
        let view = self.view.as_ref().ok_or("the board has not loaded yet")?;
        let named = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let lead = match self.current.as_deref() {
            Some(super::LEADS) => named(self.selected_row().and_then(|row| row["name"].as_str())),
            Some(super::ALL) if view.home.is_some() => named(
                self.home_entries()
                    .get(self.selected)
                    .and_then(|entry| entry.lead),
            ),
            Some(super::ALL) => named(
                self.selected_row()
                    .and_then(|row| row["fields"]["lead"].as_str()),
            ),
            _ => named(view.document["squad"]["lead"]["name"].as_str()),
        };
        lead.ok_or_else(|| match self.current.as_deref() {
            Some(super::LEADS | super::ALL) if self.selected_row().is_none() => {
                "no row is selected".to_owned()
            }
            _ => "this squad has no lead; set one with tmt squad lead <name>".to_owned(),
        })
    }

    fn ask_lead(&mut self) -> Effect {
        if self.current.as_deref().is_none_or(crate::tabs::aggregate) {
            return self.say("Ask the lead from the squad's own tab.");
        }
        let Some(view) = &self.view else {
            return Effect::None;
        };
        let Some(sender) = view.me.clone() else {
            return self.say("Who is sending? Record yourself with tmt squad me <name>.");
        };
        let to = match self.lead() {
            Ok(to) => to,
            Err(reason) => return self.say(format!("ask lead: {reason}.")),
        };
        let text = view.ask_lead.clone();
        let squad = self.current.clone().unwrap();
        self.ask(
            format!("→ lead {to}"),
            Compose::AskLead { to, sender },
            squad,
        );
        self.input.as_mut().unwrap().text = text;
        Effect::None
    }

    pub(super) fn ask(&mut self, prompt: String, compose: Compose, squad: String) -> Effect {
        self.input = Some(Input {
            row_send: None,
            alternative: None,
            quote: None,
            link: None,
            prompt,
            text: String::new(),
            compose,
            squad,
            hint: None,
        });
        Effect::None
    }

    /// Stable occurrence used by composer placement, validation and feedback.
    pub(super) fn row_target(&self, index: usize) -> Option<RowTarget> {
        if self.view.as_ref()?.home.is_some() {
            return self
                .home_entries()
                .get(index)
                .map(|entry| RowTarget::Home(entry.target.clone()));
        }
        let (slot, row) = *self.rows().get(index)?;
        let tab = self.shown_tab()?.to_owned();
        let squad = row["squad"].as_str().unwrap_or(&tab).to_owned();
        let id = row["id"].as_str()?.to_owned();
        Some(match slot {
            RowOrigin::Lead => RowTarget::Lead { tab, squad, id },
            RowOrigin::Section(section) => RowTarget::Member {
                squad,
                tab,
                section,
                id,
            },
        })
    }

    fn target_row(&self, target: &RowTarget) -> Option<&Value> {
        match target {
            RowTarget::Home(target) => self
                .home_entries()
                .into_iter()
                .find(|entry| &entry.target == target)
                .map(|entry| entry.row),
            RowTarget::Member {
                tab,
                section,
                squad,
                id,
            } if self.shown_tab() == Some(tab) => self
                .rows()
                .into_iter()
                .find(|(slot, row)| {
                    *slot == RowOrigin::Section(*section)
                        && row["id"].as_str() == Some(id)
                        && row["squad"].as_str().unwrap_or(tab) == squad
                })
                .map(|(_, row)| row),
            RowTarget::Lead { tab, id, .. } if self.shown_tab() == Some(tab) => {
                let lead = &self.view.as_ref()?.document["squad"]["lead"];
                (lead["id"].as_str() == Some(id)).then_some(lead)
            }
            _ => None,
        }
    }

    fn note_recipient(&self, target: &RowTarget) -> Option<String> {
        match target {
            RowTarget::Home(target) => self
                .home_entries()
                .into_iter()
                .find(|entry| &entry.target == target)
                .and_then(|entry| entry.lead.map(str::to_owned)),
            RowTarget::Member { tab, .. } if tab == super::LEADS => {
                self.target_row(target)?["name"].as_str().map(str::to_owned)
            }
            RowTarget::Member { tab, .. } | RowTarget::Lead { tab, .. }
                if !super::tabs::aggregate(tab) =>
            {
                self.view.as_ref()?.document["squad"]["lead"]["name"]
                    .as_str()
                    .map(str::to_owned)
            }
            _ => None,
        }
    }

    pub(super) fn row_send(&self, index: usize, member_note: bool) -> Option<RowSend> {
        let target = self.row_target(index)?;
        self.send_target(target, member_note)
    }

    fn send_target(&self, target: RowTarget, member_note: bool) -> Option<RowSend> {
        let sender = self.view.as_ref()?.me.clone()?;
        let name = self.target_row(&target)?["name"].as_str()?.to_owned();
        let to = if member_note {
            Some(name.clone())
        } else {
            self.note_recipient(&target)
        };
        Some(RowSend {
            target,
            sender,
            name: name.clone(),
            note_member: member_note,
            note: to.map(|to| Compose::Annotate { to, row: name }),
        })
    }

    /// One composer for row answers and notes; several requests still need a choice.
    pub(super) fn compose_row(&mut self, mut send: RowSend, verb: Verb, squad: String) -> Effect {
        let row = self.target_row(&send.target).expect("opening row exists");
        let pending = row["pending"].as_str().is_some_and(|text| !text.is_empty());
        let open: Vec<MenuEntry> = row["waitingOnYou"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, item)| {
                Some(MenuEntry {
                    key: (index + 1).to_string(),
                    label: super::notes::sanitize(
                        item["preview"].as_str().unwrap_or("(question unavailable)"),
                    ),
                    choice: Choice::Reply {
                        request: item["requestId"].as_str()?.to_owned(),
                        from: send.name.clone(),
                    },
                })
            })
            .collect();
        if verb != Verb::Talk && !(verb == Verb::Annotate && send.note_member) && !open.is_empty() {
            if open.len() > 1 {
                self.menu = Some(Menu {
                    row_send: Some(send.clone()),
                    link: None,
                    prefill: String::new(),
                    title: format!("answer {}", send.name),
                    entries: open,
                    selected: 0,
                    surface: Default::default(),
                });
                return Effect::None;
            }
            let effect = self.choose(open[0].choice.clone());
            self.attach_row(send, squad);
            return effect;
        }
        let compose = match verb {
            Verb::Talk => Compose::Talk {
                to: send.name.clone(),
            },
            // A decision owed without a request to answer: write the member a
            // note. Opening sends, clears and acknowledges nothing.
            Verb::Reply if pending => {
                let note = Compose::Annotate {
                    to: send.name.clone(),
                    row: send.name.clone(),
                };
                send.note = Some(note.clone());
                send.note_member = true;
                note
            }
            Verb::Reply => return self.say(format!("{} is not waiting on you.", send.name)),
            _ => match send.note.clone() {
                Some(note) => note,
                None => return self.say(format!("This squad has no lead; set one with tmt squad lead <name> --squad {squad}, or use annotate member.")),
            },
        };
        let effect = self.ask(String::new(), compose, squad.clone());
        self.attach_row(send, squad);
        effect
    }

    fn compose(&mut self, action: &Action, row: &Value) -> Effect {
        if self
            .view
            .as_ref()
            .and_then(|view| view.me.as_ref())
            .is_none()
        {
            return self.say("Who is sending? Record yourself with tmt squad me <name>, or open the board from your named pane.");
        }
        let rows = self.rows();
        let index = rows
            .get(self.selected)
            .filter(|(_, candidate)| *candidate == row)
            .map(|_| self.selected)
            .or_else(|| rows.iter().position(|(_, candidate)| *candidate == row));
        let target = index.and_then(|index| self.row_target(index)).or_else(|| {
            let view = self.view.as_ref()?;
            (row == &view.document["squad"]["lead"])
                .then(|| {
                    Some(RowTarget::Lead {
                        tab: self.shown_tab()?.to_owned(),
                        squad: self.current.clone()?,
                        id: row["id"].as_str()?.to_owned(),
                    })
                })
                .flatten()
        });
        let Some(send) = target.and_then(|target| {
            self.send_target(
                target,
                action.args.first().and_then(|arg| arg.literal()) == Some("member"),
            )
        }) else {
            return self.say("This row has no stable member target.");
        };
        let squad = row["squad"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| self.current.clone())
            .unwrap_or_default();
        self.compose_row(send, action.verb, squad)
    }

    fn choose(&mut self, choice: Choice) -> Effect {
        match choice {
            Choice::Action(action) => self.perform(&action),
            Choice::Cron(request) => Effect::Act(Request::Cron(request)),
            Choice::Dismiss => Effect::None,
            Choice::Leads { recipients, all } => self.compose_leads(recipients, all),
            Choice::Reply { request, from } => self.ask(
                format!("→ {from}"),
                Compose::Reply { request, from },
                self.current.clone().unwrap_or_default(),
            ),
        }
    }

    fn attach_row(&mut self, send: RowSend, squad: String) {
        let quote = self.input.as_ref().and_then(|input| match &input.compose {
            Compose::Reply { request, .. } => self.target_row(&send.target)?["waitingOnYou"]
                .as_array()?
                .iter()
                .find(|item| item["requestId"].as_str() == Some(request))
                .map(|item| {
                    super::notes::sanitize(
                        item["preview"].as_str().unwrap_or("(question unavailable)"),
                    )
                }),
            _ => None,
        });
        if let Some(input) = &mut self.input {
            input.squad = squad;
            input.alternative = matches!(input.compose, Compose::Reply { .. })
                .then(|| send.note.clone())
                .flatten();
            input.quote = quote;
            input.row_send = Some(send);
            input.prompt = input.header();
        }
        self.follow = true;
    }

    fn annotate_note(&mut self) -> Effect {
        let Some(view) = &self.view else {
            return Effect::None;
        };
        let Notes::Text(text) = &view.notes else {
            return self.say("No notebook line to annotate.");
        };
        if view.me.is_none() {
            return self.say("Record yourself with tmt squad me <name> before annotating.");
        }
        let Some(to) = view.document["squad"]["lead"]["name"]
            .as_str()
            .map(str::to_owned)
        else {
            return self.say("This squad has no lead to annotate for.");
        };
        let squad = self.current.clone().unwrap_or_default();
        let (row, prompt) = {
            let mut cursors = self.note_cursors.borrow_mut();
            let cursor = cursors.entry(squad.clone()).or_default();
            cursor.reconcile(text);
            let source = text.split('\n').nth(cursor.source).unwrap_or_default();
            (
                crate::requests::note_row(&squad, cursor.source, source),
                format!(
                    "note for {to} · L{} “{}”",
                    cursor.source + 1,
                    super::notes::display_quote(source)
                ),
            )
        };
        self.ask(prompt, Compose::Annotate { to, row }, squad)
    }

    fn link_member(&self, name: &str) -> Option<&Value> {
        self.view.as_ref()?.document["sections"]
            .as_array()?
            .iter()
            .flat_map(|section| section["rows"].as_array().into_iter().flatten())
            .find(|row| row["name"].as_str() == Some(name) || row["id"].as_str() == Some(name))
            .or_else(|| {
                let lead = &self.view.as_ref()?.document["squad"]["lead"];
                (lead["name"].as_str() == Some(name) || lead["id"].as_str() == Some(name))
                    .then_some(lead)
            })
    }

    pub(super) fn selected_link(&self) -> Option<super::markdown::Link> {
        let (target, offset) = self.note_link.as_ref()?;
        self.view
            .as_ref()?
            .derived
            .borrow()
            .notes
            .as_ref()?
            .links
            .iter()
            .find(|link| &link.target == target && &link.offset == offset)
            .cloned()
    }

    fn note_binding(&self, key: KeyEvent) -> Option<Action> {
        let view = self.view.as_ref()?;
        let event = event_name(key)?;
        self.rows()
            .get(self.selected)
            .and_then(|(slot, _)| view.section_bindings.get(slot.section()?))
            .and_then(|bindings| bindings.get(&event))
            .or_else(|| view.configured_bindings.get(&event))
            .cloned()
    }

    fn choose_link(&mut self, id: usize) -> bool {
        let Some(view) = &self.view else {
            return false;
        };
        let derived = view.derived.borrow();
        let Some(notes) = &derived.notes else {
            return false;
        };
        let Some(link) = notes.links.get(id) else {
            return false;
        };
        self.note_link = Some((link.target.clone(), link.offset));
        self.notice = None;
        if let (Some(key), Notes::Text(text), Some(hit)) = (
            self.shown_tab(),
            &view.notes,
            notes.hits.iter().find(|hit| hit.link == id),
        ) {
            let mut cursors = self.note_cursors.borrow_mut();
            let cursor = cursors.entry(key.to_owned()).or_default();
            cursor.select_visual(text, &notes.sources, hit.line);
            cursor.follow = true;
        }
        true
    }

    fn select_link(&mut self, step: isize) -> Effect {
        let state = self.view.as_ref().and_then(|view| {
            view.derived.borrow().notes.as_ref().map(|notes| {
                (
                    notes.links.len(),
                    notes.links.iter().position(|link| {
                        self.note_link.as_ref() == Some(&(link.target.clone(), link.offset))
                    }),
                )
            })
        });
        let Some((len, old)) = state else {
            return Effect::None;
        };
        if len == 0 {
            self.note_link = None;
            self.next_pane();
            return Effect::None;
        }
        let next = old.map_or(if step < 0 { len - 1 } else { 0 }, |old| {
            (old as isize + step).rem_euclid(len as isize) as usize
        });
        self.choose_link(next);
        Effect::None
    }

    fn activate_link(&mut self) -> Effect {
        if self.loading() {
            return self.say("The board is loading; link refused.");
        }
        let Some(link) = self.selected_link() else {
            return Effect::None;
        };
        let view = self.view.as_ref().expect("rendered link has a view");
        match link.kind {
            Kind::Web | Kind::Github => Effect::Act(Request::Open {
                link: link.target,
                opener: view.opener.clone(),
            }),
            Kind::File(path) => Effect::Act(Request::RevealFile {
                path,
                opener: view.opener.clone(),
            }),
            Kind::Custom { scheme, path } => {
                match crate::links::argv(&view.links, &scheme, &path) {
                    Ok(argv) => Effect::Act(Request::Run(argv)),
                    Err(reason) => self.say(reason),
                }
            }
            Kind::Tmt {
                verb: Verb::Back, ..
            } => Effect::Act(Request::Back),
            Kind::Tmt {
                verb,
                member: Some(member),
                text,
            } => {
                let Some(row) = self.link_member(&member).cloned() else {
                    return self.say("Link target is not a current squad member.");
                };
                let member = row["name"].as_str().unwrap_or_default().to_owned();
                match verb {
                    Verb::Jump => Effect::Act(Request::Jump(member)),
                    Verb::Copy => match Action::parse("copy").unwrap().args[0].fill(&row) {
                        Ok(text) => Effect::Act(Request::Copy {
                            text,
                            program: view.clipboard.clone(),
                        }),
                        Err(reason) => self.say(reason),
                    },
                    Verb::Open => match effects::default_link(&row)
                        .ok_or_else(|| "This member has no link.".to_owned())
                        .and_then(effects::web_link)
                    {
                        Ok(link) => Effect::Act(Request::Open {
                            link: link.to_owned(),
                            opener: view.opener.clone(),
                        }),
                        Err(reason) => self.say(reason),
                    },
                    Verb::Talk | Verb::Reply | Verb::Annotate => {
                        let sender = view.me.clone().unwrap_or_default();
                        let action = Action::parse(verb.name()).expect("built-in verb");
                        let effect = self.compose(&action, &row);
                        if let Some(input) = &mut self.input {
                            input.text = text.clone();
                            input.link = Some(LinkSend {
                                member: member.clone(),
                                sender: sender.clone(),
                            });
                        }
                        if let Some(menu) = &mut self.menu {
                            menu.prefill = text;
                            menu.link = Some(LinkSend { member, sender });
                        }
                        effect
                    }
                    _ => self.say("Unsupported link verb."),
                }
            }
            _ => self.say("Invalid link target."),
        }
    }

    fn move_note(&self, step: Step) {
        let (Some(view), Some(key)) = (&self.view, self.shown_tab()) else {
            return;
        };
        let Notes::Text(text) = &view.notes else {
            return;
        };
        let derived = view.derived.borrow();
        let sources = derived
            .notes
            .as_ref()
            .map_or(&[][..], |notes| notes.sources.as_slice());
        self.note_cursors
            .borrow_mut()
            .entry(key.to_owned())
            .or_default()
            .move_by(text, sources, step, self.scrolls.page_lines(Pane::Notes));
    }

    fn input_key(&mut self, key: KeyEvent) -> Effect {
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::ReadLead { .. }))
        {
            return self.message_key(key);
        }
        let Some(input) = &mut self.input else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Esc => {
                let cron = matches!(input.compose, Compose::Cron);
                self.input = None;
                if cron {
                    self.cron_draft = None;
                    return self.say("Cancelled; nothing changed.");
                }
                return self.say("Nothing sent.");
            }
            KeyCode::Tab => {
                if let Some(alternative) = input.alternative.take() {
                    input.alternative = Some(std::mem::replace(&mut input.compose, alternative));
                    input.prompt = input.header();
                }
                return Effect::None;
            }
            KeyCode::Enter => {}
            KeyCode::Backspace => {
                input.text.pop();
                return Effect::None;
            }
            KeyCode::Char(character)
                if !character.is_control()
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && input.text.chars().count() < INPUT_LIMIT =>
            {
                input.text.push(character);
                return Effect::None;
            }
            _ => return Effect::None,
        }
        let input = self.input.take().expect("composing");
        if matches!(input.compose, Compose::Cron) {
            // A job message is stored exactly as typed: no trim, unlike talk.
            return self.cron_submit(input.text);
        }
        let text = input.text.trim().to_owned();
        if input
            .row_send
            .as_ref()
            .is_some_and(|send| !send.valid(self, &input))
        {
            return self.say("The row target, sender, lead or request changed; nothing sent.");
        }
        if let Compose::AskLead { to, sender } = &input.compose
            && (self.loading()
                || self.current.as_deref() != Some(&input.squad)
                || self.lead().as_ref() != Ok(to)
                || self.view.as_ref().and_then(|view| view.me.as_ref()) != Some(sender))
        {
            return self.say("The squad, sender or lead changed; nothing sent.");
        }
        if let Some(LinkSend { member, sender }) = &input.link {
            let row = self.link_member(member);
            let valid = self.view.as_ref().and_then(|view| view.me.as_ref()) == Some(sender)
                && !self.loading()
                && self.current.as_deref() == Some(&input.squad)
                && row.is_some_and(|row| match &input.compose {
                    Compose::AskLead { .. } => false,
                    Compose::Talk { to } => to == member,
                    Compose::Annotate { to, .. } => self.lead().as_ref() == Ok(to),
                    Compose::Cron | Compose::ReadLead { .. } | Compose::Leads { .. } => false,
                    Compose::Reply { request, from } => {
                        from == member
                            && row["waitingOnYou"].as_array().is_some_and(|items| {
                                items
                                    .iter()
                                    .any(|item| item["requestId"].as_str() == Some(request))
                            })
                    }
                });
            if !valid {
                return self.say("The link target or request changed; nothing sent.");
            }
        }
        let squad = input.squad;
        let Some(me) = self.view.as_ref().and_then(|view| view.me.clone()) else {
            return Effect::None;
        };
        if text.is_empty() {
            return self.say("Nothing sent.");
        }
        self.pending_send = input.row_send.map(|send| {
            let home = matches!(&send.target, RowTarget::Home(target) if target.member.is_some())
                .then(|| {
                    self.home_entries()
                        .iter()
                        .enumerate()
                        .find(|(_, entry)| RowTarget::Home(entry.target.clone()) == send.target)
                        .map(|(index, entry)| HomeFeedback {
                            row: entry.row.clone(),
                            lead: entry.lead.map(str::to_owned),
                            index,
                        })
                })
                .flatten();
            RowFeedback {
                target: send.target,
                home,
            }
        });
        Effect::Act(match input.compose {
            Compose::Talk { to } | Compose::AskLead { to, .. } => Request::Talk {
                me,
                squad,
                to,
                text,
            },
            Compose::Annotate { to, row } => Request::Annotate {
                me,
                squad,
                to,
                row,
                text,
            },
            Compose::Leads {
                sender,
                recipients,
                all,
            } => Request::Leads {
                sender,
                recipients,
                all,
                text,
            },
            Compose::ReadLead { .. } => unreachable!("read-only mode cannot send"),
            Compose::Cron => unreachable!("a cron step is submitted before this match"),
            Compose::Reply { request, from } => Request::Reply {
                me,
                request,
                from,
                text,
            },
        })
    }

    /// Shows how a request ended.
    pub fn finished(&mut self, outcome: Result<String, String>) {
        self.sent = self.pending_send.take().filter(|_| outcome.is_ok());
        self.notice = Some(outcome.unwrap_or_else(|error| error));
    }

    fn menu_key(&mut self, key: KeyEvent) -> Effect {
        let Some(menu) = &mut self.menu else {
            return Effect::None;
        };
        let chosen = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => None,
            KeyCode::Up | KeyCode::Char('k') => {
                menu.selected = menu.selected.saturating_sub(1);
                return Effect::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                menu.selected = (menu.selected + 1).min(menu.entries.len().saturating_sub(1));
                return Effect::None;
            }
            KeyCode::Enter => menu
                .entries
                .get(menu.selected)
                .map(|entry| entry.choice.clone()),
            _ => {
                let event = event_name(key);
                match menu
                    .entries
                    .iter()
                    .find(|entry| Some(&entry.key) == event.as_ref())
                {
                    Some(entry) => Some(entry.choice.clone()),
                    None => return Effect::None,
                }
            }
        };
        let prefill = menu.prefill.clone();
        let link = menu.link.clone();
        let row_send = menu.row_send.clone();
        self.menu = None;
        let effect = chosen.map_or(Effect::None, |choice| self.choose(choice));
        if let Some(input) = &mut self.input {
            input.text = prefill;
            input.link = link;
        }
        if let Some(send) = row_send {
            let squad = match &send.target {
                RowTarget::Home(target) => target.squad.clone(),
                RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => squad.clone(),
            };
            self.attach_row(send, squad);
        }
        effect
    }

    /// A modal event is consumed before base dispatch. New surfaces add their
    /// identity and controller arm here; rendering and worker effects stay outside.
    pub(super) fn overlay_event(&mut self, event: &Event) -> Option<Effect> {
        use tmt_tui::app::{Routed, route};
        let Some(overlay) = self.overlay() else {
            self.overlay_focus.close();
            return self.jobs_event(event);
        };
        let id = overlay.id();
        let mut focus = std::mem::take(&mut self.overlay_focus);
        if focus.overlay() != Some(&id) {
            focus.reconcile_base(vec![vec![self.focused().title().into()]]);
            let fields = match overlay {
                Overlay::Switcher => vec![
                    vec!["switcher".into(), "query".into()],
                    vec!["switcher".into(), "choices".into()],
                ],
                Overlay::Theme => vec![vec!["theme-picker".into(), "choices".into()]],
                Overlay::View => vec![vec!["view-picker".into(), "choices".into()]],
                Overlay::Settings => vec![vec!["settings".into(), "content".into()]],
                Overlay::CronList => vec![vec!["cron-list".into(), "choices".into()]],
                _ => vec![],
            };
            focus.open(id, fields);
        }
        let routed = route(&mut focus, event, |field, event| {
            self.overlay_input(overlay, field, event)
        });
        // Applying a close must never replay this event into the restored base.
        if self.overlay() != Some(overlay) {
            focus.close();
        }
        self.overlay_focus = focus;
        Some(match routed {
            Routed::Handled(effect) => effect,
            Routed::Quit => Effect::Quit,
            Routed::Captured | Routed::Unhandled => Effect::None,
        })
    }

    fn overlay(&self) -> Option<Overlay> {
        if self.help {
            Some(Overlay::Help)
        } else if self.settings.is_some() {
            Some(Overlay::Settings)
        } else if self.view_picker.is_some() {
            Some(Overlay::View)
        } else if self.theme_picker.is_some() {
            Some(Overlay::Theme)
        } else if self.switcher.is_some() {
            Some(Overlay::Switcher)
        } else if self.cron_list.is_some() {
            Some(Overlay::CronList)
        } else {
            None
        }
    }

    fn overlay_input(
        &mut self,
        overlay: Overlay,
        field: &tmt_tui::app::ComponentId,
        event: &Event,
    ) -> Option<Effect> {
        match overlay {
            Overlay::Help => {
                let input = self.help_state.borrow_mut().input(event);
                match input? {
                    super::help::Input::Close => self.help = false,
                    super::help::Input::Scroll => {}
                }
                Some(Effect::None)
            }
            Overlay::Settings => match event {
                Event::Key(key) => {
                    let overlay = self.settings.as_mut()?;
                    let editing = overlay.editing();
                    let input = overlay.key(*key);
                    if !editing && overlay.editing() {
                        overlay.opening_focus = self.focus;
                    }
                    Some(match input {
                        super::settings::Input::None => Effect::None,
                        super::settings::Input::Save => Effect::SaveSetting,
                        super::settings::Input::Preview => {
                            self.settings_preview();
                            Effect::None
                        }
                        super::settings::Input::Close => {
                            self.settings_preview();
                            self.settings = None;
                            Effect::CancelSettings
                        }
                    })
                }
                Event::Mouse(mouse) => {
                    self.settings.as_ref()?.mouse(*mouse);
                    Some(Effect::None)
                }
                _ => None,
            },
            Overlay::View => Some(match self.view_picker.as_mut()?.input(event)? {
                super::view_picker::Input::Preview => {
                    self.restore_focus();
                    Effect::None
                }
                super::view_picker::Input::Save => Effect::SaveView,
                super::view_picker::Input::Cancel => {
                    self.close_view_picker(false);
                    Effect::CancelView
                }
            }),
            Overlay::Theme => Some(match self.theme_picker.as_mut()?.input(event)? {
                super::theme_picker::Input::Preview => Effect::None,
                super::theme_picker::Input::Save => Effect::SaveTheme,
                super::theme_picker::Input::Cancel => {
                    self.theme_picker = None;
                    Effect::None
                }
            }),
            Overlay::Switcher => self.switcher_event(event, field),
            Overlay::CronList => self.cron_list_event(event),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        if let Some(effect) = self.overlay_event(&Event::Key(key)) {
            return effect;
        }
        self.notice = None;
        if self.sent.take().is_some() {
            self.clamp();
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        // Refresh keeps text inputs intact and uses the same override owner
        // as ordinary keys; rebinding ctrl-r does not force a refresh.
        if event_name(key).as_deref() == Some("ctrl-r")
            && let Some(action) = self.bindings().remove("ctrl-r")
            && action.verb == Verb::Refresh
        {
            return self.perform(&action);
        }
        if self.input.is_some() {
            return self.input_key(key);
        }
        if self.menu.is_some() {
            return self.menu_key(key);
        }
        if self.searching {
            match key.code {
                KeyCode::Char(character)
                    if !character.is_control()
                        && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.search.push(character);
                }
                KeyCode::Backspace => {
                    self.search.pop();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Esc => {
                    self.search.clear();
                    self.searching = false;
                }
                _ => {}
            }
            self.clamp();
            return Effect::None;
        }
        if self.focused_pane().is_none()
            && !self.bound(key)
            && matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Char('j' | 'k')
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            return Effect::None;
        }
        if self.focused_pane() == Some(Pane::Notes) && !self.help {
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter)
                && let Some(action) = self.note_binding(key)
            {
                return self.perform(&action);
            }
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                return self.select_link(if key.code == KeyCode::BackTab { -1 } else { 1 });
            }
            if key.code == KeyCode::Enter && self.note_link.is_some() {
                return self.activate_link();
            }
            if key.code == KeyCode::Esc && self.note_link.take().is_some() {
                return Effect::None;
            }
            let step = match key.code {
                KeyCode::Up | KeyCode::Char('k') => Some(Step::Lines(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Step::Lines(1)),
                KeyCode::PageUp => Some(Step::Pages(-1)),
                KeyCode::PageDown => Some(Step::Pages(1)),
                KeyCode::Home | KeyCode::Char('g') => Some(Step::Top),
                KeyCode::End | KeyCode::Char('G') => Some(Step::Bottom),
                _ => None,
            };
            if let Some(step) = step {
                if let Some(action) =
                    event_name(key).and_then(|event| self.bindings().remove(&event))
                {
                    return self.perform(&action);
                }
                self.note_link = None;
                self.move_note(step);
                return Effect::None;
            }
        }
        match key.code {
            KeyCode::Char('q') => return Effect::Quit,
            KeyCode::Esc if self.search.is_empty() && !self.help => return Effect::Quit,
            KeyCode::Esc => {
                self.help = false;
                self.search.clear();
                self.clamp();
            }
            // Any pane but rows scrolls its text; rows moves the selection.
            KeyCode::Up | KeyCode::Char('k') if self.focused() != Pane::Rows => {
                self.scrolls.scroll(self.focused(), Step::Lines(-1));
            }
            KeyCode::Down | KeyCode::Char('j') if self.focused() != Pane::Rows => {
                self.scrolls.scroll(self.focused(), Step::Lines(1));
            }
            // Paging keys scroll unless the user bound them (then the
            // binding runs, below).
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                if self.focused() != Pane::Rows && !self.bound(key) =>
            {
                self.scrolls.scroll(self.focused(), page_step(key.code));
            }
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select(self.selected + 1),
            KeyCode::PageUp if !self.bound(key) => {
                self.page_rows(-1);
            }
            KeyCode::PageDown if !self.bound(key) => self.page_rows(1),
            KeyCode::Home if !self.bound(key) => self.select(0),
            KeyCode::End if !self.bound(key) => self.select(usize::MAX),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.move_current(-1);
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
                return self.move_current(1);
            }
            KeyCode::Left => return self.switch(-1),
            KeyCode::Right => return self.switch(1),
            KeyCode::Char('/') => self.searching = true,
            // The switcher's key, unless the user bound `s` to something.
            KeyCode::Char('s') if !self.bound(key) => self.switcher = Some(Switcher::default()),
            // Every squad's jobs, unless the user bound `c` to something.
            KeyCode::Char('c') if !self.bound(key) && self.cron_shown() => {
                return self.open_cron_list(None);
            }
            KeyCode::Char('?') => {
                self.help_state.borrow_mut().open();
                self.help = true;
            }
            _ => {
                if let Some(action) =
                    event_name(key).and_then(|event| self.bindings().remove(&event))
                {
                    return self.perform(&action);
                }
            }
        }
        Effect::None
    }

    /// Every tab the switcher offers: the tab line's, then hidden ones.
    pub fn switchable(&self) -> Vec<String> {
        self.tabs.iter().chain(&self.hidden).cloned().collect()
    }

    fn reconcile_switcher(&self, reset: bool) {
        let keys = self.switcher_keys();
        let Some(switcher) = &self.switcher else {
            return;
        };
        let mut surface = switcher.surface.borrow_mut();
        let query = surface.picker.query().unwrap_or("");
        let rows: Vec<_> = super::tabs::matching(&keys, query)
            .into_iter()
            .map(|key| tmt_tui::components::ListRow {
                id: key.clone(),
                disabled: false,
            })
            .collect();
        let first = rows.first().map(|row| row.id.clone());
        surface.reconcile(rows);
        if reset && let Some(id) = first {
            surface.select(&id);
        }
    }

    fn switcher_event(
        &mut self,
        event: &Event,
        field: &tmt_tui::app::ComponentId,
    ) -> Option<Effect> {
        use tmt_tui::components::{PickerEvent, PickerField, PickerInput};
        let field = match field.last().map(String::as_str) {
            Some("query") => PickerField::Query,
            Some("choices") => PickerField::List,
            _ => return None,
        };
        self.reconcile_switcher(false);
        if let Event::Key(key) = event
            && event_name(*key).is_some_and(|name| self.pick_keys().contains(&name))
        {
            let selected = self
                .switcher
                .as_ref()?
                .surface
                .borrow()
                .picker
                .list
                .selected()
                .map(str::to_owned);
            return Some(selected.map_or(Effect::None, |key| self.toggle_tab_pick(key)));
        }
        let input = self
            .switcher
            .as_ref()?
            .surface
            .borrow_mut()
            .input(event, field);
        match input {
            Some(PickerInput::Event(PickerEvent::Cancel)) => self.switcher = None,
            Some(PickerInput::Event(PickerEvent::Confirm(key))) => {
                self.switcher = None;
                return Some(self.go(key));
            }
            Some(PickerInput::Event(PickerEvent::QueryChanged(_))) => self.reconcile_switcher(true),
            Some(_) => {}
            None if matches!(event, Event::Key(key) if key.code == KeyCode::Enter)
                && self
                    .switcher
                    .as_ref()?
                    .surface
                    .borrow()
                    .picker
                    .list
                    .selected()
                    .is_none() =>
            {
                self.switcher = None
            }
            None => return None,
        }
        Some(Effect::None)
    }

    pub(super) fn invalidate_overlay_frames(&self) {
        if let Some(settings) = &self.settings {
            settings.surface.borrow_mut().invalidate();
        }
        if let Some(picker) = &self.theme_picker {
            picker.surface.borrow_mut().invalidate();
        }
        if let Some(picker) = &self.view_picker {
            picker.surface.borrow_mut().invalidate();
        }
        if let Some(switcher) = &self.switcher {
            switcher.surface.borrow_mut().invalidate();
        }
        if let Some(list) = &self.cron_list {
            list.invalidate();
        }
        for pane in self.jobs.borrow_mut().values_mut() {
            pane.invalidate();
        }
    }

    fn bound(&self, key: KeyEvent) -> bool {
        event_name(key).is_some_and(|event| self.bindings().contains_key(&event))
    }

    /// Selects a row and keeps it on screen.
    pub(super) fn select(&mut self, row: usize) {
        self.home_start = false;
        self.home_target = None;
        self.selected = row;
        self.clamp();
        self.follow = true;
    }

    /// Page by visual lines; an oversized row can scroll within itself.
    fn page_rows(&mut self, direction: isize) {
        let starts = self.row_starts.borrow();
        if starts.is_empty() {
            drop(starts);
            let lines = direction * self.scrolls.page_lines(Pane::Rows) as isize;
            self.select(self.selected.saturating_add_signed(lines));
            return;
        }
        let origin = starts
            .get(self.selected)
            .copied()
            .unwrap_or(0)
            .max(self.scrolls.offset(Pane::Rows));
        let target =
            origin.saturating_add_signed(direction * self.scrolls.page_lines(Pane::Rows) as isize);
        let row = starts
            .partition_point(|start| *start <= target)
            .saturating_sub(1);
        drop(starts);
        if row == self.selected {
            self.scrolls.scroll(Pane::Rows, Step::Pages(direction));
            self.follow = false;
        } else {
            self.select(row);
        }
    }

    /// The wheel scrolls the pane under the pointer, whichever is focused.
    /// A left click focuses the pane under it and selects the row under it, then runs its `click` binding;
    /// a second click on the same row soon after runs `double-click`.
    pub fn mouse(&mut self, event: MouseEvent, now: Instant) -> Effect {
        if let Some(effect) = self.overlay_event(&Event::Mouse(event)) {
            return effect;
        }
        if self.menu.is_some() || self.input.is_some() {
            return Effect::None;
        }
        if let Some(effect) = self.jobs_mouse(event) {
            return effect;
        }
        let lines = match event.kind {
            MouseEventKind::ScrollUp => Some(-(WHEEL_LINES as isize)),
            MouseEventKind::ScrollDown => Some(WHEEL_LINES as isize),
            _ => None,
        };
        if let Some(lines) = lines {
            if let Some(pane) = self
                .scrolls
                .pane_at(event.column, event.row)
                .filter(|pane| !self.collapsed_panes().contains(pane))
            {
                self.scrolls.scroll(pane, Step::Lines(lines));
                if pane == Pane::Notes
                    && let Some(key) = self.shown_tab()
                {
                    self.note_cursors
                        .borrow_mut()
                        .entry(key.to_owned())
                        .or_default()
                        .follow = false;
                }
                if pane == Pane::Rows {
                    self.follow = false;
                }
            }
            return Effect::None;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && self.unpicked_hit.get().is_some_and(|rect| {
                rect.contains(ratatui::layout::Position::new(event.column, event.row))
            })
        {
            self.dragging = None;
            self.switcher = Some(Switcher {
                unpicked_only: true,
                ..Switcher::default()
            });
            self.reconcile_switcher(false);
            return Effect::None;
        }
        // A press on a tab shows it; releasing it over another tab moves it
        // there and saves the order.
        let tab = self.tab_hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        match (event.kind, tab) {
            (MouseEventKind::Down(MouseButton::Left), Some(hit)) => {
                self.dragging = Some(hit.tab);
                return match self.tabs.get(hit.tab).cloned() {
                    Some(key) => self.go(key),
                    None => Effect::None,
                };
            }
            (MouseEventKind::Up(MouseButton::Left), hit) => {
                return match (self.dragging.take(), hit) {
                    (Some(from), Some(to)) => self.move_tab(from, to.tab),
                    _ => Effect::None,
                };
            }
            _ => {}
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Effect::None;
        }
        let title = self
            .title_hits
            .borrow()
            .iter()
            .find(|hit| {
                hit.area
                    .contains(ratatui::layout::Position::new(event.column, event.row))
            })
            .map(|hit| hit.pane);
        if let Some(pane) = title {
            self.notice = None;
            return self.toggle_panes(&[pane]);
        }
        self.focus_at(event.column, event.row);
        if self.focused_pane() == Some(Pane::Notes) {
            let hit = self
                .link_hits
                .borrow()
                .iter()
                .find(|(area, _)| {
                    area.contains(ratatui::layout::Position::new(event.column, event.row))
                })
                .map(|(_, link)| *link);
            if let Some(id) = hit {
                let previous = self.note_link.clone();
                if self.choose_link(id) {
                    return if previous == self.note_link {
                        self.activate_link()
                    } else {
                        Effect::None
                    };
                }
            }
            self.note_link = None;
            if let (Some(key), Some(view)) = (self.shown_tab(), &self.view)
                && let Notes::Text(text) = &view.notes
                && let Some((_, visual)) = self.note_hits.borrow().iter().find(|(area, _)| {
                    area.contains(ratatui::layout::Position::new(event.column, event.row))
                })
            {
                let mut cursors = self.note_cursors.borrow_mut();
                let cursor = cursors.entry(key.to_owned()).or_default();
                if let Some(notes) = &view.derived.borrow().notes {
                    cursor.select_visual(text, &notes.sources, *visual);
                    cursor.follow = true;
                }
            }
            return Effect::None;
        }
        let hit = self.hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        let Some(hit) = hit.filter(|_| !self.collapsed_panes().contains(&Pane::Rows)) else {
            return Effect::None;
        };
        self.notice = None;
        self.select(hit.row);
        let double = self
            .last_click
            .is_some_and(|(row, at)| row == hit.row && now.duration_since(at) <= DOUBLE_CLICK);
        self.last_click = (!double).then_some((hit.row, now));
        let event = if double { "double-click" } else { "click" };
        match self.bindings().remove(event) {
            Some(action) => self.perform(&action),
            None => Effect::None,
        }
    }

    /// How the board draws now: the shown squad's theme, or the default
    /// one before the first load.
    pub fn look(&self) -> crate::look::Look {
        let saved = self.view.as_ref().map_or_else(
            || crate::look::Look::new(tmt_cli_style::Theme::default()),
            |view| view.look,
        );
        self.theme_picker
            .as_ref()
            .map_or(saved, |picker| picker.preview(saved.depth))
    }

    pub(super) fn selected_read(&self) -> Option<super::refresh::SelectedRead> {
        if self.loading() {
            return None;
        }
        if let Some(Input {
            compose: Compose::ReadLead { key, .. },
            ..
        }) = &self.input
        {
            return Some(super::refresh::SelectedRead::Message(key.clone()));
        }
        self.notebook_identity()
            .map(super::refresh::SelectedRead::Notebook)
    }

    /// Lazy acquisition follows the effective detail, including folds and tabs.
    pub(super) fn notebook_identity(&self) -> Option<String> {
        let board = self.effective_board()?;
        if self.loading()
            || self.selected_is_lead()
            || !self.scrolls.visible(Pane::Detail)
            || !board.panes.contains(&Pane::Detail)
            || self.collapsed_panes().contains(&Pane::Detail)
            || (board.mode == BoardMode::Tabs && self.focused_pane() != Some(Pane::Detail))
        {
            return None;
        }
        let row = self.selected_row()?;
        (row["lifetime"] == "saved")
            .then(|| row["id"].as_str().map(str::to_owned))
            .flatten()
    }

    pub fn selected_row(&self) -> Option<&Value> {
        self.rows().get(self.selected).map(|(_, row)| *row)
    }

    /// The displayed lead occurrence owns no detail notebook acquisition.
    pub(super) fn selected_is_lead(&self) -> bool {
        self.rows()
            .get(self.selected)
            .is_some_and(|(origin, _)| *origin == RowOrigin::Lead)
    }
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use serde_json::json;

    fn loaded(name: &str) -> Snapshot {
        let mut snapshot = super::tests::snapshot(name, json!([]));
        snapshot.tabs = [
            super::super::ALL,
            "product",
            "omitted",
            "infra",
            "@tab:needs-me",
        ]
        .map(String::from)
        .to_vec();
        snapshot.hidden = vec!["hidden".into()];
        snapshot.pinned = 1;
        snapshot
    }

    fn board() -> App {
        let mut app = App::new(Some("product".into()));
        app.apply(loaded("product"));
        app.picks =
            super::super::pick::Picks::parse(Some("product,infra"), &app.switchable()).unwrap();
        app
    }

    fn key(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn highlight(app: &mut App, name: &str) {
        app.reconcile_switcher(false);
        app.switcher
            .as_ref()
            .unwrap()
            .surface
            .borrow_mut()
            .select(name);
    }

    #[test]
    fn space_toggles_only_the_highlighted_tab_and_current_unpick_uses_normal_load() {
        let mut app = board();
        let other = board();
        let canonical = app.tabs.clone();
        key(&mut app, KeyCode::Char('s'));
        highlight(&mut app, "product");
        assert_eq!(
            key(&mut app, KeyCode::Char(' ')),
            Effect::Load("infra".into())
        );
        assert!(!app.picks.contains("product"));
        assert_eq!(app.current.as_deref(), Some("infra"));
        assert!(
            app.loading(),
            "an uncached tab retains the old view with the normal stale fence"
        );
        assert!(app.switcher.is_some());
        assert_eq!(app.switcher.as_ref().unwrap().query(), "");
        assert_eq!(app.tabs, canonical);
        assert!(
            other.picks.contains("product"),
            "another board is independent"
        );
        app.apply(loaded("infra"));
        highlight(&mut app, "infra");
        assert_eq!(
            key(&mut app, KeyCode::Char(' ')),
            Effect::Load(super::super::ALL.into())
        );
        assert!(
            app.picks.contains(super::super::ALL),
            "last unpick opens home"
        );
        assert!(!app.picks.contains("infra"));
    }

    #[test]
    fn opening_unpicked_or_hidden_entries_picks_without_changing_global_hide() {
        let mut app = board();
        key(&mut app, KeyCode::Char('s'));
        highlight(&mut app, "omitted");
        assert_eq!(
            key(&mut app, KeyCode::Enter),
            Effect::Load("omitted".into())
        );
        assert!(app.picks.contains("omitted"));
        assert!(app.switcher.is_none());
        app.apply(loaded("omitted"));
        app.go("hidden".into());
        assert!(app.picks.contains("hidden"));
        assert!(!app.tabs.contains(&"hidden".into()));
        assert_eq!(app.hidden, ["hidden"]);
    }

    #[test]
    fn rebindable_pick_action_opens_the_switcher_and_overrides_space_in_both_fields() {
        let mut app = board();
        app.view
            .as_mut()
            .unwrap()
            .bindings
            .insert("p".into(), Action::parse("pick-tab").unwrap());
        assert_eq!(app.pick_keys(), ["p"]);
        assert_eq!(key(&mut app, KeyCode::Char('p')), Effect::None);
        highlight(&mut app, "infra");
        key(&mut app, KeyCode::Char('p'));
        assert!(!app.picks.contains("infra"));
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Char('p'));
        assert!(app.picks.contains("infra"));
        assert_eq!(app.switcher.as_ref().unwrap().query(), "");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.search, "p", "text inputs keep printable bindings");
        assert!(app.switcher.is_none());
    }

    #[test]
    fn picks_survive_refresh_cache_switches_and_reorder_retains_omitted_keys() {
        let mut app = board();
        let canonical = app.tabs.clone();
        app.go("infra".into());
        app.apply(loaded("infra"));
        app.go("product".into());
        assert!(!app.loading(), "visited view comes from its original cache");
        assert!(!app.picks.contains("omitted"));
        let mut refreshed = loaded("product");
        refreshed.tabs.push("new-squad".into());
        app.apply(refreshed);
        assert!(!app.picks.contains("new-squad"));
        assert!(app.cache.contains_key("infra"));
        assert_eq!(app.picked_indices(), [1, 3]);
        let moved = app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        let Effect::Act(Request::Reorder(order)) = moved else {
            panic!("expected reorder");
        };
        assert_eq!(order.len(), canonical.len() + 1);
        assert!(canonical.iter().all(|key| order.contains(key)));
        assert_eq!(order[0], super::super::ALL);
        assert!(order.contains(&"omitted".into()));
    }

    #[test]
    fn default_follows_additions_but_explicit_removal_reconciles_to_home() {
        let mut app = board();
        app.picks = Default::default();
        let mut refreshed = loaded("product");
        refreshed.tabs.push("new-squad".into());
        app.apply(refreshed);
        assert!(app.picked_indices().contains(&5));
        app.picks = super::super::pick::Picks::parse(Some("product"), &app.switchable()).unwrap();
        let mut removed = loaded("product");
        removed.tabs.retain(|key| key != "product");
        app.apply(removed);
        assert_eq!(
            app.reconcile_pick_current(),
            Effect::Load(super::super::ALL.into())
        );
        assert!(!app.picks.contains("product"));
        assert!(app.picks.contains(super::super::ALL));
    }

    #[test]
    fn an_unavailable_inventory_does_not_erase_this_boards_picks() {
        let mut app = board();
        let mut failed = loaded("product");
        failed.tabs.clear();
        failed.hidden.clear();
        failed.view = Err("inventory unavailable".into());
        app.apply(failed);
        assert_eq!(app.reconcile_pick_current(), Effect::None);
        app.apply(loaded("product"));
        assert!(app.picks.contains("product") && app.picks.contains("infra"));
        assert!(!app.picks.contains("omitted"));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(sections: Value) -> View {
        View {
            ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
            home_replies: true,
            token_rate: None,
            home_rate: Default::default(),
            home: None,
            derived: Default::default(),
            document: json!({"squad": {"name": "product"}, "sections": sections}),
            rows: crate::rows::Rows::preset(),
            refresh: Some(crate::config::DEFAULT_REFRESH),
            board: crate::config::Board::simple(
                crate::config::BoardMode::Split,
                crate::config::Direction::LeftRight,
                vec![crate::config::Pane::Rows],
                &[100],
            ),
            notes: super::Notes::NotShown,
            render: crate::config::NotesRender::Markdown,
            bindings: crate::action::preset(true, &[]),
            section_bindings: Vec::new(),
            configured_bindings: Default::default(),
            opener: None,
            clipboard: None,
            links: Default::default(),
            tab_colors: Default::default(),
            look: Default::default(),
            theme_notice: None,
            me: None,
            me_id: None,
            replies: Vec::new(),
        }
    }

    #[test]
    fn switcher_fields_keep_query_text_and_refresh_selection_by_identity() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot("product", json!([])));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Down);
        let chosen = app
            .switcher
            .as_ref()
            .unwrap()
            .surface
            .borrow()
            .picker
            .list
            .selected()
            .unwrap()
            .to_owned();
        let mut updated = snapshot("product", json!([]));
        updated.tabs.reverse();
        app.apply(updated);
        assert_eq!(
            app.switcher
                .as_ref()
                .unwrap()
                .surface
                .borrow()
                .picker
                .list
                .selected(),
            Some(chosen.as_str())
        );
        let current = app.current.clone();
        let focus = app.focus;
        press(&mut app, KeyCode::Char('q'));
        assert_eq!(app.switcher.as_ref().unwrap().query(), "q");
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.switcher.as_ref().unwrap().query(), "q?");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.overlay_focus.focused().unwrap().last().unwrap(),
            "choices"
        );
        press(&mut app, KeyCode::Char('q'));
        assert!(
            app.switcher.is_none(),
            "q closes the list field and stays text in the query"
        );
        assert_eq!(app.current, current);
        assert_eq!(app.focus, focus);
    }

    #[test]
    fn common_overlay_route_consumes_close_release_and_unknown_mouse() {
        use ratatui::crossterm::event::KeyEventKind;
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot("product", json!([])));
        let current = app.current.clone();
        let focus = app.focus;
        app.switcher = Some(Switcher::default());
        let mut released = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        released.kind = KeyEventKind::Release;
        assert_eq!(app.key(released), Effect::None);
        assert!(app.switcher.is_some(), "release cannot close a modal");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Effect::None
        );
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                },
                Instant::now()
            ),
            Effect::None
        );
        assert_eq!(app.current, current);
        assert_eq!(app.focus, focus);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Effect::None
        );
        assert!(app.switcher.is_none());
        assert!(app.overlay_focus.overlay().is_none());
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Effect::Refresh
        );
        app.help_state.borrow_mut().open();
        app.help = true;
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Effect::Quit
        );
        assert!(app.help, "quit is an effect, not a replayed close");
    }

    #[test]
    fn late_home_snapshot_is_retained_under_its_own_key() {
        let mut app = App::new(Some("product".into()));
        let mut home = snapshot(super::super::ALL, json!([]));
        home.tabs.push(super::super::ALL.into());
        home.view.as_mut().unwrap().home = Some(super::super::home::Home {
            windows: crate::config::TokenWindow::DEFAULTS,
            summary: super::super::home::Counts {
                members: 7,
                ..Default::default()
            },
            sections: Vec::new(),
            squads: Vec::new(),
            failures: Vec::new(),
            incomplete: false,
        });
        app.apply(home);
        assert_eq!(app.current.as_deref(), Some("product"));
        assert!(app.view.is_none());
        assert_eq!(
            app.cache[super::super::ALL]
                .home
                .as_ref()
                .unwrap()
                .summary
                .members,
            7
        );
    }

    pub(crate) fn snapshot(squad: &str, sections: Value) -> Snapshot {
        Snapshot {
            squad_keys: vec!["infra".into(), "product".into()],
            tabs: vec!["infra".into(), "product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some(squad.into()),
            view: Ok(view(sections)),
        }
    }

    fn row(name: &str, task: &str) -> Value {
        json!({"id": name, "name": name, "fields": {"task": task}})
    }

    fn names(app: &App) -> Vec<&str> {
        app.items()
            .iter()
            .filter_map(|item| match item {
                Item::Section(title) => *title,
                Item::Rule(_) => Some("--"),
                Item::Row(_, row) => row["name"].as_str(),
            })
            .collect()
    }

    #[test]
    fn paging_without_drawn_row_positions_keeps_record_navigation() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{ "title": null, "rows": [
            row("a", ""), row("b", ""), row("c", ""), row("d", "")
        ] }]),
        ));
        app.selected = 2;
        assert!(app.row_starts.borrow().is_empty());
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.selected, 3);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected, 2);
        app.selected = 0;
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn search_filters_rows_by_name_or_field_and_keeps_selection_in_range() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([
                {"title": "Needs me", "rows": [row("auth-fix", "Rotate tokens")]},
                {"title": "Everyone", "rows": [row("auth-fix", "Rotate tokens"), row("docs-sweep", "install guide")]}
            ]),
        ));
        assert_eq!(
            names(&app),
            ["Needs me", "auth-fix", "Everyone", "auth-fix", "docs-sweep"]
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            app.selected_row().unwrap()["name"],
            "docs-sweep",
            "clamped to the last row"
        );
        press(&mut app, KeyCode::Char('/'));
        for character in "TOKEN".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        assert_eq!(
            names(&app),
            ["Needs me", "auth-fix", "Everyone", "auth-fix"]
        );
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Esc);
        assert!(app.search.is_empty() && !app.searching);
        assert!(matches!(press(&mut app, KeyCode::Esc), Effect::Quit));
    }

    #[test]
    fn squad_switch_keeps_the_frame_caches_and_never_shows_another_squads_result() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        let Effect::Load(next) = press(&mut app, KeyCode::Right) else {
            panic!("switch");
        };
        assert_eq!(next, "infra");
        // Nothing is blanked: product's frame stays until infra arrives, and
        // its rows take no actions meanwhile.
        assert_eq!(names(&app), ["a"]);
        assert!(app.loading() && app.loading_since.is_some());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Loading infra…"));
        // A late product result is kept for switching back, never shown.
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("late", "")]}]),
        ));
        assert_eq!(names(&app), ["a"]);
        app.apply(snapshot(
            "infra",
            json!([{"title": null, "rows": [row("i", "")]}]),
        ));
        assert_eq!(names(&app), ["i"]);
        assert!(!app.loading() && app.loading_since.is_none());
        // Back to product: at once, from the cache (the late result).
        assert_eq!(
            press(&mut app, KeyCode::Left),
            Effect::Load("product".into())
        );
        assert_eq!(names(&app), ["late"]);
        assert!(!app.loading(), "a cached squad is the current one at once");
    }

    #[test]
    fn a_failed_switch_returns_to_the_retained_view_and_shows_its_error() {
        for error in [
            "infra: room not found",
            "tmt did not finish in time; the outcome is unknown.",
        ] {
            let mut app = App::new(Some("product".into()));
            app.apply(snapshot(
                "product",
                json!([{"title": null, "rows": [row("a", "")]}]),
            ));
            app.meter = Some(super::super::meter::Meter::new(
                crate::config::TokenRate {
                    enabled: true,
                    ..Default::default()
                },
                &super::super::rate::tests::input(100),
                Instant::now(),
            ));
            press(&mut app, KeyCode::Right);
            assert!(app.meter.is_none() && app.meters.contains_key("product"));
            app.apply(Snapshot {
                squad_keys: Vec::new(),
                tabs: vec!["product".into(), "infra".into()],
                hidden: Vec::new(),
                pinned: 0,
                attention: Default::default(),
                squad: Some("infra".into()),
                view: Err(error.into()),
            });
            assert!(!app.loading() && app.loading_since.is_none());
            assert_eq!(app.current.as_deref(), Some("product"));
            assert_eq!(app.shown_tab(), Some("product"));
            assert_eq!(names(&app), ["a"]);
            assert_eq!(app.error.as_deref(), Some(error));
            assert!(app.notice.is_none(), "the footer must show the error");
            assert!(app.meter.is_some() && !app.meters.contains_key("product"));
        }
    }

    fn member(name: &str, fields: Value) -> Value {
        json!({"id": name, "name": name, "state": "working", "fields": fields})
    }

    pub(super) fn crew(
        bindings: crate::action::Bindings,
        sections: Vec<crate::action::Bindings>,
    ) -> App {
        let mut app = App::new(Some("product".into()));
        let mut snapshot = snapshot(
            "product",
            json!([
                {"title": "Mine", "rows": [member("auth-fix", json!({"task": "rotate", "pr_link": "https://example.com/pull/412"}))]},
                {"title": "Others", "rows": [member("docs", json!({"task": "guide", "link": "file:///etc/passwd"}))]}
            ]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.configured_bindings = bindings.clone();
        view.bindings = bindings;
        view.section_bindings = sections;
        view.clipboard = Some(vec!["pbcopy".into()]);
        app.apply(snapshot);
        app
    }

    pub(super) fn bind(entries: &[(&str, &str)]) -> crate::action::Bindings {
        crate::action::parse_bindings(entries.iter().map(|(e, a)| (*e, Some(*a))), "bind").unwrap()
    }

    #[test]
    fn ask_lead_requires_confirmation_and_revalidates_sender_and_lead() {
        let mut app = crew(crate::action::preset(true, &[]), Vec::new());
        app.view.as_mut().unwrap().me = Some("Ben".into());
        app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        assert_eq!(press(&mut app, KeyCode::Char('A')), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().prompt, "→ lead sol");
        assert_eq!(
            app.input.as_ref().unwrap().text,
            crate::config::DEFAULT_ASK_LEAD
        );
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none());
        press(&mut app, KeyCode::Char('A'));
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "product".into(),
                to: "sol".into(),
                text: crate::config::DEFAULT_ASK_LEAD.into(),
            })
        );
        for change in ["lead", "sender", "squad"] {
            let mut app = crew(crate::action::preset(true, &[]), Vec::new());
            app.view.as_mut().unwrap().me = Some("Ben".into());
            app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
            press(&mut app, KeyCode::Char('A'));
            match change {
                "lead" => {
                    app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "other"})
                }
                "sender" => app.view.as_mut().unwrap().me = Some("other".into()),
                _ => app.current = Some("other".into()),
            }
            assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
            assert!(app.notice.as_deref().unwrap().contains("nothing sent"));
        }
    }

    #[test]
    fn ask_lead_missing_lead_and_rebinding_never_send_on_open() {
        let mut app = crew(bind(&[("z", "ask-lead")]), Vec::new());
        app.view.as_mut().unwrap().me = Some("Ben".into());
        press(&mut app, KeyCode::Char('z'));
        assert!(app.input.is_none());
        assert!(app.notice.as_deref().unwrap().contains("no lead"));
        app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        app.view.as_mut().unwrap().ask_lead = "What needs a decision?".into();
        assert_eq!(press(&mut app, KeyCode::Char('z')), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().text, "What needs a decision?");
        app.input.as_mut().unwrap().text.clear();
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
    }

    /// A bound `jump lead` goes to the squad's lead on its own tab, and to the
    /// selected row's squad's lead on the leads and all tabs; without one it
    /// says why and nothing runs.
    #[test]
    fn jump_lead_goes_to_the_lead_of_the_tab_or_the_selected_row_s_squad() {
        let lead = |app: &mut App| press(app, KeyCode::Char('L'));
        // No preset binds `jump lead`; a user binding does.
        let bound = |mut snapshot: Snapshot| {
            let view = snapshot.view.as_mut().unwrap();
            view.bindings
                .insert("L".into(), Action::parse("jump lead").unwrap());
            snapshot
        };
        let mut app = App::new(Some("product".into()));
        let mut own = snapshot(
            "product",
            json!([{"title": null, "rows": [row("rin", "x")]}]),
        );
        own.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        app.apply(bound(own));
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));

        // An empty squad still has its lead.
        let mut app = App::new(Some("product".into()));
        let mut empty = snapshot("product", json!([{"title": null, "rows": []}]));
        empty.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        app.apply(bound(empty));
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));

        let mut app = App::new(Some("product".into()));
        app.apply(bound(snapshot(
            "product",
            json!([{"title": null, "rows": [row("rin", "x")]}]),
        )));
        assert_eq!(lead(&mut app), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("jump lead: this squad has no lead; set one with tmt squad lead <name>.")
        );

        // The all tab: a row per squad, with its lead as a field.
        let mut app = App::new(Some(crate::board::ALL.into()));
        app.apply(bound(snapshot(
            crate::board::ALL,
            json!([{"title": null, "rows": [
                {"name": "product", "squad": "product", "fields": {"lead": "sol"}},
                {"name": "quiet", "squad": "quiet", "fields": {"lead": null}},
            ]}]),
        )));
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("sol".into())));
        press(&mut app, KeyCode::Down);
        assert_eq!(lead(&mut app), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("jump lead: this squad has no lead; set one with tmt squad lead <name>.")
        );

        // A missing lead is data, so a custom binding gets the same missing-field
        // notice as any other absent value instead of copying a display glyph.
        assert_eq!(
            app.perform(&Action::parse("copy {lead}").unwrap()),
            Effect::None
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("copy: lead is empty for this row.")
        );

        // The leads tab: the selected row is the lead.
        let mut app = App::new(Some(crate::board::LEADS.into()));
        app.apply(bound(snapshot(
            crate::board::LEADS,
            json!([{"title": null, "rows": [
                {"name": "sol", "squad": "product", "fields": {}},
                {"name": "rin", "squad": "infra", "fields": {}},
            ]}]),
        )));
        press(&mut app, KeyCode::Down);
        assert_eq!(lead(&mut app), Effect::Act(Request::Jump("rin".into())));
    }

    #[test]
    fn the_leads_tab_jumps_to_a_lead_and_talks_in_that_lead_s_squad() {
        let mut app = App::new(Some(crate::board::LEADS.into()));
        let mut snapshot = snapshot(
            crate::board::LEADS,
            json!([{"title": null, "rows": [
                member("sol", json!({"squad": "product"})),
                member("rin", json!({"squad": "infra"})),
            ]}]),
        );
        for (row, squad) in [(0, "product"), (1, "infra")] {
            snapshot.view.as_mut().unwrap().document["sections"][0]["rows"][row]["squad"] =
                json!(squad);
        }
        snapshot.view.as_mut().unwrap().me = Some("Ben".into());
        snapshot.tabs = ["product", crate::board::LEADS, "infra"]
            .map(String::from)
            .to_vec();
        app.apply(snapshot);
        press(&mut app, KeyCode::Down);
        // Enter jumps through tmt focus, as on a squad's own tab.
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Jump("rin".into()))
        );
        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "status?");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "infra".into(),
                to: "rin".into(),
                text: "status?".into()
            }),
            "sent in the lead's own squad room, not a tab's"
        );
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.input.as_ref().unwrap().header(), "✎ note → rin");
        typed(&mut app, "check the queue");
        assert!(matches!(press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Annotate { squad, to, .. }) if squad == "infra" && to == "rin"));
        // ← → walk every tab, the built-in one included.
        assert_eq!(
            press(&mut app, KeyCode::Right),
            Effect::Load("infra".into())
        );
    }

    #[test]
    fn removed_user_tabs_drop_cached_views_and_late_results() {
        let key = "@tab:needs-me";
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot("product", json!([])));
        let mut late = snapshot(key, json!([]));
        late.hidden = vec![key.into()];
        app.apply(late);
        assert!(app.cache.contains_key(key), "hidden views stay reachable");
        app.apply(snapshot("product", json!([])));
        assert!(!app.cache.contains_key(key), "removed definition is pruned");
        app.apply(snapshot(key, json!([])));
        assert!(!app.cache.contains_key(key), "late load cannot restore it");
    }

    #[test]
    fn enter_on_the_all_tab_opens_that_squad_s_tab() {
        let mut app = App::new(Some(crate::board::ALL.into()));
        let mut snapshot = snapshot(
            crate::board::ALL,
            json!([{"title": null, "rows": [
                {"name": "product", "squad": "product", "fields": {}},
                {"name": "infra", "squad": "infra", "fields": {}},
            ]}]),
        );
        snapshot.view.as_mut().unwrap().bindings =
            bind(&[("enter", "tab"), ("t", "talk"), ("o", "open")]);
        app.apply(snapshot);
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Load("infra".into())
        );
        assert_eq!(app.current.as_deref(), Some("infra"));
        // A member's row has no squad of its own to open.
        let mut app = crew(bind(&[("enter", "tab")]), Vec::new());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("tab: this row has no squad."));
    }

    #[test]
    fn keys_resolve_the_selected_row_into_requests_or_refuse_with_a_notice() {
        let mut app = crew(crate::action::preset(true, &[]), Vec::new());
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Jump("auth-fix".into()))
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open {
                link: "https://example.com/pull/412".into(),
                opener: None
            })
        );
        assert_eq!(
            press(&mut app, KeyCode::Char('y')),
            Effect::Act(Request::Copy {
                text: "auth-fix: rotate (working)".into(),
                program: Some(vec!["pbcopy".into()])
            })
        );
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Char('o')), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("Only http(s) links open"),
            "{:?}",
            app.notice
        );
        assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .starts_with("Who is sending? Record yourself with tmt squad me"),
            "sending needs me"
        );
        assert_eq!(press(&mut app, KeyCode::Char('x')), Effect::None);
        assert_eq!(app.notice, None, "an unbound key does nothing");
        assert_eq!(press(&mut app, KeyCode::Char('q')), Effect::Quit);
    }

    fn typed(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    #[test]
    fn talk_annotate_and_reply_compose_one_line_and_empty_sends_nothing() {
        let mut app = crew(crate::action::preset(true, &[]), Vec::new());
        {
            let view = app.view.as_mut().unwrap();
            view.me = Some("Ben".into());
            view.document["squad"]["lead"] = json!({"id": "L", "name": "sol"});
            view.document["sections"][0]["rows"][0]["waitingOnYou"] = json!([
                {"requestId": "q2", "preview": "approve the plan?"},
                {"requestId": "q1", "preview": "which database?"}
            ]);
            view.document["sections"][1]["rows"][0]["waitingOnYou"] =
                json!([{"requestId": "q9", "preview": "ok to merge?"}]);
        }
        // The cursor starts on the lead; these cases act on the first member.
        app.select(1);
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.input.as_ref().unwrap().prompt, "→ auth-fix (product)");
        typed(&mut app, "q j -rf; $(x)");
        assert!(app.input.is_some(), "q and j are text while composing");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "product".into(),
                to: "auth-fix".into(),
                text: "q j -rf; $(x)".into()
            })
        );
        assert!(app.input.is_none());

        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "   ");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "draft");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.notice.as_deref() == Some("Nothing sent."));

        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.menu.as_ref().unwrap().entries.len(), 2);
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().prompt,
            "✎ note → sol · about auth-fix"
        );
        typed(&mut app, "split the job");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Annotate {
                me: "Ben".into(),
                squad: "product".into(),
                to: "sol".into(),
                row: "auth-fix".into(),
                text: "split the job".into()
            })
        );

        // Two open requests: the user picks; the newest is never assumed.
        press(&mut app, KeyCode::Char('r'));
        let menu = app.menu.as_ref().expect("picker");
        assert_eq!(menu.title, "answer auth-fix");
        assert_eq!(menu.entries.len(), 2);
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.input.as_ref().unwrap().prompt, "→ auth-fix (product)");
        typed(&mut app, "postgres");
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply {
                me: "Ben".into(),
                request: "q1".into(),
                from: "auth-fix".into(),
                text: "postgres".into()
            })
        );
        // One open request goes straight to the composer.
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('r'));
        assert!(app.menu.is_none());
        typed(&mut app, "yes");
        assert!(matches!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply { request, .. }) if request == "q9"
        ));
        assert!(!Request::Back.sends());

        // Nothing open, or no lead to annotate for, is a notice.
        app.view.as_mut().unwrap().document["sections"][1]["rows"][0]["waitingOnYou"] = json!([]);
        app.view.as_mut().unwrap().document["squad"]["lead"] = Value::Null;
        app.clamp();
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.notice.as_deref(), Some("docs is not waiting on you."));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(
            app.notice.as_deref(),
            Some(
                "This squad has no lead; set one with tmt squad lead <name> --squad product, or use annotate member."
            )
        );
        assert!(app.input.is_none());
    }

    #[test]
    fn row_composer_switches_modes_preserves_draft_and_marks_only_success() {
        let mut app = crew(crate::action::preset(true, &[]), vec![]);
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        view.document["squad"]["lead"] = json!({"name":"sol"});
        view.document["sections"][0]["rows"][0]["waitingOnYou"] =
            json!([{"requestId":"q", "preview":"Ship tonight?"}]);
        app.select(1);
        press(&mut app, KeyCode::Char('a'));
        assert!(app.menu.is_none(), "a single request opens in place");
        assert_eq!(app.input.as_ref().unwrap().header(), "→ auth-fix (product)");
        assert_eq!(
            app.input.as_ref().unwrap().quote.as_deref(),
            Some("Ship tonight?")
        );
        typed(&mut app, "draft");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "✎ note → sol · about auth-fix"
        );
        assert_eq!(app.input.as_ref().unwrap().text, "draft");
        press(&mut app, KeyCode::Tab);
        assert!(
            matches!(app.input.as_ref().unwrap().compose, Compose::Reply { ref request, .. } if request == "q")
        );
        let target = app.row_target(1).unwrap();
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Reply { ref text, .. }) if text == "draft")
        );
        assert!(app.sent.is_none(), "dispatch intent is not send success");
        app.finished(Err("delivery uncertain".into()));
        assert!(app.sent.is_none());
        press(&mut app, KeyCode::Char('a'));
        typed(&mut app, "approved");
        press(&mut app, KeyCode::Enter);
        app.finished(Ok("Replied".into()));
        assert_eq!(
            app.sent.as_ref().map(|feedback| &feedback.target),
            Some(&target)
        );
        assert_eq!(app.selected, 1);
        press(&mut app, KeyCode::Char('x'));
        assert!(
            app.sent.is_none(),
            "the next key clears the row confirmation"
        );
    }

    #[test]
    fn composer_anchors_the_selected_occurrence_and_explicit_member_notes_stay_notes() {
        let mut app = crew(crate::action::preset(true, &[]), vec![]);
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        view.document["squad"]["lead"] = json!({"name":"sol"});
        let mut duplicate = member("auth-fix", json!({}));
        duplicate["waitingOnYou"] = json!([{"requestId":"q", "preview":"Choose?"}]);
        view.document["sections"] = json!([
            {"rows":[duplicate.clone()]}, {"rows":[duplicate]}
        ]);
        app.selected = 1;
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(
            app.input
                .as_ref()
                .unwrap()
                .row_send
                .as_ref()
                .unwrap()
                .target,
            app.row_target(1).unwrap()
        );
        press(&mut app, KeyCode::Esc);
        let note = Action::parse("annotate member").unwrap();
        app.perform(&note);
        assert_eq!(app.input.as_ref().unwrap().header(), "✎ note → auth-fix");
        typed(&mut app, "context");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Annotate { ref to, ref row, .. }) if to == "auth-fix" && row == "auth-fix")
        );
    }

    #[test]
    fn row_composer_follows_identity_on_refresh_and_refuses_changed_authority() {
        for change in ["reorder", "sender", "lead", "request", "identity", "tab"] {
            let mut app = crew(crate::action::preset(true, &[]), vec![]);
            let view = app.view.as_mut().unwrap();
            view.me = Some("Ben".into());
            view.document["squad"]["lead"] = json!({"name":"sol"});
            view.document["sections"][0]["rows"][0]["waitingOnYou"] =
                json!([{"requestId":"q", "preview":"Choose?"}]);
            app.select(1);
            press(&mut app, KeyCode::Char('a'));
            typed(&mut app, "yes");
            if change == "lead" {
                press(&mut app, KeyCode::Tab);
            }
            match change {
                "reorder" => app.view.as_mut().unwrap().document["sections"][0]["rows"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, member("before", json!({}))),
                "sender" => app.view.as_mut().unwrap().me = Some("other".into()),
                "lead" => {
                    app.view.as_mut().unwrap().document["squad"]["lead"]["name"] = json!("other")
                }
                "request" => {
                    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["waitingOnYou"] =
                        json!([])
                }
                "identity" => {
                    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] =
                        json!("replacement")
                }
                _ => app.current = Some("infra".into()),
            }
            app.clamp();
            let effect = press(&mut app, KeyCode::Enter);
            if change == "reorder" {
                assert_eq!(app.selected, 2);
                assert!(
                    matches!(effect, Effect::Act(Request::Reply { ref request, ref from, .. }) if request == "q" && from == "auth-fix")
                );
            } else {
                assert_eq!(effect, Effect::None, "{change}");
                assert!(
                    app.notice.as_ref().unwrap().contains("nothing sent"),
                    "{change}"
                );
                assert!(app.sent.is_none());
            }
        }
    }

    #[test]
    fn back_is_one_request_and_outcomes_become_the_notice() {
        let mut app = crew(crate::action::preset(true, &[]), Vec::new());
        assert_eq!(
            press(&mut app, KeyCode::Backspace),
            Effect::Act(Request::Back)
        );
        app.finished(Ok("Nothing to go back to.".into()));
        assert_eq!(app.notice.as_deref(), Some("Nothing to go back to."));
        app.finished(Err("Pane '%5' was not found.".into()));
        assert_eq!(app.notice.as_deref(), Some("Pane '%5' was not found."));
    }

    #[test]
    fn ctrl_r_refreshes_without_typing_into_search_or_message_inputs() {
        for tmux in [false, true] {
            let mut app = crew(crate::action::preset(tmux, &[]), Vec::new());
            let refresh = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
            assert_eq!(app.key(refresh), Effect::Refresh);
            assert_eq!(press(&mut app, KeyCode::F(5)), Effect::None);
            app.searching = true;
            app.search = "auth".into();
            assert_eq!(app.key(refresh), Effect::Refresh);
            assert!(app.searching);
            assert_eq!(app.search, "auth");
            app.searching = false;
            for compose in [
                Compose::Talk {
                    to: "auth-fix".into(),
                },
                Compose::Annotate {
                    to: "sol".into(),
                    row: "auth-fix".into(),
                },
                Compose::Reply {
                    request: "q1".into(),
                    from: "auth-fix".into(),
                },
            ] {
                app.input = Some(Input {
                    row_send: None,
                    alternative: None,
                    quote: None,
                    link: None,
                    prompt: "message".into(),
                    text: "draft".into(),
                    compose: compose.clone(),
                    squad: "product".into(),
                    hint: None,
                });
                assert_eq!(app.key(refresh), Effect::Refresh);
                let input = app.input.as_ref().unwrap();
                assert_eq!(input.text, "draft");
                assert_eq!(input.compose, compose);
            }
        }
    }

    #[test]
    fn ctrl_r_keeps_normalization_and_effective_binding_overrides() {
        let mut global = crate::action::preset(true, &[]);
        global.extend(bind(&[("ctrl-r", "copy"), ("f5", "refresh")]));
        let mut app = crew(global, vec![bind(&[("ctrl-r", "refresh")])]);
        let refresh = KeyEvent::new(
            KeyCode::Char('R'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(event_name(refresh).as_deref(), Some("ctrl-r"));
        assert_eq!(app.key(refresh), Effect::Refresh, "section wins");
        app.view.as_mut().unwrap().section_bindings.clear();
        assert!(matches!(
            app.key(refresh),
            Effect::Act(Request::Copy { .. })
        ));
        assert_eq!(
            press(&mut app, KeyCode::F(5)),
            Effect::Refresh,
            "explicit F5 works"
        );
        app.searching = true;
        app.search = "auth".into();
        assert_eq!(
            app.key(refresh),
            Effect::None,
            "rebound ctrl-r does not refresh an input"
        );
        assert_eq!(app.search, "auth", "control keys never become search text");
    }

    #[test]
    fn section_bindings_win_over_bind_which_wins_over_the_preset() {
        let mut global = crate::action::preset(true, &[]);
        global.extend(bind(&[("o", "open {pr_link}"), ("f5", "refresh")]));
        let sections = vec![
            Bindings::new(),
            bind(&[("o", "run code {name} --task={task}")]),
        ];
        let mut app = crew(global, sections);
        assert_eq!(press(&mut app, KeyCode::F(5)), Effect::Refresh);
        assert!(matches!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open { .. })
        ));
        press(&mut app, KeyCode::Down);
        assert_eq!(
            press(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Run(vec![
                "code".into(),
                "docs".into(),
                "--task=guide".into()
            ]))
        );
        // A missing field refuses rather than running with a gap.
        let mut app = crew(bind(&[("o", "open {pr_link}")]), Vec::new());
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Char('o')), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("open: pr_link is empty for this row.")
        );
    }

    #[test]
    fn the_menu_lists_the_rows_actions_first_then_the_boards_and_user_keys_follow_their_verb() {
        let mut bindings = crate::action::preset(false, &[]);
        bindings.insert(
            "x".into(),
            crate::action::Action::parse("reply").expect("reply"),
        );
        let mut app = crew(bindings, Vec::new());
        press(&mut app, KeyCode::Enter);
        let menu = app.menu.as_ref().expect("menu");
        let order: Vec<&str> = menu.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            order,
            [
                "r",
                "x",
                "t",
                "a",
                "o",
                "y",
                "n",
                "A",
                "l",
                "T",
                ",",
                "ctrl-r",
                "backspace"
            ]
        );
    }

    #[test]
    fn the_plain_host_menu_lists_row_actions_and_runs_the_chosen_one() {
        let mut app = crew(crate::action::preset(false, &[]), Vec::new());
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        let menu = app.menu.as_ref().expect("menu");
        assert_eq!(menu.title, "auth-fix");
        let events: Vec<&str> = menu.entries.iter().map(|e| e.key.as_str()).collect();
        assert!(events.contains(&"y") && events.contains(&"backspace"));
        assert!(!events.iter().any(|e| ["click", "enter", "tab"].contains(e)));
        assert_eq!(press(&mut app, KeyCode::Char('j')), Effect::None);
        assert!(app.menu.is_some(), "j moves inside the menu");
        assert!(matches!(
            press(&mut app, KeyCode::Char('y')),
            Effect::Act(Request::Copy { .. })
        ));
        assert!(app.menu.is_none());
        press(&mut app, KeyCode::Enter);
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.menu.is_none(), "Esc closes the menu, not the board");
    }

    #[test]
    fn clicks_select_rows_and_run_click_or_double_click_bindings() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut global = crate::action::preset(true, &[]);
        global.extend(bind(&[("click", "notes"), ("double-click", "jump")]));
        let mut app = crew(global, Vec::new());
        *app.hits.borrow_mut() = vec![
            Hit {
                y: 3,
                x: 0,
                width: 40,
                row: 0,
            },
            Hit {
                y: 5,
                x: 0,
                width: 40,
                row: 1,
            },
        ];
        let click = |row, column| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let start = Instant::now();
        assert_eq!(app.mouse(click(5, 3), start), Effect::None);
        assert_eq!(app.selected, 1);
        assert_eq!(
            app.notice.as_deref(),
            Some("The notes pane is not on this board; add it to panes.")
        );
        assert_eq!(
            app.mouse(click(5, 3), start + Duration::from_millis(200)),
            Effect::Act(Request::Jump("docs".into()))
        );
        assert_eq!(
            app.mouse(click(3, 3), start + Duration::from_millis(300)),
            Effect::None,
            "a click on another row is a new single click"
        );
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.mouse(click(3, 3), start + Duration::from_secs(2)),
            Effect::None,
            "too slow for a double click"
        );
        assert_eq!(app.mouse(click(9, 3), start), Effect::None);
        assert_eq!(app.mouse(click(3, 45), start), Effect::None);
        assert_eq!(app.selected, 0, "clicks outside rows change nothing");
    }

    #[test]
    fn failed_refreshes_keep_the_last_view() {
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [row("a", "")]}]),
        ));
        app.apply(Snapshot {
            squad_keys: Vec::new(),
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Err("tmt did not finish in time".into()),
        });
        assert_eq!(
            names(&app),
            ["a"],
            "a failed refresh keeps showing the last good rows"
        );
        assert!(app.error.is_some());
        assert!(matches!(
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Effect::Quit
        ));
    }
    fn folding_snapshot(squad: &str) -> Snapshot {
        let mut snapshot = snapshot(squad, json!([]));
        let view = snapshot.view.as_mut().unwrap();
        view.board = Board::simple(
            BoardMode::Split,
            crate::config::Direction::TopBottom,
            vec![Pane::Rows, Pane::Detail, Pane::Notes],
            &[40, 30, 30],
        );
        view.bindings = crate::action::preset(true, &view.board.panes);
        snapshot
    }

    #[test]
    fn fold_focus_skips_hidden_bodies_and_recovers_from_every_pane_folded() {
        let mut app = App::new(Some("product".into()));
        app.apply(folding_snapshot("product"));
        app.focus = 1;
        // No selected row is needed: d goes through the normal binding path.
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focused_pane(), Some(Pane::Notes));
        app.perform(&Action::parse("toggle rows").unwrap());
        assert_eq!(app.focused_pane(), Some(Pane::Notes));
        app.perform(&Action::parse("toggle notes").unwrap());
        assert_eq!(app.focused_pane(), None);
        for key in [KeyCode::Tab, KeyCode::Down, KeyCode::End, KeyCode::PageDown] {
            press(&mut app, key);
            assert_eq!(app.selected, 0);
            assert_eq!(app.focused_pane(), None);
        }
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.focused_pane(), Some(Pane::Detail));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.focused_pane(), Some(Pane::Notes));
        assert!(!app.collapsed_panes().contains(&Pane::Notes));
        assert!(
            app.view.as_ref().unwrap().board.collapsed.is_empty(),
            "configured board remains immutable"
        );
    }

    #[test]
    fn fold_state_survives_refresh_and_cached_switch_but_config_change_resets_it() {
        let mut app = App::new(Some("product".into()));
        app.apply(folding_snapshot("product"));
        press(&mut app, KeyCode::Char('d'));
        app.apply(folding_snapshot("product"));
        assert!(app.collapsed_panes().contains(&Pane::Detail));
        app.apply(folding_snapshot("infra"));
        app.go("infra".into());
        assert!(app.collapsed_panes().is_empty());
        app.go("product".into());
        assert!(app.collapsed_panes().contains(&Pane::Detail));
        let mut changed = folding_snapshot("product");
        changed
            .view
            .as_mut()
            .unwrap()
            .board
            .collapsed
            .insert(Pane::Rows);
        app.apply(changed);
        assert_eq!(app.collapsed_panes(), BTreeSet::from([Pane::Rows]));
        let mut removed = folding_snapshot("product");
        removed.tabs = vec!["product".into()];
        app.apply(removed);
        assert!(!app.folds.contains_key("infra"));
    }

    #[test]
    fn responsive_folds_respect_toggles_resize_focus_and_config_reset() {
        let responsive = || {
            let mut snapshot = folding_snapshot("product");
            snapshot.view.as_mut().unwrap().board.fold_below = Some(crate::config::FoldBelow {
                width: 100,
                panes: [Pane::Detail].into(),
            });
            snapshot
        };
        let mut app = App::new(Some("product".into()));
        app.set_body_width(120);
        app.apply(responsive());
        assert!(app.collapsed_panes().is_empty());
        app.focus = 1;
        app.set_body_width(80);
        assert_eq!(app.collapsed_panes(), [Pane::Detail].into());
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        app.set_body_width(120);
        assert!(app.collapsed_panes().is_empty());
        assert_eq!(
            app.focused_pane(),
            Some(Pane::Rows),
            "widening never steals focus"
        );
        app.set_body_width(80);
        app.toggle_panes(&[Pane::Detail]);
        assert!(
            app.collapsed_panes().is_empty(),
            "explicit expansion wins below threshold"
        );
        app.set_body_width(120);
        app.apply(responsive());
        assert!(
            app.collapsed_panes().is_empty(),
            "override survives refresh and widening"
        );
        app.toggle_panes(&[Pane::Detail]);
        assert!(
            app.collapsed_panes().contains(&Pane::Detail),
            "explicit collapse wins above threshold"
        );
        app.set_body_width(80);
        app.set_body_width(200);
        assert!(app.collapsed_panes().contains(&Pane::Detail));
        let mut changed = responsive();
        changed
            .view
            .as_mut()
            .unwrap()
            .board
            .fold_below
            .as_mut()
            .unwrap()
            .width = 90;
        app.apply(changed);
        assert!(
            app.collapsed_panes().is_empty(),
            "changed board clears overrides"
        );
        assert!(app.view.as_ref().unwrap().board.collapsed.is_empty());
        app.apply(folding_snapshot("product"));
        for width in [80, 120, 200] {
            app.set_body_width(width);
            assert!(
                app.collapsed_panes().is_empty(),
                "no rule keeps the original presentation"
            );
        }
    }

    #[test]
    fn grouped_toggle_folds_mixed_panes_then_expands_and_keeps_each_override() {
        for tmux in [false, true] {
            let mut snapshot = folding_snapshot("product");
            let view = snapshot.view.as_mut().unwrap();
            view.board = crate::config::Config::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/markup-parity.toml"),
            )
            .unwrap()
            .board("team")
            .unwrap();
            view.bindings = crate::action::preset(tmux, &view.board.panes);
            let mut app = App::new(Some("product".into()));
            app.set_body_width(120);
            app.apply(snapshot);
            app.focus = 1;
            press(&mut app, KeyCode::Char('d'));
            assert_eq!(app.collapsed_panes(), [Pane::Detail, Pane::Replies].into());
            assert_eq!(app.focused_pane(), Some(Pane::Rows));
            press(&mut app, KeyCode::Char('d'));
            assert_eq!(
                app.focused_pane(),
                Some(Pane::Rows),
                "unfolding never steals focus"
            );
            assert!(app.collapsed_panes().is_empty());
            app.toggle_panes(&[Pane::Detail]);
            assert_eq!(app.collapsed_panes(), [Pane::Detail].into());
            press(&mut app, KeyCode::Char('d'));
            assert_eq!(
                app.collapsed_panes(),
                [Pane::Detail, Pane::Replies].into(),
                "any expanded member folds the whole group"
            );
            press(&mut app, KeyCode::Char('d'));
            for width in [80, 120, 200] {
                app.set_body_width(width);
                assert!(
                    app.collapsed_panes().is_empty(),
                    "both explicit expansions override fold_below"
                );
            }
            press(&mut app, KeyCode::Char('d'));
            app.set_body_width(200);
            assert_eq!(app.collapsed_panes(), [Pane::Detail, Pane::Replies].into());
            assert!(app.view.as_ref().unwrap().board.collapsed.is_empty());
        }
    }

    #[test]
    fn pane_defaults_cover_every_layout_and_group_state_is_per_squad_and_session() {
        let config = crate::config::Config::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/markup-parity.toml"),
        )
        .unwrap();
        for layout in ["crew", "pr-queue", "minimal", "team"] {
            let board = config.board(layout).unwrap();
            let snapshot = |squad: &str| {
                let mut snapshot = folding_snapshot(squad);
                let view = snapshot.view.as_mut().unwrap();
                view.board = board.clone();
                view.bindings = config.bindings(true, &board.panes).unwrap();
                snapshot
            };
            let mut app = App::new(Some("product".into()));
            app.set_body_width(120);
            app.apply(snapshot("product"));
            let targets: BTreeSet<_> = board
                .panes
                .iter()
                .filter(|pane| [Pane::Detail, Pane::Replies].contains(pane))
                .copied()
                .collect();
            press(&mut app, KeyCode::Char('d'));
            assert_eq!(app.collapsed_panes(), targets, "{layout}");
            assert!(app.notice.is_none());
            app.apply(snapshot("product"));
            assert_eq!(app.collapsed_panes(), targets, "reload retains overrides");
            app.apply(snapshot("infra"));
            app.go("infra".into());
            assert!(
                app.collapsed_panes().is_empty(),
                "another squad has its own defaults"
            );
            app.go("product".into());
            assert_eq!(
                app.collapsed_panes(),
                targets,
                "tab switch retains group state"
            );
            let mut restarted = App::new(Some("product".into()));
            restarted.set_body_width(120);
            restarted.apply(snapshot("product"));
            assert!(
                restarted.collapsed_panes().is_empty(),
                "restart uses config, never persisted overrides"
            );
        }
    }

    #[test]
    fn partial_group_toggles_present_panes_and_an_absent_group_is_silent() {
        let mut snapshot = folding_snapshot("product");
        let view = snapshot.view.as_mut().unwrap();
        view.board = Board::simple(
            BoardMode::Split,
            crate::config::Direction::TopBottom,
            vec![Pane::Detail],
            &[100],
        );
        view.bindings = crate::action::preset(true, &view.board.panes);
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot);
        app.perform(&Action::parse("toggle detail replies").unwrap());
        assert_eq!(
            app.collapsed_panes(),
            [Pane::Detail].into(),
            "partially present group acts on its available subset"
        );
        assert!(app.notice.is_none());
        app.perform(&Action::parse("toggle rows notes").unwrap());
        assert_eq!(
            app.collapsed_panes(),
            [Pane::Detail].into(),
            "entirely absent group changes nothing"
        );
        assert!(app.notice.is_none());
        app.perform(&Action::parse("toggle detail replies").unwrap());
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.focused_pane(), None);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.focused_pane(), Some(Pane::Detail));
    }

    #[test]
    fn folding_refuses_a_stale_frame_tabs_and_an_absent_pane() {
        let mut app = App::new(Some("product".into()));
        app.apply(folding_snapshot("product"));
        app.go("infra".into());
        press(&mut app, KeyCode::Char('d'));
        assert!(app.collapsed_panes().is_empty());
        assert_eq!(app.notice.as_deref(), Some("Loading infra…"));
        let mut tabs = folding_snapshot("infra");
        tabs.view.as_mut().unwrap().board.mode = BoardMode::Tabs;
        app.apply(tabs);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(
            app.notice.as_deref(),
            Some("toggle applies to split mode only.")
        );
        app.apply(snapshot("infra", json!([])));
        press(&mut app, KeyCode::Char('d'));
        app.notice = None;
        press(&mut app, KeyCode::Char('d'));
        assert!(
            app.notice.is_none(),
            "no detail/replies means d is a silent no-op"
        );
    }
}

#[cfg(test)]
mod lead_row_tests {
    use super::tests::snapshot;
    use super::*;
    use serde_json::json;

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn lead() -> Value {
        json!({"id": "L", "name": "sol", "fields": {"task": "coordinate"}})
    }

    fn member(name: &str) -> Value {
        json!({"id": name, "name": name, "fields": {"task": "work"}})
    }

    /// `product` with a lead and the named members, as a loaded board.
    fn board(members: &[&str]) -> App {
        let mut app = App::new(Some("product".into()));
        let rows: Vec<_> = members.iter().map(|name| member(name)).collect();
        let mut loaded = snapshot("product", json!([{"title": null, "rows": rows}]));
        let view = loaded.view.as_mut().unwrap();
        view.document["squad"]["lead"] = lead();
        view.bindings = crate::action::preset(true, &[]);
        view.configured_bindings = view.bindings.clone();
        view.me = Some("Ben".into());
        app.apply(loaded);
        app
    }

    fn shape(app: &App) -> Vec<String> {
        app.items()
            .iter()
            .filter_map(|item| match item {
                Item::Section(title) => title.map(|title| format!("# {title}")),
                Item::Rule(rule) => Some(format!("-- {}", rule.label())),
                Item::Row(_, row) => Some(row["name"].as_str().unwrap().to_owned()),
            })
            .collect()
    }

    #[test]
    fn the_lead_is_the_first_row_and_the_cursor_starts_on_it() {
        let mut app = board(&["amy", "bob"]);
        assert_eq!(shape(&app), ["sol", "-- members · 2", "amy", "bob"]);
        assert_eq!(app.selected_row().unwrap()["name"], "sol");
        // The rule is not a row: one step goes from the lead to the first member.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_row().unwrap()["name"], "amy");
        press(&mut app, KeyCode::Up);
        assert_eq!(app.selected_row().unwrap()["name"], "sol");
    }

    #[test]
    fn showing_another_squad_puts_the_cursor_on_its_lead() {
        let mut app = board(&["amy", "bob"]);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_row().unwrap()["name"], "bob");
        app.current = Some("infra".into());
        let mut other = snapshot("infra", json!([{"title": null, "rows": [member("cai")]}]));
        other.view.as_mut().unwrap().document["squad"]["lead"] =
            json!({"id": "M", "name": "mia", "fields": {}});
        app.apply(other);
        assert_eq!(app.selected_row().unwrap()["name"], "mia");
    }

    #[test]
    fn a_lead_with_no_members_and_a_squad_without_a_lead() {
        let app = board(&[]);
        assert_eq!(shape(&app), ["sol", "-- members · 0 · none yet"]);
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{"title": null, "rows": [member("amy")]}]),
        ));
        assert_eq!(shape(&app), ["amy"]);
        assert_eq!(app.selected_row().unwrap()["name"], "amy");
    }

    #[test]
    fn search_matches_the_lead_and_keeps_the_rule() {
        let mut app = board(&["amy", "bob"]);
        app.search = "sol".into();
        assert_eq!(shape(&app), ["sol", "-- members · 0"]);
        app.search = "bob".into();
        assert_eq!(shape(&app), ["-- members · 1", "bob"]);
    }

    #[test]
    fn r_answers_a_request_else_notes_a_pending_only_member_else_says_so() {
        let mut app = board(&["amy", "bob", "cai"]);
        let view = app.view.as_mut().unwrap();
        view.document["sections"][0]["rows"][0]["pending"] = json!("pick a database");
        view.document["sections"][0]["rows"][0]["waitingOnYou"] =
            json!([{"requestId": "q1", "preview": "ship it?"}]);
        view.document["sections"][0]["rows"][1]["pending"] = json!("review the plan");
        app.select(1);
        // A real request wins over the pending text.
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Reply { ref request, .. } if request == "q1"
        ));
        press(&mut app, KeyCode::Esc);
        // Pending alone opens a note to that member; nothing is sent by opening,
        // cancelling or an empty Enter.
        app.select(2);
        assert_eq!(press(&mut app, KeyCode::Char('r')), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().header(), "✎ note → bob");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none());
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
        press(&mut app, KeyCode::Char('r'));
        for character in "go ahead".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Annotate {
                me: "Ben".into(),
                squad: "product".into(),
                to: "bob".into(),
                row: "bob".into(),
                text: "go ahead".into()
            })
        );
        // The pending text stays: nothing acknowledged or cleared it.
        assert_eq!(app.rows()[2].1["pending"], "review the plan");
        // Neither a request nor pending text: the notice, and no composer.
        app.select(3);
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.notice.as_deref(), Some("cai is not waiting on you."));
        assert!(app.input.is_none());
    }

    #[test]
    fn the_lead_row_takes_the_ordinary_row_actions() {
        let mut app = board(&["amy"]);
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Jump("sol".into()))
        );
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.input.as_ref().unwrap().prompt, "→ sol (product)");
        for character in "hello".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "product".into(),
                to: "sol".into(),
                text: "hello".into()
            })
        );
        // The lead's occurrence is the lead's own, so a member row never shares it.
        assert!(matches!(app.row_target(0), Some(RowTarget::Lead { .. })));
        assert!(matches!(app.row_target(1), Some(RowTarget::Member { .. })));
    }
}

#[cfg(test)]
mod token_window_tests {
    use super::tests::{bind, crew};
    use super::*;
    use crate::config::{TokenRate, TokenWindow};
    use serde_json::json;
    fn app() -> App {
        let mut app = crew(crate::action::preset(false, &[]), Vec::new());
        app.meter = Some(super::super::meter::Meter::new(
            TokenRate {
                enabled: true,
                ..Default::default()
            },
            &super::super::rate::tests::input(100),
            Instant::now(),
        ));
        app
    }
    fn home(now: Instant) -> App {
        let mut app = App::new(Some(super::super::ALL.into()));
        let mut snapshot = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        let view = snapshot.view.as_mut().unwrap();
        view.home = Some(super::super::home::Home {
            windows: TokenWindow::DEFAULTS,
            summary: Default::default(),
            sections: Vec::new(),
            failures: Vec::new(),
            incomplete: false,
            squads: vec![super::super::home::SquadLine {
                squad: "product".into(),
                lead: Some(serde_json::json!({"id":"a"})),
                counts: Default::default(),
                members: Default::default(),
            }],
        });
        let mut input = super::super::rate::tests::input(100);
        input.resumes.insert("missing".into(), Value::Null);
        let settings = TokenRate {
            enabled: true,
            ..Default::default()
        };
        view.home_rate.insert(
            "product".into(),
            RateView {
                history: None,
                settings,
                input: input.clone(),
            },
        );
        app.apply(snapshot);
        app.meters.insert(
            "product".into(),
            super::super::meter::Meter::new(settings, &input, now),
        );
        app
    }

    fn header(now: Instant) -> App {
        let mut app = home(now);
        app.view.as_mut().unwrap().document["sections"] = json!([{"rows": [
            {"squad":"product", "name":"product"}
        ]}]);
        app
    }

    fn extra(app: &mut App, input: super::super::rate::Input, settings: TokenRate, now: Instant) {
        let view = app.view.as_mut().unwrap();
        view.home
            .as_mut()
            .unwrap()
            .squads
            .push(super::super::home::SquadLine {
                squad: "extra".into(),
                lead: Some(json!({"id":"a"})),
                counts: Default::default(),
                members: Default::default(),
            });
        view.document["sections"][0]["rows"]
            .as_array_mut()
            .unwrap()
            .push(json!({"squad":"extra", "name":"extra"}));
        app.meters.insert(
            "extra".into(),
            super::super::meter::Meter::new(settings, &input, now),
        );
        view.home_rate.insert(
            "extra".into(),
            RateView {
                input,
                settings,
                history: None,
            },
        );
    }

    #[test]
    fn header_distinguishes_disabled_warmup_zero_and_missing_members() {
        let now = Instant::now();
        let mut app = header(now);
        let initial = app.home_header_usage(now).unwrap();
        assert_eq!(initial.totals, [None; 3]);
        assert_eq!(initial.unreported, 2);
        assert!(initial.top.is_none() && initial.models.is_empty());
        let time = now + Duration::from_secs(10);
        app.sample_home(Ok(&super::super::rate::tests::input(100).resumes), time);
        let zero = app.home_header_usage(time).unwrap();
        assert_eq!(zero.totals[2].unwrap().tokens, 0);
        assert!(zero.totals[2].unwrap().partial);
        assert_eq!(zero.unreported, 1);
        assert!(zero.top.is_none() && zero.models.is_empty());
        app.search = "not-shown".into();
        assert!(app.home_header_usage(time).is_none());
        app.search.clear();
        app.view
            .as_mut()
            .unwrap()
            .home_rate
            .get_mut("product")
            .unwrap()
            .settings
            .enabled = false;
        assert!(app.home_header_usage(time).is_none());
    }

    #[test]
    fn header_deduplicates_lead_members_and_uses_one_model_share_denominator() {
        let now = Instant::now();
        let mut app = header(now);
        let mut input = super::super::rate::tests::input(100);
        input.resumes.get_mut("a").unwrap()["model"] = json!("model-a");
        let mut b = input.resumes["a"].clone();
        b["model"] = json!("model-b");
        input.resumes.insert("b".into(), b);
        input.names.insert("b".into(), "worker-b".into());
        extra(
            &mut app,
            input.clone(),
            TokenRate {
                enabled: true,
                ..Default::default()
            },
            now,
        );
        let public = app.view.as_ref().unwrap().document.clone();
        let time = now + Duration::from_secs(20);
        let mut resumes = super::super::rate::tests::input(200).resumes;
        resumes.get_mut("a").unwrap()["model"] = json!("model-a");
        let mut b = super::super::rate::tests::input(400)
            .resumes
            .remove("a")
            .unwrap();
        b["model"] = json!("model-b");
        resumes.insert("b".into(), b);
        app.sample_home(Ok(&resumes), time);
        let usage = app.home_header_usage(time).unwrap();
        assert_eq!(usage.totals.map(|total| total.unwrap().tokens), [600; 3]);
        assert_eq!(
            usage.unreported, 1,
            "duplicate UUID a is counted once, not once per squad"
        );
        let top = usage.top.unwrap();
        assert_eq!(top.member, "worker-b");
        assert_eq!(
            top.share,
            UsageShare {
                fraction: 0.75,
                partial: true
            }
        );
        assert_eq!(usage.models.len(), 2);
        assert_eq!(usage.models[0].model, Some("model-b"));
        assert_eq!(usage.models[0].share.fraction, 0.75);
        assert_eq!(usage.models[1].share.fraction, 0.25);
        assert_eq!(app.view.as_ref().unwrap().document, public);
        app.search = "product".into();
        let filtered = app.home_header_usage(time).unwrap();
        assert_eq!(filtered.totals[2].unwrap().tokens, 150);
        assert_eq!(filtered.models.len(), 1);
        app.search.clear();
        app.view.as_mut().unwrap().home_rate.remove("extra");
        assert_eq!(
            app.home_header_usage(time).unwrap().totals[2]
                .unwrap()
                .tokens,
            150
        );
    }

    #[test]
    fn header_uuid_collision_prefers_verified_evidence_then_displayed_squad_order() {
        let now = Instant::now();
        let mut app = header(now);
        app.view
            .as_mut()
            .unwrap()
            .home_rate
            .get_mut("product")
            .unwrap()
            .input
            .resumes
            .remove("missing");
        let settings = TokenRate {
            enabled: true,
            ..Default::default()
        };
        extra(
            &mut app,
            super::super::rate::tests::input(100),
            settings,
            now - Duration::from_secs(60),
        );
        let time = now + Duration::from_secs(20);
        app.meters
            .get_mut("product")
            .unwrap()
            .sample(Ok(&super::super::rate::tests::input(400)), time);
        let mut better = super::super::rate::tests::input(200);
        better.resumes.get_mut("a").unwrap()["model"] = json!("better-covered");
        app.meters
            .get_mut("extra")
            .unwrap()
            .sample(Ok(&better), time);
        let usage = app.home_header_usage(time).unwrap();
        assert_eq!(
            usage.totals[2].unwrap().tokens,
            150,
            "more tokens cannot override better evidence"
        );
        assert_eq!(usage.models[0].model, Some("better-covered"));
        let mut tied =
            super::super::meter::Meter::new(settings, &super::super::rate::tests::input(100), now);
        tied.sample(Ok(&super::super::rate::tests::input(300)), time);
        app.meters.insert("extra".into(), tied);
        assert_eq!(
            app.home_header_usage(time).unwrap().totals[2]
                .unwrap()
                .tokens,
            450
        );
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .squads
            .swap(0, 1);
        assert_eq!(
            app.home_header_usage(time).unwrap().totals[2]
                .unwrap()
                .tokens,
            300
        );
    }

    #[test]
    fn header_queries_global_durations_and_keeps_shorter_squad_history_partial() {
        let now = Instant::now();
        let mut app = header(now);
        let long = TokenWindow::parse("2h").unwrap();
        app.view.as_mut().unwrap().home.as_mut().unwrap().windows =
            [TokenWindow::MINUTE, TokenWindow::HOUR, long];
        let time = now + Duration::from_secs(20);
        app.sample_home(Ok(&super::super::rate::tests::input(200).resumes), time);
        let usage = app.home_header_usage(time).unwrap();
        assert_eq!(usage.windows[2].label(), "2h");
        assert_eq!(usage.totals.map(|total| total.unwrap().tokens), [150; 3]);
        assert!(usage.totals[2].unwrap().partial);
        assert_eq!(
            app.home_usage("product", time).unwrap().windows,
            TokenWindow::DEFAULTS
        );
        // The original delta ages out of squad w2=5m, but global w2=1h retains it.
        let time = now + Duration::from_secs(360);
        app.sample_home(Ok(&super::super::rate::tests::input(200).resumes), time);
        assert_eq!(
            app.home_header_usage(time)
                .unwrap()
                .totals
                .map(|total| total.unwrap().tokens),
            [0, 150, 150]
        );
        assert_eq!(
            app.home_usage("product", time).unwrap().lead[1]
                .unwrap()
                .tokens,
            0
        );
        let mut failed = super::tests::snapshot(super::super::ALL, json!([]));
        failed.view = Err("HOME read failed".into());
        app.apply(failed);
        assert!(
            app.home_header_usage(time).unwrap().totals[2]
                .unwrap()
                .partial
        );
    }

    #[test]
    fn failed_switch_back_to_home_restores_its_member_meter() {
        let now = Instant::now();
        let mut app = home(now);
        assert!(matches!(app.go("product".into()), Effect::Load(_)));
        assert!(app.loading() && app.meter.is_some());
        assert!(!app.meters.contains_key("product"));

        let mut failed = super::tests::snapshot("product", serde_json::json!([]));
        failed.view = Err("product load failed".into());
        app.apply(failed);
        assert_eq!(app.current.as_deref(), Some(super::super::ALL));
        assert!(app.meter.is_none() && app.meters.contains_key("product"));
        assert!(app.home_usage("product", now).is_some());
        assert_eq!(app.error.as_deref(), Some("product load failed"));
    }

    #[test]
    fn home_projection_preserves_unknown_zero_partial_share_and_public_document() {
        let now = Instant::now();
        let mut app = home(now);
        let public = app.view.as_ref().unwrap().document.clone();
        assert!(app.home_usage("unknown", now).is_none());
        let initial = app.home_usage("product", now).unwrap();
        assert_eq!(initial.windows, TokenWindow::DEFAULTS);
        assert_eq!(initial.lead, [None; 3]);
        assert_eq!(initial.squad, [None; 3]);
        assert_eq!(initial.share, None);
        let mut receipt = super::super::rate::tests::input(100).resumes;
        receipt.get_mut("a").unwrap()["model"] = serde_json::json!("current-model");
        let zero_time = now + Duration::from_secs(10);
        assert!(app.sample_home(Ok(&receipt), zero_time));
        let zero = app.home_usage("product", zero_time).unwrap();
        assert_eq!(zero.lead_model, Some("current-model"));
        assert_eq!(zero.lead[2].unwrap().tokens, 0);
        assert_eq!(zero.share, None, "zero denominator has no share");
        let receipt = super::super::rate::tests::input(200).resumes;
        let time = now + Duration::from_secs(20);
        assert!(app.sample_home(Ok(&receipt), time));
        let usage = app.home_usage("product", time).unwrap();
        assert_eq!(usage.lead[2].unwrap().tokens, 150);
        assert_eq!(usage.squad[2].unwrap().tokens, 150);
        assert_eq!(
            usage.share,
            Some(UsageShare {
                fraction: 1.0,
                partial: true
            })
        );
        assert_eq!(app.view.as_ref().unwrap().document, public);
        let mut failed = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        failed.view = Err("HOME acquisition failed".into());
        app.apply(failed);
        let recovered = super::super::rate::tests::input(400).resumes;
        let recovered_time = time + Duration::from_secs(10);
        assert!(app.sample_home(Ok(&recovered), recovered_time));
        assert_eq!(
            app.home_usage("product", recovered_time).unwrap().squad[2]
                .unwrap()
                .tokens,
            150,
            "a failed HOME reload closes continuity before recovery"
        );
        app.view.as_mut().unwrap().home.as_mut().unwrap().squads[0].lead = None;
        assert_eq!(app.home_usage("product", time).unwrap().lead, [None; 3]);
        app.view
            .as_mut()
            .unwrap()
            .home_rate
            .get_mut("product")
            .unwrap()
            .settings
            .enabled = false;
        assert!(!app.sample_home(Ok(&receipt), time + Duration::from_secs(10)));
        assert!(app.home_usage("product", time).is_none());
    }

    #[test]
    fn home_model_remains_known_when_sampling_is_off_without_inventing_usage() {
        let now = Instant::now();
        let mut app = home(now);
        let public = app.view.as_ref().unwrap().document.clone();
        let rate = app
            .view
            .as_mut()
            .unwrap()
            .home_rate
            .get_mut("product")
            .unwrap();
        rate.settings.enabled = false;
        rate.input.resumes.get_mut("a").unwrap()["model"] = serde_json::json!("off-model");
        assert_eq!(app.home_lead_model("product"), Some("off-model"));
        assert!(app.home_usage("product", now).is_none());
        let mut receipt = super::super::rate::tests::input(200).resumes;
        receipt.get_mut("a").unwrap()["model"] = serde_json::json!("new-model");
        assert!(!app.sample_home(Ok(&receipt), now + Duration::from_secs(10)));
        assert_eq!(app.home_lead_model("product"), Some("new-model"));
        assert!(app.home_usage("product", now).is_none());
        assert_eq!(app.view.as_ref().unwrap().document, public);
        assert!(app.home_lead_model("unknown").is_none());
    }

    #[test]
    fn home_refresh_prunes_rosters_and_replaces_changed_window_policy() {
        let now = Instant::now();
        let mut app = home(now);
        let receipt = super::super::rate::tests::input(200).resumes;
        let time = now + Duration::from_secs(10);
        app.sample_home(Ok(&receipt), time);
        let mut view = app.view.take().unwrap();
        view.home_rate
            .get_mut("product")
            .unwrap()
            .input
            .resumes
            .remove("a");
        let mut snapshot = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        snapshot.view = Ok(view);
        app.apply(snapshot);
        assert_eq!(app.home_usage("product", time).unwrap().lead, [None; 3]);
        let mut view = app.view.take().unwrap();
        let rate = view.home_rate.get_mut("product").unwrap();
        rate.input = super::super::rate::tests::input(100);
        rate.settings.windows = [
            TokenWindow::MINUTE,
            TokenWindow::FIVE_MINUTES,
            TokenWindow::parse("2h").unwrap(),
        ];
        let mut snapshot = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        snapshot.view = Ok(view);
        app.apply(snapshot);
        let usage = app.home_usage("product", Instant::now()).unwrap();
        assert_eq!(usage.windows[2], TokenWindow::parse("2h").unwrap());
        assert_eq!(
            usage.squad, [None; 3],
            "a changed policy starts a fresh meter"
        );
    }

    #[test]
    fn home_receipts_ignore_loading_other_tabs_and_unrelated_identities() {
        let now = Instant::now();
        let mut app = home(now);
        let mut receipt = super::super::rate::tests::input(200).resumes;
        receipt.insert("outsider".into(), receipt["a"].clone());
        let time = now + Duration::from_secs(10);
        assert!(app.sample_home(Ok(&receipt), time));
        assert_eq!(
            app.home_usage("product", time).unwrap().squad[2]
                .unwrap()
                .tokens,
            150
        );
        app.go("other".into());
        assert!(!app.sample_home(Ok(&receipt), time + Duration::from_secs(10)));
        app.apply(super::tests::snapshot("other", serde_json::json!([])));
        assert!(!app.sample_home(Ok(&receipt), time + Duration::from_secs(20)));
        assert!(app.home_usage("product", time).is_none());
    }

    #[test]
    fn window_binding_overrides_and_text_inputs_keep_their_owner() {
        let mut app = app();
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        assert_eq!(app.key(key), Effect::None);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        app.searching = true;
        assert_eq!(app.key(key), Effect::None);
        assert_eq!(app.search, "w");
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        app.searching = false;
        for compose in [
            Compose::Talk { to: "a".into() },
            Compose::Reply {
                request: "q".into(),
                from: "a".into(),
            },
            Compose::Annotate {
                to: "a".into(),
                row: "a".into(),
            },
        ] {
            app.input = Some(Input {
                row_send: None,
                alternative: None,
                quote: None,
                link: None,
                prompt: "message".into(),
                text: String::new(),
                compose,
                squad: "x".into(),
                hint: None,
            });
            app.key(key);
            assert_eq!(app.input.as_ref().unwrap().text, "w");
            assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        }
        app.input = None;
        app.search.clear();
        app.view
            .as_mut()
            .unwrap()
            .bindings
            .extend(bind(&[("w", "refresh"), ("v", "token-window")]));
        assert_eq!(app.key(key), Effect::Refresh);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(app.token_window, TokenWindow::HOUR);
        app.view.as_mut().unwrap().section_bindings = vec![bind(&[("w", "token-window")])];
        app.key(key);
        assert_eq!(app.token_window, TokenWindow::MINUTE);
    }
    #[test]
    fn window_switch_updates_column_priority_even_with_unchanged_zero_values() {
        let mut app = app();
        let settings = crate::config::Config::read(
            std::env::temp_dir().join(format!("active-window-{}.toml", std::process::id())),
        )
        .unwrap();
        app.view.as_mut().unwrap().rows = settings.rows("product").unwrap();
        app.project_usage(Instant::now());
        for selected in [0, 1, 2, 0] {
            let view = app.view.as_ref().unwrap();
            for (index, column) in view.rows.columns[5..].iter().enumerate() {
                assert_eq!(column.priority == Some(1), index == selected);
            }
            app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        }
    }

    #[test]
    fn tab_rings_are_reused_and_pruned_against_configured_tabs() {
        let mut app = app();
        app.tabs = vec!["product".into(), "other".into()];
        app.go("other".into());
        assert!(app.meter.is_none());
        assert_eq!(app.meters.len(), 1);
        app.go("product".into());
        assert!(app.meter.is_some());
        assert!(app.meters.is_empty());
        app.go("other".into());
        let mut snapshot = super::tests::snapshot("other", serde_json::json!([]));
        snapshot.tabs = vec!["other".into()];
        snapshot.hidden.clear();
        app.apply(snapshot);
        assert!(app.meters.is_empty());
    }
    #[test]
    fn usage_projection_updates_duplicate_rows_without_mutating_public_document() {
        let now = Instant::now();
        let rows = serde_json::json!([{"title": null, "rows": [
            {"id":"a", "name":"member", "fields": {"task":"keep"}},
            {"id":"a", "name":"member", "fields": {"task":"keep"}},
            {"id":"never", "name":"unreported", "fields": {}}
        ]}]);
        let mut app = App::new(Some("product".into()));
        app.apply(super::tests::snapshot("product", rows));
        app.view.as_mut().unwrap().rows = crate::config::Config::read(
            std::env::temp_dir().join(format!("usage-projection-{}.toml", std::process::id())),
        )
        .unwrap()
        .rows("product")
        .unwrap();
        let public = app.view.as_ref().unwrap().document.clone();
        let mut first = super::super::rate::tests::input(100);
        first.resumes.get_mut("a").unwrap()["model"] = serde_json::json!("old");
        first.resumes.insert("never".into(), Value::Null);
        app.meter = Some(super::super::meter::Meter::new(
            TokenRate {
                enabled: true,
                ..Default::default()
            },
            &first,
            now,
        ));
        app.project_usage(now);
        assert_eq!(app.rows()[0].1["fields"]["tok_1"], "–");
        let mut next = super::super::rate::tests::input(200);
        next.resumes.get_mut("a").unwrap()["model"] = serde_json::json!("new");
        next.resumes.insert("never".into(), Value::Null);
        let time = now + Duration::from_secs(60);
        app.meter.as_mut().unwrap().sample(Ok(&next), time);
        app.project_usage(time);
        for (_, row) in app.rows().into_iter().take(2) {
            assert_eq!(row["fields"]["tok_1"], "150");
            assert_eq!(row["fields"]["tok_2"], "~150");
            assert_eq!(row["fields"]["model"], "new");
            assert_eq!(row["fields"]["task"], "keep");
        }
        assert_eq!(app.rows()[2].1["fields"]["tok_3"], "–");
        assert_eq!(app.view.as_ref().unwrap().document, public);
        let projected = app.usage_document.clone();
        app.project_usage(time);
        assert_eq!(
            app.usage_document, projected,
            "same receipt does not change display"
        );
        // The lead is an ordinary row of the display: it gets the same columns.
        app.view.as_mut().unwrap().document["squad"]["lead"] =
            serde_json::json!({"id": "a", "name": "boss", "fields": {}});
        app.project_usage(time);
        let lead = app.rows()[0].1;
        assert_eq!(lead["name"], "boss");
        assert_eq!(lead["fields"]["tok_1"], "150");
        assert_eq!(lead["fields"]["model"], "new");
        assert!(app.view.as_ref().unwrap().document["squad"]["lead"]["fields"]["tok_1"].is_null());
        app.view.as_mut().unwrap().document["squad"]["lead"] = Value::Null;
        app.project_usage(time);
        assert_eq!(app.usage_document, projected);
        app.go("uncached".into());
        app.project_usage(time);
        assert_eq!(
            app.usage_document, projected,
            "loading preserves painted owner"
        );
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use serde_json::json;
    fn app(target: &str) -> App {
        let mut app = tests::crew(Default::default(), Vec::new());
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        view.document["squad"]["lead"] = json!({"name":"auth-fix"});
        view.notes = Notes::Text(format!("[action]({target})"));
        let Notes::Text(text) = &view.notes else {
            unreachable!()
        };
        let mapped = super::super::markdown::render_links(text, 80, view.look, &view.links);
        view.derived.borrow_mut().notes = Some(super::super::derived::NotebookLines {
            width: 80,
            look: view.look,
            lines: mapped.lines,
            sources: mapped.sources,
            links: mapped.links,
            hits: mapped.hits,
        });
        app.select_link(1);
        app
    }
    fn enter(app: &mut App) -> Effect {
        app.input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    }
    #[test]
    fn builtins_use_the_existing_prompt_and_cancel_or_stale_targets_send_nothing() {
        for verb in ["talk", "annotate"] {
            let mut app = app(&format!("tmt:{verb}/auth-fix?text=hello%20%24%28id%29"));
            assert_eq!(app.activate_link(), Effect::None);
            assert_eq!(app.input.as_ref().unwrap().text, "hello $(id)");
            assert_eq!(
                app.input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Effect::None
            );
            assert!(app.input.is_none());
            app.activate_link();
            app.view.as_mut().unwrap().document["sections"][0]["rows"] = json!([]);
            app.view.as_mut().unwrap().document["squad"]["lead"] = serde_json::Value::Null;
            assert_eq!(enter(&mut app), Effect::None);
            assert!(app.notice.as_ref().unwrap().contains("nothing sent"));
        }
        let mut app = app("tmt:talk/auth-fix?text=hello");
        app.activate_link();
        app.view.as_mut().unwrap().me = Some("Other".into());
        assert_eq!(enter(&mut app), Effect::None);
        app.view.as_mut().unwrap().me = Some("Ben".into());
        app.activate_link();
        assert_eq!(
            enter(&mut app),
            Effect::Act(Request::Talk {
                me: "Ben".into(),
                squad: "product".into(),
                to: "auth-fix".into(),
                text: "hello".into()
            })
        );
    }
    #[test]
    fn notebook_send_to_a_lead_outside_member_rows_keeps_its_validation() {
        for changed in [false, true] {
            let mut app = app("tmt:talk/Lead?text=hello");
            app.view.as_mut().unwrap().document["squad"]["lead"] =
                json!({"id":"lead-id", "name":"Lead"});
            assert_eq!(app.activate_link(), Effect::None);
            assert_eq!(app.input.as_ref().unwrap().header(), "→ Lead (product)");
            if changed {
                app.view.as_mut().unwrap().document["squad"]["lead"]["id"] = json!("replacement");
                assert_eq!(enter(&mut app), Effect::None);
            } else {
                assert!(
                    matches!(enter(&mut app), Effect::Act(Request::Talk { ref to, .. }) if to == "Lead")
                );
            }
        }
    }

    #[test]
    fn answer_picker_is_prefilled_and_revalidates_current_open_requests() {
        let mut app = app("tmt:answer/auth-fix?text=answer");
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["waitingOnYou"] = json!([
            {"requestId":"one", "preview":"first"}, {"requestId":"two", "preview":"second"}
        ]);
        assert_eq!(app.activate_link(), Effect::None);
        assert!(app.input.is_none());
        assert_eq!(app.menu.as_ref().unwrap().prefill, "answer");
        app.menu_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input.as_ref().unwrap().text, "answer");
        assert_eq!(
            enter(&mut app),
            Effect::Act(Request::Reply {
                me: "Ben".into(),
                request: "one".into(),
                from: "auth-fix".into(),
                text: "answer".into()
            })
        );
        app.activate_link();
        app.menu_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
        assert_eq!(enter(&mut app), Effect::None);
    }
    #[test]
    fn all_member_verbs_refuse_nonmembers_and_a_retained_loading_frame() {
        for verb in ["jump", "talk", "answer", "open", "copy", "annotate"] {
            let mut app = app(&format!("tmt:{verb}/missing"));
            assert_eq!(app.activate_link(), Effect::None);
            assert!(app.input.is_none() && app.menu.is_none());
        }
        let mut lead = app("tmt:jump/Lead");
        lead.view.as_mut().unwrap().document["squad"]["lead"] =
            json!({"id":"lead-id", "name":"Lead"});
        assert_eq!(
            lead.activate_link(),
            Effect::Act(Request::Jump("Lead".into()))
        );
        let mut app = app("tmt:jump/auth-fix");
        assert_eq!(
            app.activate_link(),
            Effect::Act(Request::Jump("auth-fix".into()))
        );
        app.current = Some("infra".into());
        assert_eq!(app.activate_link(), Effect::None);
    }
}
