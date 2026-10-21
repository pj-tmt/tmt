//! Who the user is, and who sends. Core owns the person's identity: the
//! `user.name` setting and the `user.identity` API operation (#513). This module
//! asks Core and keeps no copy. A user an earlier `ops.toml` recorded (`me`,
//! `me_id`) is carried into `user.name` once, then the record is dropped.

use crate::{
    config::Config,
    core::{Core, SquadError},
};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub name: String,
}

/// Core's codes for a name it cannot turn into an identity; each message names
/// `tmt config set user.name <name>`. They mean "no user here", not a failure.
const UNRESOLVED: [&str; 3] = [
    "USER_NAME_UNAVAILABLE",
    "USER_NAME_INVALID",
    "USER_NAME_TAKEN",
];

/// `tmt identity show` by name or UUID; `None` when core has no such identity.
fn lookup(core: &Core, selector: &str) -> Result<Option<Me>, SquadError> {
    match core.json(&["identity", "show", selector]) {
        Ok(shown) => match (
            shown["identity"]["id"].as_str(),
            shown["identity"]["name"].as_str(),
        ) {
            (Some(id), Some(name)) => Ok(Some(Me {
                id: id.into(),
                name: name.into(),
            })),
            _ => Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "tmt identity show returned no identity.",
            )),
        },
        Err(error) if error.code == "NAME_NOT_FOUND" => Ok(None),
        Err(error) => Err(error),
    }
}

/// The user: Core's `user.identity`, created and marked as a person on first
/// use. `None` when Core cannot resolve a name (no login name, an invalid one,
/// or agents holding both candidates).
pub fn current(core: &Core) -> Result<Option<Me>, SquadError> {
    match core.api("user.identity", json!({})) {
        Ok(shown) => match (shown["identityId"].as_str(), shown["name"].as_str()) {
            (Some(id), Some(name)) => Ok(Some(Me {
                id: id.into(),
                name: name.into(),
            })),
            _ => Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "user.identity returned no identity.",
            )),
        },
        Err(error) if UNRESOLVED.contains(&error.code.as_str()) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Carries the user an earlier `ops.toml` recorded into Core, once: the
/// identity `me_id` names (else `me`) becomes `user.name` unless Core already
/// has a name set, and the record is dropped. That identity also gets Core's
/// `user` entry first when it has none, so `user.identity` reuses it even if
/// a pane was ever bound to it (an unmarked bound identity is an agent and
/// would be passed over for `<name>-user`). Without a record this reads
/// nothing from Core. A failure keeps the record, so the next run retries.
pub fn migrate(core: &Core, config: &mut Config) -> Result<(), SquadError> {
    let name = config.me()?.map(str::to_owned);
    let id = config.me_id()?.map(str::to_owned);
    if name.is_none() && id.is_none() {
        return Ok(());
    }
    if core.json(&["config", "show"])?["sources"]["user"]["name"] == "default" {
        let recorded = match id {
            Some(id) => lookup(core, &id)?,
            None => None,
        };
        let recorded = match (recorded, name) {
            (Some(recorded), _) => Some(recorded),
            (None, Some(name)) => lookup(core, &name)?,
            (None, None) => None,
        };
        if let Some(recorded) = recorded {
            mark_as_person(core, &recorded.id)?;
            set_name(core, &recorded.name)?;
        }
    }
    config.clear_me()
}

/// Core's reserved `user` entry (contracts/extension-api.md) for the migrated
/// identity: written only when absent and the metadata table has room; an
/// existing value, even `false`, is the person's own choice and is kept.
fn mark_as_person(core: &Core, id: &str) -> Result<(), SquadError> {
    let listed = core.json(&["identity", "meta", "list", "--identity", id])?;
    if let Some(metadata) = listed["metadata"].as_object()
        && (metadata.contains_key("user")
            || metadata.len() >= crate::membership::METADATA_ENTRY_LIMIT)
    {
        return Ok(());
    }
    core.json(&["identity", "meta", "set", "user", "true", "--identity", id])
        .map(drop)
}

fn set_name(core: &Core, name: &str) -> Result<(), SquadError> {
    core.json(&["config", "set", "user.name", name, "--global"])
        .map(drop)
}

/// `tmt ops squad me <name>`: the same as `tmt config set user.name <name>
/// --global`, then the identity that name resolves to.
pub fn set(core: &Core, name: &str) -> Result<Me, SquadError> {
    set_name(core, name)?;
    current(core)?.ok_or_else(|| {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "Core could not resolve the user name that was just set.",
        )
    })
}

/// An `identity.renamed` observation: when it is the user, `user.name` takes the
/// new name now. The renamed identity is the user when it carries the `user`
/// entry and the name in effect no longer resolves to any identity. Anything
/// else is not squad's concern.
pub fn renamed(core: &Core, identity_id: &str) -> Result<bool, SquadError> {
    let Some(renamed) = lookup(core, identity_id)? else {
        return Ok(false);
    };
    let listed = core.json(&["identity", "meta", "list", "--identity", &renamed.id])?;
    if listed["metadata"]["user"] != "true" {
        return Ok(false);
    }
    let shown = core.json(&["config", "show"])?;
    if let Some(effective) = shown["resolved"]["user"]["name"].as_str()
        && (effective == renamed.name || lookup(core, effective)?.is_some())
    {
        return Ok(false);
    }
    set_name(core, &renamed.name)?;
    Ok(true)
}

/// The identity core attributes the invocation to (`tmt whoami`): `None`
/// outside a pane and on an unbound pane. A host that cannot say which agent
/// called is core's error, never a guess.
pub fn caller(core: &Core) -> Result<Option<Caller>, SquadError> {
    match core.json(&["whoami"]) {
        Ok(shown) if shown["bound"] == true => match (shown["id"].as_str(), shown["name"].as_str())
        {
            (Some(id), Some(name)) => Ok(Some(Caller {
                me: Me {
                    id: id.into(),
                    name: name.into(),
                },
                saved: shown["lifetime"] == "saved",
            })),
            _ => Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "tmt whoami returned no identity.",
            )),
        },
        Ok(_) => Ok(None),
        Err(error) if error.code == "PANE_NOT_FOUND" => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub me: Me,
    pub saved: bool,
}

/// Who "waiting on you" is for: the user, otherwise the saved identity bound
/// to the calling pane. Temporary agents are never "you".
pub fn you(recorded: Option<Me>, caller: Option<&Caller>) -> Option<Me> {
    recorded.or_else(|| {
        caller
            .filter(|caller| caller.saved)
            .map(|caller| caller.me.clone())
    })
}

/// Where "you" came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Recorded,
    Pane,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::Pane => "pane",
        }
    }
}

/// [`you`] for a command: the user, then the calling pane. A caller that core
/// cannot attribute only means no one is "you" here.
pub fn resolve_you(core: &Core) -> Result<Option<(Me, Source)>, SquadError> {
    if let Some(recorded) = current(core)? {
        return Ok(Some((recorded, Source::Recorded)));
    }
    Ok(you(None, caller(core).ok().flatten().as_ref()).map(|pane| (pane, Source::Pane)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(name: &str) -> Me {
        Me {
            id: format!("{name}-id"),
            name: name.into(),
        }
    }

    fn caller(name: &str, saved: bool) -> Caller {
        Caller {
            me: me(name),
            saved,
        }
    }

    #[test]
    fn you_is_the_recorded_user_then_a_saved_caller_never_a_temporary_one() {
        assert_eq!(
            you(Some(me("ben")), Some(&caller("sol", true))),
            Some(me("ben"))
        );
        assert_eq!(you(None, Some(&caller("sol", true))), Some(me("sol")));
        assert_eq!(you(None, Some(&caller("auth-fix", false))), None);
        assert_eq!(you(None, None), None);
    }

    /// A stand-in tmt that answers `whoami --json` with a fixed document on
    /// stdout, or a core error with its exit status.
    fn whoami(name: &str, stdout: &str, status: i32) -> Result<Option<Caller>, SquadError> {
        let dir =
            std::env::temp_dir().join(format!("tmt-ops-whoami-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!("#!/bin/sh\ncat <<'EOF'\n{stdout}\nEOF\nexit {status}\n"),
        );
        let found = super::caller(&Core::at(fake));
        let _ = std::fs::remove_dir_all(dir);
        found
    }

    /// A stand-in tmt: every call is appended to a log, then answered by the
    /// first rule whose pattern matches its arguments (`api` for any API
    /// request); a call no rule matches fails with `UNEXPECTED`.
    struct Fake {
        dir: std::path::PathBuf,
        core: Core,
    }

    impl Fake {
        fn new(name: &str, rules: &[(&str, &str, i32)]) -> Self {
            let dir =
                std::env::temp_dir().join(format!("tmt-ops-me-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let log = dir.join("calls.log");
            let arms: String = rules
                .iter()
                .map(|(pattern, document, status)| {
                    format!("'{pattern}') cat <<'EOF'\n{document}\nEOF\nexit {status};;\n")
                })
                .collect();
            let executable = dir.join("tmt");
            crate::test_support::write_ready_executable(
                &executable,
                &format!(
                    "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n{arms}\
                     *) echo '{{\"error\":{{\"code\":\"UNEXPECTED\",\"message\":\"unexpected call\"}}}}'; exit 1;;\nesac\n",
                    log.display()
                ),
            );
            Self {
                core: Core::at(executable),
                dir,
            }
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.join("calls.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn sets(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| call.starts_with("config set"))
                .collect()
        }

        fn marks(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| call.starts_with("identity meta set"))
                .collect()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const BEN_ID: &str = "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f";
    const USER: &str =
        r#"{"identityId":"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f","name":"ben","created":false}"#;
    const BEN_UI: &str = r#"{"identity":{"id":"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f","name":"ben-ui","lifetime":"saved"}}"#;
    const NOT_FOUND: &str = r#"{"error":{"code":"NAME_NOT_FOUND","message":"not found"}}"#;
    const UNSET: &str =
        r#"{"sources":{"user":{"name":"default"}},"resolved":{"user":{"name":"benhsieh"}}}"#;
    const SET: &str =
        r#"{"sources":{"user":{"name":"global"}},"resolved":{"user":{"name":"ben"}}}"#;
    const OK: &str = r#"{"ok":true}"#;
    const NO_META: &str = r#"{"metadata":{}}"#;
    const MARKED: &str = r#"{"metadata":{"user":"false"}}"#;

    fn legacy(name: &str, text: &str) -> (std::path::PathBuf, Config) {
        let dir =
            std::env::temp_dir().join(format!("tmt-ops-legacy-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ops.toml");
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        (path, config)
    }

    fn recorded_ben_ui() -> String {
        format!("me = \"ben-ui\"\nme_id = \"{BEN_ID}\"\n[squad.product]\nlayout = \"pr-queue\"\n")
    }

    #[test]
    fn the_user_is_what_core_resolves_and_an_unresolvable_name_is_no_user() {
        let fake = Fake::new("current", &[("api", USER, 0)]);
        assert_eq!(
            current(&fake.core).unwrap(),
            Some(Me {
                id: BEN_ID.into(),
                name: "ben".into()
            })
        );
        for code in [
            "USER_NAME_UNAVAILABLE",
            "USER_NAME_INVALID",
            "USER_NAME_TAKEN",
        ] {
            let document = format!(r#"{{"error":{{"code":"{code}","message":"set one"}}}}"#);
            let fake = Fake::new(code, &[("api", &document, 1)]);
            assert_eq!(current(&fake.core).unwrap(), None, "{code}");
        }
        let broken = r#"{"error":{"code":"API_UNAVAILABLE","message":"down"}}"#;
        let fake = Fake::new("down", &[("api", broken, 1)]);
        assert_eq!(current(&fake.core).unwrap_err().code, "API_UNAVAILABLE");
    }

    #[test]
    fn a_recorded_user_moves_into_core_once_by_its_uuid_and_the_record_is_dropped() {
        let (path, mut config) = legacy("moves", &recorded_ben_ui());
        let fake = Fake::new(
            "moves",
            &[
                ("config show --json", UNSET, 0),
                (&format!("identity show {BEN_ID} --json"), BEN_UI, 0),
                (
                    &format!("identity meta list --identity {BEN_ID} --json"),
                    NO_META,
                    0,
                ),
                (
                    &format!("identity meta set user true --identity {BEN_ID} --json"),
                    OK,
                    0,
                ),
                ("config set user.name ben-ui --global --json", OK, 0),
            ],
        );
        migrate(&fake.core, &mut config).unwrap();
        assert_eq!(fake.sets(), ["config set user.name ben-ui --global --json"]);
        // The person is marked before the name points at it.
        assert_eq!(
            fake.marks(),
            [format!(
                "identity meta set user true --identity {BEN_ID} --json"
            )]
        );
        let order: Vec<_> = fake.calls();
        let mark = order
            .iter()
            .position(|c| c.starts_with("identity meta set"))
            .unwrap();
        let set = order
            .iter()
            .position(|c| c.starts_with("config set"))
            .unwrap();
        assert!(mark < set, "{order:?}");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "[squad.product]\nlayout = \"pr-queue\"\n");
        // The record is gone, so a second run asks Core nothing.
        let calls = fake.calls().len();
        migrate(&fake.core, &mut config).unwrap();
        assert_eq!(fake.calls().len(), calls);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_existing_user_entry_is_the_persons_choice_and_is_never_overwritten() {
        let (path, mut config) = legacy("marked", &recorded_ben_ui());
        let fake = Fake::new(
            "marked",
            &[
                ("config show --json", UNSET, 0),
                (&format!("identity show {BEN_ID} --json"), BEN_UI, 0),
                (
                    &format!("identity meta list --identity {BEN_ID} --json"),
                    MARKED,
                    0,
                ),
                ("config set user.name ben-ui --global --json", OK, 0),
            ],
        );
        migrate(&fake.core, &mut config).unwrap();
        assert!(fake.marks().is_empty(), "{:?}", fake.calls());
        assert_eq!(fake.sets().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_name_core_already_has_wins_and_an_unknown_record_sets_nothing() {
        let (path, mut config) = legacy("wins", &recorded_ben_ui());
        let fake = Fake::new("wins", &[("config show --json", SET, 0)]);
        migrate(&fake.core, &mut config).unwrap();
        assert!(fake.sets().is_empty(), "Core's own name is not replaced");
        assert_eq!(config.me().unwrap(), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        let (path, mut config) = legacy("gone", "me = \"ghost\"\n");
        let fake = Fake::new(
            "gone",
            &[
                ("config show --json", UNSET, 0),
                ("identity show ghost --json", NOT_FOUND, 3),
            ],
        );
        migrate(&fake.core, &mut config).unwrap();
        assert!(fake.sets().is_empty(), "nothing to carry over");
        assert_eq!(config.me().unwrap(), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_failed_move_keeps_the_record_for_the_next_run() {
        let (path, mut config) = legacy("failed", &recorded_ben_ui());
        let refused = r#"{"error":{"code":"CONFIG_ERROR","message":"no"}}"#;
        let fake = Fake::new(
            "failed",
            &[
                ("config show --json", UNSET, 0),
                (&format!("identity show {BEN_ID} --json"), BEN_UI, 0),
                (
                    &format!("identity meta list --identity {BEN_ID} --json"),
                    MARKED,
                    0,
                ),
                ("config set user.name ben-ui --global --json", refused, 1),
            ],
        );
        assert_eq!(
            migrate(&fake.core, &mut config).unwrap_err().code,
            "CONFIG_ERROR"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), recorded_ben_ui());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn me_sets_the_core_name_then_returns_the_identity_it_resolves_to() {
        let fake = Fake::new(
            "set",
            &[
                ("config set user.name ben --global --json", OK, 0),
                ("api", USER, 0),
            ],
        );
        assert_eq!(set(&fake.core, "ben").unwrap().id, BEN_ID);
        assert_eq!(fake.sets(), ["config set user.name ben --global --json"]);
    }

    #[test]
    fn a_renamed_user_identity_carries_user_name_along_and_nobody_else_does() {
        let renamed_ben =
            format!(r#"{{"identity":{{"id":"{BEN_ID}","name":"benjamin","lifetime":"saved"}}}}"#);
        let list = |name: &str| format!("identity meta list --identity {name} --json");
        let user_mark = r#"{"metadata":{"user":"true"}}"#;
        let show = format!("identity show {BEN_ID} --json");

        // The name in effect no longer resolves: the user was renamed.
        let fake = Fake::new(
            "renamed",
            &[
                (&show, &renamed_ben, 0),
                (&list(BEN_ID), user_mark, 0),
                ("config show --json", SET, 0),
                ("identity show ben --json", NOT_FOUND, 3),
                ("config set user.name benjamin --global --json", OK, 0),
            ],
        );
        assert!(renamed(&fake.core, BEN_ID).unwrap());
        assert_eq!(
            fake.sets(),
            ["config set user.name benjamin --global --json"]
        );

        // The name in effect still names an identity: someone else was renamed.
        let fake = Fake::new(
            "other",
            &[
                (&show, &renamed_ben, 0),
                (&list(BEN_ID), user_mark, 0),
                ("config show --json", SET, 0),
                ("identity show ben --json", BEN_UI, 0),
            ],
        );
        assert!(!renamed(&fake.core, BEN_ID).unwrap());
        assert!(fake.sets().is_empty());

        // An agent was renamed.
        let fake = Fake::new(
            "agent",
            &[
                (&show, &renamed_ben, 0),
                (&list(BEN_ID), r#"{"metadata":{}}"#, 0),
            ],
        );
        assert!(!renamed(&fake.core, BEN_ID).unwrap());
        assert!(fake.sets().is_empty());
    }

    #[test]
    fn whoami_maps_to_a_caller_no_caller_or_cores_error() {
        let bound = r#"{"bound":true,"id":"7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f","name":"sol","pane":"%1","lifetime":"saved"}"#;
        assert_eq!(
            whoami("bound", bound, 0).unwrap(),
            Some(Caller {
                me: Me {
                    id: "7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f".into(),
                    name: "sol".into()
                },
                saved: true
            })
        );
        let temporary =
            r#"{"bound":true,"id":"x","name":"auth-fix","pane":"%3","lifetime":"temporary"}"#;
        assert!(!whoami("temporary", temporary, 0).unwrap().unwrap().saved);
        assert_eq!(
            whoami("unbound", r#"{"bound":false,"pane":"%0"}"#, 0).unwrap(),
            None
        );
        let outside = r#"{"error":{"code":"PANE_NOT_FOUND","message":"Not running inside a resolvable tmux pane."}}"#;
        assert_eq!(whoami("outside", outside, 3).unwrap(), None);
        let shared = r#"{"error":{"code":"CALLER_IDENTITY_AMBIGUOUS","message":"The runtime host does not establish which agent invoked this command."}}"#;
        assert_eq!(
            whoami("shared", shared, 1).unwrap_err().code,
            "CALLER_IDENTITY_AMBIGUOUS",
            "a host that cannot attribute the caller is never a fallback to the user"
        );
    }
}
