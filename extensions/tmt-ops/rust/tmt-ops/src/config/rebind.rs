//! Changing the key of one workspace action: the `keys` group on the settings page and
//! `config set keys.<action> <key>`. The stored form is `[bind] NEW = "<action>"`, plus for the old
//! key `unbound` when it was a default or no entry when it was authored; one draft, one CAS write.
use super::workspace::{OpsKeyScope, WorkspaceKeys, defaults};
use super::*;
use crate::action::valid_event;
use std::collections::{BTreeMap, BTreeSet};

/// Settings keys of this group are `keys.<action>`.
pub(super) const KEYS_PREFIX: &str = "keys.";

/// One row of the `keys` group.
pub(super) struct KeyRow {
    pub action: String,
    pub keys: String,
    pub source: String,
    pub description: String,
}

impl Config {
    fn scope_keys(&self) -> Result<Vec<(OpsKeyScope, WorkspaceKeys)>, SquadError> {
        OpsKeyScope::ALL
            .into_iter()
            .map(|scope| Ok((scope, self.workspace_keys(scope, None, None)?)))
            .collect()
    }

    /// The actions that have a key by default somewhere: the rows of the group.
    fn rebindable() -> BTreeMap<String, crate::action::Action> {
        OpsKeyScope::ALL
            .into_iter()
            .flat_map(|scope| defaults(scope).into_values())
            .map(|action| (action.text.clone(), action))
            .collect()
    }

    /// The keys an action holds in any scope, with where each comes from.
    fn holders(scopes: &[(OpsKeyScope, WorkspaceKeys)], action: &str) -> BTreeMap<String, String> {
        let mut held = BTreeMap::new();
        for (_, keys) in scopes {
            for (key, bound) in &keys.bindings {
                if bound.text == action {
                    held.entry(key.clone())
                        .or_insert_with(|| keys.sources.get(key).cloned().unwrap_or_default());
                }
            }
        }
        held
    }

    pub(super) fn key_rows(&self) -> Result<Vec<KeyRow>, SquadError> {
        let scopes = self.scope_keys()?;
        Ok(Self::rebindable()
            .into_values()
            .map(|action| {
                let held = Self::holders(&scopes, &action.text);
                let sources: BTreeSet<_> = held.values().map(String::as_str).collect();
                let source = match (held.len(), sources.len()) {
                    (0, _) => "unbound".to_owned(),
                    (_, 1) => sources.into_iter().next().unwrap().to_owned(),
                    _ => "several".to_owned(),
                };
                KeyRow {
                    keys: if held.is_empty() {
                        "(none)".into()
                    } else {
                        held.keys().cloned().collect::<Vec<_>>().join(", ")
                    },
                    description: action.description(),
                    action: action.text,
                    source,
                }
            })
            .collect())
    }

    /// The one key the action holds, or none; an error says why the key cannot be changed here.
    pub(super) fn rebind_old(&self, action: &str) -> Result<Option<String>, SquadError> {
        if !Self::rebindable().contains_key(action) {
            return Err(invalid(format!("`{action}` is not an action with a key.")));
        }
        let scopes = self.scope_keys()?;
        let held = Self::holders(&scopes, action);
        if held.len() > 1 {
            return Err(invalid(format!(
                "`{action}` has several keys; edit them in ops.toml."
            )));
        }
        let Some(old) = held.into_keys().next() else {
            return Ok(None);
        };
        if scopes.iter().any(|(_, keys)| {
            keys.bindings
                .get(&old)
                .is_some_and(|bound| bound.text != action)
        }) {
            return Err(invalid(format!(
                "`{old}` runs different actions in different places; edit it in ops.toml."
            )));
        }
        Ok(Some(old))
    }

    /// The draft that moves `action` to `key`. Nothing is written here.
    pub(super) fn preview_rebind(&self, action: &str, key: &str) -> Result<Self, SquadError> {
        let old = self.rebind_old(action)?;
        if !valid_event(key) || key.contains("click") {
            return Err(invalid(format!("`{key}` is not a key a binding can use.")));
        }
        if old.as_deref() == Some(key) {
            // Opening the editor previews the current key; it is valid and changes nothing.
            return Ok(self.clone());
        }
        for (scope, keys) in self.scope_keys()? {
            if let Some(bound) = keys.bindings.get(key) {
                return Err(invalid(format!(
                    "`{key}` already runs `{}` in {}; pick another key.",
                    bound.text,
                    format!("{scope:?}").to_lowercase()
                )));
            }
        }
        let global = BindingLocation::Global;
        let mut draft = self.preview_binding(&global, key, Some(action))?;
        if let Some(old) = old {
            if self.binding_at(&global, &old)?.is_some() {
                draft = draft.preview_binding(&global, &old, None)?;
            }
            if OpsKeyScope::ALL
                .into_iter()
                .any(|scope| defaults(scope).contains_key(&old))
            {
                draft = draft.preview_binding(&global, &old, Some("unbound"))?;
            }
        }
        Ok(draft)
    }

    /// Back to the default key: the authored keys of the action go, and so does any `unbound`
    /// that took its default away.
    pub fn reset_rebind(&mut self, action: &str) -> Result<bool, SquadError> {
        if !Self::rebindable().contains_key(action) {
            return Err(invalid(format!("`{action}` is not an action with a key.")));
        }
        let default_keys: BTreeSet<String> = OpsKeyScope::ALL
            .into_iter()
            .flat_map(defaults)
            .filter(|(_, bound)| bound.text == action)
            .map(|(key, _)| key)
            .collect();
        let global = BindingLocation::Global;
        let mut draft = self.clone();
        for (key, bound) in self.configured_bindings()? {
            if bound.text == action
                || (bound.verb == crate::action::Verb::Unbound && default_keys.contains(&key))
            {
                draft = draft.preview_binding(&global, &key, None)?;
            }
        }
        if draft
            .document
            .get("bind")
            .is_some_and(empty_table_without_comments)
        {
            draft.document.remove("bind");
        }
        let changed = self.document.to_string() != draft.document.to_string();
        if changed {
            self.write(|document| *document = draft.document)?;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn file(name: &str, text: &str) -> (std::path::PathBuf, Config) {
        let directory =
            std::env::temp_dir().join(format!("tmt-ops-rebind-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        (path, config)
    }

    fn member_keys(config: &Config) -> WorkspaceKeys {
        config
            .workspace_keys(OpsKeyScope::Member, None, None)
            .unwrap()
    }

    #[test]
    fn moving_a_default_key_writes_the_new_key_and_unbinds_the_old_one() {
        let (path, mut config) = file("default", "# mine\n[board]\nrefresh = '1m'\n");
        assert!(config.can_edit_setting("keys.member-checklist", None));
        config
            .set_setting(None, "keys.member-checklist", "K")
            .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.starts_with("# mine\n[board]\nrefresh = '1m'\n"),
            "{written}"
        );
        assert!(written.contains("K = \"member-checklist\""), "{written}");
        assert!(written.contains("C = \"unbound\""), "{written}");
        let reread = Config::read(path.clone()).unwrap();
        for scope in [OpsKeyScope::Member, OpsKeyScope::Reader] {
            let keys = reread.workspace_keys(scope, None, None).unwrap();
            assert_eq!(keys.bindings["K"].text, "member-checklist", "{scope:?}");
            assert_eq!(keys.sources["K"], "bind.K");
            assert!(!keys.bindings.contains_key("C"), "{scope:?}");
        }
        let row = reread
            .key_rows()
            .unwrap()
            .into_iter()
            .find(|row| row.action == "member-checklist")
            .unwrap();
        assert_eq!((row.keys.as_str(), row.source.as_str()), ("K", "bind.K"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn moving_an_authored_key_replaces_its_entry() {
        let (path, mut config) = file(
            "authored",
            "[bind]\nK = 'member-checklist'\nC = 'unbound'\n",
        );
        config
            .set_setting(None, "keys.member-checklist", "L")
            .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(!written.contains("K ="), "{written}");
        assert!(written.contains("L = \"member-checklist\""), "{written}");
        assert!(
            written.contains("C = 'unbound'"),
            "the default stays unbound: {written}"
        );
        let keys = member_keys(&Config::read(path.clone()).unwrap());
        assert_eq!(keys.bindings["L"].text, "member-checklist");
        assert!(!keys.bindings.contains_key("K") && !keys.bindings.contains_key("C"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_taken_or_unusable_key_is_refused_with_nothing_written() {
        let (path, mut config) = file("refused", "# untouched\n");
        let before = fs::read(&path).unwrap();
        for (key, reason) in [
            ("q", "already runs `quit`"),
            ("double-click", "not a key a binding can use"),
            ("esc", "not a key a binding can use"),
            ("two words", "not a key a binding can use"),
        ] {
            let error = config
                .set_setting(None, "keys.member-checklist", key)
                .unwrap_err();
            assert!(error.message.contains(reason), "{key}: {}", error.message);
            assert_eq!(fs::read(&path).unwrap(), before, "{key}");
        }
        assert!(
            !config
                .set_setting(None, "keys.member-checklist", "C")
                .unwrap(),
            "the current key is valid and changes nothing"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_action_with_several_keys_is_read_only() {
        let (path, mut config) = file("several", "");
        assert!(!config.can_edit_setting("keys.scroll-up", None));
        assert!(!config.can_edit_setting("keys.no-such-action", None));
        let error = config.set_setting(None, "keys.scroll-up", "u").unwrap_err();
        assert!(error.message.contains("several keys"), "{}", error.message);
        let rows = config.key_rows().unwrap();
        let row = rows.iter().find(|row| row.action == "scroll-up").unwrap();
        assert_eq!(row.keys, "k, up");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reset_brings_the_default_back_and_drops_every_trace() {
        let (path, mut config) = file("reset", "# mine\n");
        config
            .set_setting(None, "keys.member-checklist", "K")
            .unwrap();
        assert!(config.reset_rebind("member-checklist").unwrap());
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("K =") && !written.contains("C ="),
            "{written}"
        );
        let keys = member_keys(&Config::read(path.clone()).unwrap());
        assert_eq!(keys.bindings["C"].text, "member-checklist");
        assert_eq!(keys.sources["C"], "workspace default");
        assert!(!config.reset_rebind("member-checklist").unwrap());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_file_changed_underneath_refuses_the_write_and_stays_intact() {
        let (path, mut config) = file("cas", "[bind]\nz = 'refresh'\n");
        fs::write(&path, "# edited elsewhere\n[bind]\nz = 'refresh'\n").unwrap();
        let changed = fs::read(&path).unwrap();
        let document = config.document.to_string();
        for attempt in [
            config.set_setting(None, "keys.member-checklist", "K"),
            config.reset_rebind("refresh").map(|_| true),
        ] {
            assert_eq!(attempt.unwrap_err().code, "SQUAD_CONFIG_CHANGED");
        }
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert_eq!(config.document.to_string(), document);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
