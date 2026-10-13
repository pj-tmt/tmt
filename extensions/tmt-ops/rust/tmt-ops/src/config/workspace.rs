//! The effective list action table. Dispatch, footer and help consume the same
//! result after default/global/host/section precedence, never independent hints.
use super::*;
use crate::action::{Action, Verb};
use std::collections::BTreeSet;

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpsPreferences {
    pub pinned: BTreeSet<String>,
    pub sort: OpsSort,
    pub reversed: bool,
    pub layout: OpsLayout,
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
    Overview,
    Squad,
    Member,
    Pool,
}

pub struct WorkspaceKeys {
    pub bindings: Bindings,
    pub notices: Vec<String>,
}

fn defaults(scope: OpsKeyScope) -> Bindings {
    let mut entries = vec![
        ("1", "overview"),
        ("2", "agents"),
        ("3", "digest-dashboard"),
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
        ("ctrl-r", "refresh"),
        (",", "settings"),
        ("c", "timers"),
        ("n", "create-squad"),
    ];
    match scope {
        OpsKeyScope::Overview => entries.extend([
            ("t", "talk"),
            ("j", "jump"),
            ("o", "open"),
            ("s", "sort"),
            ("S", "reverse-sort"),
            ("p", "pin"),
            ("m", "manage"),
            ("x", "remove-membership"),
            ("D", "digest"),
        ]),
        OpsKeyScope::Squad => {
            entries.extend([("p", "pin"), ("m", "manage"), ("n", "create-squad")])
        }
        OpsKeyScope::Member => entries.extend([
            ("t", "talk"),
            ("j", "jump"),
            ("o", "open"),
            ("x", "remove-membership"),
            ("m", "move-membership"),
            ("D", "digest"),
        ]),
        OpsKeyScope::Pool => entries.extend([("m", "manage"), ("n", "create-squad")]),
    }
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

fn check(action: &Action, path: &str) -> Result<(), SquadError> {
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
    ) {
        return Err(invalid(format!(
            "`{path}`: '{}' belongs to the retired squad-tab/HOME layout. Select a squad or named lead/member in the workspace; choose a new binding explicitly.",
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
                for key in ["rows", "columns", "sections"] {
                    if table.get(key).is_some() {
                        notices.push(format!("`squad.{name}.{key}` remains available to CLI listing; custom board sections/columns no longer compose the workspace. Use its squad tree and selected-agent details."));
                    }
                }
            }
        }
        if let Some(tabs) = self.document.get("tabs").and_then(Item::as_table_like) {
            for (key, _) in tabs.iter() {
                notices.push(format!("`tabs.{key}` remains available to CLI listing; old squad-tab arrangement does not control the three-tab workspace. Use UUID squad pins/sort and guarded 1/2/3."));
            }
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
                "reverse" => {
                    result.reversed = item
                        .as_bool()
                        .ok_or_else(|| invalid("`workspace.reverse` must be true or false."))?
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
        self.write(|document| {
            if document.get("workspace").is_none() {
                document["workspace"] = Item::Table(Table::new());
            }
            let mut pins = toml_edit::Array::new();
            for id in &next.pinned {
                pins.push(id.as_str());
            }
            document["workspace"]["pinned"] = value(pins);
            document["workspace"]["sort"] = value(match next.sort {
                OpsSort::Name => "name",
                OpsSort::Attention => "attention",
                OpsSort::Members => "members",
            });
            document["workspace"]["reverse"] = value(next.reversed);
            document["workspace"]["layout"] = value(match next.layout {
                OpsLayout::Auto => "auto",
                OpsLayout::Side => "side",
                OpsLayout::Stack => "stack",
            });
        })
    }

    pub fn workspace_keys(
        &self,
        scope: OpsKeyScope,
        host: Option<&str>,
        section: Option<&Bindings>,
    ) -> Result<WorkspaceKeys, SquadError> {
        let mut effective = defaults(scope);
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
            if key == "w" {
                notices.push("`bind.w`: w is reserved for Workspace snapshots in list scopes; move this binding to another key explicitly.".into());
                continue;
            }
            if let Err(error) = check(&action, &format!("bind.{key}")) {
                notices.push(error.message);
                continue;
            }
            effective.insert(key, action);
        }
        Ok(WorkspaceKeys {
            bindings: effective,
            notices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .workspace_keys(OpsKeyScope::Overview, None, None)
            .unwrap()
            .bindings;
        assert_eq!(defaults["j"].verb, Verb::Copy);
        assert_eq!(defaults["s"].verb, Verb::Talk);
        let hosted = config
            .workspace_keys(OpsKeyScope::Overview, Some("tmux"), None)
            .unwrap()
            .bindings;
        assert_eq!(hosted["j"].verb, Verb::Open);
        let scoped = config
            .workspace_keys(OpsKeyScope::Overview, Some("tmux"), Some(&section))
            .unwrap()
            .bindings;
        assert_eq!(scoped["j"].verb, Verb::Run);
        assert!(scoped.contains_key("S")); // Real defaults remain in validation/dispatch.
        for (event, action) in scoped {
            assert!(!event.is_empty());
            assert!(!action.description().is_empty());
        }
    }
    #[test]
    fn every_approved_scope_has_its_real_keys_without_legacy_fallbacks() {
        let config = config("");
        let overview = config
            .workspace_keys(OpsKeyScope::Overview, None, None)
            .unwrap()
            .bindings;
        assert_eq!(overview["s"].text, "sort");
        assert_eq!(overview["S"].text, "reverse-sort");
        assert_eq!(overview["j"].text, "jump");
        assert_eq!(overview["x"].text, "remove-membership");
        assert_eq!(overview["n"].text, "create-squad");
        assert_eq!(overview["p"].text, "pin");
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
        assert!(!member.contains_key("r"));
        assert_eq!(member["D"].text, "digest");
        assert_eq!(member["x"].text, "remove-membership");
        assert!(!squad.contains_key("w"));
        assert!(!member.contains_key("A"));
        assert_eq!(overview["left"].text, "fold");
        assert_eq!(overview["right"].text, "expand");
    }
    #[test]
    fn reserved_snapshot_and_retired_bindings_are_reported_without_rewriting_config() {
        for binding in [
            "w='talk'",
            "A='ask-lead'",
            "s='home-pick'",
            "L='jump lead'",
            "V='view'",
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
        ] {
            assert!(
                notices.iter().any(|notice| notice.contains(path)),
                "missing {path}"
            );
        }
        assert!(!notices.iter().any(|notice| notice.contains("token_rate")));
        assert!(
            !notices
                .iter()
                .any(|notice| notice.contains("squad.product.layout"))
        );
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
            sort: OpsSort::Attention,
            reversed: true,
            layout: OpsLayout::Stack,
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
}
