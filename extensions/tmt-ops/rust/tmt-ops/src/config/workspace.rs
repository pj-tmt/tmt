//! The effective workspace action table. Dispatch, footer and help consume the same
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
    Reader,
}

pub struct WorkspaceKeys {
    pub bindings: Bindings,
    pub notices: Vec<String>,
}

pub(crate) fn defaults(scope: OpsKeyScope) -> Bindings {
    let mut entries = if scope == OpsKeyScope::Reader {
        vec![
            ("up", "scroll-up"),
            ("k", "scroll-up"),
            ("down", "scroll-down"),
            ("j", "scroll-down"),
            ("pageup", "scroll-page-up"),
            ("pagedown", "scroll-page-down"),
            ("home", "scroll-top"),
            ("end", "scroll-bottom"),
            ("tab", "next-pane"),
            ("?", "help"),
            ("q", "quit"),
            ("ctrl-r", "refresh"),
            (",", "settings"),
            ("t", "talk"),
            ("o", "open"),
            ("D", "digest"),
            ("c", "timers"),
        ]
    } else {
        vec![
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
            ("u", "usage-controls"),
        ]
    };
    match scope {
        OpsKeyScope::Reader => {}
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
    if matches!(
        scope,
        OpsKeyScope::Squad | OpsKeyScope::Member | OpsKeyScope::Pool
    ) {
        entries.push(("w", "workspace-snapshots"));
        entries.push(("a", "discover-agents"));
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

pub(crate) fn admit_action(action: &Action, path: &str) -> Result<(), SquadError> {
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
            if scope == OpsKeyScope::Reader && matches!(key.as_str(), "1" | "2" | "3") {
                notices.push(format!("`bind.{key}` is not active in Reader focus: readers own shell digits. Its authored value and list-scope binding are preserved."));
                continue;
            }
            if let Err(error) = admit_action(&action, &format!("bind.{key}")) {
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
    fn reader_defaults_and_explicit_overrides_use_the_same_precedence() {
        let plain = config("")
            .workspace_keys(OpsKeyScope::Reader, None, None)
            .unwrap();
        for (event, name) in [
            ("j", "scroll-down"),
            ("k", "scroll-up"),
            ("pageup", "scroll-page-up"),
            ("end", "scroll-bottom"),
        ] {
            assert_eq!(plain.bindings[event].text, name);
        }
        for key in ["1", "2", "3", "left", "right", "enter", "s", "p", "x"] {
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
    fn reader_digit_guards_are_reported_instead_of_advertised_as_active_bindings() {
        let config = config("[bind]\n1='copy $name'\n2='agents'\n3='digest-dashboard'\n");
        let original = config.document.to_string();
        let reader = config
            .workspace_keys(OpsKeyScope::Reader, None, None)
            .unwrap();
        let list = config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap();
        for key in ["1", "2", "3"] {
            assert!(!reader.bindings.contains_key(key));
            assert!(
                reader
                    .notices
                    .iter()
                    .any(|notice| notice.contains(&format!("`bind.{key}`"))
                        && notice.contains("Reader focus"))
            );
            assert!(list.bindings.contains_key(key));
        }
        assert_eq!(config.document.to_string(), original);
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
        assert_eq!(squad["w"].text, "workspace-snapshots");
        assert!(!member.contains_key("A"));
        assert_eq!(overview["left"].text, "fold");
        assert_eq!(overview["right"].text, "expand");
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
        assert!(!defaults(OpsKeyScope::Overview).contains_key("w"));
        assert!(!defaults(OpsKeyScope::Reader).contains_key("w"));
    }
    #[test]
    fn retired_bindings_are_reported_without_rewriting_config() {
        for binding in [
            "A='ask-lead'",
            "s='home-pick'",
            "L='jump lead'",
            "V='view'",
            "N='notes'",
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
