//! Bounded launch preferences in identity metadata. A launcher is associated
//! with a provider only by an admitted session belonging to that launch owner.

use super::{RuntimeCommand, RuntimeRegistry};
use crate::storage::{Storage, StorageError};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::OsString};
use tmt_core::{
    binding::session::RememberedSession,
    endpoint::ProcessIncarnation,
    identity_metadata::{
        IdentityMetadataRepository, MetadataAction, MetadataApply, MetadataChange, MetadataChanges,
        MetadataExpectation, MetadataKey, MetadataLookup, MetadataValue,
    },
};

pub const KEY: &str = "resume.launch";
pub const ENV_KEYS: [&str; 2] = ["CLAUDE_AUTOCOMPACT_PCT_OVERRIDE", "CLAUDE_MINI_PCT"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSettings {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl LaunchSettings {
    pub fn valid(&self) -> bool {
        self.model
            .as_deref()
            .is_none_or(super::driver_state::valid_model)
            && self
                .effort
                .as_deref()
                .is_none_or(super::driver_state::valid_model)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchPreset {
    pub executable: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub env: BTreeMap<String, String>,
    pub binding: String,
    pub owner_pid: u64,
    pub owner_start: String,
    pub harness: Option<String>,
    pub session: Option<String>,
}

impl LaunchPreset {
    pub fn capture(
        command: &RuntimeCommand,
        registry: &RuntimeRegistry,
        binding: &str,
        owner: &ProcessIncarnation,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        let executable = command.executable.to_str()?.to_owned();
        let settings = registry.launch_settings(&command.args);
        let env = ENV_KEYS
            .into_iter()
            .filter_map(|key| environment(key).map(|value| (key.into(), value)))
            .collect();
        let preset = Self {
            executable,
            model: settings.model,
            effort: settings.effort,
            env,
            binding: binding.into(),
            owner_pid: owner.pid(),
            owner_start: owner.start_identity().into(),
            harness: None,
            session: None,
        };
        preset.document()?;
        Some(preset)
    }

    pub fn settings(&self) -> LaunchSettings {
        LaunchSettings {
            model: self.model.clone(),
            effort: self.effort.clone(),
        }
    }

    fn valid(&self) -> bool {
        !self.executable.is_empty()
            && !self.executable.starts_with('-')
            && !self.executable.chars().any(char::is_control)
            && self.settings().valid()
            && self.env.iter().all(|(key, value)| {
                ENV_KEYS.contains(&key.as_str())
                    && value.len() <= 128
                    && !value.chars().any(char::is_control)
            })
            && ProcessIncarnation::new(self.owner_pid, &self.owner_start).is_ok()
            && tmt_core::dispatch::canonical_id(&self.binding)
            && self.harness.is_some() == self.session.is_some()
            && self
                .harness
                .as_deref()
                .is_none_or(|h| tmt_core::binding::session::HarnessId::new(h).is_ok())
            && self
                .session
                .as_deref()
                .is_none_or(|s| tmt_core::binding::session::ProviderSessionId::new(s).is_ok())
    }

    fn document(&self) -> Option<MetadataValue> {
        self.valid()
            .then(|| serde_json::to_string(self).ok())
            .flatten()
            .and_then(|document| MetadataValue::parse(&document).ok())
    }

    pub fn matches(&self, session: &RememberedSession) -> bool {
        self.harness.as_deref() == Some(session.harness.as_str())
            && self.session.as_deref() == Some(session.provider_session.as_str())
    }

    pub fn owner(&self) -> ProcessIncarnation {
        ProcessIncarnation::new(self.owner_pid, &self.owner_start).expect("validated preset owner")
    }

    pub fn associate(&mut self, session: &RememberedSession, registry: &RuntimeRegistry) {
        self.harness = Some(session.harness.as_str().into());
        self.session = Some(session.provider_session.as_str().into());
        if let Some(model) = registry.lifecycle(&session.harness).and_then(|driver| {
            session
                .state
                .as_ref()
                .and_then(|state| driver.state_model(state))
        }) {
            self.model = Some(model);
        }
    }

    /// Stored env is added only when the resume caller did not supply that key,
    /// including an explicit empty value. No process-wide mutation occurs.
    pub fn environment(&self, supplied: impl Fn(&str) -> bool) -> Vec<(OsString, OsString)> {
        self.env
            .iter()
            .filter(|(key, _)| !supplied(key))
            .map(|(key, value)| (key.into(), value.into()))
            .collect()
    }
}

pub fn read(storage: &Storage, identity: &str) -> Result<Option<LaunchPreset>, StorageError> {
    Ok(read_document(storage, identity)?.and_then(|document| decode(&document)))
}

fn read_document(storage: &Storage, identity: &str) -> Result<Option<String>, StorageError> {
    Ok(
        match storage.get_metadata(identity, &MetadataKey::parse(KEY).unwrap())? {
            MetadataLookup::Found(value) => Some(value),
            _ => None,
        },
    )
}

fn decode(document: &str) -> Option<LaunchPreset> {
    let preset: LaunchPreset = serde_json::from_str(document).ok()?;
    preset.valid().then_some(preset)
}

/// Conditional metadata updates prevent a late hook from replacing a newer
/// launch or recreating a preset the user explicitly cleared.
pub fn remember(
    storage: &mut Storage,
    identity: &str,
    preset: &LaunchPreset,
) -> Result<bool, StorageError> {
    let Some(document) = preset.document() else {
        return Ok(false);
    };
    apply(
        storage,
        identity,
        MetadataExpectation::Any,
        MetadataAction::Set(document),
    )
}

pub fn clear(storage: &mut Storage, identity: &str) -> Result<bool, StorageError> {
    apply(
        storage,
        identity,
        MetadataExpectation::Any,
        MetadataAction::Remove,
    )
}

pub fn associate(
    storage: &mut Storage,
    identity: &str,
    binding: &str,
    owner: &ProcessIncarnation,
    session: &RememberedSession,
    registry: &RuntimeRegistry,
) -> Result<bool, StorageError> {
    let Some(document) = read_document(storage, identity)? else {
        return Ok(false);
    };
    let Some(mut preset) =
        decode(&document).filter(|p| p.binding == binding && p.owner() == *owner)
    else {
        return Ok(false);
    };
    preset.associate(session, registry);
    let Some(next) = preset.document() else {
        return Ok(false);
    };
    apply(
        storage,
        identity,
        MetadataExpectation::Value(MetadataValue::parse(&document).unwrap()),
        MetadataAction::Set(next),
    )
}

fn apply(
    storage: &mut Storage,
    identity: &str,
    expect: MetadataExpectation,
    then: MetadataAction,
) -> Result<bool, StorageError> {
    let changes = MetadataChanges::new(vec![MetadataChange {
        key: MetadataKey::parse(KEY).unwrap(),
        expect,
        then,
    }])
    .unwrap();
    Ok(matches!(
        storage.apply_metadata(identity, &changes)?,
        MetadataApply::Applied { .. }
    ))
}

/// Exact allow-listed provider flags only, stopping at the positional boundary.
/// Drivers select which syntax they recognize; no prompts or other argv survive.
pub(crate) fn flags(
    args: &[OsString],
    model_flags: &[&str],
    effort_flags: &[&str],
    codex_config: bool,
) -> LaunchSettings {
    let mut settings = LaunchSettings::default();
    let mut words = args.iter();
    while let Some(word) = words.next().and_then(|w| w.to_str()) {
        if word == "--" || !word.starts_with('-') {
            break;
        }
        let (flag, inline) = word
            .split_once('=')
            .map_or((word, None), |(f, v)| (f, Some(v)));
        if model_flags.contains(&flag) || effort_flags.contains(&flag) {
            let value = inline.or_else(|| words.next().and_then(|w| w.to_str()));
            if let Some(value) = value.filter(|value| super::driver_state::valid_model(value)) {
                if model_flags.contains(&flag) {
                    settings.model = Some(value.into());
                } else {
                    settings.effort = Some(value.into());
                }
            }
        } else if codex_config && ["-c", "--config"].contains(&flag) {
            let value = inline.or_else(|| words.next().and_then(|w| w.to_str()));
            if let Some(value) = value.and_then(|v| v.strip_prefix("model_reasoning_effort=")) {
                let value = value.trim_matches('"');
                if super::driver_state::valid_model(value) {
                    settings.effort = Some(value.into());
                }
            }
        } else {
            // Unknown valued options are ambiguous. Do not parse their values
            // as model/effort flags or treat a prompt as launch preferences.
            continue;
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use tmt_core::{
        binding::session::{DriverState, HarnessId, ProviderSessionId, RuntimeMode},
        identity::{Lifetime, create_or_resolve},
    };

    fn session(harness: &str, model: &str) -> RememberedSession {
        RememberedSession {
            harness: HarnessId::new(harness).unwrap(),
            mode: RuntimeMode::new("default").unwrap(),
            provider_session: ProviderSessionId::new("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                .unwrap(),
            state: Some(DriverState::new(1, &format!("{{\"model\":\"{model}\"}}")).unwrap()),
            stale_at_ms: None,
            resume_pending_at_ms: None,
        }
    }

    fn capture(args: &[&str]) -> LaunchPreset {
        let command = RuntimeCommand {
            executable: "/task/claude_mini".into(),
            args: args.iter().map(OsString::from).collect(),
        };
        LaunchPreset::capture(
            &command,
            &RuntimeRegistry::first_party(),
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            &ProcessIncarnation::new(123, "fixture-owner").unwrap(),
            |key| Some(if key == ENV_KEYS[0] { "55" } else { "60" }.into()),
        )
        .unwrap()
    }

    #[test]
    fn captures_only_driver_settings_and_the_exact_allowlisted_environment() {
        let preset = capture(&[
            "--model",
            "opus",
            "--effort=high",
            "prompt",
            "--model",
            "credential",
        ]);
        assert_eq!(preset.executable, "/task/claude_mini");
        assert_eq!(
            preset.settings(),
            LaunchSettings {
                model: Some("opus".into()),
                effort: Some("high".into())
            }
        );
        assert_eq!(
            preset.env,
            BTreeMap::from([
                (ENV_KEYS[0].into(), "55".into()),
                (ENV_KEYS[1].into(), "60".into())
            ])
        );
        let document = preset.document().unwrap();
        assert!(!document.as_str().contains("prompt"));
        assert!(!document.as_str().contains("credential"));
        assert_eq!(
            capture(&["-m", "sol", "-c", "model_reasoning_effort=\"xhigh\""]).settings(),
            LaunchSettings {
                model: Some("sol".into()),
                effort: Some("xhigh".into())
            }
        );
        assert_eq!(
            preset.environment(|key| key == ENV_KEYS[0]),
            vec![(ENV_KEYS[1].into(), "60".into())]
        );
    }

    #[test]
    fn persistence_association_and_clear_are_fenced_and_survive_reopen() {
        let root = TestDirectory::new();
        let database = root.path.join("state.db");
        let mut storage = Storage::open(&database).unwrap();
        let identity = create_or_resolve(&mut storage, "Seat", Lifetime::Saved)
            .unwrap()
            .identity;
        let preset = capture(&["--model", "opus", "--effort", "high"]);
        let session = session("claude", "sonnet");
        assert!(remember(&mut storage, &identity.id, &preset).unwrap());
        assert!(
            !read(&storage, &identity.id)
                .unwrap()
                .unwrap()
                .matches(&session)
        );
        let other_owner = ProcessIncarnation::new(123, "different-incarnation").unwrap();
        assert!(
            !associate(
                &mut storage,
                &identity.id,
                &preset.binding,
                &other_owner,
                &session,
                &RuntimeRegistry::first_party()
            )
            .unwrap()
        );
        assert!(
            associate(
                &mut storage,
                &identity.id,
                &preset.binding,
                &preset.owner(),
                &session,
                &RuntimeRegistry::first_party()
            )
            .unwrap()
        );
        storage.close().unwrap();
        let mut storage = Storage::open(&database).unwrap();
        let saved = read(&storage, &identity.id).unwrap().unwrap();
        assert!(saved.matches(&session));
        assert_eq!(saved.model.as_deref(), Some("sonnet"));
        assert_eq!(saved.effort.as_deref(), Some("high"));
        assert!(clear(&mut storage, &identity.id).unwrap());
        assert!(clear(&mut storage, &identity.id).unwrap());
        assert!(
            !associate(
                &mut storage,
                &identity.id,
                &preset.binding,
                &preset.owner(),
                &session,
                &RuntimeRegistry::first_party()
            )
            .unwrap()
        );
        assert!(read(&storage, &identity.id).unwrap().is_none());
        storage.close().unwrap();
        assert!(
            read(&Storage::open(&database).unwrap(), &identity.id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn invalid_metadata_cannot_supply_extra_environment_or_option_like_tokens() {
        let mut preset = capture(&[]);
        preset
            .env
            .insert("ANTHROPIC_API_KEY".into(), "secret".into());
        assert!(preset.document().is_none());
        assert!(decode(&serde_json::to_string(&preset).unwrap()).is_none());
        preset.env.remove("ANTHROPIC_API_KEY");
        preset.model = Some("--dangerous".into());
        assert!(preset.document().is_none());
        assert!(
            !LaunchSettings {
                model: None,
                effort: Some("two words".into())
            }
            .valid()
        );
    }

    #[test]
    fn drivers_compose_only_selected_settings_in_their_own_resume_grammar() {
        let mut registry = RuntimeRegistry::first_party();
        for harness in ["claude", "codex"] {
            let mut remembered = session(harness, "old");
            if harness == "codex" {
                remembered.mode = RuntimeMode::new("shared").unwrap();
            }
            let tmt_core::driver::ActionResult::Completed(mut command) =
                registry.resume(&remembered)
            else {
                panic!("driver must generate an exact resume command");
            };
            let lifecycle = registry
                .lifecycle(&HarnessId::new(harness).unwrap())
                .unwrap();
            assert!(lifecycle.resume_settings(
                &mut command,
                &LaunchSettings {
                    model: Some("selected".into()),
                    effort: Some("high".into())
                }
            ));
            let expected = if harness == "claude" {
                vec![
                    "--resume",
                    remembered.provider_session.as_str(),
                    "--model",
                    "selected",
                    "--effort",
                    "high",
                ]
            } else {
                vec![
                    "resume",
                    "-m",
                    "selected",
                    "-c",
                    "model_reasoning_effort=\"high\"",
                    remembered.provider_session.as_str(),
                ]
            };
            assert_eq!(
                command.args,
                expected.into_iter().map(OsString::from).collect::<Vec<_>>()
            );
            let before = command.clone();
            assert!(!lifecycle.resume_settings(
                &mut command,
                &LaunchSettings {
                    model: Some("--injected".into()),
                    effort: None
                }
            ));
            assert_eq!(command, before);
        }
    }
}
