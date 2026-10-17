//! Workspace action vocabulary. Keys belong to Config scopes; this catalog
//! supplies one name/label/description used by parsing, menus and key guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsUiAction {
    ResizePane,
    Overview,
    Agents,
    DigestDashboard,
    QuotaDetails,
    SavingsUnit,
    UsageControls,
    Inspect,
    MonitorDetails,
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
    OverviewView,
    AgentsFilter,
    Timers,
    AllTimers,
    WorkspaceSnapshots,
    DiscoverAgents,
    Pin,
    HideSquad,
    ShowSquad,
    RevealHidden,
    ClearScope,
    Manage,
    CreateSquad,
    RemoveMembership,
    MoveMembership,
    RenameMember,
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
    spec!(Overview, "overview", "Overview", "show Overview", 20),
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
    spec!(Sort, "sort", "sort", "cycle Overview sort", 10),
    spec!(
        OverviewView,
        "overview-view",
        "squads/agents",
        "switch Overview between squads and agents",
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
        "hide the selected squad in Agents and Overview",
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
        CreateSquad,
        "create-squad",
        "create",
        "create a squad with reviewed lead and members",
        7
    ),
    spec!(
        RemoveMembership,
        "remove-membership",
        "remove",
        "confirm removal of this squad membership only",
        8
    ),
    spec!(
        MoveMembership,
        "move-membership",
        "assign",
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
    pub fn acts_on_member(self) -> bool {
        matches!(
            self,
            Self::RemoveMembership
                | Self::MoveMembership
                | Self::RenameMember
                | Self::MonitorDetails
        )
    }
}

/// Stable names for retained pane ratios and explicitly authored resize actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResizeBoundary {
    OverviewColumns,
    AgentsColumns,
    AgentsRows,
    DigestColumns,
    DigestRows,
    OverviewUsage,
    OverviewList,
    OverviewLowerColumns,
    OverviewLowerRows,
    OverviewExchange,
    OverviewTimers,
}
impl ResizeBoundary {
    pub const ALL: [Self; 11] = [
        Self::OverviewColumns,
        Self::AgentsColumns,
        Self::AgentsRows,
        Self::DigestColumns,
        Self::DigestRows,
        Self::OverviewUsage,
        Self::OverviewList,
        Self::OverviewLowerColumns,
        Self::OverviewLowerRows,
        Self::OverviewExchange,
        Self::OverviewTimers,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::OverviewColumns => "overview-columns",
            Self::AgentsColumns => "agents-columns",
            Self::AgentsRows => "agents-rows",
            Self::DigestColumns => "digest-columns",
            Self::DigestRows => "digest-rows",
            Self::OverviewUsage => "overview-usage",
            Self::OverviewList => "overview-list",
            Self::OverviewLowerColumns => "overview-lower-columns",
            Self::OverviewLowerRows => "overview-lower-rows",
            Self::OverviewExchange => "overview-exchange",
            Self::OverviewTimers => "overview-timers",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.name() == name)
    }
    pub fn columns(self) -> bool {
        matches!(
            self,
            Self::OverviewColumns
                | Self::AgentsColumns
                | Self::DigestColumns
                | Self::OverviewLowerColumns
        )
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
