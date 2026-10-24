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
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

mod carry;
mod panels;
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
}

#[derive(Debug)]
pub(super) struct UsageTop<'a> {
    pub member: &'a str,
    pub share: UsageShare,
}

#[derive(Clone, Copy)]
struct HeaderObservation {
    reading: super::rate::Reading,
    covered_slots: u64,
}

impl HeaderObservation {
    fn evidence(&self) -> (u64, bool, u64) {
        (self.covered_slots, !self.reading.partial, self.reading.span)
    }
}

struct HeaderMember<'a> {
    name: &'a str,
    readings: [Option<HeaderObservation>; 3],
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
    pub render: NotesRender,
    /// The host preset overridden by `[bind]`.
    pub bindings: Bindings,
    pub opener: Option<Vec<String>>,
    pub clipboard: Option<Vec<String>>,
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
    /// Core's change cursor taken just before this load; `None` when it could not be read.
    pub(super) cursor: Option<u64>,
}

/// Clipboard feedback follows the gesture; the worker keeps one copy implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFeedback {
    Destination,
    Selection,
}

/// An action with every value already taken from the row.
/// Message variants carry the complete frozen body, including any row context
/// and source quotes. Workers and outgoing matching must not reformat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show this member's pane in the invoking client.
    Jump(String),
    Swap {
        target: String,
        name: String,
        direction: String,
    },
    /// Return the invoker's client to where its last jump came from.
    Back,
    Open {
        link: String,
        opener: Option<Vec<String>>,
    },
    Copy {
        text: String,
        program: Option<Vec<String>>,
        feedback: CopyFeedback,
    },
    Run(Vec<String>),
    /// The effect lane supplies reserved invocation context before argv filling.
    PreparedRun {
        action: Action,
        row: Value,
        ops: Value,
    },
    Talk {
        me: String,
        squad: String,
        to: String,
        text: String,
    },
    Chat {
        sender: String,
        recipient: String,
        name: String,
        text: String,
        fallback: String,
    },
    Annotate {
        me: String,
        squad: String,
        to: String,
        row: String,
        text: String,
    },
    /// Take one of the person's own held or waiting messages out of the recipient's Digest hold
    /// and type it now; Core decides whether it may.
    SendNow {
        me: String,
        request: String,
        to: String,
    },
    Status(Box<super::status_update::Action>),
    /// Save this tab order to `[tabs] order` (tab keys, in order).
    WorkspacePreferences {
        expected: crate::config::OpsPreferences,
        next: crate::config::OpsPreferences,
        saved: Option<String>,
    },
    WorkspaceOperation(super::workspace::manage::Change),
    /// A cron job control, with its actor, job and viewed revision resolved.
    CronPreview(Box<super::cronboard::preview::Task>),
    Cron(super::cronboard::CronRequest),
    /// Check that the person at the board may change a member's digest.
    DigestOpen(super::digest::Ask),
    /// Apply a digest value the user chose.
    Digest(super::digest::Set),
    /// Turn Digest on (`true`) or off for everyone.
    DigestSwitch(bool),
}

impl Request {
    /// What the board shows while the lane runs this request.
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Jump(_) | Self::Back => "Jumping…",
            Self::Swap { .. } => "Swapping…",
            Self::Open { .. } | Self::Run(_) | Self::PreparedRun { .. } => "Opening…",
            Self::DigestOpen(_) => "Checking digest access…",
            Self::Digest(_) => "Saving digest…",
            Self::DigestSwitch(_) => "Switching Digest…",
            Self::Copy { .. } => "Copying…",
            Self::Talk { .. } | Self::Chat { .. } | Self::Annotate { .. } => "Sending…",
            Self::SendNow { .. } => "Sending now…",
            Self::Status(_) => "Updating status…",
            Self::WorkspacePreferences { .. } => "Saving workspace preferences…",
            Self::WorkspaceOperation(_) => "Applying workspace operation…",
            Self::CronPreview(_) => "Reading the timer preview…",
            Self::Cron(_) => "Updating the job…",
        }
    }

    /// Sending changes request state, and a saved order changes the tabs, so
    /// the view reloads afterwards.
    pub fn sends(&self) -> bool {
        matches!(
            self,
            Self::Talk { .. }
                | Self::Chat { .. }
                | Self::Annotate { .. }
                | Self::SendNow { .. }
                | Self::WorkspacePreferences { .. }
                | Self::WorkspaceOperation(_)
                | Self::Cron(_)
                | Self::Status(_)
                | Self::Digest(_)
                | Self::DigestSwitch(_)
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
    /// The settings overlay in this squad's context, reached from the Workspace settings menu.
    SquadSettings(String),
    HostAction,
    SaveSetting,
    ResetSetting,
    CycleTokenWindow,
    CancelSettings,
    PickTheme,
    SaveTheme,
    Act(Request),
    Checklist(super::checklist::load::Task),
    Recovery(super::recovery::load::Task),
    Discovery(super::discovery::load::Task),
    MemberSession(super::member_session::Task),
    /// A read or a write of the Notes of the conversation on screen.
    Notes(super::conversation_checklist::Task),
    NewMember(super::new_member::Task),
    IdleAlert(super::idle_alert_editor::Task),
    Foreground(super::member_session::Foreground),
}

/// What a menu entry does: run a binding, or reply to one open request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    MemberLink {
        target: super::workspace::OpsTarget,
        jump: bool,
    },
    MemberLinkAction {
        target: super::workspace::OpsTarget,
        action: Action,
    },
    /// A visible but disabled dropdown choice, with the reason it is unavailable.
    Unavailable(String),
    /// A link's popover: open the address, copy it, or show all of it.
    LinkOpen(String),
    LinkCopy(String),
    LinkShow {
        url: String,
        anchor: ratatui::layout::Rect,
    },
    FileCopy(std::sync::Arc<str>),
    FileShow {
        path: String,
        anchor: ratatui::layout::Rect,
    },
    AgentsFilter(super::workspace::AgentsFilter),
    /// A click on a Talk/Measure chip selects that pane; the `M` key toggles.
    RightTab(super::shell::RightTab),
    WorkspaceTarget(super::workspace::OpsTarget),
    /// A click on the Notes chip of a member's header: opens the list.
    MemberNotes,
    AddToNotes(super::conversation_checklist::Seed),
    WorkspaceBegin(super::workspace::manage::Kind),
    WorkspaceRoom {
        kind: super::workspace::manage::Kind,
        identity_id: String,
        room_id: String,
    },
    HostAction,
    LaunchTemplates,
    /// Opens the squad settings overlay for this squad from the Workspace settings menu.
    SquadSettings(String),
    IdleAlert(String),
    /// A rule between menu groups: painted, never chosen.
    Rule,
    /// The muted line above a menu's rows: painted, never chosen, not counted.
    Heading,
    WorkspaceLayout(crate::config::OpsLayout),
    DigestWindow(crate::config::DigestWindow),
    /// The shared saved TOK range for Agents and Digest.
    TokRange(crate::config::TokRange),
    /// A click on an Agents column header: sort by it, or reverse it.
    SortBy(super::header_sort::Header),
    /// A click on an exchange matrix cell: Talk opens with the row's agent.
    ExchangeCell {
        sender: String,
        recipient: String,
    },
    /// A click on a painted Digest estimate: the next unit.
    SavingsUnit,
    /// A click on a painted TOK figure of the Agents list: tokens or list-price money.
    TokMoney,
    /// A click on the unit word of the Agents TOK header: the same switch as the figures.
    TokUnit,
    ClearSearch,
    Panels,
    Checklist,
    /// The room UUID of a fresh squad-row count, independent of the previous selection.
    RoomChecklist(String),
    Action(Action),
    /// Confirms a cron control picked from a menu.
    Cron(super::cronboard::CronRequest),
    /// Leaves the menu without doing anything.
    Dismiss,
    /// A digest value picked from the dropdown.
    Digest(super::digest::Set),
    /// The dropdown's own entry: type a value instead.
    DigestCustom(Box<super::digest::Granted>),
    /// The Digest tab's on/off chip: flip the one switch.
    DigestSwitch,
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

/// The row's action menu.
pub struct Menu {
    pub(super) row_send: Option<RowSend>,
    pub link: Option<LinkSend>,
    pub prefill: String,
    pub title: String,
    pub entries: Vec<MenuEntry>,
    pub selected: usize,
    pub(super) anchor: Option<ratatui::layout::Rect>,
    pub(super) surface: RefCell<Option<super::menu_surface::MenuSurface>>,
}

impl Menu {
    /// Moves the highlight by `delta` entries, stepping over a rule and stopping at either end.
    pub(super) fn step(&mut self, delta: isize) {
        let direction = delta.signum();
        let mut at = self.selected;
        for _ in 0..delta.unsigned_abs() {
            let mut next = at;
            while let Some(moved) = next.checked_add_signed(direction) {
                if moved >= self.entries.len() {
                    break;
                }
                next = moved;
                if !matches!(self.entries[next].choice, Choice::Rule | Choice::Heading) {
                    at = next;
                    break;
                }
            }
        }
        self.selected = at;
    }
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
    Status,
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
    pub(super) id: Option<String>,
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
        if matches!(input.compose, Compose::Talk { .. }) {
            return app
                .view
                .as_ref()
                .and_then(|view| view.me.as_ref())
                .is_some()
                && app.view.as_ref().and_then(|view| view.me_id.as_ref())
                    == self.sender_id.as_ref()
                && app.target_row(&self.target).is_some();
        }
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
            Compose::Talk { .. } => {
                unreachable!("member admission handled above")
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
#[derive(Clone)]
pub struct Input {
    pub(super) draft_key: Option<(String, String)>,
    pub(super) row_send: Option<RowSend>,

    pub(super) inline_quotes: std::collections::BTreeMap<u64, super::composer::QuoteSource>,

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
                    format!("→ {to}")
                } else {
                    format!("→ {to} ({})", self.squad)
                }
            }
            Compose::Annotate { to, row } if to != row => {
                format!("→ {to} ({}) · note · about {row}", self.squad)
            }
            Compose::Annotate { to, .. } => format!("→ {to} ({}) · note", self.squad),
            Compose::AskLead { to, .. } => format!("→ lead {to}"),
            Compose::Cron | Compose::Workspace | Compose::Digest(_) => self.prompt.clone(),
            Compose::Status => format!(
                "Update status → {} / {}",
                self.squad,
                self.row_send
                    .as_ref()
                    .map_or("agent", |send| send.name.as_str())
            ),
        }
    }
}

/// Small form field budget, in UTF-8 bytes.
pub(super) const INPUT_LIMIT: usize = 4000;
/// Session message limit, in UTF-8 bytes; reject oversized insertions atomically.
pub(super) use crate::requests::message::MESSAGE_LIMIT;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Shared routing identifies the active surface; its controller owns effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Overlay {
    Workspace,
    Recovery,
    Help,
    Settings,
    Theme,
    Panels,
    CronList,
    CronReview,
    MonitorDetails,
    ReplyReader,
    Checklist,
    QuotaDetails,
    Discovery,
    MemberSession,
    Notes,
    NewMember,
    IdleAlert,
    UsageControls,
    PastePreview,
}
impl Overlay {
    fn id(self) -> tmt_tui::app::ComponentId {
        vec![
            match self {
                Self::Workspace => "workspace-form",
                Self::Recovery => "recovery",
                Self::Help => "help",
                Self::Settings => "settings",
                Self::Theme => "theme-picker",
                Self::Panels => "panels-picker",
                Self::CronList => "cron-list",
                Self::CronReview => "cron-review",
                Self::MonitorDetails => "monitor-details",
                Self::ReplyReader => "reply-reader",
                Self::Checklist => "checklist",
                Self::QuotaDetails => "quota-details",
                Self::Discovery => "discovery",
                Self::MemberSession => "member-session",
                Self::Notes => "notes",
                Self::NewMember => "new-member",
                Self::IdleAlert => "idle-alert",
                Self::UsageControls => "usage-controls",
                Self::PastePreview => "paste-preview",
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

/// A clipped timer count freezes its semantic UUID scope, not the row index.
#[derive(Clone)]
pub(in crate::board) struct TimerHit {
    pub area: ratatui::layout::Rect,
    pub target: super::workspace::OpsTarget,
    pub room: Option<String>,
    pub owner: Option<String>,
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
    Quota,
    AllRoomTimers,
}
impl Hit {
    pub fn row(&self) -> Option<usize> {
        match self.target {
            HitTarget::Row(row) => Some(row),
            HitTarget::Meter(_) | HitTarget::Quota | HitTarget::AllRoomTimers => None,
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

/// One cold-start request, consumed by the existing workspace reconciliation.
#[derive(Debug, Clone)]
pub(super) struct WorkspaceStartup {
    pub room_id: Option<String>,
    pub tab: Option<super::shell::OpsTab>,
}

#[derive(Default)]
pub struct App {
    pub(super) selected_request: super::refresh::SelectedRequest,
    pub(super) buttons: super::buttons::State,
    pub(super) pane_resize: super::pane_resize::State,
    /// Injected by the renderer, never inferred from NO_COLOR or stored preferences.
    pub cron_glyphs: super::overview::cron_counts::GlyphSet,
    pub(in crate::board) conversation_return: Option<super::shell::OpsTab>,
    pub(in crate::board) monitor_details: Option<super::overview::detail::Details>,
    pub(in crate::board) invoker_tmux: bool,
    /// Terminal-owner observation for hints only; never persisted or inferred.
    pub(in crate::board) keyboard_enhanced: bool,
    pub(super) shell: super::shell::OpsShell,
    pub(super) workspace: super::workspace::OpsWorkspace,
    frame_workspace_rows: RefCell<Option<std::rc::Rc<[super::workspace::OpsTreeRow]>>>,
    pub(super) workspace_startup: Option<WorkspaceStartup>,
    pub(super) workspace_form: Option<super::workspace::manage::Draft>,
    pub(in crate::board) cron_scope_hits: RefCell<Vec<TimerHit>>,
    pub(in crate::board) view_layer: super::view_state::Layer,
    /// Talk or Measure in the Agents tab's right column (`t`).
    pub(in crate::board) right_tab: super::shell::RightTab,
    pub(in crate::board) overview_rows: RefCell<super::overview::monitor::Cache>,
    pub(in crate::board) digest_rows: RefCell<super::digest_dashboard::Cache>,
    pub(in crate::board) agents_layout: RefCell<super::workspace::columns::AgentsLayout>,
    pub(in crate::board) workspace_strip_hits: RefCell<Vec<(ratatui::layout::Rect, Choice)>>,
    pub(in crate::board) workspace_state_cells: RefCell<Vec<ratatui::layout::Rect>>,
    pub(in crate::board) workspace_action_hits:
        RefCell<Vec<(ratatui::layout::Rect, super::workspace::OpsTarget, Choice)>>,
    pub(super) timing: Option<super::timing::Ui>,
    pub(super) checklist: Option<super::checklist::Controller>,
    pub(super) recovery: Option<super::recovery::Controller>,
    pub(super) discovery: Option<super::discovery::Controller>,
    pub(super) member_session: Option<super::member_session::Controller>,
    pub(super) member_session_generation: u64,
    pub(super) notes_overlay: Option<super::conversation_checklist::overlay::Controller>,
    pub(super) new_member: Option<super::new_member::Controller>,
    pub(super) new_member_generation: u64,
    pub(super) idle_alert_editor: Option<super::idle_alert_editor::Controller>,
    pub(super) idle_alert_generation: u64,
    pub(super) foreground: Option<super::member_session::Foreground>,
    pub(super) discovered_uuid: Option<String>,
    pub(super) checklist_generation: u64,
    pub(super) initial_look: Option<crate::look::Look>,
    pub(super) cron: super::cronboard::State,
    pub(super) home_leads: super::home_leads::State,
    pub(super) unread: super::unread::publish::Session,
    /// What the configured label sources last supplied for member rows.
    pub(super) labels: crate::labels::Supplied,
    /// The Digest tab's Usage report: its window and the newest read.
    pub(in crate::board) usage_report: super::usage_report::State,
    /// The cron form being filled in on the input line, if any.
    pub(super) status_draft: Option<super::status_update::Draft>,
    pub(in crate::board) cron_review: Option<super::cronboard::preview::Review>,
    pub(in crate::board) cron_review_generation: u64,
    pub(super) cron_draft: Option<super::cronboard::Draft>,
    pub(super) home_counters: RefCell<super::home::counters::Counters>,
    /// Provider limits as the worker last sampled them; `None` until the first sample.
    pub(super) limits: Option<super::limits::Snapshot>,
    /// Cell widths and row values that outlive a snapshot, so a refresh cannot move the layout.
    pub(super) meter: Option<super::meter::Meter>,
    meters: BTreeMap<String, super::meter::Meter>,
    usage_document: Option<Value>,
    pub(super) token_window: crate::config::TokenWindow,
    /// Acquired at startup, so formatting collection dates performs no paint I/O.
    pub(super) local_zone: Option<jiff::tz::TimeZone>,
    pub(super) excluded_counters: Vec<String>,
    pub(super) meter_hover: Option<Option<usize>>,
    /// The Agents column header under the pointer.
    pub(super) header_hover: Option<super::header_sort::Control>,
    pub(super) idle_alert_hover: Option<String>,
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
    pub search: tmt_tui::components::Editor,
    pub searching: bool,
    /// Tab whose empty search may retain the current read-only pane target.
    pub(super) search_preview_tab: Option<super::shell::OpsTab>,
    /// Index among visible rows (headers excluded).
    pub selected: usize,
    pub notice: Option<String>,
    pub(super) notice_clock: super::notice::Clock,
    pub(super) migration_notice: Option<String>,
    /// Why `tmt whoami` could not name this pane, when it failed outright.
    pub(super) caller_failure: Option<String>,
    pub(in crate::board) usage_chart: RefCell<super::overview::chart::host::State>,
    pub(in crate::board) usage_controls: bool,
    pub(super) quota_details: Option<RefCell<super::home::quota_detail::Details>>,
    pub help: bool,
    pub(super) help_state: RefCell<super::help::Help>,
    pub(in crate::board) overlay_focus: tmt_tui::app::FocusStack,
    pub menu: Option<Menu>,
    pub(super) theme_picker: Option<super::theme_picker::Picker>,
    pub(super) panels_picker: Option<super::panels_picker::Picker>,
    pub(super) panels_save_pending: bool,
    pub(super) settings: Option<super::settings::Overlay>,
    pub input: Option<Input>,
    pub(super) composer: super::composer::State,
    pub(super) mentions: super::mentions::State,
    pub(super) member_links: RefCell<super::member_links::State>,
    pub(super) references: super::inventory::Inventory,
    /// Lane jobs submitted whose events have not arrived.
    pub(super) in_flight: usize,
    /// The Notes summaries of the conversations shown, read on the lane.
    pub(in crate::board) notes: super::conversation_checklist::State,
    /// The running versions, read once on the lane; none until then or when unreadable.
    pub(in crate::board) versions: Option<super::versions::Versions>,
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
    /// Clock text belongs to painted surfaces and is invalidated with frame geometry.
    pub(in crate::board) frame_clocks: RefCell<Vec<super::view::ClockSurface>>,
    /// The last paint drew a row in conversation (its tint follows the selection's word policy).
    pub(in crate::board) conversation_tinted: std::cell::Cell<bool>,
    pub(super) peek_areas: super::peek::PeekAreas,
    /// Where a row's choice chip was last drawn, with its row.
    pub(super) chip_hits: RefCell<Vec<(ratatui::layout::Rect, usize)>>,
    /// Each record's first visual line in the last rows draw, for paging.
    pub row_starts: RefCell<Vec<usize>>,
    /// Where tabs were last drawn.
    pub tab_hits: RefCell<Vec<TabHit>>,
    body_width: u16,
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
    if key
        .modifiers
        .intersects(KeyModifiers::HYPER | KeyModifiers::META)
    {
        return None;
    }
    // Control letters retain their established case normalization (Ctrl-Shift-R
    // and Ctrl-R name the same authored action). Other modified events stay exact.
    if key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::CONTROL
        && let KeyCode::Char(character) = key.code
    {
        return Some(format!("ctrl-{}", character.to_ascii_lowercase()));
    }
    let modified = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    let name = match key.code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(character) if character.is_ascii_graphic() => {
            if modified {
                character.to_ascii_lowercase().to_string()
            } else {
                character.to_string()
            }
        }
        KeyCode::Enter => "enter".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::BackTab => "backtab".into(),
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => "backtab".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::F(number) => format!("f{number}"),
        _ => return None,
    };
    let mut prefixes = String::new();
    for (modifier, prefix) in [
        (KeyModifiers::CONTROL, "ctrl-"),
        (KeyModifiers::ALT, "alt-"),
        (KeyModifiers::SUPER, "super-"),
    ] {
        if key.modifiers.contains(modifier) {
            prefixes.push_str(prefix);
        }
    }
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && (modified || !matches!(key.code, KeyCode::Char(_) | KeyCode::Tab | KeyCode::BackTab))
    {
        prefixes.push_str("shift-");
    }
    Some(format!("{prefixes}{name}"))
}

/// The key the board names for a new line: Shift+Enter when the terminal
/// reports it distinctly (kitty enhancement or the tmux csi-u request), else
/// Alt+Enter, which every terminal sends. Every caption derives from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board) enum NewlineKey {
    Shift,
    Alt,
}

impl NewlineKey {
    /// The footer caption, in words.
    pub(in crate::board) fn hint(self) -> &'static str {
        match self {
            Self::Shift => "Shift+Enter new line",
            Self::Alt => "Alt+Enter new line",
        }
    }
    /// The key as the line under the input spells it.
    pub(in crate::board) fn glyph(self) -> &'static str {
        match self {
            Self::Shift => "Shift+⏎",
            Self::Alt => "Alt+⏎",
        }
    }
}

impl App {
    pub(in crate::board) fn newline_key(&self) -> NewlineKey {
        if self.keyboard_enhanced {
            NewlineKey::Shift
        } else {
            NewlineKey::Alt
        }
    }

    pub(in crate::board) fn newline_hint(&self) -> &'static str {
        self.newline_key().hint()
    }

    pub fn new(squad: Option<String>) -> Self {
        Self {
            selected_request: Default::default(),
            cron_glyphs: super::overview::cron_counts::GlyphSet::from_ascii(false),
            current: squad,
            local_zone: Some(jiff::tz::TimeZone::system()),
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
                .iter()
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

    /// Independent source rows: lead, authored section headers and admitted
    /// members. Interactive workspace rows are projected from the roster.
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
            // Keep lead/authored sections in place; only member order changes.
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
        self.usage_chart.get_mut().invalidate_data();
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
        super::rate::resume_model(view.home_rate.get(squad)?.input.resumes.get(id)?)
    }

    pub(in crate::board) fn home_meter(&self, squad: &str) -> Option<&super::meter::Meter> {
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

    pub(in crate::board) fn overview_member_usage(&self, id: &str) -> Option<super::rate::Reading> {
        if self.tok_range() != crate::config::TokRange::Hour {
            let super::usage_report::View::Report(report) =
                self.usage_report.view_for(self.tok_range())
            else {
                return None;
            };
            let seat = report.seat(id)?;
            return Some(super::rate::Reading {
                tokens: u128::from(seat.observed_total()?),
                partial: !seat.complete,
                span: seat.covered_ms,
            });
        }
        self.member_usage_observation(id, Instant::now())
            .map(|(_, reading)| reading)
    }

    pub(in crate::board) fn overview_member_trend<const N: usize>(
        &self,
        id: &str,
    ) -> [Option<f64>; N] {
        let now = Instant::now();
        self.member_usage_observation(id, now)
            .map_or([None; N], |(meter, _)| meter.member_trend(id, now))
    }

    fn member_usage_observation(
        &self,
        id: &str,
        now: Instant,
    ) -> Option<(&super::meter::Meter, super::rate::Reading)> {
        self.view
            .as_ref()?
            .home_rate
            .iter()
            .filter(|(_, rate)| rate.settings.enabled)
            .filter_map(|(squad, _)| {
                let meter = self.home_meter(squad)?;
                let (reading, evidence) =
                    meter.observation(id, crate::config::TokenWindow::HOUR, now)?;
                Some((meter, reading, evidence))
            })
            .max_by_key(|(_, reading, evidence)| (*evidence, reading.span, !reading.partial))
            .map(|(meter, reading, _)| (meter, reading))
    }

    /// The same HOUR readings and per-room UUID de-duplication supply both tables.
    pub(in crate::board) fn overview_squad_usage(
        &self,
        squad: &super::workspace::OpsSquad,
    ) -> Option<(u128, bool)> {
        let mut total = 0u128;
        let mut known = false;
        let mut partial = squad.observation != super::workspace::OpsObservation::Complete;
        let mut seen = std::collections::BTreeSet::new();
        for id in &squad.members {
            if !seen.insert(id) {
                continue;
            }
            if let Some(reading) = self.overview_member_usage(id) {
                known = true;
                total = total.saturating_add(reading.tokens);
                partial |= reading.partial;
            } else {
                partial = true;
            }
        }
        known.then_some((total, partial))
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
        let mut top = None;
        if let Some(total) = totals[2].filter(|total| total.tokens > 0) {
            let share = |tokens| UsageShare {
                fraction: tokens as f64 / total.tokens as f64,
                partial: total.partial,
            };
            let mut highest = 0;
            for member in &members {
                if let Some(HeaderObservation { reading, .. }) = member.readings[2]
                    && reading.tokens > highest
                {
                    highest = reading.tokens;
                    top = Some(UsageTop {
                        member: member.name,
                        share: share(reading.tokens),
                    });
                }
            }
        }
        Some(HomeHeaderUsage {
            windows,
            totals,
            top,
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
        self.overview_rows.borrow_mut().clear();
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
        if sampled {
            self.usage_chart.get_mut().invalidate_data();
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
        self.search_preview_tab = None;
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
        self.overview_rows.borrow_mut().clear();
        self.clamp();
    }

    /// Swaps in a loaded squad in one step. A result for a squad the user
    /// already left is kept for switching back, never shown.
    pub(super) fn apply_history(
        &mut self,
        read: super::refresh::HistoryRead,
        now: Instant,
    ) -> bool {
        if read.retained.as_ref().map(|(request, _)| *request) != self.retained_history_request()
            || read.quota.is_some() != self.quota_history_visible()
        {
            return false;
        }
        self.overview_rows.borrow_mut().clear();
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
            if let Some((_, seeds)) = &read.retained {
                meter.seed_retained(&current.input, seeds, now);
            }
            meter.seed_quota(&current.input, read.quota.as_ref(), now);
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
        self.digest_rows.get_mut().clear();
        self.overview_rows.borrow_mut().clear();
        self.conversation.borrow_mut().refresh_pane_output();
        self.observe_cursor(snapshot.cursor);
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
            self.set_notice(None, false);
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
                    self.token_window = home.window.available(home.windows);
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
                if let Some(home) = &view.home {
                    self.unread.writer.observed(&home.workspace.read_markers);
                }
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
                self.discovery_refreshed();
                self.member_session_changed();
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
        self.reconcile_notes_scope();
        self.project_usage(Instant::now());
        self.prune_views();
        if self
            .settings
            .as_ref()
            .is_some_and(|overlay| !overlay.from_menu && overlay.settings.context != self.shown)
        {
            self.settings = None;
            self.set_notice(Some("Shown tab changed; reopen settings.".into()), false);
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
        self.header_hover = None;
        self.idle_alert_hover = None;
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

    /// Project one selected occurrence from held public observations, never from a first membership.
    pub(super) fn parameter_observations(&self, row: &Value) -> Value {
        self.parameter_observations_for(row, self.workspace.selected.as_ref())
    }

    fn parameter_observations_for(
        &self,
        row: &Value,
        target: Option<&super::workspace::OpsTarget>,
    ) -> Value {
        use super::overview::cron_counts::CronCountScope;
        use super::workspace::OpsTarget;
        let mut ops = row["id"]
            .as_str()
            .and_then(|id| self.references.agents.get(id))
            .map(|agent| agent.parameters.clone())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let room = match target {
            Some(OpsTarget::Squad(room)) => Some(room.as_str()),
            Some(OpsTarget::Member { room_id, .. }) => room_id.as_deref(),
            _ => None,
        };
        if let Some(room) = room {
            ops["squad"] = json!({"roomId":room});
            if let Some(squad) = self.workspace_snapshot().and_then(|snapshot| {
                snapshot
                    .roster
                    .squads
                    .iter()
                    .find(|squad| squad.room_id == room)
            }) {
                ops["squad"]["name"] = json!(squad.name);
            }
        }
        let scope = match target {
            Some(OpsTarget::Squad(room)) => Some(CronCountScope::Room(room)),
            Some(OpsTarget::Member { .. }) => row["id"]
                .as_str()
                .map(|id| CronCountScope::Owner { id, room }),
            Some(OpsTarget::Pool | OpsTarget::DigestAll) => Some(CronCountScope::All),
            _ => None,
        };
        if let Some(count) = scope.and_then(|scope| self.cron.counts.count(scope)) {
            ops["timers"] = json!({"active":count.active, "total":count.total});
        }
        ops
    }

    pub(super) fn host_action_selection(&self) -> super::settings::host_action::Selection {
        let recipient = self
            .workspace_message_target()
            .and_then(|target| self.workspace_snapshot()?.rows.get(&target).cloned());
        let ops = self.parameter_observations(
            recipient
                .as_ref()
                .or_else(|| self.selected_row())
                .unwrap_or(&Value::Null),
        );
        super::settings::host_action::Selection {
            row: self.selected_row().cloned(),
            recipient,
            factory_bindings: self.workspace_factory_bindings(),
            squad: self
                .workspace
                .selected
                .as_ref()
                .and_then(|target| self.target_squad(&RowTarget::Workspace(target.clone())))
                .filter(|name| !name.is_empty()),
            scope: self.workspace_scope(),
            invoker_tmux: self.invoker_tmux,
            ops,
        }
    }

    pub(super) fn open_host_action(
        &mut self,
        config: Config,
        selection: super::settings::host_action::Selection,
        board_pane: Option<String>,
    ) -> Result<(), crate::core::SquadError> {
        let mut overlay =
            super::settings::Overlay::open_host_action(config, selection, board_pane)?;
        overlay.settings.context = self.shown.clone();
        overlay.opening_focus = self.focus;
        overlay.squad_keys = self.squad_keys.clone();
        self.settings = Some(overlay);
        self.help = false;
        Ok(())
    }

    pub(super) fn open_settings(&mut self, config: Config) -> Result<(), crate::core::SquadError> {
        self.open_settings_in(config, None)
    }

    /// Opens the overlay for `squad` when given (from the Workspace settings menu), else for the shown tab.
    pub(super) fn open_settings_in(
        &mut self,
        config: Config,
        squad: Option<&str>,
    ) -> Result<(), crate::core::SquadError> {
        let window = self
            .meter
            .as_ref()
            .filter(|meter| meter.settings.enabled)
            .map(|_| self.token_window.label().to_owned());
        let mut overlay = super::settings::Overlay::open(
            config,
            squad.or(self.shown_tab()),
            self.selected_section(),
            window,
        )?;
        overlay.from_menu = squad.is_some();
        if squad.is_some() {
            overlay.reveal_idle.set(true);
            overlay.surface.borrow_mut().select("idle_alert.enabled");
        }
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

    /// Reader focus alone may come from Tab. The existing UUID draft owner
    /// distinguishes an explicit message reveal without another open flag.
    fn conversation_revealed(&self) -> bool {
        self.focused() == Pane::Replies
            && self
                .input
                .as_ref()
                .is_some_and(|input| input.draft_key.is_some())
    }

    pub fn collapsed_panes(&self) -> BTreeSet<Pane> {
        if self.workspace_snapshot().is_none() {
            return [Pane::Replies].into();
        }
        let mut collapsed = BTreeSet::new();
        match self.shell.tab {
            super::shell::OpsTab::Agents => {
                let policy = self.overview_panels();
                if !policy.shown(crate::config::OverviewPanel::Monitor) {
                    collapsed.insert(Pane::Rows);
                }
                if !policy.side_shown() && !self.conversation_revealed() {
                    collapsed.insert(Pane::Replies);
                }
            }
            super::shell::OpsTab::Digest => {
                let policy = self.digest_panels();
                if !policy.shown(crate::config::DigestPanel::List) {
                    collapsed.insert(Pane::Rows);
                }
                // Exchanges remain accessible when Details are hidden.
            }
        }
        collapsed
    }

    pub fn set_body_width(&mut self, width: u16) {
        self.body_width = width;
        self.restore_focus();
    }

    fn focus_panes(&self) -> &[Pane] {
        &[Pane::Rows, Pane::Replies]
    }

    pub(in crate::board) fn workspace_placement(
        &self,
        area: ratatui::layout::Rect,
    ) -> Option<super::conversation::layout::Placement> {
        use super::conversation::layout::{self, Preference};
        let snapshot = self.workspace_snapshot()?;
        let mut placement = if self.shell.tab == super::shell::OpsTab::Agents {
            // The Usage band is the top of the tab, as on Overview; the columns sit below it.
            let usage = self.overview_usage_band(area);
            let below = ratatui::layout::Rect::new(
                area.x,
                area.y + usage.height,
                area.width,
                area.height.saturating_sub(usage.height),
            );
            let mut placement = layout::agents_tree(
                below,
                match snapshot.preferences.layout {
                    crate::config::OpsLayout::Auto => Preference::Auto,
                    crate::config::OpsLayout::Side => Preference::Side,
                    crate::config::OpsLayout::Stack => Preference::Stack,
                },
                self.focused() == Pane::Replies,
            );
            // The Panels list and side switches claim the whole body for the other pane.
            let policy = self.overview_panels();
            let whole = match (placement.list, placement.conversation) {
                (Some(list), Some(conversation)) => list.union(conversation),
                (Some(only), None) | (None, Some(only)) => only,
                (None, None) => below,
            };
            if !policy.shown(crate::config::OverviewPanel::Monitor) {
                placement = layout::Placement {
                    list: None,
                    conversation: Some(whole),
                };
            } else if !policy.side_shown() && !self.conversation_revealed() {
                placement = layout::Placement {
                    list: Some(whole),
                    conversation: None,
                };
            }
            placement
        } else {
            self.digest_placement(area)
        };
        use crate::action::ResizeBoundary as Boundary;
        if self.shell.tab == super::shell::OpsTab::Agents {
            placement = self.pane_resize.panes(
                placement,
                Boundary::AgentsColumns,
                Boundary::AgentsRows,
                self.agents_list_min_columns(),
            );
        }
        Some(placement)
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

    /// A strip control keeps the focus with the pane that paints it: Talk and Measure sit in the
    /// conversation pane's title rule, every other strip belongs to the list.
    fn focus_strip(&mut self, column: u16, row: u16) {
        let pane = match self.pointer_pane(column, row) {
            Some(Pane::Replies) => Pane::Replies,
            _ => Pane::Rows,
        };
        self.focus = self
            .focus_panes()
            .iter()
            .position(|p| *p == pane)
            .unwrap_or(0);
    }

    /// A hidden focus returns to rows; boards without visible rows use the next pane.
    fn restore_focus(&mut self) {
        if self.workspace_snapshot().is_some() {
            self.focus = self.focus.min(self.focus_panes().len().saturating_sub(1));
            if self.collapsed_panes().contains(&self.focused()) {
                self.next_pane();
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
            && self.focus != next
        {
            self.focus = next;
            self.invalidate_frame_geometry();
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

    pub(in crate::board) fn set_notice(&mut self, notice: Option<String>, failed: bool) {
        self.notice = notice.map(|text| {
            super::notice::names(&text, |id| {
                self.references
                    .agents
                    .get(id)
                    .map(|agent| agent.name.clone())
                    .or_else(|| {
                        self.workspace_snapshot()?
                            .roster
                            .members
                            .get(id)
                            .map(|member| member.name.clone())
                    })
                    .or_else(|| {
                        self.workspace_snapshot()?
                            .roster
                            .squads
                            .iter()
                            .find(|squad| squad.room_id == id)
                            .map(|squad| squad.name.clone())
                    })
            })
        });
        if self.notice.is_some() {
            self.notice_clock.start(Instant::now(), failed);
        } else {
            self.notice_clock.clear();
        }
    }
    /// The notice for a job still running: held back until the spinner delay, so a job
    /// that finishes sooner paints nothing. A result or failure uses `set_notice`.
    pub(in crate::board) fn set_progress(&mut self, text: String) {
        self.set_notice(Some(text), false);
        let delay = super::view::SPINNER_DELAY.as_millis() as u64;
        self.notice_clock
            .hold_until(crate::status::now_ms() + delay);
    }

    /// The notice to paint now; a held progress notice asks the frame loop to wake
    /// at the instant it becomes visible.
    pub(in crate::board) fn shown_notice(&self) -> Option<&str> {
        let text = self.notice.as_deref()?;
        match self.notice_clock.hidden_until(crate::status::now_ms()) {
            Some(until_ms) => {
                super::view::ClockSurface::admit_delay_gate(self, until_ms);
                None
            }
            None => Some(text),
        }
    }

    pub(in crate::board) fn expire_notice(&mut self, now: Instant) -> bool {
        if self.notice_clock.expired(now) {
            self.set_notice(None, false);
            true
        } else {
            false
        }
    }

    pub(super) fn say(&mut self, notice: impl Into<String>) -> Effect {
        self.set_notice(Some(notice.into()), false);
        Effect::None
    }

    /// Resolves an action against the selected row. Missing values refuse
    /// the action with a notice; nothing runs half-filled. While a switch
    /// loads, the rows on screen are another squad's, so nothing acts on them.
    pub fn perform(&mut self, action: &Action) -> Effect {
        self.perform_context(action, None)
    }
    pub(super) fn perform_member_host(
        &mut self,
        action: &Action,
        target: &super::workspace::OpsTarget,
    ) -> Effect {
        if !matches!(action.verb, Verb::Jump | Verb::Run)
            || !matches!(target, super::workspace::OpsTarget::Member { .. })
            || !self.workspace_snapshot().is_some_and(|snapshot| {
                snapshot.roster.complete() && snapshot.roster.contains(target)
            })
        {
            return self.say("The mentioned member is unavailable for this host action.");
        }
        self.perform_context(action, Some(target))
    }
    pub(super) fn perform_context(
        &mut self,
        action: &Action,
        member: Option<&super::workspace::OpsTarget>,
    ) -> Effect {
        if self.panels_save_pending {
            return self.say("Wait for the panel save to finish.");
        }
        if action.verb == Verb::Ui(crate::action::OpsUiAction::UsageControls)
            && self.usage_panel_hidden()
        {
            return self.say("Usage chart is hidden by the panel policy.");
        }
        if let Err(error) = crate::config::admit_workspace_action(action, "action") {
            self.set_notice(Some(error.message), true);
            return Effect::None;
        }
        if action.verb == Verb::Ui(crate::action::OpsUiAction::ResizePane) {
            return self.resize_action(action);
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
        // The Digest tab's `all` row is everybody: its digest is the one global switch.
        if action.verb == Verb::Digest
            && member.is_none()
            && self.workspace.selected == Some(super::workspace::OpsTarget::DigestAll)
        {
            return self.digest_switch();
        }
        // UI actions dispatch before resolving a row. A member action uses the
        // same verified occurrence as paint, including an observed squad lead.
        let parameter_target = member.or(self.workspace.selected.as_ref()).cloned();
        let target = member.cloned().or_else(|| self.workspace_message_target());
        let member = target.as_ref();
        if self.workspace_snapshot().is_some()
            && action.verb.acts_on_member()
            && member.is_none()
            && self.workspace_selected_member().is_none()
        {
            return self.say("Select a visible named member before running this action.");
        }
        match action.verb {
            Verb::TokenWindow => {
                return self.cycle_token_window();
            }
            Verb::HomeMessage => return self.toggle_row_detail(),
            Verb::ViewReply => {
                let effect = self.view_row_reply();
                return if self.row_details.reader.is_none() {
                    self.say("No response is available for the selected agent yet.")
                } else {
                    effect
                };
            }
            Verb::AskLead => return self.ask_lead(),
            Verb::Digest => return self.digest_row(self.selected),
            Verb::Settings => {
                return if self.workspace_snapshot().is_some() {
                    self.workspace_settings()
                } else {
                    Effect::Settings
                };
            }
            Verb::Theme => return if self.panels_picker.is_some() {
                self.say("Close Panels before opening Theme.")
            } else { Effect::PickTheme },
            Verb::NextPane => {
                self.next_pane();
                return Effect::None;
            }
            Verb::Refresh => {
                self.set_notice(Some("Refreshing…".into()), false);
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
        let row = member
            .map_or_else(
                || self.selected_row(),
                |target| self.target_row(&RowTarget::Workspace(target.clone())),
            )
            .cloned();
        let Some(row) = row else {
            return self.say("No row is selected.");
        };
        if action.verb == Verb::Jump && self.workspace_snapshot().is_some() {
            if !self.invoker_tmux {
                self.conversation_return = Some(self.shell.tab);
                return self.workspace_action(crate::action::OpsUiAction::Inspect);
            }
            if let Err(reason) = super::overview::actions::host(&row, self.invoker_tmux) {
                return self.say(reason);
            }
        }
        let ops = self.parameter_observations_for(&row, parameter_target.as_ref());
        let parameters = crate::template::Parameters {
            row: &row,
            ops: Some(&ops),
            board_pane: None,
        };
        let view = self.view.as_ref().expect("a selected row has a view");
        let request = match action.verb {
            Verb::Talk | Verb::Annotate | Verb::Reply | Verb::UpdateStatus => {
                return self.compose(action, &row);
            }
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
            Verb::Swap => match (
                row["id"].as_str(),
                row["name"].as_str(),
                action.args.first().and_then(|arg| arg.literal()),
            ) {
                (Some(target), Some(name), Some(direction)) => Ok(Request::Swap {
                    target: target.into(),
                    name: super::notes::sanitize(name),
                    direction: direction.into(),
                }),
                _ => Err("Swap needs a visible agent and a direction.".into()),
            },
            Verb::Open => {
                effects::resolve_open_parameters(action, parameters).map(|link| Request::Open {
                    link,
                    opener: view.opener.clone(),
                })
            }
            Verb::Copy => action.args[0]
                .fill_parameters(parameters)
                .map(|text| Request::Copy {
                    text,
                    program: view.clipboard.clone(),
                    feedback: CopyFeedback::Destination,
                }),
            Verb::Run if action.needs_board_pane() => Ok(Request::PreparedRun {
                action: action.clone(),
                row,
                ops,
            }),
            Verb::Run => action.argv_parameters(parameters).map(Request::Run),
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
                    && !matches!(action.verb, Verb::Menu | Verb::NextPane | Verb::TokenWindow | Verb::Ui(crate::action::OpsUiAction::ResizePane))
                    && (row.is_some() || !action.verb.acts_on_member())
                    && (action.verb != Verb::ViewReply || self.reply_hint())
                    // Without a digest source there is nothing for this entry to do.
                    && (action.verb != Verb::Digest
                        || row.as_ref().is_some_and(|row| self.digest_choice(row).is_some()))
            })
            .map(|(key, action)| MenuEntry {
                key,
                label: match action.verb {
                    Verb::Ui(ui) => ui.menu_label().unwrap_or(&action.text).to_owned(),
                    _ => action.text.clone(),
                },
                choice: Choice::Action(action),
            })
            .collect();
        if self.shell.tab == super::shell::OpsTab::Agents
            && self.workspace_scope() == crate::config::OpsKeyScope::Member
            && !self.loading()
            && self.workspace_selected_member().is_some()
        {
            for ui in crate::action::OpsUiAction::menu_actions() {
                if !entries.iter().any(|entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::Ui(ui))) {
                    entries.push(MenuEntry {
                        key: String::new(),
                        label: ui.menu_label().expect("catalog menu action").into(),
                        choice: Choice::Action(Action::parse(ui.spec().name).expect("catalog action")),
                    });
                }
            }
        }
        if self.selected_row().is_some_and(|row| row["id"].is_string())
            && self.view.as_ref().and_then(|view| view.me_id.as_ref()).is_some()
            && !entries.iter().any(|entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::UpdateStatus))
            && self.workspace_message_target().is_none_or(|target| !matches!(target, super::workspace::OpsTarget::Member { room_id: None, .. }))
        {
            entries.push(MenuEntry {
                key: String::new(), label: "Update status".into(),
                choice: Choice::Action(Action::parse("update-status").expect("status action")),
            });
        }
        // Mark read has no default key: the menu is where an unbound action is found.
        if let Some(super::workspace::OpsTarget::Member { identity_id, .. }) =
            self.workspace_selected_member()
            && self.unread_count(identity_id).total > 0
            && !entries.iter().any(|entry| {
                matches!(&entry.choice, Choice::Action(action)
                    if action.verb == Verb::Ui(crate::action::OpsUiAction::MarkRead))
            })
        {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Mark read".into(),
                choice: Choice::Action(Action::parse("mark-read").expect("mark-read action")),
            });
        }
        entries.sort_by_key(|entry| match &entry.choice {
            Choice::Action(action) => action.order(),
            _ => u8::MAX,
        });
        if let Some(draft) = self.workspace_form.as_ref().filter(|draft| !draft.shown) {
            entries.push(MenuEntry {
                key: String::new(),
                label: if draft.stage == super::workspace::manage::Stage::Pending {
                    format!("Pending {}", draft.kind.label())
                } else {
                    format!("{} result", draft.kind.label())
                },
                choice: Choice::WorkspaceBegin(draft.kind),
            });
        }
        if self.quota_details_available()
            && !entries.iter().any(|entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::Ui(crate::action::OpsUiAction::QuotaDetails)))
        {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Quota details".into(),
                choice: Choice::Action(Action::parse("quota-details").expect("quota action")),
            });
        }
        // The estimate answers a click and an authored key; the menu is the
        // discoverable path for everyone else, only beside a painted figure.
        if self.savings_visible()
            && !entries.iter().any(|entry| {
                matches!(&entry.choice, Choice::Action(action)
                    if action.verb == Verb::Ui(crate::action::OpsUiAction::SavingsUnit))
            })
        {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Savings unit".into(),
                choice: Choice::Action(Action::parse("savings-unit").expect("savings unit action")),
            });
        }
        if self.usage_chart_available()
            && !entries.iter().any(|entry| {
                matches!(&entry.choice, Choice::Action(action)
                    if action.verb == Verb::Ui(crate::action::OpsUiAction::UsageControls))
            })
        {
            entries.push(MenuEntry {
                key: String::new(),
                label: "Usage chart".into(),
                choice: Choice::Action(
                    Action::parse("usage-controls").expect("usage controls action"),
                ),
            });
        }
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
            anchor: None,
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

            inline_quotes: Default::default(),

            row_send: None,
            link: None,
            prompt,
            text: Default::default(),
            compose,
            squad,
            hint: None,
        });
        if matches!(
            self.input.as_ref().unwrap().compose,
            Compose::AskLead { .. }
        ) {
            self.bind_draft();
            self.restore_draft();
        }
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
            if index == self.selected {
                return self.workspace_message_target().map(RowTarget::Workspace);
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

    /// Member messages resolve open requests only when the draft is submitted.
    /// Notes are an explicit, separate action; they never become a message mode.
    pub(super) fn compose_row(&mut self, send: RowSend, verb: Verb, squad: String) -> Effect {
        if squad.is_empty() && matches!(verb, Verb::Annotate | Verb::UpdateStatus) {
            return self
                .say("Notes require a squad context; use Talk for this Common pool identity.");
        }
        if verb == Verb::UpdateStatus
            && self
                .view
                .as_ref()
                .and_then(|view| view.me_id.as_ref())
                .is_none()
        {
            return self
                .say("Status updates require an observed sender identity; nothing changed.");
        }
        self.status_draft = None;
        let compose = if verb == Verb::UpdateStatus {
            Compose::Status
        } else if verb == Verb::Annotate {
            let Some(note) = send.note.clone() else {
                return self.say("This squad has no observed lead to receive the note.");
            };
            note
        } else {
            Compose::Talk {
                to: send.name.clone(),
            }
        };
        if self.shell.tab == super::shell::OpsTab::Digest
            && matches!(verb, Verb::Talk | Verb::Reply)
        {
            // The admitted UUID remains the recipient; the existing conversation
            // and draft owners move together to their Agents host.
            self.shell
                .select(super::shell::OpsTab::Agents, &mut self.workspace.selected);
            self.right_tab = super::shell::RightTab::Talk;
            self.workspace.selected = match &send.target {
                RowTarget::Workspace(target) => Some(target.clone()),
                _ => self.workspace.selected.clone(),
            };
            if let RowTarget::Workspace(super::workspace::OpsTarget::Member { room_id, .. }) =
                &send.target
            {
                if let Some(room) = room_id {
                    self.workspace.expanded.insert(room.clone());
                } else {
                    self.workspace.pool_expanded = true;
                }
            }
            self.workspace_reconcile();
            self.focus = 1;
        }
        let effect = self.ask(String::new(), compose, squad.clone());
        self.attach_row(send, squad);
        if verb == Verb::UpdateStatus {
            self.start_status();
        }
        effect
    }

    /// What to tell a user whose board has no sender. A pane the host could not
    /// name is not an unbound pane: say why, in core's own words, because the
    /// next step depends on the host.
    pub(super) fn unknown_sender(&self) -> String {
        match &self.caller_failure {
            Some(why) => format!(
                "Could not identify this pane: {why} Fix that, or record yourself with tmt ops squad me <name>."
            ),
            None => "Who is sending? Record yourself with tmt ops squad me <name>, or open the board from your named pane.".into(),
        }
    }

    fn compose(&mut self, action: &Action, row: &Value) -> Effect {
        if self
            .view
            .as_ref()
            .and_then(|view| view.me.as_ref())
            .is_none()
        {
            return self.say(self.unknown_sender());
        }
        // Workspace selection already owns the exact membership. Do not infer
        // its target from a filtered flat row index or equal row payload.
        let target = if self.workspace_snapshot().is_some() {
            self.workspace_message_target().map(RowTarget::Workspace)
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
            Choice::MemberLink { target, jump } => self.member_link_action(&target, jump),
            Choice::MemberLinkAction { target, action } => {
                self.member_link_command(&target, &action)
            }
            Choice::IdleAlert(room) => self.open_idle_alert(Some(room)),
            Choice::Unavailable(reason) => self.say(reason),
            Choice::AddToNotes(seed) => self.add_to_notes(seed),
            Choice::LinkOpen(url) => self.link_open(url),
            Choice::LinkCopy(url) => self.link_copy(url),
            Choice::LinkShow { url, anchor } => self.link_show(&url, anchor),
            Choice::FileCopy(path) => self.link_copy(path.to_string()),
            Choice::FileShow { path, anchor } => self.file_show(&path, anchor),
            Choice::AgentsFilter(filter) => self.agents_filter(filter),
            Choice::RightTab(tab) => self.select_right_tab(tab),
            Choice::WorkspaceTarget(target) => {
                if self
                    .workspace_snapshot()
                    .is_some_and(|snapshot| snapshot.roster.contains(&target))
                {
                    if let super::workspace::OpsTarget::Member {
                        room_id: Some(room),
                        ..
                    } = &target
                    {
                        self.workspace.expanded.insert(room.clone());
                    } else if matches!(
                        target,
                        super::workspace::OpsTarget::Member { room_id: None, .. }
                    ) {
                        self.workspace.pool_expanded = true;
                    }
                    self.workspace.selected = Some(target);
                    self.workspace_reconcile();
                    self.invalidate_frame_geometry();
                }
                Effect::None
            }
            Choice::WorkspaceBegin(kind) => self.workspace_begin(kind),
            Choice::WorkspaceRoom {
                kind,
                identity_id,
                room_id,
            } => self.workspace_choose_room(kind, &identity_id, &room_id),
            Choice::HostAction => Effect::HostAction,
            Choice::LaunchTemplates => self.open_launch_templates(),
            Choice::SquadSettings(squad) => Effect::SquadSettings(squad),
            Choice::Rule | Choice::Heading => Effect::None,
            Choice::WorkspaceLayout(layout) => self.workspace_layout(layout),
            Choice::DigestWindow(window) => self.select_digest_window(window),
            Choice::TokRange(range) => self.select_tok_range(range),
            Choice::SortBy(header) => self.sort_by_header(header),
            Choice::ExchangeCell { sender, recipient } => {
                self.open_exchange_pair(&sender, &recipient)
            }
            Choice::SavingsUnit => self.cycle_savings_unit(),
            Choice::TokMoney | Choice::TokUnit => self.toggle_tok_money(),
            Choice::ClearSearch => {
                super::search_field::clear(&mut self.search, &mut self.searching);
                self.clamp();
                self.invalidate_frame_geometry();
                Effect::None
            }
            Choice::Panels => self.open_panels(),
            Choice::Checklist => self.open_checklist(),
            Choice::RoomChecklist(room) => self.open_room_checklist(room),
            Choice::MemberNotes => self.open_notes(),
            Choice::Action(action) => self.perform(&action),
            Choice::Cron(request) => Effect::Act(Request::Cron(request)),
            Choice::Dismiss => Effect::None,
            Choice::Digest(set) => Effect::Act(Request::Digest(set)),
            Choice::DigestCustom(granted) => self.digest_custom(*granted),
            Choice::DigestSwitch => self.digest_switch(),
        }
    }

    /// Attach the selected member occurrence to the shared UUID draft owner.
    pub(super) fn attach_row(&mut self, send: RowSend, squad: String) {
        if let Some(input) = &mut self.input {
            input.squad = squad;
            input.row_send = Some(send);
            input.prompt = input.header();
        }
        self.bind_draft();
        self.restore_draft();
        self.workspace_focus_conversation();
        self.follow = true;
    }

    /// Cycle the acquired workspace window or the independent sampled source.
    fn cycle_token_window(&mut self) -> Effect {
        if self.next_token_window().is_some() {
            Effect::CycleTokenWindow
        } else {
            Effect::None
        }
    }
    pub(super) fn next_token_window(&self) -> Option<crate::config::TokenWindow> {
        let windows = self
            .view
            .as_ref()
            .and_then(|view| view.home.as_ref())
            .map(|home| home.windows)
            .or_else(|| self.meter.as_ref().map(|meter| meter.settings.windows))?;
        Some(self.token_window.next(windows))
    }
    pub(super) fn apply_token_window(&mut self, config: &Config) -> String {
        let shared = self.view.as_ref().is_some_and(|view| view.home.is_some());
        let key = if shared {
            String::new()
        } else {
            self.current.clone().unwrap_or_default()
        };
        let window = config
            .token_rate(&key)
            .expect("validated window write")
            .window;
        self.token_window = window;
        if let Some(home) = self.view.as_mut().and_then(|view| view.home.as_mut()) {
            home.window = window.available(home.windows);
            self.token_window = home.window;
        }
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
        self.composer.paste_preview.get_mut().take();
        self.composer.invalidate();
        if self.checklist_shown()
            || self.recovery_shown()
            || self.discovery_shown()
            || self.new_member.is_some()
            || self.idle_alert_editor.is_some()
            || self.member_session.is_some()
            || self.notes_overlay.is_some()
        {
            return self
                .overlay_event(&Event::Paste(text.to_owned()))
                .unwrap_or(Effect::None);
        }
        if self.searching && self.input.is_none() && !self.modal_open() {
            if !super::search_field::paste(&mut self.search, text) {
                return self.say("Search exceeds 4,096 bytes; query unchanged.");
            }
            self.clamp();
            self.invalidate_frame_geometry();
            return Effect::None;
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
        if let Some(effect) = self.drop_files(text) {
            return effect;
        }
        let message = self.message_field();
        let limit = if message { MESSAGE_LIMIT } else { INPUT_LIMIT };
        if let Some(input) = self.input.as_mut()
            && !matches!(input.compose, Compose::Status)
        {
            let label = if message {
                super::composer::paste::label(text).map(|label| {
                    if input.draft_key.is_some() {
                        format!("{label} ×")
                    } else {
                        label
                    }
                })
            } else {
                None
            };
            let inserted = match label {
                Some(label) => input.text.insert_folded(text, label, limit),
                None => input.text.insert(text, limit),
            };
            if !inserted {
                return self.say(format!("Paste exceeds {limit} bytes; draft unchanged."));
            }
        }
        self.update_mentions();
        Effect::None
    }

    pub(in crate::board) fn input_key(&mut self, key: KeyEvent) -> Effect {
        // Enhanced terminal reporting distinguishes a held Enter from a new
        // submit press. Opening a retained message never arms a repeat send.
        if key.code == KeyCode::Enter
            && key.modifiers.is_empty()
            && key.kind != ratatui::crossterm::event::KeyEventKind::Press
            && self
                .input
                .as_ref()
                .is_some_and(|input| input.draft_key.is_some())
        {
            return Effect::None;
        }
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
                let conversation = input.draft_key.clone();
                self.input = None;
                self.status_draft = None;
                if conversation.is_some() {
                    self.focus = 0;
                    self.invalidate_frame_geometry();
                }
                if cron {
                    self.cron_draft = None;
                    return self.say("Cancelled; nothing changed.");
                }
                if digest {
                    return self.say("Digest not changed.");
                }
                if conversation
                    .as_ref()
                    .is_some_and(|key| self.has_outgoing(key))
                {
                    self.set_notice(None, false);
                    return Effect::None;
                }
                return self.say("Nothing sent.");
            }
            KeyCode::Tab => return Effect::None,
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || (message && key.modifiers.contains(KeyModifiers::ALT)) =>
            {
                if message {
                    input.text.insert("\n", limit);
                    self.composer.invalidate();
                }
                return Effect::None;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if input.draft_key.is_some() {
                    return self.preview_paste_at_cursor();
                } else if message {
                    input.text.toggle_fold();
                }
                return Effect::None;
            }
            KeyCode::Char('j' | 'n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if message {
                    input.text.insert("\n", limit);
                    self.composer.invalidate();
                }
                return Effect::None;
            }
            KeyCode::Enter => {}
            _ => {
                let width = self.input_viewport.get().map_or(80, |(area, _)| area.width);
                input.text.key(key, width, limit);
                self.composer.invalidate();
                return Effect::None;
            }
        }
        if self.in_flight > 0 {
            return self
                .say("Still working on the previous request; press Enter again in a moment.");
        }
        if self.input.as_ref().is_some_and(|input| {
            input.exact_quotes().any(|quote| {
                input.target().is_none_or(|target| {
                    !self.response_quote_current(
                        &super::row_detail::Target::Row(target.clone()),
                        &quote,
                    )
                })
            })
        }) {
            return self.say("The response source changed; nothing sent. Draft retained.");
        }
        let persistent = self
            .input
            .as_ref()
            .is_some_and(|input| input.draft_key.is_some());
        let mut input = if persistent {
            self.input.as_ref().unwrap().clone()
        } else {
            self.input.take().expect("composing")
        };
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
        if input.text.as_str().trim().is_empty() {
            return self.say("Nothing sent.");
        }
        if matches!(input.compose, Compose::Talk { .. })
            && let Some(send) = input.row_send.as_ref()
        {
            let Some(row) = self.target_row(&send.target) else {
                return self
                    .say("This member is no longer available; nothing sent. Draft retained.");
            };
            let Some(name) = row["name"].as_str() else {
                return self
                    .say("This member is no longer available; nothing sent. Draft retained.");
            };
            if self
                .view
                .as_ref()
                .and_then(|view| view.me.as_ref())
                .is_none()
            {
                return self.say(self.unknown_sender());
            }
            input.compose = Compose::Talk {
                to: name.to_owned(),
            };
            input.squad = self.target_squad(&send.target).unwrap_or_default();
            input.row_send.as_mut().unwrap().name = name.to_owned();
        }
        let row = input.row_send.as_ref().map(|send| send.name.as_str()).or({
            if let Compose::Annotate { row, .. } = &input.compose {
                Some(row.as_str())
            } else {
                None
            }
        });
        let squad = if input.squad.is_empty() {
            "Common pool"
        } else {
            &input.squad
        };
        let starts_quote = input
            .text
            .folds()
            .iter()
            .any(|fold| fold.source.start == 0 && input.inline_quotes.contains_key(&fold.id));
        let text = crate::requests::message::inline_body(
            row.map(|row| (squad, row)),
            &input.quoted_message(),
            starts_quote,
        );
        if input
            .row_send
            .as_ref()
            .is_some_and(|send| !send.valid(self, &input))
        {
            return self.say("The note or status target changed; nothing sent. Draft retained.");
        }
        if let Compose::AskLead { to, sender } = &input.compose
            && (self.loading()
                || self.current.as_deref() != Some(&input.squad)
                || self.lead().as_ref() != Ok(to)
                || self.view.as_ref().and_then(|view| view.me.as_ref()) != Some(sender)
                || input.draft_key.as_ref() != self.draft_identity(&input).as_ref())
        {
            return self.say("The squad, sender or lead changed; nothing sent.");
        }
        if let Some(LinkSend { member, sender }) = &input.link {
            let row = self.link_member(member);
            let valid = self.view.as_ref().and_then(|view| view.me.as_ref()) == Some(sender)
                && !self.loading()
                && self.current.as_deref() == Some(&input.squad)
                && row.is_some_and(|_| match &input.compose {
                    Compose::AskLead { .. } => false,
                    Compose::Talk { to } => to == member,
                    Compose::Annotate { to, .. } => self.lead().as_ref() == Ok(to),
                    Compose::Cron | Compose::Workspace | Compose::Status | Compose::Digest(_) => {
                        false
                    }
                });
            if !valid {
                return self.say("The link target or request changed; nothing sent.");
            }
        }

        let chat = input
            .row_send
            .as_ref()
            .filter(|_| matches!(input.compose, Compose::Talk { .. }))
            .and_then(|send| {
                let recipient = match &send.target {
                    RowTarget::Workspace(super::workspace::OpsTarget::Member {
                        identity_id,
                        ..
                    }) => identity_id.clone(),
                    RowTarget::Member { id, .. } | RowTarget::Lead { id, .. } => id.clone(),
                    _ => return None,
                };
                Some((send.sender_id.clone()?, recipient, send.name.clone()))
            });
        let fallback = text;
        let text = if chat.is_some() {
            input.quoted_message()
        } else {
            fallback.clone()
        };
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
        self.begin_outgoing(&input, &text);
        self.sending_draft(&input);
        if persistent {
            self.input = Some(Input {
                text: Default::default(),

                inline_quotes: Default::default(),

                compose: input.compose.clone(),
                ..input.clone()
            });
        }
        let squad = input.squad;
        Effect::Act(match input.compose {
            Compose::Talk { .. } if chat.is_some() => {
                let (sender, recipient, name) = chat.unwrap();
                Request::Chat {
                    sender,
                    recipient,
                    name,
                    text,
                    fallback,
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
            Compose::Cron | Compose::Workspace => {
                unreachable!("form steps are submitted before this match")
            }
            Compose::Digest(_) => unreachable!("a digest value is submitted before this match"),
        })
    }

    pub(super) fn finished_status(&mut self, outcome: crate::membership::status_update::Outcome) {
        if let Some(draft) = &mut self.status_draft {
            draft.finished(outcome);
            let notice = draft.error.clone();
            self.set_notice(notice, true);
        }
    }
    /// Shows the observed outcome without a second row-feedback cache.
    pub fn finished(&mut self, outcome: Result<String, String>) {
        if self.panels_save_pending {
            self.panels_finished(outcome);
            return;
        }
        let failed = outcome.is_err();
        self.set_notice(Some(outcome.unwrap_or_else(|error| error)), failed);
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
                menu.step(step);
                Effect::None
            }
            _ => {
                if event.kind == MouseEventKind::Down(MouseButton::Left) && menu.anchor.is_some() {
                    self.menu = None;
                }
                Effect::None
            }
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

        if key.code == KeyCode::Esc
            && let Some((target, anchor)) = self.destination_full_menu()
        {
            return match target {
                super::member_links::Target::Url(url) => self.link_popover(&url, anchor),
                super::member_links::Target::File(path) => self.file_popover(&path, anchor),
                super::member_links::Target::Member(_) => Effect::None,
            };
        }
        let Some(menu) = &mut self.menu else {
            return Effect::None;
        };
        let direct_jump = menu.anchor.is_some() && key.code == KeyCode::Char('j');
        let chosen = match key.code {
            KeyCode::Char('j') if direct_jump => menu
                .entries
                .iter()
                .find(|entry| entry.key == "j")
                .map(|entry| entry.choice.clone()),
            KeyCode::Esc | KeyCode::Char('q') => None,
            KeyCode::Up | KeyCode::Char('k') => {
                menu.step(-1);
                return Effect::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                menu.step(1);
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
                Overlay::Panels => vec![vec!["panels-picker".into(), "choices".into()]],
                Overlay::Settings => vec![vec!["settings".into(), "content".into()]],
                Overlay::CronList => vec![vec!["cron-list".into(), "choices".into()]],
                Overlay::CronReview => ["cancel", "read", "apply"]
                    .map(|s| vec!["cron-review".into(), s.into()])
                    .into(),
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

    pub(super) fn button_owner_is_active(&self, owner: Option<Overlay>) -> bool {
        self.menu.is_none() && self.overlay() == owner && (owner.is_some() || !self.searching)
    }

    pub(in crate::board) fn usage_controls_active(&self) -> bool {
        self.overlay() == Some(Overlay::UsageControls) && self.menu.is_none()
    }

    pub(super) fn overlay(&self) -> Option<Overlay> {
        if self.paste_preview_shown() {
            Some(Overlay::PastePreview)
        } else if self.monitor_details.is_some() {
            Some(Overlay::MonitorDetails)
        } else if self.cron_review.is_some() {
            Some(Overlay::CronReview)
        } else if self
            .workspace_form
            .as_ref()
            .is_some_and(|draft| draft.shown)
        {
            Some(Overlay::Workspace)
        } else if self.recovery_shown() {
            Some(Overlay::Recovery)
        } else if self.row_details.reader.is_some() {
            Some(Overlay::ReplyReader)
        } else if self.checklist_shown() {
            Some(Overlay::Checklist)
        } else if self.usage_controls {
            Some(Overlay::UsageControls)
        } else if self.quota_details.is_some() {
            Some(Overlay::QuotaDetails)
        } else if self.idle_alert_editor.is_some() {
            Some(Overlay::IdleAlert)
        } else if self.new_member.is_some() {
            Some(Overlay::NewMember)
        } else if self.member_session.is_some() {
            Some(Overlay::MemberSession)
        } else if self.notes_overlay.is_some() {
            Some(Overlay::Notes)
        } else if self.discovery_shown() {
            Some(Overlay::Discovery)
        } else if self.help {
            Some(Overlay::Help)
        } else if self.settings.is_some() {
            Some(Overlay::Settings)
        } else if self.panels_picker.is_some() {
            Some(Overlay::Panels)
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
            Overlay::PastePreview => Some(self.paste_preview_event(event)),
            Overlay::Workspace => self.workspace_form_event(field, event),
            Overlay::Panels => self.panels_event(event),
            Overlay::Recovery => self.recovery_event(event),
            Overlay::ReplyReader => Some(self.reader_event(event)),
            Overlay::Checklist => self.checklist_event(event),
            Overlay::UsageControls => {
                if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')))
                {
                    self.usage_controls = false;
                    self.usage_chart.get_mut().release_focus();
                    self.invalidate_frame_geometry();
                    return Some(Effect::None);
                }
                Some(match event {
                    Event::Key(key) => {
                        super::overview::chart::host::key(self, *key).unwrap_or(Effect::None)
                    }
                    Event::Mouse(mouse) => {
                        super::overview::chart::host::mouse(self, *mouse).unwrap_or(Effect::None)
                    }
                    _ => Effect::None,
                })
            }
            Overlay::QuotaDetails => {
                if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')))
                {
                    self.quota_details = None;
                } else {
                    self.quota_details.as_ref()?.borrow_mut().input(event);
                }
                Some(Effect::None)
            }
            Overlay::MemberSession => self.member_session_event(event),
            Overlay::Notes => self.notes_event(event),
            Overlay::IdleAlert => self.idle_alert_event(event),
            Overlay::NewMember => self.new_member_event(event),
            Overlay::Discovery => self.discovery_event(event),
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
                            let from_menu = self
                                .settings
                                .take()
                                .is_some_and(|overlay| overlay.from_menu);
                            if from_menu {
                                // Back to the Workspace settings menu the overlay was opened from.
                                self.workspace_settings();
                            }
                            Effect::CancelSettings
                        }
                        super::settings::Input::Pick("actions") => {
                            self.settings_preview();
                            self.settings = None;
                            self.context_menu()
                        }
                        super::settings::Input::Pick("launch-templates") => {
                            self.settings_preview();
                            self.settings = None;
                            self.open_launch_templates()
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
            Overlay::CronReview => self.cron_review_event(field, event),
            Overlay::MonitorDetails => self.monitor_details_event(event),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Effect {
        self.buttons.clear_hover();
        self.meter_hover = None;
        self.header_hover = None;
        self.composer.unread_hover.set(false);
        self.idle_alert_hover = None;
        if key.kind == ratatui::crossterm::event::KeyEventKind::Release {
            return Effect::None;
        }
        if key.code == KeyCode::Esc && self.pane_resize.cancel_drag() {
            self.invalidate_frame_geometry();
            return Effect::None;
        }
        self.set_notice(None, false);
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Effect::Quit;
        }
        // Completion owns its first Escape even when its editor lives inside
        // a workspace overlay. Closing it must not cancel the enclosing form.
        let owner = self.workspace_input_owner();
        if key.code == KeyCode::Esc
            && owner == super::shell::OpsInputOwner::Completion
            && let Some(effect) = self.mention_key(key)
        {
            return effect;
        }
        let typing = matches!(
            owner,
            super::shell::OpsInputOwner::Composer
                | super::shell::OpsInputOwner::Search
                | super::shell::OpsInputOwner::Completion
        );
        // The painted Send now chip's key. It does nothing while no chip is painted. In the
        // message editor it is the chip's key, as no binding is active while typing; elsewhere an
        // authored binding on the same key wins, so it never swallows a chord others could read.
        if key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('s')
            && self.conversation.borrow().send_now_hit.is_some()
            && (owner == super::shell::OpsInputOwner::Composer
                || !typing && !self.bindings().contains_key("ctrl-s"))
        {
            return self.send_now_press();
        }
        if !typing
            && (self.modal_open() || self.usage_chart.borrow().focused())
            && let Some(action) = event_name(key).and_then(|name| self.bindings().remove(&name))
            && matches!(
                action.verb,
                Verb::Ui(
                    crate::action::OpsUiAction::Agents
                        | crate::action::OpsUiAction::DigestDashboard
                )
            )
        {
            let mut cleanup = Effect::None;
            // A destination dismisses an entire read-only overlay through its
            // existing back owner. The deepest current ladder has three levels;
            // a fourth back is a bound, never a submit/retry or an event replay.
            let back = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            for _ in 0..4 {
                if !self.modal_open() {
                    break;
                }
                let effect = if self.menu.is_some() && self.overlay().is_none() {
                    self.menu_key(back)
                } else {
                    self.overlay_event(&Event::Key(back))
                        .unwrap_or(Effect::None)
                };
                match effect {
                    Effect::None => {}
                    Effect::CancelSettings => cleanup = effect,
                    other => return other,
                }
            }
            if self.modal_open() {
                return self.say(
                    "This operation is still pending; inspect its outcome before switching tabs.",
                );
            }
            // A covered editor is not the receiving input owner. Once its
            // overlay is gone, close it through the existing draft-saving path.
            if self.input.is_some() {
                let effect = self.input_key(back);
                if effect != Effect::None {
                    return effect;
                }
            }
            self.searching = false;
            let effect = self.perform(&action);
            return if effect == Effect::None {
                cleanup
            } else {
                effect
            };
        }
        if !typing
            && key.code == KeyCode::Char('q')
            && key.modifiers.is_empty()
            && self.modal_open()
        {
            let back = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            return if self.menu.is_some() && self.overlay().is_none() {
                self.menu_key(back)
            } else {
                self.overlay_event(&Event::Key(back))
                    .unwrap_or(Effect::None)
            };
        }
        if let Some(effect) = self.overlay_event(&Event::Key(key)) {
            return effect;
        }
        self.set_notice(None, false);
        // Refresh keeps text inputs intact and uses the same override owner
        // as ordinary keys; rebinding ctrl-r does not force a refresh.
        if event_name(key).as_deref() == Some("ctrl-r")
            && let Some(action) = self.bindings().remove("ctrl-r")
            && action.verb == Verb::Refresh
        {
            return self.perform(&action);
        }
        if self.menu.is_some() && self.menu.as_ref().is_some_and(|menu| menu.anchor.is_some()) {
            return self.menu_key(key);
        }
        if self.input.is_none()
            && !self.modal_open()
            && !self.searching
            && let Some(effect) = self.member_link_key(key)
        {
            return effect;
        }
        // Alt letters are editing keys unless this exact event is authored.
        // Factory bindings contain no Alt events; an ordinary f/b/d never matches.
        if typing
            && key.modifiers.contains(KeyModifiers::ALT)
            && let Some(action) = event_name(key).and_then(|name| self.bindings().remove(&name))
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
        if let Some(input) = super::search_field::key(&mut self.search, &mut self.searching, key) {
            self.clamp();
            match input {
                super::search_field::Input::Previous => {
                    self.workspace_select(self.selected.saturating_sub(1))
                }
                super::search_field::Input::Next => {
                    self.workspace_select(self.selected.saturating_add(1))
                }
                super::search_field::Input::Changed => self.invalidate_frame_geometry(),
            }
            return Effect::None;
        }
        if let Some(effect) = super::overview::chart::host::key(self, key) {
            return effect;
        }
        if let Some(effect) = self.conversation_source_key(key) {
            return effect;
        }
        if key.code == KeyCode::Esc
            && key.kind != ratatui::crossterm::event::KeyEventKind::Release
            && self.conversation_source_escape()
        {
            return Effect::None;
        }
        self.workspace_key(key)
    }

    /// Refuse pointer and viewport authority from a superseded frame. Offsets
    /// and drafts survive; only a new paint can publish interactive geometry.
    pub(super) fn invalidate_frame_geometry(&self) {
        self.pane_resize.invalidate();
        self.buttons.invalidate();
        if let Some(picker) = &self.panels_picker {
            picker.invalidate();
        }
        if let Some(menu) = &self.menu
            && let Some(surface) = menu.surface.borrow_mut().as_mut()
        {
            surface.invalidate();
        }
        self.member_links.borrow_mut().invalidate();
        self.composer.invalidate();
        if self.usage_panel_hidden() && !self.usage_controls {
            self.usage_chart.borrow_mut().release_focus();
        }
        self.usage_chart.borrow_mut().invalidate_geometry();
        self.input_viewport.set(None);
        self.input_band.set(None);
        self.hits.borrow_mut().clear();
        self.frame_clocks.borrow_mut().clear();
        self.row_starts.borrow_mut().clear();
        self.tab_hits.borrow_mut().clear();
        self.workspace_action_hits.borrow_mut().clear();
        self.cron_scope_hits.borrow_mut().clear();
        self.workspace_strip_hits.borrow_mut().clear();
        self.workspace_state_cells.borrow_mut().clear();
        if let Some(details) = &self.monitor_details {
            details.invalidate();
        }
        if let Some(review) = &self.cron_review {
            review.invalidate();
        }
        self.chip_hits.borrow_mut().clear();
        if let Some(details) = &self.quota_details {
            details.borrow_mut().invalidate();
        }
        if let Some(draft) = &self.workspace_form {
            draft.invalidate();
        }
        self.conversation.borrow_mut().pane = None;
        self.conversation.borrow_mut().header_hits.clear();
        self.conversation.borrow_mut().invalidate_source_geometry();
        self.scrolls.begin_frame();
        self.mentions.invalidate();
        if let Some(reader) = &self.row_details.reader {
            reader.source_area.set(None);
        }
        if let Some(checklist) = &self.checklist {
            checklist.invalidate();
        }
        if let Some(recovery) = &self.recovery {
            recovery.invalidate();
        }
        if let Some(editor) = &self.idle_alert_editor {
            editor.invalidate();
        }
        if let Some(member) = &self.new_member {
            member.invalidate();
        }
        if let Some(session) = &self.member_session {
            session.invalidate();
        }
        if let Some(notes) = &self.notes_overlay {
            notes.invalidate();
        }
        if let Some(discovery) = &self.discovery {
            discovery.invalidate();
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
        let buttons_changed = if self.menu.is_none() {
            self.buttons
                .move_pointer((event.column, event.row).into(), self.overlay())
        } else {
            self.buttons.clear_hover();
            false
        };
        let resize_changed = self.pane_resize.move_pointer(
            (event.column, event.row).into(),
            !self.modal_open() && self.input.is_none() && !self.searching && !self.loading(),
        );
        let chart_changed = if self.usage_controls_active()
            || (!self.modal_open() && self.input.is_none() && !self.searching)
        {
            super::overview::chart::host::pointer(self, event)
        } else {
            false
        };
        let links_changed = self.member_links.borrow_mut().move_pointer(
            (event.column, event.row).into(),
            !self.modal_open() && !self.loading(),
        );
        let control =
            if self.loading() || self.modal_open() || self.input.is_some() || self.searching {
                None
            } else {
                self.workspace_strip_hits
                    .borrow()
                    .iter()
                    .find(|(area, _)| area.contains((event.column, event.row).into()))
                    .map(|(_, choice)| choice.clone())
            };
        let header = match &control {
            Some(Choice::SortBy(header)) => Some(super::header_sort::Control::Sort(*header)),
            Some(Choice::TokUnit) => Some(super::header_sort::Control::Unit),
            Some(Choice::TokRange(_)) => Some(super::header_sort::Control::Window),
            _ => None,
        };
        let idle_alert_hover = match control {
            Some(Choice::IdleAlert(room)) => Some(room),
            _ => None,
        };
        let unread = !self.loading()
            && self.overlay().is_none()
            && self.menu.is_none()
            && self.input.is_none()
            && !self.searching
            && self
                .composer
                .unread_hit
                .borrow()
                .as_ref()
                .is_some_and(|hit| hit.area.contains((event.column, event.row).into()));
        let unread_changed = unread != self.composer.unread_hover.replace(unread);
        let changed = chart_changed
            || resize_changed
            || buttons_changed
            || links_changed
            || unread_changed
            || next != self.meter_hover
            || header != self.header_hover
            || idle_alert_hover != self.idle_alert_hover;
        self.meter_hover = next;
        self.header_hover = header;
        self.idle_alert_hover = idle_alert_hover;
        changed
    }

    fn pointer_pane(&self, column: u16, row: u16) -> Option<Pane> {
        // Only the current paint can authorize its outer title/padding. The
        // same placement feeds paint; invalidation clears both visible owners.
        if (self.scrolls.visible(Pane::Rows) || self.conversation.borrow().pane.is_some())
            && let Some(placement) = self.workspace_placement(self.conversation.borrow().area)
        {
            for (pane, area) in [
                (Pane::Rows, placement.list),
                (Pane::Replies, placement.conversation),
            ] {
                if area.is_some_and(|area| area.contains((column, row).into())) {
                    return Some(pane);
                }
            }
        }
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
        self.conversation_source_outside_press(event);
        // Activation never transfers hover to a replacement modal or control.
        self.buttons.clear_hover();
        // A button/wheel event carries the current position too. Reconcile an
        // existing hover, without inventing hover on terminals lacking motion.
        if self.meter_hover.is_some()
            || self.header_hover.is_some()
            || self.composer.unread_hover.get()
        {
            self.move_pointer(event);
        }
        // A peek window closes on an outside press and the press goes on below.
        self.light_dismiss(event);
        if let Some(effect) = self.overlay_event(&Event::Mouse(event)) {
            return effect;
        }
        if self.menu.is_some() {
            return self.menu_mouse(event);
        }
        if self.resize_mouse(event, now) {
            if event.kind == MouseEventKind::Down(MouseButton::Left) {
                self.focus_at(event.column, event.row);
            }
            self.last_click = None;
            return Effect::None;
        }
        if let Some(effect) = self.conversation_menu_mouse(event) {
            return effect;
        }
        if let Some(effect) = self.paste_chip_mouse(event) {
            return effect;
        }
        if let Some(effect) = self.room_checklist_mouse(event) {
            return effect;
        }
        if let Some(effect) = self.member_link_mouse(event) {
            return effect;
        }
        if self.mention_mouse(event) {
            return Effect::None;
        }
        let unread = self
            .composer
            .unread_hit
            .borrow()
            .clone()
            .filter(|hit| hit.area.contains((event.column, event.row).into()));
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(hit) = unread
        {
            if self
                .composer_projection()
                .and_then(|projection| projection.key)
                .as_ref()
                != Some(&hit.key)
            {
                return self.say("The painted unread count changed; repaint before marking read.");
            }
            return if self.mark_read(&hit.key.1) {
                Effect::None
            } else {
                self.say("Nothing unread here.")
            };
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            let send = self
                .composer
                .send_hit
                .borrow()
                .clone()
                .filter(|hit| hit.area.contains((event.column, event.row).into()));
            if let Some(hit) = send {
                if self
                    .composer_projection()
                    .and_then(|projection| projection.key)
                    .as_ref()
                    != Some(&hit.key)
                {
                    return self
                        .say("The painted message recipient changed; repaint before sending.");
                }
                if self.input.is_none() {
                    self.focus_composer();
                }
                if self
                    .input
                    .as_ref()
                    .and_then(|input| input.draft_key.as_ref())
                    == Some(&hit.key)
                {
                    let effect = self.input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    self.update_mentions();
                    return effect;
                }
                return Effect::None;
            }
            let in_composer = self
                .input_band
                .get()
                .is_some_and(|area| area.contains((event.column, event.row).into()));
            if self.input.is_none() && in_composer && self.composer_projection().is_some() {
                let viewport = self.input_viewport.get();
                let effect = self.focus_composer();
                if let Some((area, skip)) = viewport
                    && area.contains((event.column, event.row).into())
                    && let Some(input) = self
                        .input
                        .as_mut()
                        .filter(|input| input.draft_key.is_some())
                {
                    input.text.place(
                        area.width,
                        skip + usize::from(event.row - area.y),
                        usize::from(event.column - area.x),
                        false,
                    );
                }
                return effect;
            }
            if !in_composer
                && self
                    .input
                    .as_ref()
                    .is_some_and(|input| input.draft_key.is_some())
            {
                self.remember_draft();
                self.input = None;
                self.mentions = Default::default();
                self.focus = 0;
            }
        }
        if let Some(effect) = self.conversation_source_mouse(event) {
            return effect;
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
        if !self.searching
            && let Some(effect) = super::overview::chart::host::mouse(self, event)
        {
            return effect;
        }
        if !self.loading() && event.kind == MouseEventKind::Down(MouseButton::Left) {
            let strip = self
                .workspace_strip_hits
                .borrow()
                .iter()
                .find(|(area, _)| area.contains((event.column, event.row).into()))
                .map(|(_, choice)| choice.clone());
            if let Some(choice) = strip {
                // Scope, range and savings controls work on every tab that paints them.
                let visibility = matches!(
                    choice,
                    Choice::SavingsUnit
                        | Choice::TokMoney
                        | Choice::TokUnit
                        | Choice::ClearSearch
                        | Choice::TokRange(_)
                ) || matches!(&choice, Choice::Action(action) if matches!(action.verb,
                    Verb::Ui(crate::action::OpsUiAction::ClearScope | crate::action::OpsUiAction::RevealHidden | crate::action::OpsUiAction::Search | crate::action::OpsUiAction::WorkspaceSnapshots)));
                if !self.searching && (self.shell.tab == super::shell::OpsTab::Agents || visibility)
                {
                    self.focus_strip(event.column, event.row);
                    return self.choose(choice);
                }
                return Effect::None;
            }
            let timer = self
                .cron_scope_hits
                .borrow()
                .iter()
                .find(|hit| hit.area.contains((event.column, event.row).into()))
                .cloned();
            if let Some(hit) = timer {
                if self
                    .workspace_snapshot()
                    .is_none_or(|snapshot| !snapshot.roster.contains(&hit.target))
                {
                    return self.say("The painted timer target is no longer observed.");
                }
                self.workspace.selected = Some(hit.target);
                self.workspace_reconcile();
                return self.workspace_owner_timers(hit.room, hit.owner);
            }

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
                self.set_notice(None, false);
                self.select(row);
                return self.digest_row(row);
            }
        }

        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && !self.loading()
            && self
                .conversation
                .borrow()
                .send_now_hit
                .as_ref()
                .is_some_and(|hit| hit.area.contains((event.column, event.row).into()))
        {
            return self.send_now_press();
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) && !self.loading() {
            let control = self
                .conversation
                .borrow()
                .header_hits
                .iter()
                .find(|hit| hit.area.contains((event.column, event.row).into()))
                .cloned();
            if let Some(hit) = control {
                if self.conversation_target().as_ref() == Some(&hit.target)
                    && self
                        .conversation
                        .borrow()
                        .pane
                        .is_some_and(|pane| hit.area.intersection(pane) == hit.area)
                {
                    return self.overview_details();
                }
                return Effect::None;
            }
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && !self.loading()
            && self
                .hits
                .borrow()
                .iter()
                .any(|hit| hit.contains(event.column, event.row) && hit.target == HitTarget::Quota)
        {
            return self.open_quota_details();
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && !self.loading()
            && self.hits.borrow().iter().any(|hit| {
                hit.contains(event.column, event.row) && hit.target == HitTarget::AllRoomTimers
            })
        {
            return self.workspace_owner_timers(None, None);
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
                0 => crate::action::OpsUiAction::Agents,
                1 => crate::action::OpsUiAction::DigestDashboard,
                _ => return Effect::None,
            };
            self.focus = 0;
            return self.workspace_action(action);
        }
        let right_click = event.kind == MouseEventKind::Down(MouseButton::Right);
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            && !(right_click
                && self.shell.tab == super::shell::OpsTab::Agents
                && !self.loading()
                && !self.modal_open()
                && !self.searching)
        {
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
        self.set_notice(None, false);
        let row = hit.row().unwrap();
        self.select(row);
        if right_click {
            self.last_click = None;
            return self.context_menu();
        }
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
        if self.screen_mode() {
            // The chat is not shown, so none of its reads (finals, sends, bodies, pane
            // output) are made; the screen is read when it is due.
            return self
                .pane_screen_read()
                .map(|(key, round)| super::refresh::SelectedRead::PaneScreen(key, round));
        }
        if let Some(plan) = self.conversation_read() {
            return Some(super::refresh::SelectedRead::Conversation(plan));
        }
        if let Some(key) = self.detail_read() {
            return Some(super::refresh::SelectedRead::Message(key));
        }
        self.pane_output_read()
            .map(super::refresh::SelectedRead::PaneOutput)
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
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(in crate::board) fn configure_workspace(app: &mut App, text: &str) {
        let path = std::env::temp_dir().join(format!(
            "ops-workspace-keys-{}.toml",
            crate::id::new_v4().unwrap()
        ));
        std::fs::write(&path, text).unwrap();
        let config = crate::config::Config::read(path.clone()).unwrap();
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
        app.overview_rows.borrow_mut().clear();
        app.invalidate_frame_geometry();
    }

    /// Install a coherent held rate observation through its existing Meter owner.
    pub(in crate::board) fn install_rate_observation(app: &mut App, name: String, rate: RateView) {
        install_rate_observation_at(app, name, rate, Instant::now());
    }

    /// A fixture may pin the monotonic origin while the scoped wall clock is frozen.
    pub(in crate::board) fn install_rate_observation_at(
        app: &mut App,
        name: String,
        rate: RateView,
        now: Instant,
    ) {
        let mut meter = super::super::meter::Meter::new(rate.settings, &rate.input, now);
        if let Some(seeds) = &rate.history {
            meter.seed(&rate.input, seeds, now, false);
        }
        app.meters.insert(name.clone(), meter);
        app.view.as_mut().unwrap().home_rate.insert(name, rate);
    }

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
            render: crate::config::NotesRender::Markdown,
            bindings: crate::action::with_action_keys(crate::action::preset(true, &[])),
            opener: None,
            clipboard: None,
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
            window: crate::config::TokenWindow::MINUTE,
            windows: crate::config::TokenWindow::DEFAULTS,
            reduced_motion: false,
            summary: super::super::home::Counts {
                members: 7,
                ..Default::default()
            },
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
            cursor: None,
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

    fn member(name: &str, fields: Value) -> Value {
        json!({"id": name, "name": name, "state": "working", "fields": fields})
    }

    pub(in crate::board) fn crew(bindings: crate::action::Bindings) -> App {
        let mut app = App::new(Some("product".into()));
        let mut snapshot = snapshot(
            "product",
            json!([
                {"title": "Mine", "rows": [member("auth-fix", json!({"task": "rotate", "pr_link": "https://example.com/pull/412"}))]},
                {"title": "Others", "rows": [member("docs", json!({"task": "guide", "link": "file:///etc/passwd"}))]}
            ]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.bindings = bindings;
        view.clipboard = Some(vec!["pbcopy".into()]);
        app.apply(snapshot);
        app
    }

    #[test]
    fn newline_caption_uses_only_the_observed_terminal_capability() {
        let mut app = App::new(None);
        assert_eq!(app.newline_hint(), "Alt+Enter new line");
        assert_eq!(
            super::super::glyph_guard::glyph_error(app.newline_hint()),
            None
        );
        app.keyboard_enhanced = true;
        assert_eq!(app.newline_hint(), "Shift+Enter new line");
        assert_eq!(
            super::super::glyph_guard::glyph_error(app.newline_hint()),
            None
        );
        app.keyboard_enhanced = false;
        assert_eq!(app.newline_hint(), "Alt+Enter new line");
    }

    #[test]
    fn message_newline_keys_keep_the_draft_until_plain_enter() {
        for key in [
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
        ] {
            let mut app = crew(Default::default());
            app.view.as_mut().unwrap().me = Some("Ben".into());
            app.view.as_mut().unwrap().me_id = Some("ben-id".into());
            app.perform(&Action::parse("talk").unwrap());
            assert!(app.input.is_some(), "Talk opens the message editor");
            app.paste("first");
            assert_eq!(app.key(key), Effect::None, "{key:?} must not submit");
            assert_eq!(app.input.as_ref().unwrap().text, "first\n", "{key:?}");
            app.paste("second");
            let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            else {
                panic!("plain Enter must submit the retained multiline draft");
            };
            match request {
                Request::Talk { text, .. } | Request::Chat { text, .. } => {
                    assert_eq!(text, "first\nsecond");
                }
                other => panic!("expected Talk, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_host_that_cannot_name_the_pane_says_why_instead_of_asking_who_is_sending() {
        let talk = || Action::parse("talk").expect("built-in Talk action");
        let mut app = crew(Default::default());
        app.view.as_mut().unwrap().me = None;
        app.view.as_mut().unwrap().me_id = None;
        // An unbound pane or no pane at all: the host answered, nobody is recorded.
        app.perform(&talk());
        let prompt = "Who is sending? Record yourself with tmt ops squad me <name>, or open the board from your named pane.";
        assert_eq!(app.notice.as_deref(), Some(prompt));
        assert!(app.input.is_none());
        // `whoami` failed outright: the host's own words and the way out.
        app.set_notice(None, false);
        app.caller_failure = Some("Herdr reported an invalid pane.".into());
        app.perform(&talk());
        assert_eq!(
            app.notice.as_deref(),
            Some(
                "Could not identify this pane: Herdr reported an invalid pane. Fix that, or record yourself with tmt ops squad me <name>."
            )
        );
        assert!(app.input.is_none());
        // A recorded sender needs no pane identity, so the failure is never shown.
        app.set_notice(None, false);
        app.view.as_mut().unwrap().me = Some("Ben".into());
        app.view.as_mut().unwrap().me_id = Some("ben-id".into());
        app.perform(&talk());
        assert!(app.notice.is_none(), "{:?}", app.notice);
        assert!(app.input.is_some());
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
    fn message_aliases_leave_settlement_to_core_and_notes_stay_explicit() {
        for verb in ["talk", "reply"] {
            let mut app = workspace_composer();
            app.view.as_mut().unwrap().me = Some("Ben".into());
            composer_row(app.view.as_mut().unwrap(), super::workspace::tests::MEMBER)["waitingOnYou"] = json!([
                {"requestId":"old","preparedAtMs":10}, {"requestId":"new","preparedAtMs":20}
            ]);
            app.perform(&Action::parse(verb).unwrap());
            assert!(app.menu.is_none());
            assert_eq!(
                press(&mut app, KeyCode::Enter),
                Effect::None,
                "empty sends nothing"
            );
            typed(&mut app, "q j -rf; $(x)");
            assert_eq!(
                press(&mut app, KeyCode::Enter),
                Effect::Act(Request::Chat {
                    sender: app.view.as_ref().unwrap().me_id.clone().unwrap(),
                    recipient: super::workspace::tests::MEMBER.into(),
                    name: "auth-fix".into(),
                    text: "q j -rf; $(x)".into(),
                    fallback: "[product · auth-fix] q j -rf; $(x)".into(),
                })
            );
            app.finish_draft(true);
            press(&mut app, KeyCode::Esc);
            app.perform(&Action::parse("annotate").unwrap());
            typed(&mut app, "split the job");
            assert!(
                matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Annotate { to, row, text, .. })
                if to == "sol" && row == "auth-fix" && text == "[product · auth-fix] split the job")
            );
        }
    }

    #[test]
    fn a_member_message_preserves_its_draft_and_reports_observed_outcomes() {
        let mut app = workspace_composer();
        composer_row(app.view.as_mut().unwrap(), super::workspace::tests::MEMBER)["waitingOnYou"] =
            json!([{"requestId":"q", "preview":"Ship tonight?"}]);
        app.workspace_reconcile();
        app.perform(&Action::parse("reply").unwrap());
        assert!(app.menu.is_none());
        assert_eq!(app.input.as_ref().unwrap().header(), "→ auth-fix (product)");
        typed(&mut app, "draft");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.input.as_ref().unwrap().text, "draft");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Chat { text, .. })
            if text == "draft")
        );
        assert!(app.notice.is_none(), "submission is not completion");
        app.finished(Err("delivery uncertain".into()));
        assert_eq!(app.notice.as_deref(), Some("delivery uncertain"));
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
    fn composer_standard_keys_edit_unicode_and_only_exact_authored_alt_bindings_win() {
        let mut app = workspace_composer();
        composer_bindings(&mut app, "[bind]\nf = 'quit'\nalt-b = 'refresh'\n");
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        app.paste("one 你我");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Effect::None
        );
        assert_eq!(app.input.as_ref().unwrap().text.cursor(), 0);
        assert!(app.input.as_ref().unwrap().text.selection().is_empty());
        // A bare f binding must not consume the terminal's ESC f / Alt-F.
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            Effect::None
        );
        assert_eq!(app.input.as_ref().unwrap().text.cursor(), 3);
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(app.input.as_ref().unwrap().text.cursor(), "one 你".len());
        app.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "one 你");
        // This exact Alt event was authored, so its action keeps precedence.
        let before = app.input.as_ref().unwrap().text.clone();
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
            Effect::Refresh
        );
        assert_eq!(app.input.as_ref().unwrap().text, before);
        app.key(KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "");
        app.paste("keep words");
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(app.input.as_ref().unwrap().text.is_empty());
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.input.is_none());
    }

    #[test]
    fn binding_event_names_preserve_the_actual_modifiers() {
        assert_eq!(
            event_name(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)).as_deref(),
            Some("alt-f")
        );
        assert_eq!(
            event_name(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)).as_deref(),
            Some("ctrl-left")
        );
        assert_eq!(
            event_name(KeyEvent::new(
                KeyCode::Left,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ))
            .as_deref(),
            Some("ctrl-shift-left")
        );
        assert_eq!(
            event_name(KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::ALT | KeyModifiers::CONTROL
            ))
            .as_deref(),
            Some("ctrl-alt-f")
        );
        // Under tmux extended-keys csi-u, Shift+Tab arrives as Tab with SHIFT.
        assert_eq!(
            event_name(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)).as_deref(),
            Some("backtab")
        );
        assert_eq!(
            event_name(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)).as_deref(),
            Some("backtab")
        );
    }

    #[test]
    fn enter_focuses_talk_and_an_authored_t_keeps_its_override() {
        let mut app = workspace_composer();
        app.view.as_mut().unwrap().me = Some("Ben".into());
        app.workspace_reconcile();
        assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.input.as_ref().unwrap().header(), "→ auth-fix (product)");
        app.input = None;
        composer_bindings(&mut app, "[bind]\nt = 'menu'\n");
        press(&mut app, KeyCode::Char('t'));
        assert!(app.input.is_none() && app.menu.is_some());
    }

    #[test]
    fn tab_never_changes_a_member_message_into_a_lead_note() {
        let mut app = workspace_composer();
        composer_row(app.view.as_mut().unwrap(), super::workspace::tests::MEMBER)["waitingOnYou"] =
            json!([{"requestId":"q", "preview":"Ship?"}]);
        app.perform(&Action::parse("talk").unwrap());
        typed(&mut app, "keep");
        for _ in 0..4 {
            press(&mut app, KeyCode::Tab);
        }
        assert!(
            matches!(&app.input.as_ref().unwrap().compose, Compose::Talk { to } if to == "auth-fix")
        );
        assert_eq!(app.input.as_ref().unwrap().text, "keep");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Chat { name, text, .. })
            if name == "auth-fix" && text == "keep")
        );
    }

    #[test]
    fn a_member_without_a_lead_or_request_still_accepts_a_message() {
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
        app.workspace_reconcile();
        app.perform(&Action::parse("reply").unwrap());
        typed(&mut app, "hi");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.input.as_ref().unwrap().text, "hi");
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Chat { name, text, .. })
            if name == "auth-fix" && text == "hi")
        );
    }

    #[test]
    fn an_explicit_member_note_binding_stays_a_note() {
        let mut app = workspace_composer();
        composer_bindings(&mut app, "[bind]\nx = 'annotate member'\n");
        app.workspace_reconcile();
        press(&mut app, KeyCode::Char('x'));
        typed(&mut app, "observed");
        press(&mut app, KeyCode::Tab);
        assert!(
            matches!(press(&mut app, KeyCode::Enter), Effect::Act(Request::Annotate { to, row, text, .. })
            if to == "auth-fix" && row == "auth-fix" && text == "[product · auth-fix] observed")
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
            app.perform(&Action::parse("reply").unwrap());
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
            if change == "identity" {
                assert_eq!(effect, Effect::None, "{change}");
                assert!(app.notice.is_some());
                assert_eq!(app.input.as_ref().unwrap().text, "yes");
            } else if change == "request" {
                assert!(
                    matches!(effect, Effect::Act(Request::Chat { name, text, .. }) if name == "auth-fix" && text == "yes")
                );
            } else {
                assert_eq!(app.row_target(app.selected).as_ref(), Some(&target));
                assert!(
                    matches!(effect, Effect::Act(Request::Chat { name, .. }) if name == "auth-fix"),
                    "{change}"
                );
            }
        }
    }

    #[test]
    fn chat_freezes_the_sender_uuid_and_uses_the_typed_recipient() {
        use super::workspace::tests::MEMBER;
        for change_sender in [false, true] {
            let mut app = workspace_composer();
            app.perform(&Action::parse("talk").unwrap());
            let sender = app.view.as_ref().unwrap().me_id.clone().unwrap();
            typed(&mut app, "kept draft");
            // Display JSON cannot retarget a frozen workspace member UUID.
            composer_row(app.view.as_mut().unwrap(), MEMBER)["id"] = json!("display-only-id");
            if change_sender {
                app.view.as_mut().unwrap().me_id = Some("another-person".into());
            }
            let effect = press(&mut app, KeyCode::Enter);
            if change_sender {
                assert_eq!(effect, Effect::None);
                assert_eq!(app.input.as_ref().unwrap().text, "kept draft");
            } else {
                assert!(
                    matches!(effect, Effect::Act(Request::Chat { sender: actual, recipient, text, .. })
                    if actual == sender && recipient == MEMBER && text == "kept draft")
                );
            }
        }
    }

    #[test]
    fn chat_keeps_plain_text_and_does_not_answer_the_newest_question() {
        use super::workspace::tests::MEMBER;
        let mut app = workspace_composer();
        composer_row(app.view.as_mut().unwrap(), MEMBER)["waitingOnYou"] = json!([
            {"requestId":"old", "preparedAtMs":1, "preview":"First?"},
            {"requestId":"new", "preparedAtMs":2, "preview":"Second?"}
        ]);
        app.perform(&Action::parse("reply").unwrap());
        typed(&mut app, "/yes!");
        let effect = press(&mut app, KeyCode::Enter);
        assert!(
            matches!(effect, Effect::Act(Request::Chat { text, .. }) if text == "/yes!"),
            "plain chat must leave question settlement to Core and preserve the typed body"
        );
    }

    #[test]
    fn ctrl_r_refreshes_without_typing_into_search_or_message_inputs() {
        for tmux in [false, true] {
            let mut app = crew(crate::action::preset(tmux, &[]));
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
            ] {
                app.input = Some(Input {
                    draft_key: None,

                    inline_quotes: Default::default(),

                    row_send: None,
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
            cursor: None,
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
mod token_window_tests {
    use super::tests::crew;
    use super::*;
    use crate::config::{TokenRate, TokenWindow};
    use serde_json::json;
    fn saved_cycle(app: &mut App, key: KeyEvent) {
        assert_eq!(app.key(key), Effect::CycleTokenWindow);
        save_window(app);
    }

    fn save_window(app: &mut App) {
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
        let notice = app.apply_token_window(&saved);
        app.set_notice(Some(notice), false);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn app() -> App {
        let mut app = crew(crate::action::with_action_keys(crate::action::preset(
            false,
            &[],
        )));
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
            window: crate::config::TokenWindow::MINUTE,
            windows: TokenWindow::DEFAULTS,
            reduced_motion: false,
            summary: Default::default(),
            failures: Vec::new(),
            incomplete: false,
            squads: vec![super::super::home::SquadLine {
                squad: "product".into(),
                lead: Some(serde_json::json!({"id":"a"})),
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
                retained: None,
                quota: app.quota_history_visible().then(Default::default),
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
        assert!(initial.top.is_none());
        let time = now + Duration::from_secs(10);
        app.sample_home(Ok(&super::super::rate::tests::input(100).resumes), time);
        let zero = app.home_header_usage(time).unwrap();
        assert_eq!(zero.totals[2].unwrap().tokens, 0);
        assert!(zero.totals[2].unwrap().partial);
        assert!(zero.top.is_none());
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
    fn header_deduplicates_memberships_and_uses_one_observed_total_for_top_share() {
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
        let top = usage.top.unwrap();
        assert_eq!(top.member, "worker-b");
        assert_eq!(
            top.share,
            UsageShare {
                fraction: 0.75,
                partial: true
            }
        );
        assert_eq!(app.view.as_ref().unwrap().document, public);
        app.search = "product".into();
        let filtered = app.home_header_usage(time).unwrap();
        assert_eq!(filtered.totals[2].unwrap().tokens, 150);
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
    fn home_lead_model_follows_the_last_completed_turn() {
        let mut app = home(Instant::now());
        fn resume(app: &mut App) -> &mut serde_json::Value {
            app.view
                .as_mut()
                .unwrap()
                .home_rate
                .get_mut("product")
                .unwrap()
                .input
                .resumes
                .get_mut("a")
                .unwrap()
        }
        resume(&mut app)["model"] = serde_json::json!("start");
        assert_eq!(app.home_lead_model("product"), Some("start"));
        resume(&mut app)["consumption"]["modelId"] = serde_json::json!("switched");
        assert_eq!(app.home_lead_model("product"), Some("switched"));
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

    fn configure_window_keys(app: &mut App, text: &str) {
        let path = std::env::temp_dir().join(format!(
            "ops-window-keys-{}.toml",
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
    fn window_binding_overrides_and_text_inputs_keep_their_owner() {
        let mut app = super::workspace::tests::board();
        configure_window_keys(&mut app, "[bind]\nu = \"token-window\"\n");
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE);
        saved_cycle(&mut app, key);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        app.searching = true;
        assert_eq!(app.key(key), Effect::None);
        assert_eq!(app.search, "u");
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.searching = false;
        for compose in [
            Compose::Talk { to: "a".into() },
            Compose::Annotate {
                to: "a".into(),
                row: "a".into(),
            },
        ] {
            app.input = Some(Input {
                draft_key: None,

                inline_quotes: Default::default(),

                row_send: None,
                link: None,
                prompt: "message".into(),
                text: Default::default(),
                compose,
                squad: "x".into(),
                hint: None,
            });
            app.key(key);
            assert_eq!(app.input.as_ref().unwrap().text, "u");
            assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        }
        app.input = None;
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Effect::None,
            "clear the retained filter while keeping its current selection before testing member bindings"
        );
        configure_window_keys(&mut app, "[bind]\nu = \"refresh\"\nU = \"token-window\"\n");
        assert_eq!(app.key(key), Effect::Refresh);
        assert_eq!(app.token_window, TokenWindow::FIVE_MINUTES);
        saved_cycle(
            &mut app,
            KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE),
        );
        assert_eq!(app.token_window, TokenWindow::HOUR);
        configure_window_keys(
            &mut app,
            "[bind]\nu = \"refresh\"\n[[squad.product.section]]\ntitle = \"Members\"\n[squad.product.section.bind]\nu = \"token-window\"\n",
        );
        saved_cycle(&mut app, key);
        assert_eq!(app.token_window, TokenWindow::MINUTE);
    }
    #[test]
    fn source_window_changes_prioritize_columns_even_with_unchanged_zero_values() {
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
            assert_eq!(
                app.perform(&Action::parse("token-window").unwrap()),
                Effect::CycleTokenWindow
            );
            save_window(&mut app);
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
