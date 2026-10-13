//! Board state and key handling, independent of the terminal. Every keypress
//! works on what is already loaded; loading happens in the refresh worker,
//! and actions leave as fully resolved requests.

use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::display_rows::{Item, RowOrigin};
use crate::{
    action::{Action, Bindings, Verb},
    attention::Attention,
    config::{Board, Config, NotesRender, Pane},
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

mod carry;
pub(in crate::board) mod workspace;

/// What the refresh worker loaded for one squad.
pub struct RateView {
    pub settings: crate::config::TokenRate,
    pub input: super::rate::Input,
    pub history: Option<super::rate::history::Seeds>,
}

/// Raw observations for tiles and headers; formatting belongs to the caller.
#[derive(Debug)]
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
    pub history_pending: bool,
    pub ask_lead: String,
    pub token_rate: Option<RateView>,
    /// HOME sampling templates, separate from the painter/controller model.
    pub home_rate: BTreeMap<String, RateView>,
    pub(super) exchanges: Vec<super::home_leads::Lead>,
    /// The `status --json` document, so the board and `status` never differ.
    pub document: Value,
    /// Retained home composition; only the aggregate board view owns it.
    pub home: Option<super::home::Home>,
    pub(super) derived: RefCell<super::derived::Derived>,
    /// Authored CLI row projection, independent of interactive workspace geometry.
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
    /// `[bind.<host>]`: over `bindings` for a row whose pane is on that host.
    pub host_bindings: std::collections::BTreeMap<String, Bindings>,
    pub opener: Option<Vec<String>>,
    pub clipboard: Option<Vec<String>>,
    pub links: crate::links::Handlers,
    /// `[tabs.colors]`, re-read with every load like the rest of ops.toml.
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
    pub(super) references: super::inventory::Inventory,
    pub(super) timing: Option<super::timing::Load>,
    /// Squad names in core list order, before tab policy.
    pub squad_keys: Vec<String>,
    /// Tab keys in order: squad names and built-in tabs (`board::tabs`).
    pub tabs: Vec<String>,
    /// Hidden keys retained by the independent CLI tab projection.
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
    DirectTalk {
        sender: String,
        recipient: String,
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
    Status(Box<super::status_update::Action>),
    /// Save this tab order to `[tabs] order` (tab keys, in order).
    WorkspacePreferences {
        expected: crate::config::OpsPreferences,
        next: crate::config::OpsPreferences,
    },
    WorkspaceOperation(crate::membership::workspace::Operation),
    /// A cron job control, with its actor, job and viewed revision resolved.
    Cron(super::cronboard::CronRequest),
    /// Check that the person at the board may change a member's digest.
    DigestOpen(super::digest::Ask),
    /// Apply a digest value the user chose.
    Digest(super::digest::Set),
}

impl Request {
    /// What the board shows while the lane runs this request.
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Jump(_) | Self::Back => "Jumping…",
            Self::Open { .. } | Self::RevealFile { .. } | Self::Run(_) => "Opening…",
            Self::DigestOpen(_) => "Checking digest access…",
            Self::Digest(_) => "Saving digest…",
            Self::Copy { .. } => "Copying…",
            Self::Talk { .. }
            | Self::DirectTalk { .. }
            | Self::Annotate { .. }
            | Self::Reply { .. } => "Sending…",
            Self::Status(_) => "Updating status…",
            Self::WorkspacePreferences { .. } => "Saving workspace preferences…",
            Self::WorkspaceOperation(_) => "Applying workspace operation…",
            Self::Cron(_) => "Updating the job…",
        }
    }

    /// Sending changes request state, and a saved order changes the tabs, so
    /// the view reloads afterwards.
    pub fn sends(&self) -> bool {
        matches!(
            self,
            Self::Talk { .. }
                | Self::DirectTalk { .. }
                | Self::Annotate { .. }
                | Self::Reply { .. }
                | Self::WorkspacePreferences { .. }
                | Self::WorkspaceOperation(_)
                | Self::Cron(_)
                | Self::Status(_)
                | Self::Digest(_)
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
    ResetSetting,
    CycleTokenWindow,
    CancelSettings,
    PickTheme,
    SaveTheme,
    Act(Request),
    Checklist(super::checklist::load::Task),
}

/// What a menu entry does: run a binding, or reply to one open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    WorkspaceBegin(super::workspace::manage::Kind),
    WorkspaceLayout(crate::config::OpsLayout),
    Checklist,
    Action(Action),
    /// Confirms a cron control picked from a menu.
    Cron(super::cronboard::CronRequest),
    /// Leaves the menu without doing anything.
    Dismiss,
    Reply {
        request: String,
        from: String,
    },
    /// A digest value picked from the dropdown.
    Digest(super::digest::Set),
    /// The dropdown's own entry: type a value instead.
    DigestCustom(Box<super::digest::Granted>),
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
    Workspace,
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
    Status,
    /// Choose one acquired request through the existing request menu.
    AnswerPicker,
    /// One step of a cron form; its draft lives in `App::cron_draft`.
    Cron,
    /// A digest value typed for a member the user may change.
    Digest(Box<super::digest::Granted>),
}

/// A row occurrence, independent of its position after refresh or sorting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Workspace(super::workspace::OpsTarget),
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

/// The observed note recipient, including its identity when the source supplies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoteRecipient {
    id: Option<String>,
    name: String,
}

impl NoteRecipient {
    fn from_row(row: &Value) -> Option<Self> {
        Some(Self {
            id: row["id"].as_str().map(str::to_owned),
            name: row["name"].as_str()?.to_owned(),
        })
    }
}

#[derive(Clone)]
pub(super) struct RowSend {
    pub target: RowTarget,
    pub sender: String,
    pub sender_id: Option<String>,
    pub name: String,
    pub note: Option<Compose>,
    pub note_recipient: Option<NoteRecipient>,
    pub note_member: bool,
}

impl RowSend {
    pub(super) fn valid(&self, app: &App, input: &Input) -> bool {
        if app.loading()
            || app.view.as_ref().and_then(|view| view.me.as_ref()) != Some(&self.sender)
            || app.view.as_ref().and_then(|view| view.me_id.as_ref()) != self.sender_id.as_ref()
        {
            return false;
        }
        let Some(squad) = app.target_squad(&self.target) else {
            return false;
        };
        let Some(row) = app.target_row(&self.target) else {
            return false;
        };
        if squad != input.squad || row["name"].as_str() != Some(&self.name) {
            return false;
        }
        match &input.compose {
            Compose::Status => true,
            Compose::AnswerPicker => false,
            Compose::Talk { to } => to == &self.name,
            Compose::Reply { request, from } => {
                from == &self.name
                    && row["waitingOnYou"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["requestId"].as_str() == Some(request))
                    })
            }
            Compose::Annotate { .. } => {
                self.note.as_ref() == Some(&input.compose)
                    && (self.note_member || app.note_recipient(&self.target) == self.note_recipient)
            }
            Compose::AskLead { .. } | Compose::Cron | Compose::Workspace | Compose::Digest(_) => {
                false
            }
        }
    }
}

/// Shared message editor; the application owns submission and recipient fences.
pub struct Input {
    pub(super) draft_key: Option<(String, String)>,
    pub(super) row_send: Option<RowSend>,
    /// The row composer's other modes in cycle order (answer, note, talk): Tab
    /// makes the first one current and sends the current one to the back.
    pub(super) others: Vec<Compose>,
    pub(super) quote: Option<String>,
    pub(super) response_quotes: Vec<super::response::Quote>,
    pub link: Option<LinkSend>,
    pub prompt: String,
    pub text: tmt_tui::components::Editor,
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
    pub(super) fn target(&self) -> Option<&RowTarget> {
        self.row_send.as_ref().map(|send| &send.target)
    }

    pub(super) fn header(&self) -> String {
        match &self.compose {
            Compose::Talk { to } => {
                if self.squad.is_empty() {
                    format!("→ {to} · talk")
                } else {
                    format!("→ {to} ({}) · talk", self.squad)
                }
            }
            Compose::Reply { from, .. } => format!("→ {from} ({}) · answer", self.squad),
            Compose::Annotate { to, row } if to != row => {
                format!("→ {to} ({}) · note · about {row}", self.squad)
            }
            Compose::Annotate { to, .. } => format!("→ {to} ({}) · note", self.squad),
            Compose::AskLead { to, .. } => format!("→ lead {to}"),
            Compose::Cron | Compose::Workspace | Compose::Digest(_) => self.prompt.clone(),
            Compose::AnswerPicker => self.prompt.clone(),
            Compose::Status => format!(
                "Update status → {} / {}",
                self.squad,
                self.row_send
                    .as_ref()
                    .map_or("agent", |send| send.name.as_str())
            ),
        }
    }

    /// The modes of a row composer in cycle order, for the hint line.
    pub(super) fn modes(&self) -> Vec<&'static str> {
        let mut modes: Vec<_> = std::iter::once(&self.compose)
            .chain(&self.others)
            .filter_map(Compose::mode)
            .collect();
        modes.sort_by_key(|(rank, _)| *rank);
        modes.into_iter().map(|(_, word)| word).collect()
    }
}

impl Compose {
    /// The row composer's mode: its place in the answer, note, talk cycle and word.
    fn mode(&self) -> Option<(u8, &'static str)> {
        match self {
            Self::Reply { .. } | Self::AnswerPicker => Some((0, "answer")),
            Self::Annotate { .. } => Some((1, "note")),
            Self::Talk { .. } => Some((2, "talk")),
            Self::Status => Some((3, "status")),
            _ => None,
        }
    }
}

/// Small form field budget, in UTF-8 bytes.
pub(super) const INPUT_LIMIT: usize = 4000;
/// Session message limit, in UTF-8 bytes; reject oversized insertions atomically.
pub(super) const MESSAGE_LIMIT: usize = 1024 * 1024;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Shared routing identifies the active surface; its controller owns effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Overlay {
    Workspace,
    Help,
    Settings,
    Theme,
    CronList,
    ReplyReader,
    Checklist,
}
impl Overlay {
    fn id(self) -> tmt_tui::app::ComponentId {
        vec![
            match self {
                Self::Workspace => "workspace-form",
                Self::Help => "help",
                Self::Settings => "settings",
                Self::Theme => "theme-picker",
                Self::CronList => "cron-list",
                Self::ReplyReader => "reply-reader",
                Self::Checklist => "checklist",
            }
            .into(),
        ]
    }
}

/// Where a shell destination was drawn, for clipped navigation clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabHit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub tab: usize,
}

/// A currently painted screen region and its row or meter target.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub y: u16,
    pub x: u16,
    pub width: u16,
    pub target: HitTarget,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    Row(usize),
    Meter(Option<usize>),
}
impl Hit {
    pub fn row(&self) -> Option<usize> {
        match self.target {
            HitTarget::Row(row) => Some(row),
            HitTarget::Meter(_) => None,
        }
    }
    fn contains(&self, x: u16, y: u16) -> bool {
        self.y == y && (self.x..self.x.saturating_add(self.width)).contains(&x)
    }
}

impl std::fmt::Debug for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut hit = f.debug_struct("Hit");
        hit.field("y", &self.y)
            .field("x", &self.x)
            .field("width", &self.width);
        match self.target {
            HitTarget::Row(row) => hit.field("row", &row),
            target => hit.field("target", &target),
        }
        .finish()
    }
}

#[derive(Default)]
pub struct App {
    pub(super) shell: super::shell::OpsShell,
    pub(super) workspace: super::workspace::OpsWorkspace,
    pub(super) workspace_startup: Option<String>,
    pub(super) workspace_form: Option<super::workspace::manage::Draft>,
    pub(in crate::board) workspace_action_hits:
        RefCell<Vec<(ratatui::layout::Rect, super::workspace::OpsTarget, Choice)>>,
    pub(super) timing: Option<super::timing::Ui>,
    pub(super) checklist: Option<super::checklist::Controller>,
    pub(super) checklist_generation: u64,
    pub(super) initial_look: Option<crate::look::Look>,
    pub(super) cron: super::cronboard::State,
    pub(super) home_leads: super::home_leads::State,
    /// What the configured label sources last supplied for member rows.
    pub(super) labels: crate::labels::Supplied,
    /// The cron form being filled in on the input line, if any.
    pub(super) status_draft: Option<super::status_update::Draft>,
    pub(super) cron_draft: Option<super::cronboard::Draft>,
    pub(super) home_counters: RefCell<super::home::counters::Counters>,
    /// Provider limits as the worker last sampled them; `None` until the first sample.
    pub(super) limits: Option<super::limits::Snapshot>,
    /// Cell widths and row values that outlive a snapshot, so a refresh cannot move the layout.
    pub(super) meter: Option<super::meter::Meter>,
    meters: BTreeMap<String, super::meter::Meter>,
    usage_document: Option<Value>,
    pub(super) token_window: crate::config::TokenWindow,
    pub(super) excluded_counters: Vec<String>,
    pub(super) meter_hover: Option<Option<usize>>,
    squad_keys: Vec<String>,
    pub tabs: Vec<String>,
    pub hidden: Vec<String>,
    pub pinned: usize,
    /// The `c` list of every squad's jobs, while open.
    pub(super) cron_list: Option<super::cronboard::List>,
    pub(super) row_details: super::row_detail::State,
    pub(super) conversation: RefCell<super::conversation::State>,
    pub attention: BTreeMap<String, Attention>,
    pub current: Option<String>,
    pub view: Option<View>,
    pub error: Option<String>,
    pub search: String,
    pub searching: bool,
    /// Index among visible rows (headers excluded).
    pub selected: usize,
    pub notice: Option<String>,
    pub(super) migration_notice: Option<String>,
    pub help: bool,
    pub(super) help_state: RefCell<super::help::Help>,
    pub(in crate::board) overlay_focus: tmt_tui::app::FocusStack,
    pub menu: Option<Menu>,
    pub(super) theme_picker: Option<super::theme_picker::Picker>,
    pub(super) settings: Option<super::settings::Overlay>,
    pub input: Option<Input>,
    pub(super) composer: super::composer::State,
    pub(super) mentions: super::mentions::State,
    pub(super) references: super::inventory::Inventory,
    /// Lane jobs submitted whose events have not arrived.
    pub(super) in_flight: usize,
    /// Current-frame inline band, reserved by the row painter.
    pub(super) input_band: std::cell::Cell<Option<ratatui::layout::Rect>>,
    /// Current-frame editor geometry; cleared before paint and on resize.
    pub(super) input_viewport: std::cell::Cell<Option<(ratatui::layout::Rect, usize)>>,
    /// Index of the focused pane (split) or visible tab (tabs).
    pub focus: usize,
    /// Every pane's scroll position, from one owner.
    pub scrolls: Scrolls,
    /// The rows pane keeps the selection on screen until the wheel moves it.
    pub follow: bool,
    /// Opened as a tmux popup: a successful jump closes the board.
    pub popup: bool,
    /// Notices a replaced binary or a reload request; inert unless `run` starts it.
    pub(super) reload: super::reload::Watch,
    /// Set when the session ends to reload: the state the next process resumes.
    pub(super) restart: Option<super::resume::Resume>,
    /// State a reload carried in, applied once to the first view of its tab.
    pending_resume: Option<super::resume::Resume>,
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
    pub(super) detail_more_hits: RefCell<Vec<(ratatui::layout::Rect, super::row_detail::Target)>>,
    /// Where a row's choice chip was last drawn, with its row.
    pub(super) chip_hits: RefCell<Vec<(ratatui::layout::Rect, usize)>>,
    /// Each record's first visual line in the last rows draw, for paging.
    pub row_starts: RefCell<Vec<usize>>,
    /// Where tabs were last drawn.
    pub tab_hits: RefCell<Vec<TabHit>>,
    body_width: u16,
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
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
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
            loading_since: Some(Instant::now()),
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
                .workspace_rows()
                .into_iter()
                .filter_map(|entry| {
                    self.workspace_snapshot()?
                        .rows
                        .get(&entry.target)
                        .map(|row| (RowOrigin::Section(0), row))
                })
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
        let mut items = crate::display_rows::project(
            self.usage_document.as_ref().unwrap_or(&view.document),
            |row| matches(row, &self.search),
        );
        if self.effective_board().is_some_and(|board| board.members) {
            let exchanges: BTreeMap<_, _> = view
                .exchanges
                .iter()
                .map(|member| (member.id(), member))
                .collect();
            // Keep lead/rule/authored sections in place; only member order changes.
            let mut start = 0;
            while start < items.len() {
                if !matches!(items[start], Item::Row(RowOrigin::Section(_), _)) {
                    start += 1;
                    continue;
                }
                let end = start
                    + items[start..]
                        .iter()
                        .take_while(|item| matches!(item, Item::Row(RowOrigin::Section(_), _)))
                        .count();
                items[start..end].sort_by(|a, b| {
                    let exchange = |item: &Item<'_>| match item {
                        Item::Row(_, row) => {
                            row["id"].as_str().and_then(|id| exchanges.get(id).copied())
                        }
                        _ => None,
                    };
                    match (exchange(a), exchange(b)) {
                        (Some(a), Some(b)) => super::home_leads::compare(a, b),
                        _ => std::cmp::Ordering::Equal,
                    }
                });
                start = end;
            }
        }
        items
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
        view.rows.select_window(
            meter
                .settings
                .windows
                .iter()
                .position(|window| *window == self.token_window)
                .unwrap_or(0),
        );
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
            .filter(|meter| meter.room == rate.input.room && meter.same_policy(rate.settings))
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

    /// Stable acquired squad order for aggregate usage and counter identity.
    /// Search admission stays shared with the source document; cursor-only
    /// attention, lead and timer entries never participate in this projection.
    fn home_usage_squads(&self) -> impl Iterator<Item = &str> {
        self.view.iter().flat_map(move |view| {
            view.home
                .iter()
                .flat_map(|home| &home.squads)
                .filter(move |squad| {
                    view.document["sections"][0]["rows"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|row| row["squad"] == squad.squad && matches(row, &self.search))
                })
                .map(|squad| squad.squad.as_str())
        })
    }

    pub(super) fn home_header_usage(&self, now: Instant) -> Option<HomeHeaderUsage<'_>> {
        let view = self.view.as_ref()?;
        let windows = view.home.as_ref()?.windows;
        // Stable displayed squad order decides equal-evidence UUID collisions.
        let mut enabled = false;
        let mut identities = BTreeMap::new();
        let mut members: Vec<HeaderMember<'_>> = Vec::new();
        for squad in self.home_usage_squads() {
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

    pub(super) fn home_counter_keys(&self, squad: &str) -> [String; 3] {
        let view = self.view.as_ref().expect("HOME counter has a view");
        let rate = &view.home_rate[squad];
        let lead = view
            .home
            .as_ref()
            .unwrap()
            .squads
            .iter()
            .find(|line| line.squad == squad)
            .and_then(|line| line.lead.as_ref())
            .and_then(|lead| lead["id"].as_str());
        let owner = format!(
            "lead:{:?}:{squad:?}:{:?}:{lead:?}:{:?}:{:?}",
            view.me_id, rate.input.room, self.token_window, rate.settings
        );
        std::array::from_fn(|index| {
            format!("{owner}:{}", rate.settings.windows[index].milliseconds())
        })
    }

    pub(super) fn home_header_counter_keys(&self) -> [String; 3] {
        let view = self.view.as_ref().expect("HOME header has a view");
        let owners = self
            .home_usage_squads()
            .filter_map(|squad| view.home_rate.get(squad))
            .filter(|rate| rate.settings.enabled)
            .map(|rate| {
                (
                    &rate.input.room,
                    rate.input.resumes.keys().collect::<Vec<_>>(),
                    rate.settings,
                )
            })
            .collect::<Vec<_>>();
        let owner = format!("header:{:?}:{owners:?}:{:?}", view.me_id, self.token_window);
        std::array::from_fn(|index| {
            format!(
                "{owner}:{}",
                view.home.as_ref().unwrap().windows[index].milliseconds()
            )
        })
    }

    pub(super) fn home_digits(
        &self,
        squad: &str,
        usage: &HomeUsage<'_>,
        now: Instant,
    ) -> [String; 3] {
        let reduced = self.view.as_ref().unwrap().home_rate[squad]
            .settings
            .reduced_motion;
        let keys = self.home_counter_keys(squad);
        let mut counters = self.home_counters.borrow_mut();
        let mut keys = keys.into_iter();
        std::array::from_fn(|index| {
            counters.digits(
                keys.next().unwrap(),
                usage.lead[index],
                reduced || self.loading(),
                now,
            )
        })
    }

    pub(super) fn home_header_digits(
        &self,
        usage: &HomeHeaderUsage<'_>,
        now: Instant,
    ) -> [String; 3] {
        let reduced = self
            .view
            .as_ref()
            .unwrap()
            .home_rate
            .values()
            .any(|rate| rate.settings.enabled && rate.settings.reduced_motion);
        let mut keys = self.home_header_counter_keys().into_iter();
        let mut counters = self.home_counters.borrow_mut();
        std::array::from_fn(|index| {
            counters.digits(keys.next().unwrap(), usage.totals[index], reduced, now)
        })
    }

    /// A sample that changes nothing paints nothing.
    pub(super) fn set_limits(&mut self, snapshot: super::limits::Snapshot) -> bool {
        let changed = self.limits.as_ref() != Some(&snapshot);
        self.limits = Some(snapshot);
        changed
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
        if resumes.is_err() {
            self.home_counters.get_mut().clear();
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
        if self.workspace_snapshot().is_some() {
            self.workspace_reconcile();
            return;
        }
        self.reconcile_row_details();
        let anchor = self.input.as_ref().and_then(Input::target);
        if let Some(target) = anchor
            && let Some(index) = (0..self.rows().len())
                .find(|&index| self.row_target(index).as_ref() == Some(target))
        {
            self.selected = index;
        }
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
    }

    /// Another squad is now on screen: its selection and scrolling start over.
    fn shown_changed(&mut self) {
        self.selected = 0;
        self.scrolls = Scrolls::default();
        self.follow = true;
        self.restore_focus();
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

    /// Completes the shown squad's view with its deferred reads. Focus follows the
    /// read exactly; bodies fill only replies still without one, matched by request.
    pub(super) fn apply_enriched(
        &mut self,
        squad: &str,
        enrichment: super::refresh::Enrichment,
    ) -> bool {
        if self.shown.as_deref() != Some(squad) || self.loading() {
            return false;
        }
        let Some(view) = self.view.as_mut().filter(|view| view.home.is_none()) else {
            return false;
        };
        crate::digest::set(&mut view.document, enrichment.digest.as_ref());
        for reply in &mut view.replies {
            if let Some(response) = enrichment
                .replies
                .iter()
                .flatten()
                .find(|read| read["requestId"] == reply["requestId"])
                .map(|read| &read["response"])
                .filter(|response| !response.is_null())
                && reply["response"].is_null()
            {
                reply["response"] = response.clone();
            }
        }
        *view.derived.borrow_mut() = Default::default();
        true
    }

    pub(super) fn apply_home_leads(&mut self, read: super::home_leads::Read) {
        self.home_leads.replace(read);
        self.clamp();
    }

    /// Swaps in a loaded squad in one step. A result for a squad the user
    /// already left is kept for switching back, never shown.
    pub(super) fn apply_history(
        &mut self,
        read: super::refresh::HistoryRead,
        now: Instant,
    ) -> bool {
        if self.loading()
            || self.current.as_deref() != Some(&read.owner)
            || self.shown.as_deref() != Some(&read.owner)
        {
            return false;
        }
        let Some(view) = self.view.as_mut() else {
            return false;
        };
        let home = view.home.is_some();
        let mut changed = false;
        for (name, (settings, input)) in read.targets {
            let current = if home {
                view.home_rate.get(&name)
            } else {
                view.token_rate.as_ref()
            };
            let Some(current) = current.filter(|rate| {
                rate.settings == settings
                    && rate.settings.enabled
                    && rate.input.room == input.room
                    && rate.input.resumes.keys().eq(input.resumes.keys())
            }) else {
                continue;
            };
            let meter = if home {
                self.meters.get_mut(&name)
            } else {
                self.meter.as_mut()
            };
            let Some(meter) =
                meter.filter(|meter| meter.room == input.room && meter.settings == settings)
            else {
                continue;
            };
            meter.seed(&current.input, &read.seeds, now, false);
            let observed = read
                .observed
                .as_ref()
                .map(|rows| current.input.joined(rows));
            meter.sample(observed.as_ref().map_err(|_| ()), now);
            changed = true;
        }
        if changed {
            view.history_pending = false;
            self.project_usage(now);
        }
        changed
    }

    pub fn apply(&mut self, mut snapshot: Snapshot) {
        if let Some(timing) = &mut self.timing {
            let accepted =
                snapshot.view.is_ok() && (self.current.is_none() || self.current == snapshot.squad);
            timing.snapshot(snapshot.timing.take(), accepted);
        }
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
        self.references = snapshot.references;
        self.squad_keys = snapshot.squad_keys;

        self.invalidate_frame_geometry();
        self.tabs = snapshot.tabs;
        self.hidden = snapshot.hidden;
        self.pinned = snapshot.pinned;
        self.meters
            .retain(|key, _| self.tabs.contains(key) || self.hidden.contains(key));
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
                self.home_leads.set_sender(view.me_id.as_deref());
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
                            meter.room != rate.input.room || !meter.same_policy(rate.settings)
                        }) {
                            self.meters.insert(
                                name.clone(),
                                super::meter::Meter::new(rate.settings, &rate.input, now),
                            );
                        } else if let Some(meter) = self.meters.get_mut(name) {
                            meter.settings = rate.settings;
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
                        self.token_window = rate.settings.window;
                        self.token_window = self.token_window.available(rate.settings.windows);
                        if let Some(meter) = self.meter.as_mut().filter(|meter| {
                            meter.room == rate.input.room && meter.same_policy(rate.settings)
                        }) {
                            meter.settings = rate.settings;
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
                if self.meter.is_none() {
                    self.meter_hover = None;
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
                let panes = self.focus_panes().len();
                self.focus = self.focus.min(panes.saturating_sub(1));
                let changed = self.loading() || self.view.is_none();
                if let Some(overlay) = &mut self.settings {
                    overlay.staleness =
                        Some(crate::staleness::Snapshot::for_preview(&view.document));
                }
                self.usage_document = None;
                // A refresh can reorder rows; the cursor follows its member.
                let anchor = (!changed).then(|| self.row_target(self.selected)).flatten();
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
                    self.restore_pending();
                } else {
                    self.restore_focus();
                    if let Some(anchor) = anchor {
                        self.follow_member(&anchor);
                    }
                }
                self.error = None;
            }
            Err(error) => {
                if let Some(snapshot) = self
                    .view
                    .as_mut()
                    .and_then(|view| view.home.as_mut())
                    .map(|home| &mut home.workspace)
                {
                    snapshot.roster.observation = super::workspace::OpsObservation::Stale;
                    for squad in &mut snapshot.roster.squads {
                        if squad.observation == super::workspace::OpsObservation::Complete {
                            squad.observation = super::workspace::OpsObservation::Stale;
                        }
                    }
                }
                // A reload's state belongs to a view that did not arrive.
                self.pending_resume = None;
                self.home_counters.get_mut().clear();
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

    /// Close the current tab's meter continuity and resume the target's cached
    /// meter, including the enabled HOME meters when its view is retained.
    fn handoff_meters(&mut self, next: &str, now: Instant) {
        self.home_counters.get_mut().clear();
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
        if let Some(snapshot) = self.workspace_snapshot() {
            let target = snapshot
                .roster
                .squads
                .iter()
                .find(|squad| squad.name == next)
                .map(|squad| super::workspace::OpsTarget::Squad(squad.room_id.clone()));
            if let Some(target) = target {
                self.workspace.selected = Some(target);
                self.shell.tab = super::shell::OpsTab::Agents;
                self.workspace_reconcile();
                return Effect::None;
            }
            return self.say("Select a squad from the workspace; old squad tabs are retired.");
        }
        self.meter_hover = None;
        if Some(&next) == self.current.as_ref() {
            return Effect::None;
        }
        if let Some(view) = self.view.as_mut() {
            view.history_pending = false;
        }
        self.handoff_meters(&next, Instant::now());
        self.current = Some(next.clone());
        self.error = None;
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

    pub(super) fn open_settings(&mut self, config: Config) -> Result<(), crate::core::SquadError> {
        let window = self
            .meter
            .as_ref()
            .filter(|meter| meter.settings.enabled)
            .map(|_| self.token_window.label().to_owned());
        let mut overlay = super::settings::Overlay::open(
            config,
            self.shown_tab(),
            self.selected_section(),
            window,
        )?;
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

    /// Authored board values belong to the independent CLI/config projection.
    pub fn effective_board(&self) -> Option<&Board> {
        self.view.as_ref().map(|view| &view.board)
    }

    /// Logical placement survives invalidating pointer rectangles. A refresh
    /// must not turn a retained Reader focus into a list/Host key scope.
    fn conversation_area(&self) -> ratatui::layout::Rect {
        let mut area = self.conversation.borrow().area;
        if self.body_width > 0 {
            area.width = self.body_width;
        }
        area
    }

    pub fn collapsed_panes(&self) -> BTreeSet<Pane> {
        if self.workspace_snapshot().is_none()
            || (self.shell.tab == super::shell::OpsTab::Overview
                && super::conversation::layout::overview(self.conversation_area())
                    .conversation
                    .is_none())
        {
            [Pane::Replies].into()
        } else {
            BTreeSet::new()
        }
    }

    pub(super) fn body_width(&self) -> u16 {
        self.body_width
    }

    pub fn set_body_width(&mut self, width: u16) {
        self.body_width = width;
        self.restore_focus();
    }

    fn focus_panes(&self) -> &[Pane] {
        &[Pane::Rows, Pane::Replies]
    }

    pub fn focused_pane(&self) -> Option<Pane> {
        self.focus_panes()
            .get(self.focus)
            .copied()
            .filter(|pane| !self.collapsed_panes().contains(pane))
    }

    pub fn focused(&self) -> Pane {
        self.focus_panes()
            .get(self.focus)
            .copied()
            .unwrap_or(Pane::Rows)
    }

    /// Focuses the pane drawn under the pointer, if any.
    fn focus_at(&mut self, column: u16, row: u16) {
        let Some(pane) = self
            .pointer_pane(column, row)
            .filter(|pane| !self.collapsed_panes().contains(pane))
        else {
            return;
        };
        if let Some(position) = self.focus_panes().iter().position(|p| *p == pane) {
            self.focus = position;
        }
    }

    /// A hidden focus returns to rows; boards without visible rows use the next pane.
    fn restore_focus(&mut self) {
        if self.workspace_snapshot().is_some() {
            self.focus = self.focus.min(self.focus_panes().len().saturating_sub(1));
            if self.focus == 1
                && self.shell.tab == super::shell::OpsTab::Overview
                && super::conversation::layout::overview(self.conversation_area())
                    .conversation
                    .is_none()
            {
                self.shell.tab = super::shell::OpsTab::Agents;
                self.invalidate_frame_geometry();
            }
            return;
        }

        if self.focused_pane().is_some() {
            return;
        }
        if let Some(position) = self
            .focus_panes()
            .iter()
            .position(|pane| *pane == Pane::Rows)
            && !self.collapsed_panes().contains(&Pane::Rows)
        {
            self.focus = position;
        } else {
            self.next_pane();
        }
    }

    fn next_pane(&mut self) {
        let panes = self.focus_panes();
        let collapsed = self.collapsed_panes();
        let count = panes.len();
        if let Some(next) = (1..=count)
            .map(|step| (self.focus + step) % count)
            .find(|index| !collapsed.contains(&panes[*index]))
        {
            self.focus = next;
        }
    }

    pub(super) fn selected_section(&self) -> Option<usize> {
        self.rows()
            .get(self.selected)
            .and_then(|(slot, _)| slot.section())
    }

    /// The selected row's section overrides its host table, which overrides the
    /// tab/global bindings and the preset.
    pub fn bindings(&self) -> Bindings {
        self.workspace_bindings()
    }

    pub(super) fn say(&mut self, notice: impl Into<String>) -> Effect {
        self.notice = Some(notice.into());
        Effect::None
    }

    /// Resolves an action against the selected row. Missing values refuse
    /// the action with a notice; nothing runs half-filled. While a switch
    /// loads, the rows on screen are another squad's, so nothing acts on them.
    pub fn perform(&mut self, action: &Action) -> Effect {
        if let Err(error) = crate::config::admit_workspace_action(action, "action") {
            return self.say(error.message);
        }
        if let Verb::Ui(action) = action.verb {
            return self.workspace_action(action);
        }
        if action.verb == Verb::Menu {
            return self.context_menu();
        }
        if self.loading() && !matches!(action.verb, Verb::NextPane | Verb::Refresh | Verb::Notes) {
            let loading = self.current.clone().unwrap_or_default();
            return self.say(format!("Loading {loading}…"));
        }
        if self.workspace_snapshot().is_some()
            && action.verb.acts_on_member()
            && self.workspace_selected_member().is_none()
        {
            return self.say("Select a visible named member before running this action.");
        }
        match action.verb {
            Verb::TokenWindow => {
                return self.cycle_token_window();
            }
            Verb::HomeMessage => return self.toggle_row_detail(),
            Verb::ViewReply => return self.view_row_reply(),
            Verb::AskLead => return self.ask_lead(),
            Verb::Digest => return self.digest_row(self.selected),
            Verb::Settings => {
                return if self.workspace_snapshot().is_some() {
                    self.workspace_settings()
                } else {
                    Effect::Settings
                };
            }
            Verb::Theme => return Effect::PickTheme,
            Verb::NextPane => {
                self.next_pane();
                return Effect::None;
            }
            Verb::Refresh => {
                self.notice = Some("Refreshing…".into());
                return Effect::Refresh;
            }
            Verb::Notes => return self.say("The old lead notebook pane is retired. The binding is preserved in Config for explicit migration; it does not change the selected agent."),
            Verb::Back => return Effect::Act(Request::Back),
            Verb::Jump if action.args.first().and_then(|arg| arg.literal()) == Some("lead") => {
                return match self.lead() {
                    Ok(lead) => Effect::Act(Request::Jump(lead)),
                    Err(reason) => self.say(format!("jump lead: {reason}.")),
                };
            }
            _ => {}
        }
        let Some(row) = self.selected_row().cloned() else {
            return self.say("No row is selected.");
        };
        let view = self.view.as_ref().expect("a selected row has a view");
        let request = match action.verb {
            Verb::Talk | Verb::Annotate | Verb::Reply => return self.compose(action, &row),
            Verb::Tab => {
                return match row["squad"].as_str() {
                    Some(squad) => self.go(squad.to_owned()),
                    None => self.say("tab: this row has no squad."),
                };
            }
            Verb::Jump => match if self.workspace_snapshot().is_some() {
                row["id"].as_str()
            } else {
                row["name"].as_str()
            } {
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

    pub(super) fn context_menu(&mut self) -> Effect {
        let row = self.selected_row();
        let mut entries: Vec<MenuEntry> = self
            .bindings()
            .into_iter()
            .filter(|(event, action)| {
                !matches!(event.as_str(), "click" | "double-click")
                    && !matches!(action.verb, Verb::Menu | Verb::NextPane | Verb::TokenWindow)
                    && (row.is_some() || !action.verb.acts_on_member())
                    // Without a digest source there is nothing for this entry to do.
                    && (action.verb != Verb::Digest
                        || row.as_ref().is_some_and(|row| self.digest_choice(row).is_some()))
            })
            .map(|(key, action)| MenuEntry {
                key,
                label: action.text.clone(),
                choice: Choice::Action(action),
            })
            .collect();
        entries.sort_by_key(|entry| match &entry.choice {
            Choice::Action(action) => action.order(),
            _ => u8::MAX,
        });
        entries.push(MenuEntry {
            key: String::new(),
            label: "Checklist".into(),
            choice: Choice::Checklist,
        });
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: row
                .and_then(|row| row["name"].as_str())
                .unwrap_or("Actions")
                .to_owned(),
            entries,
            selected: 0,
            surface: Default::default(),
        });
        Effect::None
    }

    /// The lead `jump lead` goes to: the squad's own lead on its tab; on the
    /// leads and all tabs, which list several squads, the selected row's
    /// squad's lead.
    fn lead(&self) -> Result<String, String> {
        let view = self.view.as_ref().ok_or("the board has not loaded yet")?;
        let named = |name: Option<&str>| name.filter(|name| !name.is_empty()).map(str::to_owned);
        let lead = match self.current.as_deref() {
            Some(super::LEADS) => named(self.selected_row().and_then(|row| row["name"].as_str())),
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
            _ => "this squad has no lead; set one with tmt ops squad lead <name>".to_owned(),
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
            return self.say("Who is sending? Record yourself with tmt ops squad me <name>.");
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
        if self
            .input
            .as_ref()
            .is_some_and(|input| input.text.is_empty())
        {
            self.input.as_mut().unwrap().text = text.into();
        }
        Effect::None
    }

    pub(super) fn ask(&mut self, prompt: String, compose: Compose, squad: String) -> Effect {
        self.remember_draft();
        self.mentions = Default::default();
        self.input = Some(Input {
            draft_key: None,
            response_quotes: Vec::new(),
            row_send: None,
            others: Vec::new(),
            quote: None,
            link: None,
            prompt,
            text: Default::default(),
            compose,
            squad,
            hint: None,
        });
        self.bind_draft();
        self.restore_draft();
        Effect::None
    }

    /// Stable occurrence used by composer placement, validation and feedback.
    /// Selects the same row, else the same member elsewhere; otherwise the index stays.
    fn follow_member(&mut self, anchor: &RowTarget) {
        let member = |target: &RowTarget| match target {
            RowTarget::Workspace(super::workspace::OpsTarget::Member { identity_id, .. }) => {
                Some(identity_id.clone())
            }
            RowTarget::Workspace(_) => None,
            RowTarget::Member { id, .. } | RowTarget::Lead { id, .. } => Some(id.clone()),
        };
        let targets: Vec<_> = (0..self.rows().len())
            .map(|index| self.row_target(index))
            .collect();
        let found = targets
            .iter()
            .position(|target| target.as_ref() == Some(anchor))
            .or_else(|| {
                let id = member(anchor)?;
                targets
                    .iter()
                    .position(|target| target.as_ref().and_then(member).as_ref() == Some(&id))
            });
        if let Some(index) = found {
            self.selected = index;
        }
    }

    pub(super) fn row_target(&self, index: usize) -> Option<RowTarget> {
        if self.workspace_snapshot().is_some() {
            if index == self.selected && !self.workspace_selection_visible() {
                return None;
            }
            let target = self.workspace_rows().get(index)?.target.clone();
            return matches!(target, super::workspace::OpsTarget::Member { .. })
                .then_some(RowTarget::Workspace(target));
        }
        let (slot, row) = *self.rows().get(index)?;
        self.projected_row_target(slot, row)
    }

    /// Reuse an already projected row when visiting the row stream.
    pub(super) fn projected_row_target(&self, slot: RowOrigin, row: &Value) -> Option<RowTarget> {
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

    pub(super) fn target_row(&self, target: &RowTarget) -> Option<&Value> {
        match target {
            RowTarget::Workspace(target) => {
                let snapshot = self.workspace_snapshot()?;
                snapshot
                    .roster
                    .contains(target)
                    .then(|| snapshot.rows.get(target))
                    .flatten()
            }
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

    fn note_recipient(&self, target: &RowTarget) -> Option<NoteRecipient> {
        match target {
            RowTarget::Workspace(super::workspace::OpsTarget::Member {
                room_id: Some(room),
                ..
            }) => {
                let snapshot = self.workspace_snapshot()?;
                let lead = snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| squad.room_id == *room)?
                    .lead
                    .as_ref()?;
                snapshot
                    .roster
                    .members
                    .get(lead)
                    .map(|member| NoteRecipient {
                        id: Some(member.id.clone()),
                        name: member.name.clone(),
                    })
            }
            RowTarget::Workspace(_) => None,
            RowTarget::Member { tab, .. } if tab == super::LEADS => {
                NoteRecipient::from_row(self.target_row(target)?)
            }
            RowTarget::Member { tab, .. } | RowTarget::Lead { tab, .. }
                if !super::tabs::aggregate(tab) =>
            {
                NoteRecipient::from_row(&self.view.as_ref()?.document["squad"]["lead"])
            }
            _ => None,
        }
    }

    pub(super) fn row_send(&self, index: usize, member_note: bool) -> Option<RowSend> {
        let target = self.row_target(index)?;
        self.send_target(target, member_note)
    }

    pub(super) fn send_target(&self, target: RowTarget, member_note: bool) -> Option<RowSend> {
        let sender = self.view.as_ref()?.me.clone()?;
        let name = self.target_row(&target)?["name"].as_str()?.to_owned();
        let recipient = if member_note {
            NoteRecipient::from_row(self.target_row(&target)?)
        } else {
            self.note_recipient(&target)
        };
        let note = recipient.as_ref().map(|recipient| Compose::Annotate {
            to: recipient.name.clone(),
            row: name.clone(),
        });
        Some(RowSend {
            target,
            sender,
            sender_id: self.view.as_ref()?.me_id.clone(),
            name,
            note_member: member_note,
            note,
            note_recipient: recipient,
        })
    }

    fn request_entries(&self, send: &RowSend) -> Vec<MenuEntry> {
        self.target_row(&send.target).unwrap_or(&Value::Null)["waitingOnYou"]
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
            .collect()
    }
    /// One composer for row answers and notes; several requests still need a choice.
    pub(super) fn compose_row(&mut self, mut send: RowSend, verb: Verb, squad: String) -> Effect {
        if squad.is_empty() && verb == Verb::Annotate {
            return self
                .say("Notes require a squad context; use Talk for this Common pool identity.");
        }
        self.status_draft = None;
        let row = self.target_row(&send.target).expect("opening row exists");
        let pending = row["pending"].as_str().is_some_and(|text| !text.is_empty());
        let open = self.request_entries(&send);
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
                None => Compose::Talk {
                    to: send.name.clone(),
                },
            },
        };
        if squad.is_empty() && matches!(compose, Compose::Annotate { .. }) {
            return self
                .say("Notes require a squad context; use Talk for this Common pool identity.");
        }
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
            return self.say("Who is sending? Record yourself with tmt ops squad me <name>, or open the board from your named pane.");
        }
        // Workspace selection already owns the exact membership. Do not infer
        // its target from a filtered flat row index or equal row payload.
        let target = if self.workspace_snapshot().is_some() {
            self.workspace_selected_member()
                .cloned()
                .map(RowTarget::Workspace)
        } else {
            let rows = self.rows();
            let index = rows
                .get(self.selected)
                .filter(|(_, candidate)| *candidate == row)
                .map(|_| self.selected)
                .or_else(|| rows.iter().position(|(_, candidate)| *candidate == row));
            index.and_then(|index| self.row_target(index)).or_else(|| {
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
            })
        };
        if action.verb == Verb::Annotate
            && matches!(
                target,
                Some(RowTarget::Workspace(super::workspace::OpsTarget::Member {
                    room_id: None,
                    ..
                }))
            )
        {
            return self
                .say("Notes require a squad context; use Talk for this Common pool identity.");
        }
        let Some(send) = target.and_then(|target| {
            self.send_target(
                target,
                action.args.first().and_then(|arg| arg.literal()) == Some("member"),
            )
        }) else {
            return self.say("This row has no stable member target.");
        };
        let Some(squad) = self.target_squad(&send.target) else {
            return self.say("The retained membership is unavailable.");
        };
        self.compose_row(send, action.verb, squad)
    }

    pub(super) fn choose(&mut self, choice: Choice) -> Effect {
        match choice {
            Choice::WorkspaceBegin(kind) => self.workspace_begin(kind),
            Choice::WorkspaceLayout(layout) => self.workspace_layout(layout),
            Choice::Checklist => self.open_checklist(),
            Choice::Action(action) => self.perform(&action),
            Choice::Cron(request) => Effect::Act(Request::Cron(request)),
            Choice::Dismiss => Effect::None,
            Choice::Reply { request, from } => self.ask(
                format!("→ {from}"),
                Compose::Reply { request, from },
                self.current.clone().unwrap_or_default(),
            ),
            Choice::Digest(set) => Effect::Act(Request::Digest(set)),
            Choice::DigestCustom(granted) => self.digest_custom(*granted),
        }
    }

    /// The question a reply answers, quoted above the text while composing it.
    fn reply_quote(&self, send: &RowSend, compose: &Compose) -> Option<String> {
        let Compose::Reply { request, .. } = compose else {
            return None;
        };
        self.target_row(&send.target)?["waitingOnYou"]
            .as_array()?
            .iter()
            .find(|item| item["requestId"].as_str() == Some(request))
            .map(|item| {
                super::notes::sanitize(item["preview"].as_str().unwrap_or("(question unavailable)"))
            })
    }

    /// The modes Tab reaches from `current`, in answer, note, talk order after
    /// it. Answer is offered only for the one request the row waits on; several
    /// requests are chosen from the menu `a` opens first.
    fn other_modes(&self, send: &RowSend, current: &Compose) -> Vec<Compose> {
        let answer = match current {
            Compose::Reply { .. } | Compose::AnswerPicker => Some(current.clone()),
            _ if send.note_member => None,
            _ => self.target_row(&send.target).and_then(|row| {
                let mut open = row["waitingOnYou"].as_array()?.iter();
                let (item, rest) = (open.next()?, open.next());
                Some(if rest.is_none() {
                    Compose::Reply {
                        request: item["requestId"].as_str().unwrap_or_default().to_owned(),
                        from: send.name.clone(),
                    }
                } else {
                    Compose::AnswerPicker
                })
            }),
        };
        let all = [
            answer,
            send.note.clone(),
            Some(Compose::Talk {
                to: send.name.clone(),
            }),
            (!matches!(
                &send.target,
                RowTarget::Workspace(super::workspace::OpsTarget::Member { room_id: None, .. })
            ))
            .then_some(Compose::Status),
        ];
        let rank = |compose: &Compose| compose.mode().map(|(rank, _)| rank);
        let here = rank(current);
        let mut after: Vec<_> = all.into_iter().flatten().collect();
        after.retain(|compose| rank(compose) != here);
        after.sort_by_key(|compose| (rank(compose) <= here, rank(compose)));
        after
    }

    pub(super) fn attach_row(&mut self, send: RowSend, squad: String) {
        let compose = self.input.as_ref().map(|input| input.compose.clone());
        let others = compose
            .as_ref()
            .map(|compose| self.other_modes(&send, compose))
            .unwrap_or_default();
        let quote = compose
            .as_ref()
            .and_then(|compose| self.reply_quote(&send, compose));
        if let Some(input) = &mut self.input {
            input.squad = squad;
            input.others = others;
            input.quote = quote;
            input.row_send = Some(send);
            input.prompt = input.header();
        }
        self.workspace_focus_conversation();
        self.follow = true;
    }

    /// Tab in a row composer: the next of answer, note and talk, keeping the text.
    pub(super) fn cycle_mode(&mut self) {
        self.remember_draft();
        if self.input.as_ref().is_some_and(|input| {
            input
                .row_send
                .as_ref()
                .is_some_and(|send| !send.valid(self, input))
        }) {
            self.notice = Some(
                "The composer target changed; close and reopen it before changing mode.".into(),
            );
            return;
        }
        let Some(input) = self.input.as_mut().filter(|input| !input.others.is_empty()) else {
            return;
        };
        let next = input.others.remove(0);
        let previous = std::mem::replace(&mut input.compose, next);
        input.others.push(previous);
        input.prompt = input.header();
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::AnswerPicker))
        {
            let input = self.input.take().unwrap();
            if let Some(send) = input.row_send {
                let entries = self.request_entries(&send);
                self.menu = Some(Menu {
                    title: format!("answer {}", send.name),
                    row_send: Some(send),
                    link: None,
                    prefill: input.text.into_string(),
                    entries,
                    selected: 0,
                    surface: Default::default(),
                });
            }
            return;
        }
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status))
        {
            self.start_status();
        }
        let quote = self
            .input
            .as_ref()
            .and_then(|input| self.reply_quote(input.row_send.as_ref()?, &input.compose));
        if let Some(input) = &mut self.input {
            input.quote = quote;
        }
        self.bind_draft();
    }

    /// The next token window of the summary meter, when the squad samples one.
    fn cycle_token_window(&mut self) -> Effect {
        if self.meter.is_some() {
            Effect::CycleTokenWindow
        } else {
            Effect::None
        }
    }
    pub(super) fn next_token_window(&self) -> Option<crate::config::TokenWindow> {
        self.meter
            .as_ref()
            .map(|meter| self.token_window.next(meter.settings.windows))
    }
    pub(super) fn apply_token_window(&mut self, config: &Config) -> String {
        let key = self.current.clone().unwrap_or_default();
        let window = config
            .token_rate(&key)
            .expect("validated window write")
            .window;
        self.token_window = window;
        let now = Instant::now();
        if let Some(meter) = &mut self.meter {
            meter.settings.window = window;
            meter.select(window, now);
        }
        self.project_usage(now);
        format!(
            "Token window: {}{}",
            window.label(),
            if config.has_setting_override(&key, "board.token_rate.window") {
                " · this squad override remains"
            } else {
                ""
            }
        )
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

    pub(super) fn paste(&mut self, text: &str) -> Effect {
        if self.checklist_shown() {
            return self
                .overlay_event(&Event::Paste(text.to_owned()))
                .unwrap_or(Effect::None);
        }
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status))
        {
            if let Some(editor) = self.reference_editor_mut()
                && !editor.insert(text, INPUT_LIMIT)
            {
                return self.say("Status reason exceeds 4,000 bytes; unchanged.");
            }
            self.update_mentions();
            return Effect::None;
        }
        let message = self.message_field();
        let limit = if message { MESSAGE_LIMIT } else { INPUT_LIMIT };
        if let Some(input) = self.input.as_mut()
            && !matches!(input.compose, Compose::Status)
        {
            // Terminals may deliver CR, LF or CRLF inside bracketed paste. Count
            // display lines without changing the exact source retained for send.
            let lines = text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .lines()
                .count();
            let inserted = if message && (text.chars().count() > 500 || lines > 8) {
                input.text.insert_folded(
                    text,
                    format!("[paste · {} bytes · {} lines]", text.len(), lines),
                    limit,
                )
            } else {
                input.text.insert(text, limit)
            };
            if !inserted {
                return self.say(format!("Paste exceeds {limit} bytes; draft unchanged."));
            }
        }
        self.update_mentions();
        Effect::None
    }

    pub(in crate::board) fn input_key(&mut self, key: KeyEvent) -> Effect {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            self.remember_draft();
        }
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status))
        {
            return self.status_key(key);
        }
        let message = self.message_field();
        let limit = if message { MESSAGE_LIMIT } else { INPUT_LIMIT };
        let Some(input) = &mut self.input else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Esc => {
                if input.compose == Compose::Workspace {
                    self.input = None;
                    self.workspace_form = None;
                    return self.say("Cancelled; nothing changed.");
                }
                let cron = matches!(input.compose, Compose::Cron);
                let digest = matches!(input.compose, Compose::Digest(_));
                self.input = None;
                self.status_draft = None;
                if cron {
                    self.cron_draft = None;
                    return self.say("Cancelled; nothing changed.");
                }
                if digest {
                    return self.say("Digest not changed.");
                }
                return self.say("Nothing sent.");
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                input.response_quotes.pop();
                return Effect::None;
            }
            KeyCode::Tab => {
                self.cycle_mode();
                return Effect::None;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                if message {
                    input.text.insert("\n", limit);
                }
                return Effect::None;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if message {
                    input.text.toggle_fold();
                }
                return Effect::None;
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if message {
                    input.text.insert("\n", limit);
                }
                return Effect::None;
            }
            KeyCode::Enter => {}
            _ => {
                let width = self.input_viewport.get().map_or(80, |(area, _)| area.width);
                input.text.key(key, width, limit);
                return Effect::None;
            }
        }
        if self.in_flight > 0 {
            return self
                .say("Still working on the previous request; press Enter again in a moment.");
        }
        let input = self.input.take().expect("composing");
        if input.compose == Compose::Workspace {
            return self.workspace_submit_field(input.text.into_string());
        }
        if matches!(input.compose, Compose::Cron) {
            // A job message is stored exactly as typed: no trim, unlike talk.
            return self.cron_submit(input.text.into_string());
        }
        if let Compose::Digest(granted) = input.compose {
            return self.digest_submit(*granted, &input.text);
        }
        let mut text = input.text.as_str().to_owned();
        for quote in &input.response_quotes {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&quote.reference());
        }
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
                    Compose::Cron
                    | Compose::Workspace
                    | Compose::Status
                    | Compose::AnswerPicker
                    | Compose::Digest(_) => false,
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

        let Some(me) = self.view.as_ref().and_then(|view| view.me.clone()) else {
            return Effect::None;
        };
        if text.trim().is_empty() {
            return self.say("Nothing sent.");
        }
        if text.len() > MESSAGE_LIMIT {
            return self
                .say("Message including quotes exceeds 1 MiB; nothing sent. Draft retained.");
        }
        self.sending_draft(&input);
        let direct = input.row_send.as_ref().and_then(|send| match &send.target {
            RowTarget::Workspace(super::workspace::OpsTarget::Member {
                room_id: None,
                identity_id,
            }) => Some((self.view.as_ref()?.me_id.clone()?, identity_id.clone())),
            _ => None,
        });
        let squad = input.squad;
        Effect::Act(match input.compose {
            Compose::Talk { .. } if direct.is_some() => {
                let (sender, recipient) = direct.unwrap();
                Request::DirectTalk {
                    sender,
                    recipient,
                    text,
                }
            }
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
            Compose::Status => unreachable!("status submission has its own retained form"),
            Compose::AnswerPicker => unreachable!("answer mode opens the existing request menu"),
            Compose::Cron | Compose::Workspace => {
                unreachable!("form steps are submitted before this match")
            }
            Compose::Digest(_) => unreachable!("a digest value is submitted before this match"),
            Compose::Reply { request, from } => Request::Reply {
                me,
                request,
                from,
                text,
            },
        })
    }

    pub(super) fn finished_status(&mut self, outcome: crate::membership::status_update::Outcome) {
        if let Some(draft) = &mut self.status_draft {
            draft.finished(outcome);
            self.notice = draft.error.clone();
        }
    }
    /// Shows the observed outcome without a second row-feedback cache.
    pub fn finished(&mut self, outcome: Result<String, String>) {
        self.notice = Some(outcome.unwrap_or_else(|error| error));
    }

    /// A click on an entry runs it, as Enter on that entry would; the wheel moves
    /// the highlight. The menu owns the pointer while it is open.
    fn menu_mouse(&mut self, event: MouseEvent) -> Effect {
        use super::menu_surface::Pointer;
        let Some(menu) = &mut self.menu else {
            return Effect::None;
        };
        let pointed = menu
            .surface
            .borrow()
            .as_ref()
            .and_then(|surface| surface.pointer(event));
        match pointed {
            Some(Pointer::Choose(index)) if index < menu.entries.len() => {
                menu.selected = index;
                self.menu_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            }
            Some(Pointer::Wheel(step)) => {
                menu.selected = menu
                    .selected
                    .saturating_add_signed(step)
                    .min(menu.entries.len().saturating_sub(1));
                Effect::None
            }
            _ => Effect::None,
        }
    }

    fn menu_key(&mut self, key: KeyEvent) -> Effect {
        if key.code == KeyCode::Tab
            && let Some(send) = self.menu.as_ref().and_then(|menu| menu.row_send.clone())
        {
            let Some(squad) = self.target_squad(&send.target) else {
                return self.say("The retained target is unavailable.");
            };
            let prefill = self.menu.as_ref().unwrap().prefill.clone();
            self.menu = None;
            let compose = send.note.clone().unwrap_or(Compose::Talk {
                to: send.name.clone(),
            });
            self.ask(String::new(), compose, squad.clone());
            if !prefill.is_empty() {
                self.input.as_mut().unwrap().text = prefill.into();
            }
            self.attach_row(send, squad);
            return Effect::None;
        }

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
            if !prefill.is_empty() {
                input.text = prefill.into();
            }
            input.link = link;
        }
        if let Some(send) = row_send {
            let Some(squad) = self.target_squad(&send.target) else {
                return self.say("The retained target is unavailable.");
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
            return None;
        };
        let mut id = overlay.id();
        if overlay == Overlay::Workspace
            && let Some(draft) = &self.workspace_form
        {
            id.push(draft.stage_id());
        }
        let mut focus = std::mem::take(&mut self.overlay_focus);
        if focus.overlay() != Some(&id) {
            focus.reconcile_base(vec![vec![self.focused().title().into()]]);
            let fields = match overlay {
                Overlay::Workspace => self
                    .workspace_form
                    .as_ref()
                    .map_or_else(Vec::new, |draft| draft.focus_fields()),
                Overlay::Theme => vec![vec!["theme-picker".into(), "choices".into()]],
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

    /// Whether an overlay or the action menu owns input: each draws its own key
    /// hints, so the board's footer shows none underneath it.
    pub(in crate::board) fn modal_open(&self) -> bool {
        self.overlay().is_some() || self.menu.is_some()
    }

    fn overlay(&self) -> Option<Overlay> {
        if self.workspace_form.is_some() {
            Some(Overlay::Workspace)
        } else if self.row_details.reader.is_some() {
            Some(Overlay::ReplyReader)
        } else if self.checklist_shown() {
            Some(Overlay::Checklist)
        } else if self.help {
            Some(Overlay::Help)
        } else if self.settings.is_some() {
            Some(Overlay::Settings)
        } else if self.theme_picker.is_some() {
            Some(Overlay::Theme)
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
            Overlay::Workspace => self.workspace_form_event(field, event),
            Overlay::ReplyReader => Some(self.reader_event(event)),
            Overlay::Checklist => self.checklist_event(event),
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
                        super::settings::Input::Reset => Effect::ResetSetting,
                        super::settings::Input::Preview => {
                            self.settings_preview();
                            Effect::None
                        }
                        super::settings::Input::Close => {
                            self.settings_preview();
                            self.settings = None;
                            Effect::CancelSettings
                        }
                        super::settings::Input::Pick("actions") => {
                            self.settings_preview();
                            self.settings = None;
                            self.context_menu()
                        }
                        super::settings::Input::Pick("window") => self.cycle_token_window(),
                        super::settings::Input::Pick(id) => {
                            self.settings_preview();
                            self.settings = None;
                            if id == "theme" {
                                Effect::PickTheme
                            } else {
                                self.say("Factory views are retired; choose workspace Auto, Side or Stack.")
                            }
                        }
                    })
                }
                Event::Mouse(mouse) => {
                    self.settings.as_ref()?.mouse(*mouse);
                    Some(Effect::None)
                }
                _ => None,
            },
            Overlay::Theme => Some(match self.theme_picker.as_mut()?.input(event)? {
                super::theme_picker::Input::Preview => Effect::None,
                super::theme_picker::Input::Save => Effect::SaveTheme,
                super::theme_picker::Input::Cancel => {
                    self.theme_picker = None;
                    Effect::None
                }
            }),
            Overlay::CronList => self.cron_list_event(event),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.meter_hover = None;
        if let Some(effect) = self.overlay_event(&Event::Key(key)) {
            return effect;
        }
        self.notice = None;
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
            self.update_mentions();
            if let Some(effect) = self.mention_key(key) {
                return effect;
            }
            let effect = self.input_key(key);
            self.update_mentions();
            return effect;
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
        self.workspace_key(key)
    }

    /// Refuse pointer and viewport authority from a superseded frame. Offsets
    /// and drafts survive; only a new paint can publish interactive geometry.
    pub(super) fn invalidate_frame_geometry(&self) {
        self.input_viewport.set(None);
        self.input_band.set(None);
        self.hits.borrow_mut().clear();
        self.row_starts.borrow_mut().clear();
        self.tab_hits.borrow_mut().clear();
        self.detail_more_hits.borrow_mut().clear();
        self.workspace_action_hits.borrow_mut().clear();
        self.chip_hits.borrow_mut().clear();
        if let Some(draft) = &self.workspace_form {
            draft.invalidate();
        }
        self.conversation.borrow_mut().pane = None;
        self.scrolls.begin_frame();
        self.mentions.invalidate();
        if let Some(reader) = &self.row_details.reader {
            reader.source_area.set(None);
        }
        if let Some(checklist) = &self.checklist {
            checklist.invalidate();
        }
        if let Some(settings) = &self.settings {
            settings.surface.borrow_mut().invalidate();
        }
        if let Some(picker) = &self.theme_picker {
            picker.surface.borrow_mut().invalidate();
        }
        if let Some(list) = &self.cron_list {
            list.invalidate();
        }
    }

    fn bound(&self, key: KeyEvent) -> bool {
        event_name(key).is_some_and(|event| self.bindings().contains_key(&event))
    }

    /// Selects a row and keeps it on screen.
    pub(super) fn select(&mut self, row: usize) {
        if self.workspace_snapshot().is_some() {
            self.workspace_select(row);
            return;
        }
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
    pub(super) fn move_pointer(&mut self, event: MouseEvent) -> bool {
        let next = if self.loading()
            || self.overlay().is_some()
            || self.menu.is_some()
            || self.input.is_some()
        {
            None
        } else {
            self.hits
                .borrow()
                .iter()
                .rev()
                .find(|hit| hit.contains(event.column, event.row))
                .and_then(|hit| match hit.target {
                    HitTarget::Meter(bar) => Some(bar),
                    _ => None,
                })
        };
        let changed = next != self.meter_hover;
        self.meter_hover = next;
        changed
    }

    fn pointer_pane(&self, column: u16, row: u16) -> Option<Pane> {
        if self
            .conversation
            .borrow()
            .pane
            .is_some_and(|area| area.contains((column, row).into()))
        {
            Some(Pane::Replies)
        } else {
            self.scrolls.pane_at(column, row)
        }
    }

    fn scroll_pointer(&mut self, event: MouseEvent) -> bool {
        let lines = match event.kind {
            MouseEventKind::ScrollUp => Some(-(WHEEL_LINES as isize)),
            MouseEventKind::ScrollDown => Some(WHEEL_LINES as isize),
            _ => None,
        };
        if let Some(lines) = lines {
            if let Some(pane) = self
                .pointer_pane(event.column, event.row)
                .filter(|pane| !self.collapsed_panes().contains(pane))
            {
                self.scroll_pane(pane, Step::Lines(lines));
                if pane == Pane::Rows {
                    self.follow = false;
                }
            }
            return true;
        }
        false
    }

    pub fn mouse(&mut self, event: MouseEvent, now: Instant) -> Effect {
        if event.kind == MouseEventKind::Moved {
            self.move_pointer(event);
            return Effect::None;
        }
        // A button/wheel event carries the current position too. Reconcile an
        // existing hover, without inventing hover on terminals lacking motion.
        if self.meter_hover.is_some() {
            self.move_pointer(event);
        }
        if let Some(effect) = self.overlay_event(&Event::Mouse(event)) {
            return effect;
        }
        if self.menu.is_some() {
            return self.menu_mouse(event);
        }
        if self.mention_mouse(event) {
            return Effect::None;
        }
        if self.input.is_some() {
            if self
                .input
                .as_ref()
                .is_some_and(|input| input.draft_key.is_some())
                && self
                    .input_band
                    .get()
                    .is_none_or(|area| !area.contains((event.column, event.row).into()))
                && self.scroll_pointer(event)
            {
                return Effect::None;
            }

            if let Some((area, offset)) = self.input_viewport.get()
                && area.contains((event.column, event.row).into())
                && matches!(
                    event.kind,
                    MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left)
                )
            {
                let text = if self
                    .input
                    .as_ref()
                    .is_some_and(|input| matches!(input.compose, Compose::Status))
                {
                    self.reference_editor_mut()
                } else {
                    self.input.as_mut().map(|input| &mut input.text)
                };
                if let Some(text) = text {
                    text.place(
                        area.width,
                        offset + usize::from(event.row - area.y),
                        usize::from(event.column - area.x),
                        matches!(event.kind, MouseEventKind::Drag(_))
                            || event.modifiers.contains(KeyModifiers::SHIFT),
                    );
                }
                self.update_mentions();
            }
            return Effect::None;
        }
        if !self.loading() && event.kind == MouseEventKind::Down(MouseButton::Left) {
            let workspace_action = self
                .workspace_action_hits
                .borrow()
                .iter()
                .find(|(area, _, _)| area.contains((event.column, event.row).into()))
                .cloned();
            if let Some((_, target, choice)) = workspace_action {
                if self.workspace.selected.as_ref() != Some(&target)
                    || !self.workspace_selection_visible()
                {
                    return self.say(
                        "The painted action target changed; select it and repaint before acting.",
                    );
                }
                return self.choose(choice);
            }
            let chip = self
                .chip_hits
                .borrow()
                .iter()
                .find(|(area, _)| area.contains((event.column, event.row).into()))
                .map(|(_, row)| *row);
            if let Some(row) = chip {
                self.notice = None;
                self.select(row);
                return self.digest_row(row);
            }
            let target = self
                .detail_more_hits
                .borrow()
                .iter()
                .find(|(area, _)| area.contains((event.column, event.row).into()))
                .map(|(_, target)| target.clone());
            if let Some(target) = target.filter(|target| {
                self.detail(target)
                    .is_some_and(|detail| detail.reply.is_some())
            }) {
                self.row_details.reader = Some(super::row_detail::Reader::new(target, None));
                return Effect::None;
            }
        }

        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && !self.loading()
            && self.hits.borrow().iter().any(|hit| {
                hit.contains(event.column, event.row) && matches!(hit.target, HitTarget::Meter(_))
            })
        {
            return self.cycle_token_window();
        }
        if self.scroll_pointer(event) {
            return Effect::None;
        }
        // Shell tabs are navigation only, including before the first snapshot.
        // They never address the old name-based squad tab inventory.
        let tab = self.tab_hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        if let (MouseEventKind::Down(MouseButton::Left), Some(hit)) = (event.kind, tab) {
            let action = match hit.tab {
                0 => crate::action::OpsUiAction::Overview,
                1 => crate::action::OpsUiAction::Agents,
                2 => crate::action::OpsUiAction::DigestDashboard,
                _ => return Effect::None,
            };
            self.focus = 0;
            return self.workspace_action(action);
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Effect::None;
        }
        self.focus_at(event.column, event.row);
        let hit = self.hits.borrow().iter().copied().find(|hit| {
            hit.y == event.row && (hit.x..hit.x.saturating_add(hit.width)).contains(&event.column)
        });
        let Some(hit) =
            hit.filter(|hit| hit.row().is_some() && !self.collapsed_panes().contains(&Pane::Rows))
        else {
            return Effect::None;
        };
        self.notice = None;
        let row = hit.row().unwrap();
        self.select(row);
        let double = self.last_click.is_some_and(|(previous, at)| {
            previous == row && now.duration_since(at) <= DOUBLE_CLICK
        });
        self.last_click = (!double).then_some((row, now));
        let event = if double { "double-click" } else { "click" };
        match self.bindings().remove(event) {
            Some(action) => self.perform(&action),
            None => Effect::None,
        }
    }

    /// The shown squad's theme, or the already resolved startup theme before
    /// the first snapshot arrives.
    pub fn look(&self) -> crate::look::Look {
        let saved = self
            .workspace_squad()
            .and_then(|squad| self.workspace_snapshot()?.squad_looks.get(&squad.room_id))
            .copied()
            .unwrap_or_else(|| {
                self.view.as_ref().map_or_else(
                    || {
                        self.initial_look.unwrap_or_else(|| {
                            crate::look::Look::new(tmt_cli_style::Theme::default())
                        })
                    },
                    |view| view.look,
                )
            });
        self.theme_picker
            .as_ref()
            .map_or(saved, |picker| picker.preview(saved.depth))
    }

    pub(super) fn selected_read(&self) -> Option<super::refresh::SelectedRead> {
        if self.loading() {
            return None;
        }
        if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status))
            && let Some(draft) = &self.status_draft
            && draft.preview.is_none()
            && draft.error.is_none()
        {
            return draft
                .target
                .clone()
                .map(super::refresh::SelectedRead::Status);
        }
        if let Some(key) = self.detail_read() {
            return Some(super::refresh::SelectedRead::Message(key));
        }
        None
    }

    /// Exact admitted row data for the selected workspace target.
    pub fn selected_row(&self) -> Option<&Value> {
        if self.workspace_snapshot().is_some() {
            return self
                .workspace_selection_visible()
                .then(|| {
                    self.workspace_snapshot()?
                        .rows
                        .get(self.workspace.selected.as_ref()?)
                })
                .flatten();
        }
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
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn press(app: &mut App, code: KeyCode) -> Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn view(sections: Value) -> View {
        View {
            history_pending: false,
            ask_lead: crate::config::DEFAULT_ASK_LEAD.into(),
            token_rate: None,
            home_rate: Default::default(),
            exchanges: Vec::new(),
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
            bindings: crate::action::with_action_keys(crate::action::preset(true, &[])),
            section_bindings: Vec::new(),
            configured_bindings: Default::default(),
            host_bindings: Default::default(),
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
    fn common_overlay_route_consumes_close_release_and_unknown_mouse() {
        use ratatui::crossterm::event::KeyEventKind;
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot("product", json!([])));
        let current = app.current.clone();
        let focus = app.focus;
        app.help_state.borrow_mut().open();
        app.help = true;
        let mut released = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        released.kind = KeyEventKind::Release;
        assert_eq!(app.key(released), Effect::None);
        assert!(app.help, "release cannot close a modal");
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
        assert!(!app.help);
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
            workspace: Default::default(),
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
            references: Default::default(),
            timing: None,
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
    fn deferred_reads_complete_only_the_shown_squad_like_the_synchronous_load() {
        let enrichment = |held: u64, body: &str| super::super::refresh::Enrichment {
            digest: Some(BTreeMap::from([(
                "a".to_owned(),
                json!({"active": true, "digestUntilMs": 1, "remainingMs": 1, "heldCount": held}),
            )])),
            replies: Some(vec![json!({"requestId": "r1", "response": body})]),
        };
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot(
            "product",
            json!([{ "title": null, "rows": [row("a", ""), row("b", "")] }]),
        ));
        app.view.as_mut().unwrap().replies = vec![
            json!({"requestId": "r1", "status": "retained", "response": null}),
            json!({"requestId": "r2", "status": "retained", "response": "kept"}),
        ];
        assert!(
            !app.apply_enriched("infra", enrichment(9, "x")),
            "another squad"
        );
        assert!(app.apply_enriched("product", enrichment(2, "done")));
        let view = app.view.as_ref().unwrap();
        let rows = &view.document["sections"][0]["rows"];
        assert_eq!(rows[0]["digest"]["heldCount"], 2);
        assert!(rows[1].get("digest").is_none(), "no policy, no digest");
        assert_eq!(view.replies[0]["response"], "done");
        assert_eq!(
            view.replies[1]["response"], "kept",
            "bodies never overwrite"
        );

        // A failed optional read removes digest, as a load without it would show.
        let failed = super::super::refresh::Enrichment {
            digest: None,
            replies: None,
        };
        assert!(app.apply_enriched("product", failed));
        assert!(
            app.view.as_ref().unwrap().document["sections"][0]["rows"][0]
                .get("digest")
                .is_none()
        );

        // A late result for the tab being left never lands on the next one.
        let _ = app.go("infra".into());
        assert!(!app.apply_enriched("product", enrichment(5, "late")));
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
                references: Default::default(),
                timing: None,
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

    pub(in crate::board) fn crew(
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
            Some("jump lead: this squad has no lead; set one with tmt ops squad lead <name>.")
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
            Some("jump lead: this squad has no lead; set one with tmt ops squad lead <name>.")
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
        assert_eq!(app.input.as_ref().unwrap().header(), "→ rin (infra) · note");
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
        let mut app = crew(
            crate::action::with_action_keys(crate::action::preset(true, &[])),
            Vec::new(),
        );
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
                .starts_with("Who is sending? Record yourself with tmt ops squad me"),
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

    const COMPOSER_LEAD: &str = "44444444-4444-4444-8444-444444444441";
    const COMPOSER_DOCS: &str = "44444444-4444-4444-8444-444444444442";

    fn composer_row<'a>(view: &'a mut View, id: &str) -> &'a mut Value {
        view.home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&super::super::workspace::OpsTarget::Member {
                room_id: Some(super::workspace::tests::ROOM.into()),
                identity_id: id.into(),
            })
            .unwrap()
    }

    pub(in crate::board) fn workspace_composer() -> App {
        use super::super::workspace::OpsTarget;
        use super::workspace::tests::{MEMBER, ROOM};
        let mut app = super::workspace::tests::board();
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        let workspace = &mut view.home.as_mut().unwrap().workspace;
        workspace.roster.members.get_mut(MEMBER).unwrap().name = "auth-fix".into();
        workspace
            .rows
            .get_mut(&OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: MEMBER.into(),
            })
            .unwrap()["name"] = json!("auth-fix");
        for (id, name) in [(COMPOSER_LEAD, "sol"), (COMPOSER_DOCS, "docs")] {
            let mut member = workspace.roster.members[MEMBER].clone();
            member.id = id.into();
            member.name = name.into();
            workspace.roster.members.insert(id.into(), member);
            workspace.roster.squads[0].members.push(id.into());
            workspace.rows.insert(
                OpsTarget::Member {
                    room_id: Some(ROOM.into()),
                    identity_id: id.into(),
                },
                json!({"id":id,"name":name}),
            );
        }
        workspace.roster.squads[0].lead = Some(COMPOSER_LEAD.into());
        app.workspace_reconcile();
        app
    }

    #[test]
    fn talk_annotate_and_reply_compose_one_line_and_empty_sends_nothing() {
        let mut app = workspace_composer();
        {
            let view = app.view.as_mut().unwrap();
            view.me = Some("Ben".into());
            composer_row(view, super::workspace::tests::MEMBER)["waitingOnYou"] = json!([
                {"requestId": "q2", "preview": "approve the plan?"},
                {"requestId": "q1", "preview": "which database?"}
            ]);
            composer_row(view, COMPOSER_DOCS)["waitingOnYou"] =
                json!([{"requestId": "q9", "preview": "ok to merge?"}]);
        }
        app.workspace_reconcile();
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(
            app.input.as_ref().unwrap().prompt,
            "→ auth-fix (product) · talk"
        );
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
        app.finish_draft(true);
        app.finished(Ok("sent".into()));

        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "   ");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
        press(&mut app, KeyCode::Char('t'));
        typed(&mut app, "draft");
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none() && app.notice.as_deref() == Some("Nothing sent."));

        app.perform(&Action::parse("annotate").unwrap());
        assert_eq!(app.menu.as_ref().unwrap().entries.len(), 2);
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().prompt,
            "→ sol (product) · note · about auth-fix"
        );
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
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
        app.perform(&Action::parse("reply").unwrap());
        let menu = app.menu.as_ref().expect("picker");
        assert_eq!(menu.title, "answer auth-fix");
        assert_eq!(menu.entries.len(), 2);
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(
            app.input.as_ref().unwrap().prompt,
            "→ auth-fix (product) · answer"
        );
        assert!(app.input.as_ref().unwrap().text.contains("draft"));
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
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
        // Return from the conversation to the tree before choosing the next agent.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Down);
        app.perform(&Action::parse("reply").unwrap());
        assert!(app.menu.is_none());
        typed(&mut app, "yes");
        assert!(matches!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply { request, .. }) if request == "q9"
        ));
        assert!(!Request::Back.sends());

        // Nothing open, or no lead to annotate for, is a notice.
        composer_row(app.view.as_mut().unwrap(), COMPOSER_DOCS)["waitingOnYou"] = json!([]);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .lead = None;
        app.clamp();
        app.perform(&Action::parse("reply").unwrap());
        assert_eq!(app.notice.as_deref(), Some("docs is not waiting on you."));
        app.perform(&Action::parse("annotate").unwrap());
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Talk { .. }
        ));
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Status
        ));
    }

    #[test]
    fn row_composer_switches_modes_preserves_draft_and_reports_observed_outcomes() {
        let mut app = workspace_composer();
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        composer_row(view, super::workspace::tests::MEMBER)["waitingOnYou"] =
            json!([{"requestId":"q", "preview":"Ship tonight?"}]);
        app.workspace_reconcile();
        app.perform(&Action::parse("annotate").unwrap());
        assert!(app.menu.is_none(), "a single request opens in place");
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · answer"
        );
        assert_eq!(
            app.input.as_ref().unwrap().quote.as_deref(),
            Some("Ship tonight?")
        );
        typed(&mut app, "draft");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ sol (product) · note · about auth-fix"
        );
        assert_eq!(app.input.as_ref().unwrap().text, "draft");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · talk"
        );
        assert_eq!(app.input.as_ref().unwrap().text, "draft");
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Status
        ));
        assert_eq!(app.input.as_ref().unwrap().text, "draft");
        press(&mut app, KeyCode::Tab);
        assert!(
            matches!(app.input.as_ref().unwrap().compose, Compose::Reply { ref request, .. } if request == "q")
        );
        assert_eq!(
            app.input.as_ref().unwrap().quote.as_deref(),
            Some("Ship tonight?"),
            "the question is quoted again when the mode returns to answer"
        );
        let target = app.row_target(app.selected).unwrap();
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Reply { ref text, .. }) if text == "draft")
        );
        assert!(
            app.notice.is_none(),
            "opening a dispatch does not claim success"
        );
        app.finished(Err("delivery uncertain".into()));
        assert_eq!(app.notice.as_deref(), Some("delivery uncertain"));
        app.perform(&Action::parse("annotate").unwrap());
        typed(&mut app, "approved");
        press(&mut app, KeyCode::Enter);
        app.finished(Ok("Replied".into()));
        assert_eq!(app.notice.as_deref(), Some("Replied"));
        assert_eq!(app.row_target(app.selected).as_ref(), Some(&target));
        press(&mut app, KeyCode::Tab);
        assert!(
            app.notice.is_none(),
            "the next key clears the completion notice"
        );
        app.perform(&Action::parse("annotate").unwrap());
        typed(&mut app, "later");
        press(&mut app, KeyCode::Enter);
        app.finished(Ok("Message queued in auth-fix's inbox (r).".into()));
        assert_eq!(
            app.notice.as_deref(),
            Some("Message queued in auth-fix's inbox (r)."),
            "the visible outcome reports Core acceptance"
        );
    }

    fn composer_bindings(app: &mut App, text: &str) {
        let path = std::env::temp_dir().join(format!(
            "ops-composer-bindings-{}.toml",
            crate::id::new_v4().unwrap()
        ));
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .configure(&config)
            .unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn t_opens_talk_on_a_squad_tab_and_user_bindings_still_override_it() {
        let mut app = workspace_composer();
        app.view.as_mut().unwrap().me = Some("Ben".into());
        app.workspace_reconcile();
        assert_eq!(press(&mut app, KeyCode::Char('t')), Effect::None);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · talk"
        );
        app.input = None;
        composer_bindings(&mut app, "[bind]\nt = 'menu'\n");
        press(&mut app, KeyCode::Char('t'));
        assert!(app.input.is_none() && app.menu.is_some());
    }

    fn modes_of(app: &App) -> (String, Vec<&'static str>) {
        let input = app.input.as_ref().unwrap();
        (input.header(), input.modes())
    }

    #[test]
    fn tab_cycles_answer_note_talk_for_the_modes_the_row_allows() {
        let mut app = workspace_composer();
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        // Nothing waits: `a` opens the note; Tab reaches talk and back.
        app.workspace_reconcile();
        app.perform(&Action::parse("annotate").unwrap());
        assert_eq!(
            modes_of(&app),
            (
                "→ sol (product) · note · about auth-fix".into(),
                vec!["note", "talk", "status"]
            )
        );
        typed(&mut app, "keep");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · talk"
        );
        assert!(matches!(press(&mut app, KeyCode::Tab), Effect::None));
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Status
        ));
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Annotate { .. }
        ));
        assert_eq!(app.input.as_ref().unwrap().text, "keep");
        // Each mode sends through its own request after the same revalidation.
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            press(&mut app, KeyCode::Enter),
            Effect::Act(Request::Talk { ref to, ref text, .. }) if to == "auth-fix" && text == "keep"
        ));
        // Something waits: answer, note, talk in that cycle; a talk start
        // (a user's own `talk` binding) cycles on to answer, then note.
        composer_row(app.view.as_mut().unwrap(), super::workspace::tests::MEMBER)["waitingOnYou"] =
            json!([{"requestId": "q", "preview": "Ship?"}]);
        app.perform(&Action::parse("annotate").unwrap());
        assert_eq!(
            modes_of(&app),
            (
                "→ auth-fix (product) · answer".into(),
                vec!["answer", "note", "talk", "status"]
            )
        );
        press(&mut app, KeyCode::Esc);
        composer_bindings(&mut app, "[bind]\nx = 'talk'\n");
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · talk"
        );
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            Compose::Status
        ));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (product) · answer"
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ sol (product) · note · about auth-fix"
        );
    }

    #[test]
    fn a_row_without_a_lead_or_request_can_cycle_talk_and_status() {
        let mut app = workspace_composer();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .lead = None;
        composer_bindings(&mut app, "[bind]\nx = 'talk'\n");
        app.workspace_reconcile();
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            modes_of(&app),
            ("→ auth-fix (product) · talk".into(), vec!["talk", "status"])
        );
        typed(&mut app, "hi");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "Update status → product / auth-fix"
        );
        assert_eq!(app.input.as_ref().unwrap().text, "hi");
    }

    #[test]
    fn an_explicit_member_note_binding_cycles_note_talk_status() {
        let mut app = workspace_composer();
        let view = app.view.as_mut().unwrap();
        view.me = Some("Ben".into());
        composer_row(view, super::workspace::tests::MEMBER)["waitingOnYou"] =
            json!([{"requestId": "q", "preview": "Ship?"}]);
        composer_bindings(&mut app, "[bind]\nx = 'annotate member'\n");
        app.workspace_reconcile();
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(
            modes_of(&app),
            (
                "→ auth-fix (product) · note".into(),
                vec!["note", "talk", "status"]
            )
        );
    }

    #[test]
    fn composer_anchors_the_selected_occurrence_and_explicit_member_notes_stay_notes() {
        use super::super::workspace::OpsTarget;
        use super::workspace::tests::MEMBER;
        const OTHER_ROOM: &str = "55555555-5555-4555-8555-555555555555";
        let mut app = workspace_composer();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let mut squad = snapshot.roster.squads[0].clone();
        squad.room_id = OTHER_ROOM.into();
        squad.name = "other-product".into();
        snapshot.roster.squads.push(squad);
        let selected = OpsTarget::Member {
            room_id: Some(OTHER_ROOM.into()),
            identity_id: MEMBER.into(),
        };
        let mut row = snapshot.rows[&OpsTarget::Member {
            room_id: Some(super::workspace::tests::ROOM.into()),
            identity_id: MEMBER.into(),
        }]
            .clone();
        row["waitingOnYou"] = json!([{"requestId":"q", "preview":"Choose?"}]);
        snapshot.rows.insert(selected.clone(), row);
        app.workspace.expanded.insert(OTHER_ROOM.into());
        app.workspace.selected = Some(selected.clone());
        app.workspace_reconcile();
        app.perform(&Action::parse("annotate").unwrap());
        assert_eq!(
            app.input
                .as_ref()
                .unwrap()
                .row_send
                .as_ref()
                .unwrap()
                .target,
            RowTarget::Workspace(selected.clone())
        );
        press(&mut app, KeyCode::Esc);
        app.perform(&Action::parse("annotate member").unwrap());
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ auth-fix (other-product) · note"
        );
        typed(&mut app, "context");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Annotate { ref squad, ref to, ref row, .. }) if squad == "other-product" && to == "auth-fix" && row == "auth-fix")
        );
        assert_eq!(app.workspace.selected, Some(selected));
    }

    #[test]
    fn row_composer_follows_identity_on_refresh_and_refuses_changed_authority() {
        use super::workspace::tests::MEMBER;
        for change in [
            "reorder", "sender", "lead", "request", "identity", "loading",
        ] {
            let mut app = workspace_composer();
            composer_row(app.view.as_mut().unwrap(), MEMBER)["waitingOnYou"] =
                json!([{"requestId":"q", "preview":"Choose?"}]);
            app.perform(&Action::parse("annotate").unwrap());
            let target = app
                .input
                .as_ref()
                .unwrap()
                .row_send
                .as_ref()
                .unwrap()
                .target
                .clone();
            typed(&mut app, "yes");
            if change == "lead" {
                press(&mut app, KeyCode::Tab);
            }
            match change {
                "reorder" => {
                    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                    snapshot.roster.members.get_mut(COMPOSER_DOCS).unwrap().name =
                        "a-before".into();
                    composer_row(app.view.as_mut().unwrap(), COMPOSER_DOCS)["name"] =
                        json!("a-before");
                }
                "sender" => app.view.as_mut().unwrap().me = Some("other".into()),
                "lead" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .members
                        .get_mut(COMPOSER_LEAD)
                        .unwrap()
                        .name = "other".into()
                }
                "request" => {
                    composer_row(app.view.as_mut().unwrap(), MEMBER)["waitingOnYou"] = json!([])
                }
                "identity" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .members
                        .remove(MEMBER);
                }
                _ => app.current = Some("unloaded".into()),
            }
            app.clamp();
            let effect = press(&mut app, KeyCode::Enter);
            if change == "reorder" {
                assert_eq!(app.row_target(app.selected).as_ref(), Some(&target));
                assert!(
                    matches!(effect, Effect::Act(Request::Reply { ref request, ref from, .. }) if request == "q" && from == "auth-fix")
                );
            } else {
                assert_eq!(effect, Effect::None, "{change}");
                assert!(
                    app.notice.as_ref().unwrap().contains("nothing sent"),
                    "{change}"
                );
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
                    draft_key: None,
                    response_quotes: Vec::new(),
                    row_send: None,
                    others: Vec::new(),
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

    fn located(name: &str, pane: Value) -> Value {
        let mut row = member(name, json!({}));
        row["pane"] = pane;
        row
    }

    #[test]
    fn a_host_table_overrides_bind_for_rows_on_that_host_and_only_for_them() {
        let mut flat = crate::action::preset(true, &[]);
        flat.extend(bind(&[("enter", "run flat {name}")]));
        let hosts = [
            (
                "tmux".to_owned(),
                bind(&[(
                    "enter",
                    "run tmux switch-client -t {pane} {host} {session} {window}",
                )]),
            ),
            (
                "herdr".to_owned(),
                bind(&[("enter", "run herdr pane focus {pane} {session}")]),
            ),
        ];
        let mut snapshot = snapshot(
            "product",
            json!([{"title": "Crew", "rows": [
                located("a", json!({"id": "%1", "location":
                    {"host": "tmux", "session": "work", "window": "2"}})),
                located("b", json!({"id": "%2", "location": {"host": "herdr"}})),
                located("c", json!({"id": "%3"})),
                located("d", json!({"id": "%4", "location": {"host": "other"}})),
            ]}]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.configured_bindings = flat.clone();
        view.bindings = flat;
        view.host_bindings = hosts.into_iter().collect();
        view.section_bindings = vec![Bindings::new()];
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot);
        let run = |args: &[&str]| {
            Effect::Act(Request::Run(args.iter().map(|arg| (*arg).into()).collect()))
        };
        // The tmux table fills its coordinates from Core's published location.
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            run(&["tmux", "switch-client", "-t", "%1", "tmux", "work", "2"])
        );
        // A host table with a field its host does not report refuses; nothing runs.
        press(&mut app, KeyCode::Down);
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(
            app.notice.as_deref(),
            Some("run: session is empty for this row.")
        );
        // No location, or a host with no table, keeps the flat binding.
        for _ in 0..2 {
            press(&mut app, KeyCode::Down);
            let name = app.selected_row().unwrap()["name"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                press(&mut app, KeyCode::Enter),
                run(&["flat", &name]),
                "{name}"
            );
        }
    }

    #[test]
    fn a_section_binding_still_wins_over_a_host_table() {
        let mut flat = crate::action::preset(true, &[]);
        flat.extend(bind(&[("enter", "run flat {name}")]));
        let mut snapshot = snapshot(
            "product",
            json!([{"title": "Crew", "rows": [located(
                "a",
                json!({"id": "%1", "location": {"host": "tmux", "session": "s", "window": "0"}})
            )]}]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.configured_bindings = flat.clone();
        view.bindings = flat;
        view.host_bindings = [(
            "tmux".to_owned(),
            bind(&[("enter", "run host {pane}"), ("o", "run host-o {pane}")]),
        )]
        .into_iter()
        .collect();
        view.section_bindings = vec![bind(&[("enter", "run section {name}")])];
        let mut app = App::new(Some("product".into()));
        app.apply(snapshot);
        let run = |args: &[&str]| {
            Effect::Act(Request::Run(args.iter().map(|arg| (*arg).into()).collect()))
        };
        assert_eq!(press(&mut app, KeyCode::Enter), run(&["section", "a"]));
        // The host table still supplies what the section leaves alone.
        assert_eq!(press(&mut app, KeyCode::Char('o')), run(&["host-o", "%1"]));
    }

    #[test]
    fn the_menu_lists_the_rows_actions_first_then_the_boards_and_user_keys_follow_their_verb() {
        let mut bindings = crate::action::with_action_keys(crate::action::preset(false, &[]));
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
                "e",
                "v",
                "l",
                "T",
                ",",
                "ctrl-r",
                "backspace",
                ""
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
                target: HitTarget::Row(0),
            },
            Hit {
                y: 5,
                x: 0,
                width: 40,
                target: HitTarget::Row(1),
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
            references: Default::default(),
            timing: None,
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
        view.bindings = crate::action::with_action_keys(crate::action::preset(true, &[]));
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
        assert_eq!(
            app.input.as_ref().unwrap().header(),
            "→ bob (product) · note"
        );
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
        assert_eq!(app.input.as_ref().unwrap().prompt, "→ sol (product) · talk");
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
    fn saved_cycle(app: &mut App, key: KeyEvent) {
        assert_eq!(app.key(key), Effect::CycleTokenWindow);
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-window-choice-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("ops.toml");
        let mut config = Config::read(path.clone()).unwrap();
        config
            .set_setting(
                None,
                "board.token_rate.window",
                &app.next_token_window().unwrap().label(),
            )
            .unwrap();
        let saved = Config::read(path.clone()).unwrap();
        app.notice = Some(app.apply_token_window(&saved));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn app() -> App {
        let mut app = crew(
            crate::action::with_action_keys(crate::action::preset(false, &[])),
            Vec::new(),
        );
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
    #[test]
    fn refreshed_shared_window_preserves_retained_evidence() {
        let mut app = app();
        let start = Instant::now();
        let input = super::super::rate::tests::input(100);
        let settings = app.meter.as_ref().unwrap().settings;
        app.meter = Some(super::super::meter::Meter::new(settings, &input, start));
        app.meter.as_mut().unwrap().sample(
            Ok(&super::super::rate::tests::input(200)),
            start + Duration::from_secs(10),
        );
        let before = app
            .meter
            .as_ref()
            .unwrap()
            .total(1, start + Duration::from_secs(10))
            .map(|reading| reading.tokens);
        assert_eq!(before, Some(150));
        let mut snapshot = super::tests::snapshot("product", json!([]));
        app.current = Some("product".into());
        let mut changed = settings;
        changed.window = TokenWindow::FIVE_MINUTES;
        snapshot.view.as_mut().unwrap().token_rate = Some(RateView {
            settings: changed,
            input,
            history: None,
        });
        app.apply(snapshot);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        assert_eq!(
            app.meter
                .as_ref()
                .unwrap()
                .total(1, start + Duration::from_secs(10))
                .map(|reading| reading.tokens),
            before,
            "changing shared display window cannot replace the retained rate ring"
        );
    }

    fn home(now: Instant) -> App {
        let mut app = App::new(Some(super::super::ALL.into()));
        let mut snapshot = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        let view = snapshot.view.as_mut().unwrap();
        view.home = Some(super::super::home::Home {
            workspace: Default::default(),
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
    fn delayed_history_rejects_changed_owner_room_settings_and_membership() {
        for mismatch in ["owner", "room", "settings", "members", "none"] {
            let now = Instant::now();
            let mut app = home(now);
            let rate = &app.view.as_ref().unwrap().home_rate["product"];
            let mut input = rate.input.clone();
            let mut settings = rate.settings;
            let mut owner = super::super::ALL.to_owned();
            match mismatch {
                "owner" => owner = "other".into(),
                "room" => input.room = "replacement-room".into(),
                "settings" => settings.enabled = false,
                "members" => {
                    input.resumes.remove("missing");
                }
                _ => {}
            }
            let read = super::super::refresh::HistoryRead {
                owner,
                targets: [("product".into(), (settings, input))].into(),
                seeds: BTreeMap::new(),
                observed: Err(()),
            };
            assert_eq!(
                app.apply_history(read, now),
                mismatch == "none",
                "{mismatch}"
            );
        }
    }

    #[test]
    fn home_header_paints_stepped_digits_without_interpolating_evidence_or_status() {
        let now = Instant::now();
        let mut app = header(now);
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["pending"] =
            json!("review this");
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["waitingOnYou"] =
            json!([{"requestId":"q1"}]);
        let document = app.view.as_ref().unwrap().document.clone();
        let selection = app.selected;
        let first = now + Duration::from_secs(10);
        app.sample_home(Ok(&super::super::rate::tests::input(200).resumes), first);
        let text = |app: &App, time| {
            let usage = app.home_header_usage(time).unwrap();
            app.home_header_digits(&usage, time);
            super::super::home::usage_of(app, &usage, 160, app.look())
                .unwrap()
                .spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        };
        app.home_counters.borrow_mut().begin();
        assert!(text(&app, first).contains("1h ~150"));
        app.home_counters.borrow_mut().finish(None, false);
        let start = now + Duration::from_secs(20);
        app.sample_home(Ok(&super::super::rate::tests::input(400).resumes), start);
        let raw = app.home_header_usage(start).unwrap();
        let totals = raw.totals;
        let top = raw.top.unwrap().share;
        let model = raw.models[0].share.fraction;
        assert_eq!(totals.map(|reading| reading.unwrap().tokens), [450; 3]);
        app.home_counters.borrow_mut().begin();
        assert!(
            text(&app, start).contains("1h ~150"),
            "retarget starts from visible digits"
        );
        app.home_counters.borrow_mut().finish(None, false);
        assert!(
            app.home_counters
                .borrow_mut()
                .tick(start + Duration::from_millis(250))
        );
        assert!(text(&app, start + Duration::from_millis(250)).contains("1h ~390"));
        assert!(
            app.home_counters
                .borrow_mut()
                .tick(start + Duration::from_millis(600))
        );
        assert!(text(&app, start + Duration::from_millis(600)).contains("1h ~450"));
        assert!(
            app.home_counters
                .borrow()
                .wait(start + Duration::from_millis(600))
                .is_none()
        );
        let settled = app.home_header_usage(start).unwrap();
        assert_eq!(settled.totals, totals);
        assert_eq!(settled.top.unwrap().share, top);
        assert_eq!(settled.models[0].share.fraction, model);
        assert_eq!(
            app.view.as_ref().unwrap().document,
            document,
            "pending and request state are immutable"
        );
        assert_eq!(app.selected, selection);
        app.workspace_action(crate::action::OpsUiAction::Agents);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
        terminal
            .draw(|frame| super::super::view::render(frame, &app))
            .unwrap();
        assert!(
            app.home_counters.borrow().wait(start).is_none(),
            "an Agents frame drops invisible Overview animation wakeups"
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
    fn workspace_shell_navigation_retains_scoped_usage_and_failed_inventory_marks_it_stale() {
        let now = Instant::now();
        let mut app = home(now);
        assert_eq!(
            app.workspace_action(crate::action::OpsUiAction::Agents),
            Effect::None
        );
        assert_eq!(app.current.as_deref(), Some(super::super::ALL));
        assert!(!app.loading());
        assert!(app.meter.is_none() && app.meters.contains_key("product"));
        let mut failed = super::tests::snapshot(super::super::ALL, serde_json::json!([]));
        failed.view = Err("workspace inventory failed".into());
        app.apply(failed);
        assert!(app.home_usage("product", now).is_some());
        assert_eq!(app.error.as_deref(), Some("workspace inventory failed"));
        assert_eq!(
            app.workspace_snapshot().unwrap().roster.observation,
            super::super::workspace::OpsObservation::Stale
        );
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
    fn workspace_receipts_keep_source_scope_across_shell_navigation_and_ignore_unrelated_identities()
     {
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
        assert_eq!(app.go("other".into()), Effect::None);
        assert_eq!(app.current.as_deref(), Some(super::super::ALL));
        app.workspace_action(crate::action::OpsUiAction::Agents);
        app.workspace_action(crate::action::OpsUiAction::DigestDashboard);
        app.sample_home(Ok(&receipt), time + Duration::from_secs(10));
        assert_eq!(
            app.home_usage("product", time + Duration::from_secs(10))
                .unwrap()
                .squad[2]
                .unwrap()
                .tokens,
            150
        );
        assert!(app.home_usage("product", time).is_some());
    }

    #[test]
    fn window_binding_overrides_and_text_inputs_keep_their_owner() {
        let mut app = app();
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE);
        saved_cycle(&mut app, key);
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
                draft_key: None,
                response_quotes: Vec::new(),
                row_send: None,
                others: Vec::new(),
                quote: None,
                link: None,
                prompt: "message".into(),
                text: Default::default(),
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
            .extend(bind(&[("w", "refresh"), ("W", "token-window")]));
        assert_eq!(app.key(key), Effect::Refresh);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        saved_cycle(
            &mut app,
            KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE),
        );
        assert_eq!(app.token_window, TokenWindow::HOUR);
        app.view.as_mut().unwrap().section_bindings = vec![bind(&[("w", "token-window")])];
        saved_cycle(&mut app, key);
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
            saved_cycle(
                &mut app,
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            );
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
