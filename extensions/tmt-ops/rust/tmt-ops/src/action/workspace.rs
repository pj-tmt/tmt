//! Workspace action vocabulary. Keys belong to Config scopes; this catalog
//! supplies one name/label/description used by parsing, menus and key guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsUiAction {
    ResizePane,
    Agents,
    DigestDashboard,
    QuotaDetails,
    SavingsUnit,
    UsageControls,
    CycleTokRange,
    Inspect,
    MonitorDetails,
    Edit,
    Rename,
    DigestDetails,
    MemberSession,
    SwitchTool,
    MemberNotes,
    MarkRead,
    Unread,
    Terminal,
    TerminalZoom,
    Previous,
    Next,
    Fold,
    Expand,
    PageUp,
    PageDown,
    First,
    Last,
    Search,
    Help,
    Quit,
    Sort,
    ReverseSort,
    AgentsFilter,
    TalkMeasure,
    Timers,
    AllTimers,
    WorkspaceSnapshots,
    OpenWeb,
    DiscoverAgents,
    Pin,
    HideSquad,
    ShowSquad,
    RevealHidden,
    ClearScope,
    Manage,
    CreateSquad,
    NewMember,
    IdleAlert,
    RemoveMembership,
    MoveMembership,
    RenameMember,
    RemoveMember,
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
}

pub struct OpsUiSpec {
    pub action: OpsUiAction,
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub rank: u8,
}
macro_rules! spec {
    ($action:ident, $name:literal, $label:literal, $description:literal, $rank:literal) => {
        OpsUiSpec {
            action: OpsUiAction::$action,
            name: $name,
            label: $label,
            description: $description,
            rank: $rank,
        }
    };
}
const SPECS: &[OpsUiSpec] = &[
    spec!(Edit, "edit", "edit", "edit the selected agent or squad", 7),
    spec!(
        Rename,
        "rename",
        "rename",
        "rename the selected agent or squad in Edit",
        7
    ),
    spec!(
        SwitchTool,
        "switch-tool",
        "switch tool",
        "switch the selected agent to another AI tool through its host template",
        13
    ),
    spec!(
        IdleAlert,
        "idle-alert",
        "idle alert",
        "edit the selected squad idle alert",
        7
    ),
    spec!(
        NewMember,
        "new-member",
        "new",
        "create an agent using the selected host launch template",
        7
    ),
    spec!(
        MemberSession,
        "member-session",
        "Session…",
        "read the selected agent's host and remembered session, then review resume or move",
        12
    ),
    spec!(
        MemberNotes,
        "member-checklist",
        "checklist",
        "read and edit the selected agent's Checklist",
        12
    ),
    spec!(
        Terminal,
        "terminal",
        "terminal",
        "switch the selected built-in-host agent between its chat and its live terminal",
        6
    ),
    spec!(
        TerminalZoom,
        "terminal-zoom",
        "zoom",
        "make the live terminal fill the board, and back; bound to a key it also works while typing into the terminal",
        6
    ),
    spec!(
        Unread,
        "unread",
        "unread",
        "show the unread answers of the selected agent or squad, or of everyone",
        12
    ),
    spec!(
        MarkRead,
        "mark-read",
        "mark read",
        "mark every answer of the selected agent's conversation read",
        12
    ),
    spec!(
        ResizePane,
        "resize-pane",
        "resize",
        "adjust a visible pane boundary or reset its default",
        24
    ),
    spec!(
        AgentsFilter,
        "agents-filter",
        "filter",
        "cycle Agents through all and pinned squads",
        11
    ),
    spec!(
        MonitorDetails,
        "monitor-details",
        "details",
        "read the selected agent's full monitor values",
        12
    ),
    spec!(
        DigestDetails,
        "digest-details",
        "details tab",
        "show the next Digest Details tab: Impact, Delivery, Evidence",
        12
    ),
    spec!(
        ScrollUp,
        "scroll-up",
        "scroll up",
        "scroll the focused response one line up",
        21
    ),
    spec!(
        ScrollDown,
        "scroll-down",
        "scroll down",
        "scroll the focused response one line down",
        21
    ),
    spec!(
        ScrollPageUp,
        "scroll-page-up",
        "page up",
        "scroll the focused response one page up",
        23
    ),
    spec!(
        ScrollPageDown,
        "scroll-page-down",
        "page down",
        "scroll the focused response one page down",
        23
    ),
    spec!(
        ScrollTop,
        "scroll-top",
        "top",
        "scroll to the start of the focused response",
        23
    ),
    spec!(
        ScrollBottom,
        "scroll-bottom",
        "bottom",
        "scroll to the end of the focused response",
        23
    ),
    spec!(Agents, "agents", "Agents", "show the squad workspace", 20),
    spec!(
        DigestDashboard,
        "digest-dashboard",
        "Digest",
        "show the Digest tab",
        20
    ),
    spec!(
        UsageControls,
        "usage-controls",
        "usage chart",
        "focus interval, range and usage buckets",
        12
    ),
    spec!(
        CycleTokRange,
        "cycle-tok-range",
        "TOK range",
        "cycle the shared Agents and Digest TOK range",
        12
    ),
    spec!(
        SavingsUnit,
        "savings-unit",
        "unit",
        "cycle the Digest estimate between tokens, percent and list-price cost",
        12
    ),
    spec!(
        QuotaDetails,
        "quota-details",
        "quota details",
        "inspect reported subscription windows",
        13
    ),
    spec!(
        Inspect,
        "inspect",
        "inspect",
        "inspect the selected target",
        0
    ),
    spec!(
        Previous,
        "previous",
        "previous",
        "select the previous visible target",
        21
    ),
    spec!(Next, "next", "next", "select the next visible target", 21),
    spec!(Fold, "fold", "collapse", "collapse the selected squad", 22),
    spec!(Expand, "expand", "expand", "expand the selected squad", 22),
    spec!(PageUp, "page-up", "page up", "move one page up", 23),
    spec!(PageDown, "page-down", "page down", "move one page down", 23),
    spec!(
        First,
        "first",
        "first",
        "select the first visible target",
        23
    ),
    spec!(Last, "last", "last", "select the last visible target", 23),
    spec!(Search, "search", "search", "search the current list", 15),
    spec!(Help, "help", "help", "show effective scoped bindings", 25),
    spec!(Quit, "quit", "quit", "close the board", 26),
    spec!(Sort, "sort", "sort", "cycle the flat Agents sort", 10),
    spec!(
        TalkMeasure,
        "talk-measure",
        "talk/measure",
        "switch the right column between Talk and Measure",
        11
    ),
    spec!(
        ReverseSort,
        "reverse-sort",
        "reverse",
        "reverse the current sort",
        11
    ),
    spec!(
        Timers,
        "timers",
        "timers",
        "show the selected scope's timers",
        12
    ),
    spec!(
        AllTimers,
        "all-timers",
        "timers",
        "show all-room timers",
        12
    ),
    spec!(
        HideSquad,
        "hide-squad",
        "Hide squad",
        "hide the selected squad in Agents",
        6
    ),
    spec!(
        ShowSquad,
        "show-squad",
        "Show squad",
        "remove the selected squad from saved hidden squads",
        6
    ),
    spec!(
        RevealHidden,
        "reveal-hidden",
        "show",
        "reveal hidden squads for this session",
        12
    ),
    spec!(
        ClearScope,
        "clear-scope",
        "clears",
        "clear this session's launch scope",
        12
    ),
    spec!(Pin, "pin", "pin", "pin or unpin the selected squad", 5),
    spec!(
        WorkspaceSnapshots,
        "workspace-snapshots",
        "snapshots",
        "preview saved workspace snapshots and review a guarded restore",
        12
    ),
    spec!(
        OpenWeb,
        "open-web",
        "web",
        "open the web UI in the browser with tmt remote open",
        12
    ),
    spec!(
        DiscoverAgents,
        "discover-agents",
        "add agent",
        "find an agent already running and review its binding",
        13
    ),
    spec!(Manage, "manage", "manage", "manage the selected squad", 6),
    spec!(
        RenameMember,
        "rename-member",
        "rename",
        "review a UUID-safe member rename",
        7
    ),
    spec!(
        RemoveMember,
        "remove-member",
        "close",
        "stop through its host and delete this identity after confirmation",
        8
    ),
    spec!(
        CreateSquad,
        "create-squad",
        "create",
        "create a squad with reviewed lead and members",
        7
    ),
    spec!(
        RemoveMembership,
        "remove-membership",
        "leave squad",
        "confirm removal of this squad membership only",
        8
    ),
    spec!(
        MoveMembership,
        "move-membership",
        "move",
        "add or move the selected member to a squad",
        9
    ),
];
impl OpsUiAction {
    pub fn parse(name: &str) -> Option<Self> {
        SPECS
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.action)
    }
    pub fn spec(self) -> &'static OpsUiSpec {
        SPECS
            .iter()
            .find(|spec| spec.action == self)
            .expect("every UI action has a catalog entry")
    }
    /// Strip controls already carry their configured key beside the control.
    pub fn footer_hint(self) -> bool {
        !matches!(self, Self::AgentsFilter | Self::TalkMeasure)
    }
    pub fn strip_actions() -> impl Iterator<Item = Self> {
        SPECS
            .iter()
            .map(|spec| spec.action)
            .filter(|action| !action.footer_hint())
    }
    pub fn menu_label(self) -> Option<&'static str> {
        match self {
            Self::MonitorDetails => Some("Observed values (Measure)"),
            Self::MemberSession => Some("Member session"),
            Self::RenameMember => Some("Rename agent"),
            Self::RemoveMember => Some("Close and delete"),
            _ => None,
        }
    }
    /// These member actions remain discoverable without a keyboard binding.
    pub fn menu_actions() -> impl Iterator<Item = Self> {
        SPECS
            .iter()
            .map(|spec| spec.action)
            .filter(|action| action.menu_label().is_some())
    }
    pub fn acts_on_member(self) -> bool {
        matches!(
            self,
            Self::RemoveMember
                | Self::RemoveMembership
                | Self::MoveMembership
                | Self::RenameMember
                | Self::MonitorDetails
                | Self::MemberSession
                | Self::SwitchTool
                | Self::MemberNotes
                | Self::MarkRead
                | Self::Terminal
                | Self::TerminalZoom
        )
    }
}

/// Stable names for retained pane ratios and explicitly authored resize actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResizeBoundary {
    AgentsColumns,
    AgentsRows,
    AgentsUsage,
    DigestColumns,
    DigestRows,
}
impl ResizeBoundary {
    pub const ALL: [Self; 5] = [
        Self::AgentsColumns,
        Self::AgentsRows,
        Self::AgentsUsage,
        Self::DigestColumns,
        Self::DigestRows,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::AgentsColumns => "agents-columns",
            Self::AgentsRows => "agents-rows",
            Self::AgentsUsage => "agents-usage",
            Self::DigestColumns => "digest-columns",
            Self::DigestRows => "digest-rows",
        }
    }
    /// The current name, or the Overview name that the merged Agents tab kept under a new one.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|id| id.name() == name)
            .or(match name {
                "overview-columns" => Some(Self::AgentsColumns),
                "overview-usage" => Some(Self::AgentsUsage),
                _ => None,
            })
    }
    pub fn columns(self) -> bool {
        matches!(self, Self::AgentsColumns | Self::DigestColumns)
    }
    pub fn accepts(self, direction: &str) -> bool {
        direction == "reset"
            || if self.columns() {
                matches!(direction, "left" | "right")
            } else {
                matches!(direction, "up" | "down")
            }
    }
}
