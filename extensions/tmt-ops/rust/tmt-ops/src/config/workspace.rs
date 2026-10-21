//! The effective workspace action table. Dispatch, footer and help consume the same
//! result after default/global/host/section precedence, never independent hints.
use super::*;
use crate::action::{Action, OpsUiAction, Verb};
use std::collections::BTreeSet;

mod panels;
pub use panels::{
    DigestPanel, DigestPanels, OverviewPanel, OverviewPanels, Panel, PanelSetting, Panels,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpsSort {
    #[default]
    Name,
    Attention,
    Members,
}

impl OpsSort {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Attention,
            Self::Attention => Self::Members,
            Self::Members => Self::Name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewColumn {
    Agent,
    Squad,
    State,
    TokensPerHour,
    Digest,
    Cron,
    Runtime,
    Model,
}

impl OverviewColumn {
    pub const ALL: [Self; 8] = [
        Self::Agent,
        Self::Squad,
        Self::State,
        Self::TokensPerHour,
        Self::Digest,
        Self::Cron,
        Self::Runtime,
        Self::Model,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Squad => "squad",
            Self::State => "state",
            Self::TokensPerHour => "tokens-per-hour",
            Self::Digest => "digest",
            Self::Cron => "cron",
            Self::Runtime => "runtime",
            Self::Model => "model",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverviewSort {
    pub column: OverviewColumn,
    pub direction: OverviewDirection,
}

impl OverviewSort {
    pub fn initial(column: OverviewColumn) -> Self {
        Self {
            column,
            direction: if matches!(column, OverviewColumn::TokensPerHour | OverviewColumn::Cron) {
                OverviewDirection::Descending
            } else {
                OverviewDirection::Ascending
            },
        }
    }

    pub fn reverse(self) -> Self {
        Self {
            direction: match self.direction {
                OverviewDirection::Ascending => OverviewDirection::Descending,
                OverviewDirection::Descending => OverviewDirection::Ascending,
            },
            ..self
        }
    }

    pub fn next(self) -> Self {
        let position = OverviewColumn::ALL
            .iter()
            .position(|column| *column == self.column)
            .unwrap();
        Self::initial(OverviewColumn::ALL[(position + 1) % OverviewColumn::ALL.len()])
    }
}

/// One saved TOK range shared by Agents and Digest, independent of chart zoom
/// and the Digest supplier's savings window.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TokRange {
    #[default]
    Hour,
    ThreeHours,
    SixHours,
    Day,
    Week,
}

impl TokRange {
    pub const ALL: [Self; 5] = [
        Self::Hour,
        Self::ThreeHours,
        Self::SixHours,
        Self::Day,
        Self::Week,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|range| range.argument() == value)
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::ThreeHours => "3h",
            Self::SixHours => "6h",
            Self::Day => "1d",
            Self::Week => "7d",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Hour => "TOK 1h",
            Self::ThreeHours => "TOK 3h",
            Self::SixHours => "TOK 6h",
            Self::Day => "TOK 1d",
            Self::Week => "TOK 7d",
        }
    }

    pub fn milliseconds(self) -> u64 {
        match self {
            Self::Hour => 3_600_000,
            Self::ThreeHours => 10_800_000,
            Self::SixHours => 21_600_000,
            Self::Day => 86_400_000,
            Self::Week => 604_800_000,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Hour => Self::ThreeHours,
            Self::ThreeHours => Self::SixHours,
            Self::SixHours => Self::Day,
            Self::Day => Self::Week,
            Self::Week => Self::Hour,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpsPreferences {
    pub pinned: BTreeSet<String>,
    pub hidden: BTreeSet<String>,
    pub agent_pins: BTreeSet<String>,
    pub sort: OpsSort,
    pub reversed: bool,
    /// None retains the monitor's needs-you ordering until an explicit sort.
    pub overview_sort: Option<OverviewSort>,
    pub layout: OpsLayout,
    pub tok_range: TokRange,
    pub digest_window: super::DigestWindow,
    pub savings_unit: super::SavingsUnit,
    pub overview_panels: Option<PanelSetting<OverviewPanel>>,
    pub digest_panels: Option<PanelSetting<DigestPanel>>,
}

fn canonical_room_id(value: &str) -> bool {
    uuid_like(value) && !value.bytes().any(|byte| byte.is_ascii_uppercase())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpsLayout {
    #[default]
    Auto,
    Side,
    Stack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpsKeyScope {
    Squad,
    Member,
    Pool,
    Reader,
}

impl OpsKeyScope {
    pub(crate) fn is_reader(self) -> bool {
        self == Self::Reader
    }
}

pub struct WorkspaceKeys {
    pub bindings: Bindings,
    /// Provenance follows exactly the admitted default/global/host/section merge.
    pub sources: std::collections::BTreeMap<String, String>,
    pub notices: Vec<String>,
}

pub(crate) fn defaults(scope: OpsKeyScope) -> Bindings {
    let mut entries = if scope.is_reader() {
        vec![
            ("1", "agents"),
            ("2", "digest-dashboard"),
            ("up", "scroll-up"),
            ("k", "scroll-up"),
            ("down", "scroll-down"),
            ("j", "scroll-down"),
            ("pageup", "scroll-page-up"),
            ("pagedown", "scroll-page-down"),
            ("home", "scroll-top"),
            ("end", "scroll-bottom"),
            ("tab", "next-pane"),
            ("backtab", "next-pane"),
            ("?", "help"),
            ("q", "quit"),
            ("ctrl-r", "refresh"),
            (",", "settings"),
            ("enter", "talk"),
            ("o", "open"),
            ("D", "digest"),
            ("c", "timers"),
            ("v", "view-reply"),
            ("p", "pin"),
        ]
    } else {
        vec![
            ("1", "agents"),
            ("2", "digest-dashboard"),
            ("up", "previous"),
            ("down", "next"),
            ("left", "fold"),
            ("right", "expand"),
            ("pageup", "page-up"),
            ("pagedown", "page-down"),
            ("home", "first"),
            ("end", "last"),
            ("enter", "inspect"),
            ("double-click", "inspect"),
            ("/", "search"),
            ("?", "help"),
            ("q", "quit"),
            ("tab", "next-pane"),
            ("backtab", "next-pane"),
            ("ctrl-r", "refresh"),
            (",", "settings"),
            ("c", "timers"),
            ("t", "talk-measure"),
            ("n", "create-squad"),
            ("u", "usage-controls"),
            ("esc", "clear-scope"),
            ("h", "reveal-hidden"),
        ]
    };
    match scope {
        OpsKeyScope::Reader => {}
        OpsKeyScope::Squad => entries.extend([
            ("j", "jump"),
            ("o", "open"),
            ("D", "digest"),
            ("p", "pin"),
            ("m", "manage"),
            ("n", "create-squad"),
            ("H", "hide-squad"),
            ("S", "show-squad"),
        ]),
        OpsKeyScope::Member => entries.extend([
            ("v", "view-reply"),
            ("p", "pin"),
            ("e", "monitor-details"),
            ("enter", "talk"),
            ("j", "jump"),
            ("o", "open"),
            ("x", "remove-membership"),
            ("m", "move-membership"),
            ("r", "rename-member"),
            ("D", "digest"),
        ]),
        OpsKeyScope::Pool => entries.extend([("m", "manage"), ("n", "create-squad")]),
    }
    if matches!(
        scope,
        OpsKeyScope::Squad | OpsKeyScope::Member | OpsKeyScope::Pool
    ) {
        entries.push(("s", "discover-agents"));
        entries.push(("f", "agents-filter"));
    }
    entries.extend([
        ("w", "workspace-snapshots"),
        ("y", "copy"),
        ("a", "annotate lead"),
    ]);
    entries
        .into_iter()
        .map(|(key, action)| {
            (
                key.into(),
                Action::parse(action).expect("admitted default action"),
            )
        })
        .collect()
}

pub(crate) fn admit_action(action: &Action, path: &str) -> Result<(), SquadError> {
    if let Verb::Retired(name) = action.verb {
        let instead = match name {
            "overview" => "the Overview tab was merged into Agents; use 'agents'",
            "overview-view" => "the Squads/Agents switch is now 'agents-filter'",
            _ => "choose 'agents-filter' for Squads/Agents/Pinned explicitly",
        };
        return Err(invalid(format!(
            "`{path}`: '{name}' is retired; {instead}. The authored binding is preserved for migration."
        )));
    }
    if action.verb == Verb::Ui(OpsUiAction::AgentsFilter) && !action.args.is_empty() {
        return Err(invalid(format!(
            "`{path}`: 'agents-filter needs-you' is retired; 'agents-filter' now cycles between All and Pinned. The authored binding is preserved for migration."
        )));
    }
    if matches!(
        action.verb,
        Verb::Toggle
            | Verb::Tab
            | Verb::AskLead
            | Verb::HomeMessage
            | Verb::HomeWrite
            | Verb::HomePick
            | Verb::PickTab
            | Verb::View
            | Verb::Notes
    ) {
        return Err(invalid(format!(
            "`{path}`: '{}' belongs to the retired squad-tab/HOME/notebook layout. Select a squad or named lead/member in the workspace; choose a new binding explicitly.",
            action.text
        )));
    }
    if action.verb == Verb::Jump
        && action
            .args
            .first()
            .and_then(crate::template::Template::literal)
            == Some("lead")
    {
        return Err(invalid(format!(
            "`{path}`: select the named lead row, then bind jump; implicit lead targeting is retired."
        )));
    }
    Ok(())
}
impl Config {
    /// Retired board composition remains in the authored file and in independent
    /// CLI projections. The shell reports exactly which settings need migration.
    pub fn workspace_migration_notices(&self) -> Vec<String> {
        let mut notices = Vec::new();
        fn board_notices(item: Option<&Item>, path: &str, notices: &mut Vec<String>) {
            if let Some(table) = item.and_then(Item::as_table_like) {
                for (key, _) in table.iter() {
                    if [
                        "mode",
                        "direction",
                        "panes",
                        "sizes",
                        "layout",
                        "collapsed",
                        "fold_below",
                        "view",
                        "hidden_columns",
                    ]
                    .contains(&key)
                    {
                        notices.push(format!("`{path}.{key}` belongs to the retired factory/pane layout; choose `workspace.layout = auto|side|stack` explicitly. The authored value is preserved for migration."));
                    }
                    if key == "ask_lead" {
                        notices.push(format!("`{path}.ask_lead` is retired; Talk uses the explicitly selected named agent, without this policy."));
                    }
                    if key == "home_replies" {
                        notices.push(format!("`{path}.home_replies` is retired; open the selected agent's conversation explicitly."));
                    }
                }
            }
        }
        board_notices(self.document.get("board"), "board", &mut notices);
        if let Some(squads) = self.document.get("squad").and_then(Item::as_table_like) {
            for (name, item) in squads.iter() {
                let Some(table) = item.as_table_like() else {
                    continue;
                };
                board_notices(
                    table.get("board"),
                    &format!("squad.{name}.board"),
                    &mut notices,
                );
                for key in ["layout", "rows", "columns", "sections"] {
                    if table.get(key).is_some() {
                        notices.push(format!("`squad.{name}.{key}` remains available to CLI listing; custom board sections/columns no longer compose the workspace. Use its squad tree and selected-agent details."));
                    }
                }
                if table
                    .get("notes")
                    .and_then(Item::as_table_like)
                    .is_some_and(|notes| notes.get("render").is_some())
                {
                    notices.push(format!("`squad.{name}.notes.render` belongs to the retired notebook pane. The authored value is preserved for migration; the workspace reads agent responses instead."));
                }
            }
        }
        if let Some(tabs) = self.document.get("tabs").and_then(Item::as_table_like) {
            for (key, _) in tabs.iter() {
                notices.push(format!("`tabs.{key}` remains available to CLI listing; old squad-tab arrangement does not control the three-tab workspace. Use UUID squad pins/sort and guarded 1/2/3."));
            }
        }
        if let Some(links) = self.document.get("links").and_then(Item::as_table_like) {
            for (scheme, _) in links.iter() {
                notices.push(format!("`links.{scheme}` belongs to retired notebook-link activation. The authored handler is preserved; configure an explicit workspace binding or opener for the selected agent's link."));
            }
        }
        if let Some(workspace) = self.document.get("workspace").and_then(Item::as_table_like) {
            panel_notice::<OverviewPanel>(
                workspace.get("overview_panels"),
                "overview_panels",
                &mut notices,
            );
            panel_notice::<DigestPanel>(
                workspace.get("digest_panels"),
                "digest_panels",
                &mut notices,
            );
        }
        notices
    }

    pub fn workspace_preferences(&self) -> Result<OpsPreferences, SquadError> {
        let Some(item) = self.document.get("workspace") else {
            return Ok(OpsPreferences::default());
        };
        let table = item
            .as_table_like()
            .ok_or_else(|| invalid("`workspace` must be a table."))?;
        let mut result = OpsPreferences::default();
        for (key, item) in table.iter() {
            match key {
                "read_markers" => {
                    self.read_markers()?;
                }
                "overview_panels" => result.overview_panels = Some(PanelSetting::observe(item)),
                "digest_panels" => result.digest_panels = Some(PanelSetting::observe(item)),
                "pinned" => {
                    let pins = item.as_array().ok_or_else(|| {
                        invalid("`workspace.pinned` must be an array of room UUIDs.")
                    })?;
                    for value in pins {
                        let id = value.as_str().filter(|id| uuid_like(id)).ok_or_else(|| {
                            invalid("`workspace.pinned` requires canonical room UUIDs.")
                        })?;
                        if !result.pinned.insert(id.into()) {
                            return Err(invalid(
                                "`workspace.pinned` contains a repeated room UUID.",
                            ));
                        }
                    }
                }
                "hidden" => {
                    let hidden = item.as_array().ok_or_else(|| {
                        invalid("`workspace.hidden` must be an array of room UUIDs.")
                    })?;
                    if hidden.len() > 1024 {
                        return Err(invalid("`workspace.hidden` allows at most 1024 squads."));
                    }
                    for value in hidden {
                        let id = value
                            .as_str()
                            .filter(|id| canonical_room_id(id))
                            .ok_or_else(|| {
                                invalid("`workspace.hidden` requires canonical room UUIDs.")
                            })?;
                        if !result.hidden.insert(id.into()) {
                            return Err(invalid(
                                "`workspace.hidden` contains a repeated room UUID.",
                            ));
                        }
                    }
                }
                "agent_pins" => {
                    let pins = item.as_array().ok_or_else(|| {
                        invalid("`workspace.agent_pins` must be an array of identity UUIDs.")
                    })?;
                    if pins.len() > 1024 {
                        return Err(invalid(
                            "`workspace.agent_pins` allows at most 1024 identities.",
                        ));
                    }
                    for value in pins {
                        let id = value.as_str().filter(|id| uuid_like(id)).ok_or_else(|| {
                            invalid("`workspace.agent_pins` requires canonical identity UUIDs.")
                        })?;
                        if !result.agent_pins.insert(id.into()) {
                            return Err(invalid(
                                "`workspace.agent_pins` contains a repeated identity UUID.",
                            ));
                        }
                    }
                }
                "sort" => {
                    result.sort = match item.as_str() {
                        Some("name") => OpsSort::Name,
                        Some("attention") => OpsSort::Attention,
                        Some("members") => OpsSort::Members,
                        _ => {
                            return Err(invalid(
                                "`workspace.sort` must be name, attention or members.",
                            ));
                        }
                    }
                }
                "overview_sort" => {
                    let key = item.as_str().ok_or_else(|| {
                        invalid("`workspace.overview_sort` must name a monitor column.")
                    })?;
                    let column = OverviewColumn::ALL
                        .into_iter()
                        .find(|column| column.key() == key)
                        .ok_or_else(|| {
                            invalid("`workspace.overview_sort` must name a monitor column.")
                        })?;
                    result.overview_sort = Some(OverviewSort::initial(column));
                }
                "overview_reverse" => {
                    if !item.is_bool() {
                        return Err(invalid(
                            "`workspace.overview_reverse` must be true or false.",
                        ));
                    }
                }
                "reverse" => {
                    result.reversed = item
                        .as_bool()
                        .ok_or_else(|| invalid("`workspace.reverse` must be true or false."))?
                }
                "tok_range" => {
                    result.tok_range =
                        item.as_str().and_then(TokRange::parse).ok_or_else(|| {
                            invalid("`workspace.tok_range` must be 1h, 3h, 6h, 1d or 7d.")
                        })?;
                }
                "digest_window" => {
                    result.digest_window = item
                        .as_str()
                        .and_then(super::DigestWindow::parse)
                        .ok_or_else(|| {
                            invalid("`workspace.digest_window` must be 24h, 7d or lifetime.")
                        })?;
                }
                "savings_unit" => {
                    result.savings_unit = item
                        .as_str()
                        .and_then(super::SavingsUnit::parse)
                        .ok_or_else(|| {
                            invalid("`workspace.savings_unit` must be tokens, percent or cost.")
                        })?;
                }
                "layout" => {
                    result.layout = match item.as_str() {
                        Some("auto") => OpsLayout::Auto,
                        Some("side") => OpsLayout::Side,
                        Some("stack") => OpsLayout::Stack,
                        _ => {
                            return Err(invalid("`workspace.layout` must be auto, side or stack."));
                        }
                    }
                }
                _ => {
                    return Err(invalid(format!(
                        "`workspace.{key}` is not a supported preference."
                    )));
                }
            }
        }
        if let Some(item) = table.get("overview_reverse") {
            let Some(order) = &mut result.overview_sort else {
                return Err(invalid(
                    "`workspace.overview_reverse` requires `workspace.overview_sort`.",
                ));
            };
            if item.as_bool() == Some(true) {
                *order = order.reverse();
            }
        }
        Ok(result)
    }

    /// The same format-preserving stale-file guard as other explicit settings
    /// writes. Presentation never creates a second preference file.
    pub fn save_workspace_preferences(
        &mut self,
        expected: &OpsPreferences,
        next: &OpsPreferences,
    ) -> Result<(), SquadError> {
        if self.workspace_preferences()? != *expected {
            return Err(invalid(
                "Workspace preferences changed; reload before saving.",
            ));
        }
        if next.pinned.iter().any(|id| !uuid_like(id)) {
            return Err(invalid("Workspace pins must be room UUIDs."));
        }
        if next.hidden.len() > 1024 || next.hidden.iter().any(|id| !canonical_room_id(id)) {
            return Err(invalid(
                "Hidden squads require canonical room UUIDs, at most 1024.",
            ));
        }
        if next.agent_pins.len() > 1024 || next.agent_pins.iter().any(|id| !uuid_like(id)) {
            return Err(invalid(
                "Agent pins require canonical identity UUIDs, at most 1024.",
            ));
        }
        validate_panel_save(&expected.overview_panels, &next.overview_panels)?;
        validate_panel_save(&expected.digest_panels, &next.digest_panels)?;
        self.write(|document| {
            if document.get("workspace").is_none() {
                document["workspace"] = Item::Table(Table::new());
            }
            write_panel(
                document,
                "overview_panels",
                &expected.overview_panels,
                &next.overview_panels,
            );
            write_panel(
                document,
                "digest_panels",
                &expected.digest_panels,
                &next.digest_panels,
            );
            let mut pins = toml_edit::Array::new();
            for id in &next.pinned {
                pins.push(id.as_str());
            }
            document["workspace"]["pinned"] = value(pins);
            if !next.hidden.is_empty() {
                let mut hidden = toml_edit::Array::new();
                for id in &next.hidden {
                    hidden.push(id.as_str());
                }
                document["workspace"]["hidden"] = value(hidden);
            } else if let Some(table) = document["workspace"].as_table_like_mut() {
                table.remove("hidden");
            }
            if !next.agent_pins.is_empty() {
                let mut pins = toml_edit::Array::new();
                for id in &next.agent_pins {
                    pins.push(id.as_str());
                }
                document["workspace"]["agent_pins"] = value(pins);
            } else if let Some(table) = document["workspace"].as_table_like_mut() {
                table.remove("agent_pins");
            }
            document["workspace"]["sort"] = value(match next.sort {
                OpsSort::Name => "name",
                OpsSort::Attention => "attention",
                OpsSort::Members => "members",
            });
            document["workspace"]["reverse"] = value(next.reversed);
            if let Some(order) = next.overview_sort {
                document["workspace"]["overview_sort"] = value(order.column.key());
                // Store whether the chosen column's initial direction is reversed.
                document["workspace"]["overview_reverse"] =
                    value(order.direction != OverviewSort::initial(order.column).direction);
            } else if let Some(table) = document["workspace"].as_table_like_mut() {
                table.remove("overview_sort");
                table.remove("overview_reverse");
            }
            document["workspace"]["tok_range"] = value(next.tok_range.argument());
            document["workspace"]["digest_window"] = value(next.digest_window.argument());
            document["workspace"]["savings_unit"] = value(next.savings_unit.argument());
            document["workspace"]["layout"] = value(match next.layout {
                OpsLayout::Auto => "auto",
                OpsLayout::Side => "side",
                OpsLayout::Stack => "stack",
            });
        })
    }

    /// Resolve j with the opening factory action and its concrete authored write owner.
    pub fn host_action_binding(
        &self,
        scope: OpsKeyScope,
        row: Option<&Value>,
        recipient: Option<&Value>,
        squad: Option<&str>,
        default_action: Option<&Action>,
    ) -> Result<(Option<Action>, String), SquadError> {
        let host = recipient.and_then(|row| row["pane"]["location"]["host"].as_str());
        let sections = squad
            .filter(|name| !name.is_empty())
            .map(|name| self.sections(name))
            .transpose()?
            .unwrap_or_default();
        let section = row.and_then(|row| {
            sections
                .iter()
                .enumerate()
                .find(|(_, section)| section.includes(row))
        });
        let keys = self.workspace_keys(scope, host, section.map(|(_, section)| &section.bind))?;
        let action = if keys
            .sources
            .get("j")
            .is_some_and(|source| source == "workspace default")
        {
            default_action.cloned()
        } else {
            keys.bindings.get("j").cloned()
        };
        // A retired authored override can mask a newly written global binding.
        // Repair that owner even when dispatch currently falls back to a factory action.
        let hosts = self.host_bindings()?;
        let section_bindings = section.map(|(_, section)| &section.bind);
        let mut source = if section_bindings.is_some_and(|bindings| bindings.contains_key("j"))
            || host.is_some_and(|host| hosts.get(host).is_some_and(|keys| keys.contains_key("j")))
            || self.configured_bindings()?.contains_key("j")
        {
            configured_binding_source("j", host, &hosts, section_bindings)
        } else {
            "workspace default".into()
        };
        if source == "selected squad section.bind" {
            source = format!(
                "squad.{}.section[{}].bind.j",
                squad.expect("a section has a squad"),
                section.expect("selected section source").0
            );
        }
        Ok((action, source))
    }

    pub fn workspace_keys(
        &self,
        scope: OpsKeyScope,
        host: Option<&str>,
        section: Option<&Bindings>,
    ) -> Result<WorkspaceKeys, SquadError> {
        let mut effective = defaults(scope);
        let mut sources: std::collections::BTreeMap<_, _> = effective
            .keys()
            .map(|key| (key.clone(), "workspace default".into()))
            .collect();
        let mut notices = self.workspace_migration_notices();
        let mut configured = self.configured_bindings()?;
        let hosts = self.host_bindings()?;
        if let Some(host) = host.and_then(|host| hosts.get(host)) {
            configured.extend(host.clone());
        }
        if let Some(section) = section {
            configured.extend(section.clone());
        }
        for (key, action) in configured {
            if let Err(error) = admit_action(&action, &format!("bind.{key}")) {
                notices.push(error.message);
                if matches!(
                    action.verb,
                    Verb::Retired(_) | Verb::Ui(OpsUiAction::AgentsFilter)
                ) {
                    effective.remove(&key);
                    sources.remove(&key);
                }
                continue;
            }
            let source = configured_binding_source(&key, host, &hosts, section);
            sources.insert(key.clone(), source);
            effective.insert(key, action);
        }
        Ok(WorkspaceKeys {
            bindings: effective,
            sources,
            notices,
        })
    }
}

fn configured_binding_source(
    key: &str,
    host: Option<&str>,
    hosts: &std::collections::BTreeMap<String, Bindings>,
    section: Option<&Bindings>,
) -> String {
    if section.is_some_and(|section| section.contains_key(key)) {
        "selected squad section.bind".into()
    } else if let Some(host) =
        host.filter(|host| hosts.get(*host).is_some_and(|keys| keys.contains_key(key)))
    {
        format!("bind.{host}.{key}")
    } else {
        format!("bind.{key}")
    }
}

fn panel_notice<P: Panel>(item: Option<&Item>, key: &str, notices: &mut Vec<String>) {
    if let Some(table) = item.and_then(Item::as_table_like) {
        for field in ["order", "hidden"] {
            if let Some(values) = table.get(field).and_then(Item::as_array) {
                for value in values {
                    if let Some(id) = value.as_str().filter(|id| P::RETIRED.contains(id)) {
                        notices.push(format!("`workspace.{key}.{field}`: '{id}' is retired; Digest has a fixed summary, TOK table and Exchanges. The authored value is preserved for migration."));
                    }
                }
            }
        }
        if key == "digest_panels" && table.get("order").is_some() {
            notices.push("`workspace.digest_panels.order` is retired; Digest sections have fixed positions. The authored value is preserved for migration.".into());
        }
    }
    if let Some(error) = item
        .map(PanelSetting::<P>::observe)
        .and_then(|setting| setting.error().map(str::to_owned))
    {
        notices.push(format!(
            "`workspace.{key}`: {error}; using dashboard defaults. The authored value is unchanged."
        ));
    }
}
fn validate_panel_save<P: Panel>(
    expected: &Option<PanelSetting<P>>,
    next: &Option<PanelSetting<P>>,
) -> Result<(), SquadError> {
    if expected != next && next.as_ref().is_some_and(|setting| !setting.writable()) {
        return Err(invalid(
            "An explicit panel Save must contain a complete valid policy.",
        ));
    }
    Ok(())
}
fn write_panel<P: Panel>(
    document: &mut DocumentMut,
    key: &str,
    expected: &Option<PanelSetting<P>>,
    next: &Option<PanelSetting<P>>,
) {
    if expected == next {
        return;
    }
    let workspace = document["workspace"]
        .as_table_like_mut()
        .expect("workspace validated");
    match next {
        None => {
            workspace.remove(key);
        }
        Some(setting) => {
            if let Some(item) = workspace.get_mut(key) {
                setting.write(item);
            } else {
                let mut item = Item::Table(Table::new());
                setting.write(&mut item);
                workspace.insert(key, item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_provenance_tracks_only_the_admitted_effective_merge() {
        let config =
            config("[bind]\no = \"open {pr_link}\"\n[bind.tmux]\no = \"open {review_link}\"\n");
        let section = [("o".into(), Action::parse("open {link}").unwrap())].into();
        let global = config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap();
        assert_eq!(global.sources["o"], "bind.o");
        let host = config
            .workspace_keys(OpsKeyScope::Member, Some("tmux"), None)
            .unwrap();
        assert_eq!(host.sources["o"], "bind.tmux.o");
        let selected = config
            .workspace_keys(OpsKeyScope::Member, Some("tmux"), Some(&section))
            .unwrap();
        assert_eq!(selected.sources["o"], "selected squad section.bind");
        assert_eq!(selected.bindings["o"].text, "open {link}");
        assert_eq!(selected.sources["j"], "workspace default");
    }
    fn config(text: &str) -> Config {
        Config {
            path: "/fixture/ops.toml".into(),
            original: None,
            document: text.parse().unwrap(),
            global_theme: Vec::new(),
            theme_error: None,
        }
    }
    #[test]
    fn real_defaults_and_overrides_share_one_effective_table() {
        let config = config("[bind]\nj='copy'\ns='talk'\n[bind.tmux]\nj='open'\n");
        let section: Bindings = [("j".into(), Action::parse("run /usr/bin/true").unwrap())].into();
        let defaults = config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap()
            .bindings;
        assert_eq!(defaults["j"].verb, Verb::Copy);
        assert_eq!(defaults["s"].verb, Verb::Talk);
        let hosted = config
            .workspace_keys(OpsKeyScope::Member, Some("tmux"), None)
            .unwrap()
            .bindings;
        assert_eq!(hosted["j"].verb, Verb::Open);
        let scoped = config
            .workspace_keys(OpsKeyScope::Member, Some("tmux"), Some(&section))
            .unwrap()
            .bindings;
        assert_eq!(scoped["j"].verb, Verb::Run);
        assert!(!scoped.contains_key("S")); // Authored keys still participate in admission.
        for (event, action) in scoped {
            assert!(!event.is_empty());
            assert!(!action.description().is_empty());
        }
    }
    #[test]
    fn agents_filter_default_yields_to_global_host_and_section_bindings() {
        let config = config("[bind]\nf='search'\n[bind.tmux]\nf='talk'\n");
        let section: Bindings = [("f".into(), Action::parse("run /usr/bin/true").unwrap())].into();
        for scope in [OpsKeyScope::Squad, OpsKeyScope::Member, OpsKeyScope::Pool] {
            assert_eq!(defaults(scope)["f"].text, "agents-filter");
            for (host, section, expected, source) in [
                (None, None, "search", "bind.f"),
                (Some("tmux"), None, "talk", "bind.tmux.f"),
                (
                    Some("tmux"),
                    Some(&section),
                    "run /usr/bin/true",
                    "selected squad section.bind",
                ),
            ] {
                let keys = config.workspace_keys(scope, host, section).unwrap();
                assert_eq!(keys.bindings["f"].text, expected);
                assert_eq!(keys.sources["f"], source);
            }
        }
    }
    #[test]
    fn agents_filter_default_yields_to_all_explicit_binding_scopes() {
        let config = config("[bind]\nv='search'\n[bind.tmux]\nv='talk'\n");
        for scope in [
            OpsKeyScope::Reader,
            OpsKeyScope::Member,
            OpsKeyScope::Squad,
            OpsKeyScope::Pool,
        ] {
            if scope.is_reader() {
                assert!(!defaults(scope).contains_key("f"));
            } else {
                assert_eq!(defaults(scope)["f"].text, "agents-filter");
            }
        }
        for scope in [
            OpsKeyScope::Reader,
            OpsKeyScope::Member,
            OpsKeyScope::Squad,
            OpsKeyScope::Pool,
        ] {
            assert_eq!(
                config.workspace_keys(scope, None, None).unwrap().bindings["v"].text,
                "search"
            );
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), None)
                    .unwrap()
                    .bindings["v"]
                    .text,
                "talk"
            );
            let section: Bindings =
                [("v".into(), Action::parse("run /usr/bin/true").unwrap())].into();
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), Some(&section))
                    .unwrap()
                    .bindings["v"]
                    .verb,
                Verb::Run
            );
        }
    }

    #[test]
    fn factory_t_switches_talk_and_measure_outside_readers_and_authored_t_keeps_its_precedence() {
        for scope in [
            OpsKeyScope::Squad,
            OpsKeyScope::Member,
            OpsKeyScope::Pool,
            OpsKeyScope::Reader,
        ] {
            let factory = config("").workspace_keys(scope, None, None).unwrap();
            if scope.is_reader() {
                assert!(!factory.bindings.contains_key("t"), "{scope:?}");
            } else {
                assert_eq!(factory.bindings["t"].text, "talk-measure", "{scope:?}");
            }
            let configured = config("[bind]\nt='talk'\n[bind.tmux]\nt='copy'\n");
            assert_eq!(
                configured
                    .workspace_keys(scope, None, None)
                    .unwrap()
                    .bindings["t"]
                    .verb,
                Verb::Talk
            );
            assert_eq!(
                configured
                    .workspace_keys(scope, Some("tmux"), None)
                    .unwrap()
                    .bindings["t"]
                    .verb,
                Verb::Copy
            );
            let section = [("t".into(), Action::parse("open").unwrap())].into();
            assert_eq!(
                configured
                    .workspace_keys(scope, Some("tmux"), Some(&section))
                    .unwrap()
                    .bindings["t"]
                    .verb,
                Verb::Open
            );
        }
    }

    #[test]
    fn reader_defaults_and_explicit_overrides_use_the_same_precedence() {
        let plain = config("")
            .workspace_keys(OpsKeyScope::Reader, None, None)
            .unwrap();
        for (event, name) in [
            ("j", "scroll-down"),
            ("k", "scroll-up"),
            ("pageup", "scroll-page-up"),
            ("end", "scroll-bottom"),
            ("enter", "talk"),
        ] {
            assert_eq!(plain.bindings[event].text, name);
        }
        assert_eq!(plain.bindings["p"].text, "pin");
        for key in ["left", "right", "t", "s", "x"] {
            assert!(!plain.bindings.contains_key(key), "{key}");
        }
        let config = config("[bind]\nj='copy'\n[bind.tmux]\nj='open'\n");
        let section: Bindings = [("j".into(), Action::parse("run /usr/bin/true").unwrap())].into();
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Reader, None, None)
                .unwrap()
                .bindings["j"]
                .verb,
            Verb::Copy
        );
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Reader, Some("tmux"), None)
                .unwrap()
                .bindings["j"]
                .verb,
            Verb::Open
        );
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Reader, Some("tmux"), Some(&section))
                .unwrap()
                .bindings["j"]
                .verb,
            Verb::Run
        );
    }
    #[test]
    fn reader_digit_overrides_are_active_and_leave_the_authored_config_unchanged() {
        let config = config("[bind]\n1='copy $name'\n2='agents'\n3='digest-dashboard'\n");
        let original = config.document.to_string();
        let reader = config
            .workspace_keys(OpsKeyScope::Reader, None, None)
            .unwrap();
        let list = config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap();
        for key in ["1", "2", "3"] {
            assert_eq!(reader.bindings[key], list.bindings[key]);
        }
        assert!(reader.notices.is_empty());
        assert_eq!(config.document.to_string(), original);
    }
    #[test]
    fn source_reader_and_destination_defaults_are_unique_and_yield_to_authored_keys() {
        for scope in [OpsKeyScope::Member, OpsKeyScope::Reader] {
            let defaults = defaults(scope);
            for (event, verb) in [
                ("1", "agents"),
                ("2", "digest-dashboard"),
                ("v", "view-reply"),
            ] {
                assert_eq!(defaults[event].text, verb);
                assert_eq!(
                    defaults
                        .values()
                        .filter(|action| action.text == verb)
                        .count(),
                    1
                );
            }
            let config = config("[bind]\nv='copy'\n[bind.tmux]\nv='open'\n");
            let section = [("v".into(), Action::parse("talk").unwrap())].into();
            assert_eq!(
                config.workspace_keys(scope, None, None).unwrap().bindings["v"].verb,
                Verb::Copy
            );
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), None)
                    .unwrap()
                    .bindings["v"]
                    .verb,
                Verb::Open
            );
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), Some(&section))
                    .unwrap()
                    .bindings["v"]
                    .verb,
                Verb::Talk
            );
        }
    }

    #[test]
    fn backtab_defaults_share_tab_action_per_scope_and_authored_bindings_win() {
        for scope in [
            OpsKeyScope::Squad,
            OpsKeyScope::Member,
            OpsKeyScope::Pool,
            OpsKeyScope::Reader,
        ] {
            let base = config("").workspace_keys(scope, None, None).unwrap();
            assert_eq!(base.bindings["tab"].verb, Verb::NextPane);
            assert_eq!(base.bindings["backtab"].verb, Verb::NextPane);
            let configured = config("[bind]\nbacktab='help'\n")
                .workspace_keys(scope, None, None)
                .unwrap();
            assert_eq!(configured.bindings["tab"].verb, Verb::NextPane);
            assert_eq!(configured.bindings["backtab"].text, "help");
            assert_eq!(configured.sources["backtab"], "bind.backtab");
        }
    }
    #[test]
    fn every_approved_scope_has_its_real_keys_without_legacy_fallbacks() {
        let config = config("");
        let squad = config
            .workspace_keys(OpsKeyScope::Squad, None, None)
            .unwrap()
            .bindings;
        assert_eq!(squad["n"].text, "create-squad");
        assert_eq!(squad["p"].text, "pin");
        let member = config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap()
            .bindings;
        assert_eq!(member["r"].text, "rename-member");
        assert_eq!(member["D"].text, "digest");
        assert_eq!(member["x"].text, "remove-membership");
        assert_eq!(squad["w"].text, "workspace-snapshots");
        assert!(!member.contains_key("A"));
        assert_eq!(squad["left"].text, "fold");
        assert_eq!(squad["right"].text, "expand");
    }
    #[test]
    fn snapshot_default_yields_to_explicit_global_host_and_section_bindings() {
        let config = config("[bind]\nw='talk'\n[bind.tmux]\nw='open'\n");
        let section: Bindings = [("w".into(), Action::parse("copy").unwrap())].into();
        for scope in [OpsKeyScope::Squad, OpsKeyScope::Member, OpsKeyScope::Pool] {
            assert_eq!(defaults(scope)["w"].text, "workspace-snapshots");
            let global = config.workspace_keys(scope, None, None).unwrap();
            assert_eq!(global.bindings["w"].verb, Verb::Talk);
            assert!(global.notices.is_empty());
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), None)
                    .unwrap()
                    .bindings["w"]
                    .verb,
                Verb::Open
            );
            assert_eq!(
                config
                    .workspace_keys(scope, Some("tmux"), Some(&section))
                    .unwrap()
                    .bindings["w"]
                    .verb,
                Verb::Copy
            );
        }
        assert_eq!(
            defaults(OpsKeyScope::Reader)["w"].text,
            "workspace-snapshots"
        );
    }
    #[test]
    fn rename_default_yields_to_explicit_global_host_and_section_bindings() {
        let config = config("[bind]\nr='talk'\n[bind.tmux]\nr='open'\n");
        let section: Bindings = [("r".into(), Action::parse("copy").unwrap())].into();
        assert_eq!(defaults(OpsKeyScope::Member)["r"].text, "rename-member");
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Member, None, None)
                .unwrap()
                .bindings["r"]
                .verb,
            Verb::Talk
        );
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Member, Some("tmux"), None)
                .unwrap()
                .bindings["r"]
                .verb,
            Verb::Open
        );
        assert_eq!(
            config
                .workspace_keys(OpsKeyScope::Member, Some("tmux"), Some(&section))
                .unwrap()
                .bindings["r"]
                .verb,
            Verb::Copy
        );
        for scope in [OpsKeyScope::Squad, OpsKeyScope::Pool, OpsKeyScope::Reader] {
            assert!(!defaults(scope).contains_key("r"));
        }
    }
    #[test]
    fn retired_bindings_are_reported_without_rewriting_config() {
        for binding in [
            "A='ask-lead'",
            "s='home-pick'",
            "L='jump lead'",
            "V='view'",
            "N='notes'",
            "f='overview-filter'",
        ] {
            let config = config(&format!("[bind]\n{binding}"));
            let before = config.document.to_string();
            assert!(
                config
                    .workspace_keys(OpsKeyScope::Member, None, None)
                    .unwrap()
                    .notices
                    .len()
                    == 1
            );
            assert_eq!(config.document.to_string(), before);
        }
    }
    #[test]
    fn retired_overview_filter_reports_migration_without_retargeting_the_key() {
        for text in [
            "[bind]\nf = 'overview-filter'",
            "[bind.tmux]\nf = 'overview-filter'",
        ] {
            let config = config(text);
            let before = config.document.to_string();
            let keys = config
                .workspace_keys(OpsKeyScope::Member, Some("tmux"), None)
                .unwrap();
            assert!(keys.notices.iter().any(
                |notice| notice.contains("overview-filter") && notice.contains("agents-filter")
            ));
            assert!(!keys.bindings.contains_key("f"));
            assert_eq!(config.document.to_string(), before);
            assert!(
                !keys
                    .bindings
                    .values()
                    .any(|action| action.text == "overview-view")
            );
        }
    }
    #[test]
    fn retired_factory_and_tab_settings_have_located_notices_without_rewriting_user_text() {
        let config = config(
            "[board]\nview='team'\nhome_replies=true\n[tabs]\norder=['product']\n[squad.product.board]\ndirection='left-right'\ntoken_rate={enabled=false}\n[squad.product]\nlayout='crew'\n",
        );
        let before = config.document.to_string();
        let notices = config.workspace_migration_notices();
        for path in [
            "board.view",
            "board.home_replies",
            "tabs.order",
            "squad.product.board.direction",
            "squad.product.layout",
        ] {
            assert!(
                notices.iter().any(|notice| notice.contains(path)),
                "missing {path}"
            );
        }
        assert!(!notices.iter().any(|notice| notice.contains("token_rate")));
        assert_eq!(
            config.layout("product").unwrap(),
            super::super::Layout::Crew
        );
        assert_eq!(config.document.to_string(), before);
    }
    #[test]
    fn retired_notebook_settings_and_handlers_remain_authored_with_located_notices() {
        let config =
            config("[squad.product.notes]\nrender='plain'\n[links]\nissue='run echo {path}'\n");
        let before = config.document.to_string();
        let notices = config.workspace_migration_notices();
        for path in ["squad.product.notes.render", "links.issue"] {
            assert!(notices.iter().any(|notice| notice.contains(path)), "{path}");
        }
        assert_eq!(
            config.notes_render("product").unwrap(),
            crate::config::NotesRender::Plain
        );
        assert!(config.links().unwrap().contains_key("issue"));
        assert_eq!(config.document.to_string(), before);
    }

    struct FileFixture(PathBuf);
    impl FileFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "ops-workspace-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::write(
                root.join("ops.toml"),
                "# preserve user text\n[bind]\nj='copy'\n[squad.other]\nlayout='crew'\n",
            )
            .unwrap();
            Self(root)
        }
        fn config(&self) -> Config {
            Config::read(self.0.join("ops.toml")).unwrap()
        }
    }
    impl Drop for FileFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn preferences_reload_without_mutating_membership_or_unrelated_user_scopes() {
        let fixture = FileFixture::new();
        let mut config = fixture.config();
        let expected = config.workspace_preferences().unwrap();
        let next = OpsPreferences {
            pinned: ["11111111-1111-4111-8111-111111111111".into()].into(),
            hidden: BTreeSet::new(),
            agent_pins: BTreeSet::new(),
            sort: OpsSort::Attention,
            reversed: true,
            layout: OpsLayout::Stack,
            tok_range: TokRange::default(),
            digest_window: super::super::DigestWindow::default(),
            savings_unit: super::super::SavingsUnit::Cost,
            overview_sort: None,
            overview_panels: None,
            digest_panels: None,
        };
        config.save_workspace_preferences(&expected, &next).unwrap();
        let reloaded = fixture.config();
        assert_eq!(reloaded.workspace_preferences().unwrap(), next);
        let bytes = fs::read_to_string(reloaded.path()).unwrap();
        assert!(bytes.contains("# preserve user text"));
        assert!(bytes.contains("[bind]\nj='copy'"));
        assert!(bytes.contains("[squad.other]\nlayout='crew'"));
        assert_eq!(
            reloaded
                .workspace_keys(OpsKeyScope::Member, None, None)
                .unwrap()
                .bindings["j"]
                .verb,
            Verb::Copy
        );
    }
    #[test]
    fn shared_tok_range_roundtrips_without_changing_other_workspace_settings() {
        let fixture = FileFixture::new();
        let mut config = fixture.config();
        let initial = config.workspace_preferences().unwrap();
        let mut expected = initial.clone();
        for range in TokRange::ALL {
            let next = OpsPreferences {
                tok_range: range,
                ..expected.clone()
            };
            config.save_workspace_preferences(&expected, &next).unwrap();
            let loaded = fixture.config().workspace_preferences().unwrap();
            assert_eq!(loaded, next);
            assert_eq!(loaded.digest_window, initial.digest_window);
            assert_eq!(loaded.layout, initial.layout);
            assert!(
                fs::read_to_string(config.path())
                    .unwrap()
                    .contains("# preserve user text")
            );
            expected = loaded;
        }
        assert_eq!(expected.tok_range.next(), TokRange::Hour);
        for invalid in ["24h", "30d", "0h", "2h", "1H", ""] {
            assert_eq!(TokRange::parse(invalid), None);
        }
        fs::write(config.path(), "# untouched\n[workspace]\ntok_range='30d'\n").unwrap();
        let before = fs::read(config.path()).unwrap();
        assert!(fixture.config().workspace_preferences().is_err());
        assert_eq!(fs::read(config.path()).unwrap(), before);
    }

    #[test]
    fn retired_digest_panels_report_locations_and_unrelated_saves_preserve_bytes() {
        let fixture = FileFixture::new();
        let authored = "# preserve user text\n[workspace]\ndigest_panels = { order=['report','list','status','usage','subscriptions','chart'], hidden=['usage','chart'], details=false } # legacy\n";
        fs::write(fixture.0.join("ops.toml"), authored).unwrap();
        let mut config = fixture.config();
        let expected = config.workspace_preferences().unwrap();
        let panel_bytes = config.document["workspace"]["digest_panels"].to_string();
        let mut next = expected.clone();
        next.tok_range = TokRange::Week;
        config.save_workspace_preferences(&expected, &next).unwrap();
        let reloaded = fixture.config();
        assert_eq!(
            reloaded.document["workspace"]["digest_panels"].to_string(),
            panel_bytes
        );
        assert!(
            !reloaded
                .workspace_preferences()
                .unwrap()
                .digest_panels
                .unwrap()
                .effective()
                .shown(DigestPanel::Chart)
        );
        let notices = reloaded.workspace_migration_notices();
        for id in ["report", "status", "usage"] {
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.contains("workspace.digest_panels.order")
                        && notice.contains(id))
            );
        }
    }

    #[test]
    fn digest_window_uses_one_workspace_preference_and_preserves_authored_text() {
        use super::super::DigestWindow;
        let fixture = FileFixture::new();
        let mut config = fixture.config();
        assert_eq!(
            config.workspace_preferences().unwrap().digest_window,
            DigestWindow::Day
        );
        for window in [DigestWindow::Week, DigestWindow::AllTime, DigestWindow::Day] {
            let expected = config.workspace_preferences().unwrap();
            let next = OpsPreferences {
                digest_window: window,
                ..expected.clone()
            };
            config.save_workspace_preferences(&expected, &next).unwrap();
            config = fixture.config();
            assert_eq!(config.workspace_preferences().unwrap(), next);
            let bytes = fs::read_to_string(config.path()).unwrap();
            assert!(bytes.contains("# preserve user text"));
            assert_eq!(config.document["bind"]["j"].as_str(), Some("copy"));
        }
    }

    #[test]
    fn savings_unit_is_one_additive_preference_beside_the_window() {
        use super::super::{DigestWindow, SavingsUnit};
        let fixture = FileFixture::new();
        let mut config = fixture.config();
        assert_eq!(
            config.workspace_preferences().unwrap().savings_unit,
            SavingsUnit::Tokens
        );
        for unit in [SavingsUnit::Percent, SavingsUnit::Cost, SavingsUnit::Tokens] {
            let expected = config.workspace_preferences().unwrap();
            let next = OpsPreferences {
                savings_unit: unit,
                ..expected.clone()
            };
            config.save_workspace_preferences(&expected, &next).unwrap();
            config = fixture.config();
            let loaded = config.workspace_preferences().unwrap();
            assert_eq!(loaded, next);
            assert_eq!(loaded.digest_window, expected.digest_window);
            assert!(
                fs::read_to_string(config.path())
                    .unwrap()
                    .contains("# preserve user text")
            );
        }
        for text in [
            "[workspace]\nsavings_unit='money'",
            "[workspace]\nsavings_unit=1",
        ] {
            let bad = self::config(text);
            let before = bad.document.to_string();
            assert!(bad.workspace_preferences().is_err());
            assert_eq!(bad.document.to_string(), before);
        }
        // The order is tokens, percent, cost; lifetime has no percent.
        let order = |window| {
            let mut seen = vec![SavingsUnit::Tokens];
            while seen.len() < 4 {
                let next = seen.last().unwrap().next(window);
                if next == SavingsUnit::Tokens {
                    break;
                }
                seen.push(next);
            }
            seen
        };
        assert_eq!(
            order(DigestWindow::Day),
            [SavingsUnit::Tokens, SavingsUnit::Percent, SavingsUnit::Cost]
        );
        assert_eq!(
            order(DigestWindow::AllTime),
            [SavingsUnit::Tokens, SavingsUnit::Cost]
        );
        // A saved percent reads as tokens for lifetime, and the next step is cost.
        assert_eq!(
            SavingsUnit::Percent.shown(DigestWindow::AllTime),
            SavingsUnit::Tokens
        );
        assert_eq!(
            SavingsUnit::Percent.next(DigestWindow::AllTime),
            SavingsUnit::Cost
        );
        assert_eq!(
            SavingsUnit::Percent.shown(DigestWindow::Week),
            SavingsUnit::Percent
        );
    }

    #[test]
    fn malformed_or_stale_digest_window_never_rewrites_the_config() {
        for text in [
            "[workspace]\ndigest_window='1h'",
            "[workspace]\ndigest_window=7",
        ] {
            let config = config(text);
            let before = config.document.to_string();
            assert!(config.workspace_preferences().is_err());
            assert_eq!(config.document.to_string(), before);
        }
        let fixture = FileFixture::new();
        let mut first = fixture.config();
        let mut stale = fixture.config();
        let expected = first.workspace_preferences().unwrap();
        let next = OpsPreferences {
            digest_window: super::super::DigestWindow::Week,
            ..expected.clone()
        };
        first.save_workspace_preferences(&expected, &next).unwrap();
        let before = fs::read(first.path()).unwrap();
        assert!(
            stale
                .save_workspace_preferences(&expected, &OpsPreferences::default())
                .is_err()
        );
        assert_eq!(fs::read(first.path()).unwrap(), before);
    }

    #[test]
    fn overview_order_round_trips_every_column_and_direction_then_clears() {
        let fixture = FileFixture::new();
        let mut config = fixture.config();
        assert_eq!(config.workspace_preferences().unwrap().overview_sort, None);
        for column in OverviewColumn::ALL {
            for order in [
                OverviewSort::initial(column),
                OverviewSort::initial(column).reverse(),
            ] {
                let expected = config.workspace_preferences().unwrap();
                let next = OpsPreferences {
                    overview_sort: Some(order),
                    ..expected.clone()
                };
                config.save_workspace_preferences(&expected, &next).unwrap();
                config = fixture.config();
                assert_eq!(config.workspace_preferences().unwrap(), next);
                assert_eq!(config.document["bind"]["j"].as_str(), Some("copy"));
                assert_eq!(
                    config.document["squad"]["other"]["layout"].as_str(),
                    Some("crew")
                );
            }
        }
        let expected = config.workspace_preferences().unwrap();
        let next = OpsPreferences {
            overview_sort: None,
            ..expected.clone()
        };
        config.save_workspace_preferences(&expected, &next).unwrap();
        let config = fixture.config();
        assert_eq!(config.workspace_preferences().unwrap(), next);
        assert!(config.document["workspace"].get("overview_sort").is_none());
        assert!(
            config.document["workspace"]
                .get("overview_reverse")
                .is_none()
        );
    }

    #[test]
    fn overview_order_admission_and_stale_write_preserve_the_authored_file() {
        for text in [
            "[workspace]\noverview_sort='invented'",
            "[workspace]\noverview_sort=7",
            "[workspace]\noverview_reverse=true",
            "[workspace]\noverview_sort='agent'\noverview_reverse='yes'",
        ] {
            let config = config(text);
            let before = config.document.to_string();
            assert!(config.workspace_preferences().is_err(), "{text}");
            assert_eq!(config.document.to_string(), before);
        }
        let fixture = FileFixture::new();
        let mut first = fixture.config();
        let mut stale = fixture.config();
        let expected = first.workspace_preferences().unwrap();
        let next = OpsPreferences {
            overview_sort: Some(OverviewSort::initial(OverviewColumn::TokensPerHour)),
            ..expected.clone()
        };
        first.save_workspace_preferences(&expected, &next).unwrap();
        let before = fs::read(first.path()).unwrap();
        assert_eq!(
            stale
                .save_workspace_preferences(&expected, &next)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read(first.path()).unwrap(), before);
    }

    #[test]
    fn preference_compare_and_swap_refuses_stale_model_and_intervening_file_edits() {
        let fixture = FileFixture::new();
        let mut first = fixture.config();
        let mut second = fixture.config();
        let expected = first.workspace_preferences().unwrap();
        let next = OpsPreferences {
            sort: OpsSort::Members,
            ..expected.clone()
        };
        first.save_workspace_preferences(&expected, &next).unwrap();
        let before = fs::read(first.path()).unwrap();
        assert_eq!(
            second
                .save_workspace_preferences(&expected, &next)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read(first.path()).unwrap(), before);
        let mut reloaded = fixture.config();
        assert!(
            reloaded
                .save_workspace_preferences(&expected, &next)
                .is_err()
        );
        assert_eq!(fs::read(first.path()).unwrap(), before);
    }
    #[test]
    fn hidden_squads_round_trip_unknown_ids_and_clear_without_removing_pins() {
        let fixture = FileFixture::new();
        let mut cfg = fixture.config();
        let untouched = fs::read(cfg.path()).unwrap();
        let original = cfg.workspace_preferences().unwrap();
        assert!(original.hidden.is_empty());
        assert_eq!(fs::read(cfg.path()).unwrap(), untouched);

        // Canonical room IDs do not require a live room to remain saved.
        let id = "11111111-1111-4111-8111-111111111111";
        let unknown = "22222222-2222-4222-8222-222222222222";
        let mut next = original.clone();
        next.pinned.insert(id.into());
        next.hidden.extend([id.into(), unknown.into()]);
        cfg.save_workspace_preferences(&original, &next).unwrap();
        assert_eq!(fixture.config().workspace_preferences().unwrap(), next);
        let bytes = fs::read_to_string(cfg.path()).unwrap();
        assert!(bytes.contains("# preserve user text"));
        assert!(bytes.contains("[bind]\nj='copy'"));
        assert!(bytes.contains("[squad.other]\nlayout='crew'"));

        let hidden = next.clone();
        next.reversed = true;
        cfg.save_workspace_preferences(&hidden, &next).unwrap();
        assert_eq!(fixture.config().workspace_preferences().unwrap(), next);
        let hidden = next.clone();
        next.hidden.clear();
        cfg.save_workspace_preferences(&hidden, &next).unwrap();
        let cfg = fixture.config();
        assert_eq!(cfg.workspace_preferences().unwrap(), next);
        assert!(cfg.document["workspace"].get("hidden").is_none());
        assert!(next.pinned.contains(id));
    }

    #[test]
    fn hidden_squads_validate_canonical_unique_bounded_room_ids_before_writing() {
        let id = "11111111-1111-4111-8111-111111111111";
        let ids = |count| {
            (0..count)
                .map(|n| format!("'{n:08x}-1111-4111-8111-111111111111'"))
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(
            config(&format!("[workspace]\nhidden=[{}]", ids(1024)))
                .workspace_preferences()
                .unwrap()
                .hidden
                .len(),
            1024
        );
        for hidden in [
            "'not-an-array'".to_owned(),
            "[42]".into(),
            "['not-an-id']".into(),
            "['AAAAAAAA-1111-4111-8111-111111111111']".into(),
            format!("['{id}', '{id}']"),
            format!("[{}]", ids(1025)),
        ] {
            assert!(
                config(&format!("[workspace]\nhidden={hidden}"))
                    .workspace_preferences()
                    .is_err(),
                "{hidden}"
            );
        }

        let fixture = FileFixture::new();
        let mut cfg = fixture.config();
        let original = cfg.workspace_preferences().unwrap();
        let before = fs::read(cfg.path()).unwrap();
        for hidden in [
            ["not-an-id".into()].into(),
            (0..1025)
                .map(|n| format!("{n:08x}-1111-4111-8111-111111111111"))
                .collect(),
        ] {
            let next = OpsPreferences {
                hidden,
                ..original.clone()
            };
            assert!(cfg.save_workspace_preferences(&original, &next).is_err());
            assert_eq!(fs::read(cfg.path()).unwrap(), before);
        }
    }

    #[test]
    fn hidden_squad_writes_refuse_stale_preferences_and_external_file_edits() {
        let fixture = FileFixture::new();
        let mut first = fixture.config();
        let mut stale = fixture.config();
        let original = first.workspace_preferences().unwrap();
        let next = OpsPreferences {
            hidden: ["11111111-1111-4111-8111-111111111111".into()].into(),
            ..original.clone()
        };
        first.save_workspace_preferences(&original, &next).unwrap();
        let published = fs::read(first.path()).unwrap();
        assert_eq!(
            stale
                .save_workspace_preferences(&original, &next)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read(first.path()).unwrap(), published);
        let mut reloaded = fixture.config();
        assert!(
            reloaded
                .save_workspace_preferences(&original, &next)
                .is_err()
        );
        assert_eq!(fs::read(first.path()).unwrap(), published);

        let mut cleared = next.clone();
        cleared.hidden.clear();
        let edited = format!(
            "{}\n# external edit\n",
            String::from_utf8(published).unwrap()
        );
        fs::write(first.path(), &edited).unwrap();
        assert_eq!(
            reloaded
                .save_workspace_preferences(&next, &cleared)
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read_to_string(first.path()).unwrap(), edited);
    }

    #[test]
    fn member_defaults_keep_view_reply_and_pin() {
        assert_eq!(defaults(OpsKeyScope::Member)["v"].text, "view-reply");
        assert_eq!(defaults(OpsKeyScope::Member)["p"].text, "pin");
    }

    #[test]
    fn agent_pins_round_trip_clear_and_leave_unrelated_content() {
        let fixture = FileFixture::new();
        let mut cfg = fixture.config();
        let original = cfg.workspace_preferences().unwrap();
        let mut next = original.clone();
        next.agent_pins
            .insert("11111111-1111-4111-8111-111111111111".into());
        cfg.save_workspace_preferences(&original, &next).unwrap();
        assert_eq!(fixture.config().workspace_preferences().unwrap(), next);
        let bytes = fs::read_to_string(cfg.path()).unwrap();
        assert!(bytes.contains("[squad.other]\nlayout='crew'"));
        let pinned = next.clone();
        next.agent_pins.clear();
        cfg.save_workspace_preferences(&pinned, &next).unwrap();
        assert!(
            !fs::read_to_string(cfg.path())
                .unwrap()
                .contains("agent_pins")
        );
        next.reversed = true;
        let before = cfg.workspace_preferences().unwrap();
        cfg.save_workspace_preferences(&before, &next).unwrap();
        assert!(
            !fs::read_to_string(cfg.path())
                .unwrap()
                .contains("agent_pins")
        );
    }
    #[test]
    fn agent_pin_validation_and_concurrent_edit_are_not_truncated() {
        let id = "11111111-1111-4111-8111-111111111111";
        for pins in [
            "[42]".to_owned(),
            "['not-an-id']".into(),
            format!("['{id}', '{id}']"),
            format!(
                "[{}]",
                (0..1025)
                    .map(|n| format!("'{n:08x}-1111-4111-8111-111111111111'"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ] {
            assert!(
                config(&format!("[workspace]\nagent_pins={pins}"))
                    .workspace_preferences()
                    .is_err()
            );
        }
        assert!(
            config("[workspace]\nfuture='unknown'")
                .workspace_preferences()
                .unwrap_err()
                .message
                .contains("is not a supported preference")
        );
        let fixture = FileFixture::new();
        let mut first = fixture.config();
        let original = first.workspace_preferences().unwrap();
        let mut second = fixture.config();
        let mut pinned = original.clone();
        pinned.agent_pins.insert(id.into());
        second
            .save_workspace_preferences(&original, &pinned)
            .unwrap();
        let after = fs::read(first.path()).unwrap();
        assert!(
            first
                .save_workspace_preferences(&original, &pinned)
                .is_err()
        );
        assert_eq!(fs::read(first.path()).unwrap(), after);
        let mut too_many = pinned.clone();
        too_many.agent_pins = (0..1025)
            .map(|n| format!("{n:08x}-1111-4111-8111-111111111111"))
            .collect();
        assert!(
            second
                .save_workspace_preferences(&pinned, &too_many)
                .is_err()
        );
        assert_eq!(fs::read(second.path()).unwrap(), after);
    }
    #[test]
    fn overview_sort_keys_are_authored_only() {
        let cfg = config("[bind]\ns='sort'\nS='reverse-sort'");
        let keys = cfg.workspace_keys(OpsKeyScope::Member, None, None).unwrap();
        assert_eq!(keys.bindings["s"].text, "sort");
        assert_eq!(keys.bindings["S"].text, "reverse-sort");
    }
    #[test]
    fn panel_policy_errors_fall_back_with_one_notice_without_relaxing_other_keys() {
        for field in [
            "overview_panels = 5",
            "overview_panels = { hidden = ['future'] }",
            "overview_panels = { conversation = false, hidden = ['monitor'] }",
        ] {
            let cfg = config(&format!("[workspace]\n{field}\nsort='attention'\n"));
            let preferences = cfg.workspace_preferences().unwrap();
            assert_eq!(preferences.sort, OpsSort::Attention);
            assert_eq!(
                preferences.overview_panels.as_ref().unwrap().effective(),
                OverviewPanels::default()
            );
            let notices = cfg.workspace_migration_notices();
            assert_eq!(
                notices
                    .iter()
                    .filter(|notice| notice.contains("workspace.overview_panels"))
                    .count(),
                1
            );
            assert!(preferences.digest_panels.is_none());
        }
        for field in ["sort = 5", "unknown = true", "agent_pins = true"] {
            assert!(
                config(&format!("[workspace]\noverview_panels=5\n{field}\n"))
                    .workspace_preferences()
                    .is_err()
            );
        }
    }
    #[test]
    fn unrelated_saves_preserve_absent_partial_older_and_invalid_panel_observations() {
        for panel in [
            "",
            "overview_panels = 5\n",
            "overview_panels = { hidden = [] } # inline\n",
            "overview_panels = { order = ['monitor'], hidden = [], conversation = true } # older order\n",
            "[workspace.overview_panels] # policy\nhidden = [] # partial\n",
        ] {
            let fixture = FileFixture::new();
            let text = format!("# user text\n[workspace]\nsort='name'\n{panel}");
            fs::write(fixture.0.join("ops.toml"), &text).unwrap();
            let mut cfg = fixture.config();
            let before = cfg.document["workspace"].get("overview_panels").cloned();
            let expected = cfg.workspace_preferences().unwrap();
            let mut next = expected.clone();
            next.reversed = true;
            cfg.save_workspace_preferences(&expected, &next).unwrap();
            let reloaded = fixture.config();
            assert_eq!(reloaded.workspace_preferences().unwrap(), next);
            assert_eq!(
                reloaded.document["workspace"]
                    .get("overview_panels")
                    .map(ToString::to_string),
                before.as_ref().map(ToString::to_string)
            );
            assert!(
                reloaded.document["workspace"]
                    .get("digest_panels")
                    .is_none()
            );
            assert!(
                fs::read_to_string(cfg.path())
                    .unwrap()
                    .contains("# user text")
            );
        }
    }
    #[test]
    fn panel_save_writes_full_policy_keeps_form_and_only_reset_removes_it() {
        for text in [
            "[workspace]\noverview_panels = {} # keep\n",
            "[workspace.overview_panels] # keep\n",
        ] {
            let fixture = FileFixture::new();
            fs::write(fixture.0.join("ops.toml"), text).unwrap();
            let mut cfg = fixture.config();
            let inline = cfg.document["workspace"]["overview_panels"].is_inline_table();
            let expected = cfg.workspace_preferences().unwrap();
            let mut next = expected.clone();
            next.overview_panels = Some(PanelSetting::configured(OverviewPanels::default()));
            cfg.save_workspace_preferences(&expected, &next).unwrap();
            let reloaded = fixture.config();
            assert_eq!(reloaded.workspace_preferences().unwrap(), next);
            let item = &reloaded.document["workspace"]["overview_panels"];
            assert_eq!(item.is_inline_table(), inline);
            for key in ["order", "hidden", "conversation"] {
                assert!(item.get(key).is_some());
            }
            assert!(fs::read_to_string(cfg.path()).unwrap().contains("# keep"));
            let explicit = next.clone();
            next.overview_panels = None;
            cfg.save_workspace_preferences(&explicit, &next).unwrap();
            let bytes = fs::read_to_string(cfg.path()).unwrap();
            assert!(!bytes.contains("overview_panels"));
        }
    }
    #[test]
    fn malformed_policy_change_is_stale_even_when_both_render_defaults() {
        let fixture = FileFixture::new();
        fs::write(
            fixture.0.join("ops.toml"),
            "[workspace]\noverview_panels = 5\n",
        )
        .unwrap();
        let expected = fixture.config().workspace_preferences().unwrap();
        let mut next = expected.clone();
        next.overview_panels = Some(PanelSetting::configured(OverviewPanels::default()));
        fs::write(
            fixture.0.join("ops.toml"),
            "[workspace]\noverview_panels = 6\n",
        )
        .unwrap();
        let mut fresh = fixture.config();
        let before = fs::read(fresh.path()).unwrap();
        assert!(fresh.save_workspace_preferences(&expected, &next).is_err());
        assert_eq!(fs::read(fresh.path()).unwrap(), before);
        let current = fresh.workspace_preferences().unwrap();
        let mut reset = current.clone();
        reset.overview_panels = None;
        fresh.save_workspace_preferences(&current, &reset).unwrap();
        assert!(
            !fs::read_to_string(fresh.path())
                .unwrap()
                .contains("overview_panels")
        );
    }
    #[test]
    fn both_dashboard_policies_round_trip_inside_an_inline_workspace() {
        let fixture = FileFixture::new();
        fs::write(
            fixture.0.join("ops.toml"),
            "workspace = { sort = 'name' } # workspace\n",
        )
        .unwrap();
        let mut cfg = fixture.config();
        let expected = cfg.workspace_preferences().unwrap();
        let mut next = expected.clone();
        let mut digest = DigestPanels::default();
        digest.set_shown(DigestPanel::Chart, false).unwrap();
        next.digest_panels = Some(PanelSetting::configured(digest));
        next.overview_panels = Some(PanelSetting::configured(OverviewPanels::default()));
        cfg.save_workspace_preferences(&expected, &next).unwrap();
        assert_eq!(fixture.config().workspace_preferences().unwrap(), next);
        assert!(fixture.config().document["workspace"].is_inline_table());
        assert!(
            fs::read_to_string(cfg.path())
                .unwrap()
                .contains("# workspace")
        );
    }
    #[test]
    fn retired_named_agents_mode_reports_a_located_notice_without_default_retargeting() {
        for text in [
            "[bind]\nf = 'agents-filter needs-you'",
            "[bind.tmux]\nf = 'agents-filter needs-you'",
        ] {
            let config = config(text);
            let before = config.document.to_string();
            let keys = config
                .workspace_keys(OpsKeyScope::Squad, Some("tmux"), None)
                .unwrap();
            assert!(keys.notices.iter().any(|notice| notice.contains("bind.f")
                && notice.contains("retired")
                && notice.contains("All and Pinned")));
            assert!(
                !keys.bindings.contains_key("f"),
                "a retired explicit choice must not fall through to the new cycle"
            );
            assert_eq!(config.document.to_string(), before);
        }
    }
}
