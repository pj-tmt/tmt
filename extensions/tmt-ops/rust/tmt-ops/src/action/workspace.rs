//! Workspace action vocabulary. Keys belong to Config scopes; this catalog
//! supplies one name/label/description used by parsing, menus and key guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsUiAction {
    Overview,
    Agents,
    DigestDashboard,
    Inspect,
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
    Timers,
    Pin,
    Manage,
    CreateSquad,
    RemoveMembership,
    MoveMembership,
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
    spec!(Pin, "pin", "pin", "pin or unpin the selected squad", 5),
    spec!(Manage, "manage", "manage", "manage the selected squad", 6),
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
        matches!(self, Self::RemoveMembership | Self::MoveMembership)
    }
}
