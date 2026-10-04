//! Codex: its descriptor, file locations, runtime, caller recognition
//! (`caller`) and lifecycle mapping. Shared server ancestry never selects an
//! identity.
//! SessionEnd ends the provider thread, not its live process incarnation.

pub mod attachment;
pub mod caller;
pub mod channel;
pub mod channel_context;
pub mod channel_hooks;
pub mod delivery;
pub mod lease;
pub mod pane;
pub mod queue;
pub mod record;
pub mod recovery;
pub mod server;
pub mod supervisor;
pub mod transport;
mod trust;

/// Keep work plus the process owner's cleanup inside the installed hook timeout.
pub const HOOK_TIMEOUT_MARGIN: std::time::Duration = std::time::Duration::from_millis(500);
pub const MAXIMUM_HOOK_TOTAL_DURATION: std::time::Duration =
    std::time::Duration::from_secs(crate::runtime::hook_protocol::HOOK_TIMEOUT_SECONDS)
        .saturating_sub(HOOK_TIMEOUT_MARGIN);
pub const MAXIMUM_HOOK_WORK_DURATION: std::time::Duration =
    MAXIMUM_HOOK_TOTAL_DURATION.saturating_sub(crate::process::CLEANUP_TIMEOUT);
pub const MAXIMUM_HOOK_ADMISSION_DURATION: std::time::Duration = std::time::Duration::from_secs(1);
const MAXIMUM_FRESH_DISCOVERY_DURATION: std::time::Duration = std::time::Duration::from_secs(5);

pub use crate::runtime::hook_protocol::encode_context;
use serde::Deserialize;
use tmt_core::binding::session::{
    BindingSessionState, ObservedSessionKey, ProviderSessionId, RuntimeLiveness, RuntimeState,
    SessionTransition,
};
use tmt_core::endpoint::ProcessIncarnation;

pub fn observe_in_pane(
    runner: &impl crate::process::CommandRunner,
    caller_pid: u64,
    pane_pid: u64,
    deadline: std::time::Instant,
) -> Option<ProcessIncarnation> {
    crate::runtime::evidence::observe_named_in_pane(runner, caller_pid, pane_pid, deadline, NAME)
}

#[derive(Debug, Clone)]
pub struct CodexObservation {
    pub session: ProviderSessionId,
    /// The model the provider reported for this event, unvalidated.
    pub model: Option<String>,
    pub starting: bool,
    pub transition: SessionTransition,
}

#[derive(Deserialize)]
struct Payload {
    hook_event_name: String,
    session_id: String,
    /// Documented provider field: the active model slug.
    model: Option<String>,
    source: Option<String>,
    /// `string | null`; read only for a turn end (#519), under `CODEX_HOME`.
    transcript_path: Option<String>,
}

pub fn decode_hook(bytes: &[u8]) -> Option<CodexObservation> {
    if bytes.len() > crate::runtime::hook_protocol::HOOK_INPUT_LIMIT {
        return None;
    }
    let payload: Payload = serde_json::from_slice(bytes).ok()?;
    let (starting, transition) = match payload.hook_event_name.as_str() {
        "SessionStart" => (
            true,
            match payload.source.as_deref()? {
                "startup" => SessionTransition::Started,
                "resume" => SessionTransition::Resumed,
                "clear" => SessionTransition::Cleared,
                "compact" => SessionTransition::Compacted,
                _ => return None,
            },
        ),
        "SessionEnd" => (false, SessionTransition::Ended),
        _ => return None,
    };
    Some(CodexObservation {
        session: ProviderSessionId::new(&payload.session_id).ok()?,
        model: payload.model,
        starting,
        transition,
    })
}

/// `Stop`: the turn ended. A null transcript path leaves nothing to read.
pub fn decode_turn(bytes: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
    if bytes.len() > crate::runtime::hook_protocol::HOOK_INPUT_LIMIT {
        return None;
    }
    let payload: Payload = serde_json::from_slice(bytes).ok()?;
    (payload.hook_event_name == "Stop").then_some(())?;
    Some(crate::runtime::lifecycle::TurnEnd {
        session: ProviderSessionId::new(&payload.session_id).ok()?,
        transcript: payload.transcript_path.map(Into::into),
    })
}

/// The context usage of one rollout line, from its latest `token_count`
/// event: the last request's `total_tokens`, the figure Codex's own status
/// display reports as the active context size. `cached_input_tokens` is part
/// of `input_tokens`, so it is not added. The window is the event's
/// `model_context_window` when present.
pub fn transcript_usage(line: &str) -> Option<(u64, Option<u64>)> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    if entry.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let payload = entry.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let info = payload.get("info")?;
    let last = info.get("last_token_usage")?;
    let tokens = last.get("total_tokens")?.as_u64()?;
    let window = match info.get("model_context_window") {
        None | Some(serde_json::Value::Null) => None,
        Some(window) => Some(window.as_u64()?),
    };
    Some((tokens, window))
}

impl CodexObservation {
    /// Independent mode has fresh pane/process evidence. Shared mode must first
    /// select a unique existing exact-thread mapping, never an ambient pane.
    pub fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        shared: bool,
    ) -> Option<BindingSessionState> {
        self.propose_with_resume(current, process, previous, shared, false)
    }

    /// owned_resume is supplied only after a live foreground owner and its
    /// exact remembered shared-mode coordinates have been verified.
    pub fn propose_with_resume(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        shared: bool,
        owned_resume: bool,
    ) -> Option<BindingSessionState> {
        let owned_resume = shared
            && owned_resume
            && current.launch_owner.is_some()
            && current.last_transition == Some(SessionTransition::Resumed)
            && current.state != RuntimeState::Ended;
        let key = ObservedSessionKey {
            incarnation: process.clone(),
            provider_session: Some(self.session.clone()),
        };
        if shared {
            let old = current.key.as_ref()?;
            if old.provider_session.as_ref() != Some(&self.session) {
                return None;
            }
            // A resumed thread can move from an ended embedded process to a
            // shared server, but cannot steal an independently live attachment.
            if old.incarnation != *process && previous != RuntimeLiveness::Gone && !owned_resume {
                return None;
            }
        }
        if let Some(old) = &current.key {
            if old.incarnation == *process {
                if !self.starting || matches!(self.transition, SessionTransition::Compacted) {
                    return current.transition(&key, self.transition, None);
                }
                if current.state == RuntimeState::Ended {
                    return None;
                }
                if current.state == RuntimeState::Unknown
                    && current.last_transition == Some(SessionTransition::Ended)
                {
                    return current.admit(key, self.transition, RuntimeLiveness::Alive);
                }
                if matches!(
                    self.transition,
                    SessionTransition::Cleared | SessionTransition::Resumed
                ) {
                    // Codex clear-start is the provider's context transition;
                    // unlike Claude it need not precede it with a clear-end.
                    return current.transition(old, self.transition, Some(self.session.clone()));
                }
            } else if previous != RuntimeLiveness::Gone && !owned_resume {
                return None;
            }
        }
        if !self.starting {
            return None;
        }
        let mut next = current.admit(key, self.transition, RuntimeLiveness::Alive)?;
        if owned_resume {
            next.launch_owner = current.launch_owner.clone();
        }
        Some(next)
    }
}

const NAME: &str = tmt_core::driver::descriptor::CODEX.name;

/// Codex runs in a shared app server, or embedded with `--no-daemon`.
pub const MODE_SHARED: &str = "shared";
pub const MODE_EMBEDDED: &str = "embedded";

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::CODEX,
    env: &["CODEX_HOME"],
    locate,
    runtime: Some(super::Runtime {
        driver: || Box::new(CodexRuntime),
        lifecycle: || Box::new(CodexLifecycle),
        channel: Some(|| Box::new(channel::CodexChannel)),
        identify_caller: Some(identify_caller),
    }),
};

/// `CODEX_HOME`, or `~/.codex` without it.
fn codex_home(environment: &crate::skill_installation::ProviderEnvironment) -> std::path::PathBuf {
    environment.var("CODEX_HOME").map_or_else(
        || environment.home().join(".codex"),
        |path| environment.resolve(path),
    )
}

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let home = environment.home();
    let default_home = home.join(".codex");
    let codex_home = codex_home(environment);
    // A CODEX_HOME that is the shared `~/.agents` root says nothing about Codex.
    let mut config_dirs = vec![default_home.clone()];
    if codex_home != home.join(".agents") {
        config_dirs.push(codex_home.clone());
    }
    let skills = environment.universal_skills();
    let active = skills.join(crate::skill_installation::SKILL_NAME);
    let mut legacy_skills = Vec::new();
    for candidate in [
        codex_home.join("skills/tmux-team"),
        default_home.join("skills/tmux-team"),
    ] {
        let candidate = environment.resolve(&candidate);
        if candidate != active && !legacy_skills.contains(&candidate) {
            legacy_skills.push(candidate);
        }
    }
    super::Locations {
        config_dirs,
        skills,
        legacy_skills,
        hook_settings: Some(codex_home.join("hooks.json")),
    }
}

fn identify_caller() -> tmt_core::driver::ActionResult<tmt_core::driver::caller::RuntimeCaller, ()>
{
    use tmt_core::driver::{ActionResult, Driver};
    match caller::CodexCaller::new(
        &crate::process::UnixCommandRunner,
        caller::CallerEnvironment::current(),
    )
    .identify_caller()
    {
        ActionResult::Completed(found) => ActionResult::Completed(found),
        ActionResult::Unsupported => ActionResult::Unsupported,
        ActionResult::Failed(_) => ActionResult::Failed(()),
    }
}

struct CodexRuntime;

impl tmt_core::driver::Driver for CodexRuntime {
    type Target = tmt_core::binding::BindingEntry;
    type Error = crate::runtime::RuntimeError;
    type Launch = crate::runtime::RuntimeCommand;

    fn claims(&self, command: &str) -> Option<tmt_core::binding::session::HarnessId> {
        crate::runtime::claim_named(command, NAME)
    }

    fn maximum_send_duration(&self) -> std::time::Duration {
        delivery::MAXIMUM_SEND_DURATION
    }

    fn send(
        &mut self,
        target: &Self::Target,
        message: &str,
    ) -> tmt_core::driver::ActionResult<
        tmt_core::driver::DeliveryAcceptance,
        tmt_core::driver::SendFailure<Self::Error>,
    > {
        let directory = crate::config::ConfigPaths::discover()
            .ok()
            .map(|paths| paths.channel_directory());
        delivery::send(directory.as_deref(), target, message)
    }

    fn resume(
        &mut self,
        resume: tmt_core::driver::HarnessResume<'_>,
    ) -> tmt_core::driver::ActionResult<Self::Launch, Self::Error> {
        use tmt_core::driver::ActionResult;
        let model = match crate::runtime::first_party_resume(
            &resume,
            NAME,
            &[MODE_SHARED, MODE_EMBEDDED],
        ) {
            ActionResult::Completed(model) => model,
            ActionResult::Unsupported => return ActionResult::Unsupported,
            ActionResult::Failed(error) => return ActionResult::Failed(error),
        };
        let mut args = vec!["resume".into()];
        if let Some(model) = model {
            args.extend(["-m".into(), model.into()]);
        }
        args.push(resume.session.as_str().into());
        if resume.mode.as_str() == MODE_EMBEDDED {
            args.push("--no-daemon".into());
        }
        ActionResult::Completed(crate::runtime::RuntimeCommand {
            executable: NAME.into(),
            args,
        })
    }
}

/// A foreground client exiting does not prove a shared thread ended. Require
/// exact driver-owned coordinates; never reinterpret a later provider end.
pub fn disconnected(
    current: &BindingSessionState,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Option<BindingSessionState> {
    if !is_shared_session(current.key.as_ref()?, preferences)
        || current.state == RuntimeState::Ended
    {
        return None;
    }
    let mut next = current.clone();
    next.state = RuntimeState::Unknown;
    Some(next)
}

pub fn is_shared_session(
    key: &ObservedSessionKey,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> bool {
    preferences.remembered.as_ref().is_some_and(|remembered| {
        remembered.harness.as_str() == NAME
            && remembered.mode.as_str() == MODE_SHARED
            && key.provider_session.as_ref() == Some(&remembered.provider_session)
    })
}

/// A reaped client is not a shared-runtime end. Preserve an already confirmed
/// terminal observation for this exact key; a provider end alone is nonterminal.
pub fn record_client_exit(
    current: &BindingSessionState,
    key: ObservedSessionKey,
    owner: ProcessIncarnation,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Option<BindingSessionState> {
    let runtime_ended = current.state == RuntimeState::Ended && current.key.as_ref() == Some(&key);
    let shared = is_shared_session(&key, preferences);
    let mut next = current.record_launched_exit(key, owner)?;
    if shared && !runtime_ended {
        next.state = RuntimeState::Unknown;
        next.last_transition = Some(SessionTransition::Resumed);
    }
    Some(next)
}

pub struct CodexLifecycle;

impl crate::runtime::lifecycle::RuntimeLifecycle for CodexLifecycle {
    fn hook_work_duration(&self) -> Option<std::time::Duration> {
        (std::env::var_os(channel_context::BINDING_ENV).is_some()
            && std::env::var_os(channel_context::GENERATION_ENV).is_some())
        .then_some(MAXIMUM_HOOK_WORK_DURATION)
    }
    fn wait_for_hook_admission(
        &self,
        payload: &[u8],
        deadline: std::time::Instant,
    ) -> Result<(), crate::runtime::lifecycle::LifecycleUnavailable> {
        channel_hooks::wait_for_admission(payload, deadline)
    }
    fn activity_process(
        &self,
        current: &BindingSessionState,
        observed: &ProcessIncarnation,
        session: &ProviderSessionId,
        host: crate::runtime::lifecycle::HostEvidence,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        channel_hooks::activity_process(current, observed, session, host, deadline)
    }
    fn reads_state(&self, version: u16) -> bool {
        crate::runtime::driver_state::reads(version)
    }

    fn state_model(&self, state: &tmt_core::binding::session::DriverState) -> Option<String> {
        crate::runtime::driver_state::state_model(state)
    }

    fn state_usage(
        &self,
        state: &tmt_core::binding::session::DriverState,
    ) -> Option<crate::runtime::driver_state::Usage> {
        crate::runtime::driver_state::state_usage(state)
    }

    fn decode_activity(&self, bytes: &[u8]) -> Option<tmt_core::binding::session::activity::Event> {
        crate::runtime::hook_protocol::decode_activity(bytes, true)
    }

    fn activity_state(
        &self,
        event: &tmt_core::binding::session::activity::Event,
        session: &ProviderSessionId,
        process: &ProcessIncarnation,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        crate::runtime::driver_state::after_activity(previous, session, process, event, now_ms)
    }

    fn state_activity(
        &self,
        state: &tmt_core::binding::session::DriverState,
    ) -> Option<tmt_core::binding::session::activity::Activity> {
        crate::runtime::driver_state::state_activity(state)
    }

    fn state_consumption(
        &self,
        state: &tmt_core::binding::session::DriverState,
    ) -> Option<crate::runtime::consumption::Consumption> {
        crate::runtime::driver_state::state_consumption(state).map(|state| state.value)
    }

    fn decode_prompt(&self, bytes: &[u8]) -> Option<ProviderSessionId> {
        crate::runtime::hook_protocol::decode_prompt(bytes)
    }

    fn decode_turn(&self, payload: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
        decode_turn(payload)
    }

    fn consumption_locator(
        &self,
        payload: &[u8],
        environment: &crate::skill_installation::ProviderEnvironment,
    ) -> Option<String> {
        crate::runtime::sampling::locator(
            &codex_home(environment).join("sessions"),
            &crate::runtime::sampling::payload_path(payload)?,
        )
    }

    fn sampling_turn(
        &self,
        session: &ProviderSessionId,
        locator: Option<&str>,
        environment: &crate::skill_installation::ProviderEnvironment,
        deadline: std::time::Instant,
    ) -> Option<crate::runtime::lifecycle::TurnEnd> {
        let root = codex_home(environment).join("sessions");
        if let Some(locator) = locator {
            return crate::runtime::sampling::located(&root, locator, session);
        }
        crate::runtime::sampling::codex_turn(&root, session, deadline)
    }

    fn turn_state(
        &self,
        turn: &crate::runtime::lifecycle::TurnEnd,
        environment: &crate::skill_installation::ProviderEnvironment,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
        _deadline: std::time::Instant,
    ) -> Option<tmt_core::binding::session::DriverState> {
        let root = codex_home(environment).join("sessions");
        let path = turn.transcript.as_deref()?;
        let usage = crate::runtime::transcript::latest(&root, path, transcript_usage).and_then(
            |(tokens, window)| crate::runtime::driver_state::Usage::new(tokens, window, now_ms),
        );
        let previous_consumption =
            previous.and_then(crate::runtime::driver_state::state_consumption);
        let consumption = (usage.is_some() || previous_consumption.is_some())
            .then(|| {
                crate::runtime::consumption::codex(
                    &root,
                    path,
                    previous_consumption.as_ref(),
                    now_ms,
                )
            })
            .flatten();
        crate::runtime::driver_state::after_observation(usage, consumption, previous)
    }

    fn observe_replacement(
        &self,
        pane_pid: u64,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        let process = crate::runtime::evidence::observe_replacement(
            &crate::process::SupervisedProbeRunner,
            pane_pid,
            deadline,
            NAME,
        )?;
        let observed = caller::CodexCaller::new(
            &crate::process::SupervisedProbeRunner,
            caller::CallerEnvironment {
                thread_id: None,
                process_id: u32::try_from(process.pid()).ok()?,
            },
        )
        .observe_host()
        .ok()??;
        (observed.0 == tmt_core::driver::caller::HostAttribution::Independent
            && u64::from(observed.1) == process.pid())
        .then_some(process)
    }
    fn decode(
        &self,
        payload: &[u8],
    ) -> Option<Box<dyn crate::runtime::lifecycle::LifecycleObservation>> {
        channel_hooks::decode(payload)
    }

    fn host_evidence(
        &self,
    ) -> Result<
        crate::runtime::lifecycle::HostEvidence,
        crate::runtime::lifecycle::LifecycleUnavailable,
    > {
        use crate::runtime::lifecycle::{HostEvidence, LifecycleUnavailable};
        use caller::{CallerEnvironment, CodexCaller};
        use tmt_core::driver::caller::HostAttribution;
        Ok(
            match CodexCaller::new(
                &crate::process::SupervisedProbeRunner,
                CallerEnvironment::current(),
            )
            .observe_host()
            .map_err(|_| LifecycleUnavailable)?
            {
                Some((HostAttribution::Independent, pid)) => HostEvidence::Independent {
                    runtime_pid: Some(pid),
                },
                Some((HostAttribution::Ambiguous, pid)) => {
                    HostEvidence::Ambiguous { runtime_pid: pid }
                }
                None => HostEvidence::Unsupported,
            },
        )
    }

    fn observe_in_pane(
        &self,
        caller: u64,
        pane: u64,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        observe_in_pane(
            &crate::process::SupervisedProbeRunner,
            caller,
            pane,
            deadline,
        )
    }

    fn mode(
        &self,
        host: crate::runtime::lifecycle::HostEvidence,
    ) -> Option<tmt_core::binding::session::RuntimeMode> {
        tmt_core::binding::session::RuntimeMode::new(if host.shared() {
            MODE_SHARED
        } else {
            MODE_EMBEDDED
        })
        .ok()
    }

    fn permits_owned_resume(
        &self,
        preferences: &tmt_core::binding::session::SessionPreferences,
        session: &ProviderSessionId,
        host: crate::runtime::lifecycle::HostEvidence,
    ) -> bool {
        host.shared()
            && preferences.remembered.as_ref().is_some_and(|value| {
                value.harness.as_str() == NAME
                    && value.mode.as_str() == MODE_SHARED
                    && &value.provider_session == session
            })
    }

    fn client_exit(
        &self,
        current: &BindingSessionState,
        key: ObservedSessionKey,
        owner: ProcessIncarnation,
        preferences: &tmt_core::binding::session::SessionPreferences,
    ) -> Option<BindingSessionState> {
        record_client_exit(current, key, owner, preferences)
    }

    fn disconnected(
        &self,
        current: &BindingSessionState,
        preferences: &tmt_core::binding::session::SessionPreferences,
    ) -> Option<BindingSessionState> {
        disconnected(current, preferences)
    }
}

impl crate::runtime::lifecycle::LifecycleObservation for CodexObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn driver_state(
        &self,
        previous: Option<&tmt_core::binding::session::DriverState>,
        _now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        // Codex reports no usage at a start; the next turn end records it.
        crate::runtime::driver_state::after_start(self.model.as_deref(), None, previous)
    }
    fn starting(&self) -> bool {
        self.starting
    }
    fn transition(&self) -> Option<SessionTransition> {
        Some(self.transition)
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        host: crate::runtime::lifecycle::HostEvidence,
        owned_resume: bool,
    ) -> Option<BindingSessionState> {
        self.propose_with_resume(current, process, previous, host.shared(), owned_resume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn start(source: &str, session: &str) -> CodexObservation {
        decode_hook(
            json!({"hook_event_name":"SessionStart", "source":source, "session_id":session})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn provider_end_then_start_turns_over_the_same_embedded_process() {
        let process = ProcessIncarnation::new(20, "embedded-start").unwrap();
        let running = start("startup", "old-thread")
            .propose(
                &BindingSessionState::default(),
                &process,
                RuntimeLiveness::Unknown,
                false,
            )
            .unwrap();
        let end = decode_hook(
            br#"{"hook_event_name":"SessionEnd","session_id":"old-thread","reason":"other"}"#,
        )
        .unwrap();
        let ended = end
            .propose(&running, &process, RuntimeLiveness::Alive, false)
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Unknown);
        assert_eq!(ended.key, running.key);
        assert_eq!(ended.last_transition, Some(SessionTransition::Ended));
        for source in ["startup", "resume", "clear"] {
            let next = start(source, "new-thread")
                .propose(&ended, &process, RuntimeLiveness::Alive, false)
                .unwrap();
            assert_eq!(next.state, RuntimeState::Running);
            assert_eq!(next.key.as_ref().unwrap().incarnation, process);
            assert_eq!(
                next.key
                    .as_ref()
                    .unwrap()
                    .provider_session
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "new-thread"
            );
            assert!(
                end.propose(&next, &process, RuntimeLiveness::Alive, false)
                    .is_none()
            );
            assert!(
                start("startup", "old-thread")
                    .propose(&next, &process, RuntimeLiveness::Alive, false)
                    .is_none()
            );
        }
        assert!(
            start("compact", "old-thread")
                .propose(&ended, &process, RuntimeLiveness::Alive, false)
                .is_none()
        );
        let replacement = ProcessIncarnation::new(20, "different-start").unwrap();
        for evidence in [RuntimeLiveness::Alive, RuntimeLiveness::Unknown] {
            assert!(
                start("startup", "new-thread")
                    .propose(&ended, &replacement, evidence, false)
                    .is_none()
            );
        }
        assert!(
            start("startup", "new-thread")
                .propose(&ended, &process, RuntimeLiveness::Alive, true)
                .is_none()
        );
    }

    #[test]
    fn owned_resume_preserves_owner_and_client_exit_is_not_thread_end() {
        use tmt_core::binding::session::{
            HarnessId, RememberedSession, RuntimeMode, SessionPreferences,
        };
        let client = ProcessIncarnation::new(20, "client-start").unwrap();
        let owner = ProcessIncarnation::new(10, "owner-start").unwrap();
        let server = ProcessIncarnation::new(30, "server-start").unwrap();
        let session = ProviderSessionId::new("exact-thread").unwrap();
        let preferences = SessionPreferences {
            channel: None,
            preferred_harness: Some(HarnessId::new("codex").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("codex").unwrap(),
                mode: RuntimeMode::new(super::MODE_SHARED).unwrap(),
                provider_session: session.clone(),
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        };
        let launched = BindingSessionState::default()
            .admit_launched(
                ObservedSessionKey {
                    incarnation: client,
                    provider_session: Some(session),
                },
                owner.clone(),
                SessionTransition::Resumed,
                RuntimeLiveness::Alive,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        assert!(
            start("resume", "exact-thread")
                .propose(&launched, &server, RuntimeLiveness::Alive, true)
                .is_none()
        );
        let shared = start("resume", "exact-thread")
            .propose_with_resume(&launched, &server, RuntimeLiveness::Alive, true, true)
            .unwrap();
        assert_eq!(shared.launch_owner, Some(owner));
        assert_eq!(shared.key.as_ref().unwrap().incarnation, server);
        let lost = disconnected(&shared, &preferences).unwrap();
        assert_eq!(lost.state, RuntimeState::Unknown);
        assert_eq!(lost.key, shared.key);
        assert_eq!(lost.launch_owner, shared.launch_owner);
        let restored = start("resume", "exact-thread")
            .propose(&lost, &server, RuntimeLiveness::Alive, true)
            .unwrap();
        assert_eq!(restored.state, RuntimeState::Running);
        let ended = BindingSessionState {
            state: RuntimeState::Ended,
            ..restored
        };
        assert!(disconnected(&ended, &preferences).is_none());
        let fast = record_client_exit(
            &BindingSessionState::default(),
            launched.key.clone().unwrap(),
            launched.launch_owner.clone().unwrap(),
            &preferences,
        )
        .unwrap();
        assert_eq!(fast.state, RuntimeState::Unknown);
        let terminal = record_client_exit(
            &ended,
            ended.key.clone().unwrap(),
            ended.launch_owner.clone().unwrap(),
            &preferences,
        )
        .unwrap();
        assert_eq!(terminal.state, RuntimeState::Ended);
        assert_eq!(terminal.key, ended.key);
        let mut embedded = preferences.clone();
        embedded.remembered.as_mut().unwrap().mode =
            RuntimeMode::new(super::MODE_EMBEDDED).unwrap();
        assert!(disconnected(&shared, &embedded).is_none());
        assert!(
            start("resume", "wrong-thread")
                .propose_with_resume(&launched, &server, RuntimeLiveness::Alive, true, true)
                .is_none()
        );
    }
    #[test]
    fn independent_lifecycle_and_shared_exact_mapping_are_separate() {
        let process = ProcessIncarnation::new(20, "embedded-start").unwrap();
        let server = ProcessIncarnation::new(30, "server-start").unwrap();
        let empty = BindingSessionState::default();
        assert!(
            start("startup", "a")
                .propose(&empty, &server, RuntimeLiveness::Gone, true)
                .is_none()
        );
        let running = start("startup", "a")
            .propose(&empty, &process, RuntimeLiveness::Unknown, false)
            .unwrap();
        let compact = start("compact", "a")
            .propose(&running, &process, RuntimeLiveness::Alive, false)
            .unwrap();
        assert_eq!(compact.key, running.key);
        let clear = start("clear", "b")
            .propose(&compact, &process, RuntimeLiveness::Alive, false)
            .unwrap();
        assert_eq!(
            clear
                .key
                .as_ref()
                .unwrap()
                .provider_session
                .as_ref()
                .unwrap()
                .as_str(),
            "b"
        );
        let end =
            decode_hook(br#"{"hook_event_name":"SessionEnd","session_id":"a","reason":"other"}"#)
                .unwrap();
        assert!(
            end.propose(&clear, &process, RuntimeLiveness::Alive, false)
                .is_none()
        );
        for previous in [RuntimeLiveness::Alive, RuntimeLiveness::Unknown] {
            assert!(
                start("resume", "b")
                    .propose(&clear, &server, previous, true)
                    .is_none()
            );
        }
        assert!(
            start("resume", "foreign")
                .propose(&clear, &server, RuntimeLiveness::Gone, true)
                .is_none()
        );
        let shared = start("resume", "b")
            .propose(&clear, &server, RuntimeLiveness::Gone, true)
            .unwrap();
        assert_eq!(shared.key.as_ref().unwrap().incarnation, server);
        let future_end = decode_hook(
            br#"{"hook_event_name":"SessionEnd","session_id":"b","reason":"future-provider-reason"}"#,
        );
        assert_eq!(future_end.unwrap().transition, SessionTransition::Ended);
        let end =
            decode_hook(br#"{"hook_event_name":"SessionEnd","session_id":"b","reason":"other"}"#)
                .unwrap();
        let ended = end
            .propose(&shared, &server, RuntimeLiveness::Alive, true)
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Unknown);
        assert!(
            start("resume", "foreign")
                .propose(&ended, &server, RuntimeLiveness::Alive, true)
                .is_none()
        );
        let resumed = start("resume", "b")
            .propose(&ended, &server, RuntimeLiveness::Alive, true)
            .unwrap();
        assert_eq!(resumed.state, RuntimeState::Running);
    }
}
