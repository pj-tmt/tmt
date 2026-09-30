//! Claude Code: its descriptor, file locations, runtime and hook wire
//! mapping. Payloads describe observations, never bindings.

pub use crate::runtime::hook_protocol::{
    CONTEXT_LIMIT, HOOK_INPUT_LIMIT, HOOK_TIMEOUT_SECONDS, encode_context,
};
use serde::Deserialize;
pub mod channel;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeObservation {
    pub session: ProviderSessionId,
    /// The model the provider reported for this event, unvalidated.
    pub model: Option<String>,
    /// Documented for a resumed or forked start: the tokens its first request
    /// re-sends, which is the conversation's context usage.
    pub context_tokens: Option<u64>,
    pub transition: SessionTransition,
    pub starting: bool,
}

impl ClaudeObservation {
    /// Called only with a fresh driver-verified Claude process in the verified
    /// bound pane. The caller commits this proposal using the existing full CAS.
    pub fn propose(
        &self,
        current: &BindingSessionState,
        incarnation: &ProcessIncarnation,
        previous_liveness: RuntimeLiveness,
    ) -> Option<BindingSessionState> {
        let key = ObservedSessionKey {
            incarnation: incarnation.clone(),
            provider_session: Some(self.session.clone()),
        };
        if !self.starting {
            let mut next = current.transition(&key, self.transition, None)?;
            if matches!(
                self.transition,
                SessionTransition::Cleared | SessionTransition::Resumed
            ) {
                // A conversation switch is not a terminal end, but input is
                // not deliverable until its matching start has been observed.
                next.state = RuntimeState::Unknown;
            }
            return Some(next);
        }
        if let Some(previous) = &current.key {
            if previous.incarnation != *incarnation {
                if previous_liveness != RuntimeLiveness::Gone {
                    return None;
                }
            } else {
                if current.state == RuntimeState::Ended {
                    return None;
                }
                match self.transition {
                    SessionTransition::Compacted => {
                        return current.transition(&key, self.transition, None);
                    }
                    SessionTransition::Cleared | SessionTransition::Resumed
                        if previous.provider_session != key.provider_session =>
                    {
                        // The matching preliminary end records continuation. A
                        // delayed start from an older conversation cannot jump
                        // over a completed newer start with no matching end.
                        if current.state != RuntimeState::Unknown
                            || current.last_transition != Some(self.transition)
                        {
                            return None;
                        }
                        return current.transition(
                            previous,
                            self.transition,
                            Some(self.session.clone()),
                        );
                    }
                    SessionTransition::Cleared | SessionTransition::Resumed => {
                        return current.transition(&key, self.transition, None);
                    }
                    _ => {}
                }
            }
        }
        current.admit(key, self.transition, RuntimeLiveness::Alive)
    }
}

#[derive(Deserialize)]
struct Payload {
    hook_event_name: String,
    session_id: String,
    /// Documented provider field: the active model slug.
    model: Option<String>,
    source: Option<String>,
    reason: Option<String>,
    /// Read only for a turn end (#519), under the driver's own tree.
    transcript_path: Option<String>,
    /// Documented for a resumed start; any other shape is ignored.
    context_tokens: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookInputError {
    TooLarge,
    Invalid,
    Unsupported,
}

/// Unknown future events fail without a state guess. Additional provider fields
/// are ignored; cwd is never read, and a transcript path only by a turn end.
pub fn decode_hook(bytes: &[u8]) -> Result<ClaudeObservation, HookInputError> {
    if bytes.len() > HOOK_INPUT_LIMIT {
        return Err(HookInputError::TooLarge);
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| HookInputError::Invalid)?;
    let session =
        ProviderSessionId::new(&payload.session_id).map_err(|_| HookInputError::Invalid)?;
    let (starting, transition) = match payload.hook_event_name.as_str() {
        "SessionStart" => (
            true,
            match payload.source.as_deref() {
                Some("startup") => SessionTransition::Started,
                Some("resume") => SessionTransition::Resumed,
                Some("clear") => SessionTransition::Cleared,
                Some("compact") => SessionTransition::Compacted,
                Some("fork") => SessionTransition::Forked,
                _ => return Err(HookInputError::Unsupported),
            },
        ),
        "SessionEnd" => (
            false,
            match payload.reason.as_deref() {
                Some("clear") => SessionTransition::Cleared,
                Some("resume") => SessionTransition::Resumed,
                Some("logout" | "prompt_input_exit" | "other") => SessionTransition::Ended,
                _ => return Err(HookInputError::Unsupported),
            },
        ),
        _ => return Err(HookInputError::Unsupported),
    };
    Ok(ClaudeObservation {
        session,
        model: payload.model,
        context_tokens: payload
            .context_tokens
            .as_ref()
            .and_then(serde_json::Value::as_u64),
        transition,
        starting,
    })
}

/// `Stop`: the main agent's turn ended. Subagent stops are not decoded.
pub fn decode_turn(bytes: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
    if bytes.len() > HOOK_INPUT_LIMIT {
        return None;
    }
    let payload: Payload = serde_json::from_slice(bytes).ok()?;
    (payload.hook_event_name == "Stop").then_some(())?;
    Some(crate::runtime::lifecycle::TurnEnd {
        session: ProviderSessionId::new(&payload.session_id).ok()?,
        transcript: payload.transcript_path.map(Into::into),
    })
}

/// The context usage of one transcript line: the last main-conversation
/// assistant message's input, cache-read and cache-creation tokens. The
/// top-level usage is read; per-request `iterations` are not summed again.
/// Sidechain and synthetic messages carry no conversation usage.
pub fn transcript_usage(line: &str) -> Option<u64> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    if entry.get("type")?.as_str()? != "assistant"
        || entry
            .get("isSidechain")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return None;
    }
    let message = entry.get("message")?;
    if message.get("model").and_then(serde_json::Value::as_str) == Some("<synthetic>") {
        return None;
    }
    let usage = message.get("usage")?.as_object()?;
    let cache = |name: &str| match usage.get(name) {
        None => Some(0),
        Some(value) => value.as_u64(),
    };
    usage
        .get("input_tokens")?
        .as_u64()?
        .checked_add(cache("cache_read_input_tokens")?)?
        .checked_add(cache("cache_creation_input_tokens")?)
}

/// The launcher is selected and validated by setup, retaining its stable symlink
/// rather than canonicalizing it into an immutable release directory.
const NAME: &str = tmt_core::driver::descriptor::CLAUDE.name;

/// Claude has one runtime mode.
pub const MODE_DEFAULT: &str = "default";

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::CLAUDE,
    env: &[],
    locate,
    runtime: Some(super::Runtime {
        driver: || Box::new(ClaudeRuntime),
        lifecycle: || Box::new(ClaudeLifecycle),
        channel: Some(|| Box::new(channel::ClaudeChannel)),
        identify_caller: None,
    }),
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let home = environment.home();
    super::Locations {
        config_dirs: vec![home.join(".claude")],
        skills: home.join(".claude/skills"),
        legacy_skills: vec![home.join(".claude/commands/team.md")],
        hook_settings: Some(home.join(".claude/settings.json")),
    }
}

struct ClaudeRuntime;

impl tmt_core::driver::Driver for ClaudeRuntime {
    type Target = tmt_core::binding::BindingEntry;
    type Error = crate::runtime::RuntimeError;
    type Launch = crate::runtime::RuntimeCommand;

    fn claims(&self, command: &str) -> Option<tmt_core::binding::session::HarnessId> {
        crate::runtime::claim_named(command, NAME)
    }

    /// Only an enrolled session (one whose channel server published its record)
    /// leaves the tmux path; see `contracts/claude-channel-v1.md`.
    fn send(
        &mut self,
        target: &Self::Target,
        message: &str,
    ) -> tmt_core::driver::ActionResult<
        tmt_core::driver::DeliveryAcceptance,
        tmt_core::driver::SendFailure<Self::Error>,
    > {
        let Ok(paths) = crate::config::ConfigPaths::discover() else {
            return tmt_core::driver::ActionResult::Unsupported;
        };
        channel::send(&paths.channel_directory(), target, message)
    }

    fn resume(
        &mut self,
        resume: tmt_core::driver::HarnessResume<'_>,
    ) -> tmt_core::driver::ActionResult<Self::Launch, Self::Error> {
        use tmt_core::driver::ActionResult;
        let model = match crate::runtime::first_party_resume(&resume, NAME, &[MODE_DEFAULT]) {
            ActionResult::Completed(model) => model,
            ActionResult::Unsupported => return ActionResult::Unsupported,
            ActionResult::Failed(error) => return ActionResult::Failed(error),
        };
        let mut args = vec!["--resume".into(), resume.session.as_str().into()];
        if let Some(model) = model {
            args.extend(["--model".into(), model.into()]);
        }
        ActionResult::Completed(crate::runtime::RuntimeCommand {
            executable: NAME.into(),
            args,
        })
    }
}

pub struct ClaudeLifecycle;

impl crate::runtime::lifecycle::RuntimeLifecycle for ClaudeLifecycle {
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
        crate::runtime::hook_protocol::decode_activity(bytes, false)
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

    fn decode_prompt(&self, bytes: &[u8]) -> Option<ProviderSessionId> {
        crate::runtime::hook_protocol::decode_prompt(bytes)
    }

    fn decode_turn(&self, payload: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
        decode_turn(payload)
    }

    fn turn_state(
        &self,
        turn: &crate::runtime::lifecycle::TurnEnd,
        environment: &crate::skill_installation::ProviderEnvironment,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        let tokens = crate::runtime::transcript::latest(
            &environment.home().join(".claude/projects"),
            turn.transcript.as_deref()?,
            transcript_usage,
        )?;
        let usage = crate::runtime::driver_state::Usage::new(tokens, None, now_ms)?;
        crate::runtime::driver_state::after_turn(usage, previous)
    }

    fn observe_replacement(
        &self,
        pane_pid: u64,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        crate::runtime::evidence::observe_replacement(
            &crate::process::SupervisedProbeRunner,
            pane_pid,
            deadline,
            NAME,
        )
    }
    fn decode(
        &self,
        payload: &[u8],
    ) -> Option<Box<dyn crate::runtime::lifecycle::LifecycleObservation>> {
        decode_hook(payload).ok().map(|value| Box::new(value) as _)
    }

    fn host_evidence(
        &self,
    ) -> Result<
        crate::runtime::lifecycle::HostEvidence,
        crate::runtime::lifecycle::LifecycleUnavailable,
    > {
        Ok(crate::runtime::lifecycle::HostEvidence::Independent { runtime_pid: None })
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
        _: crate::runtime::lifecycle::HostEvidence,
    ) -> Option<tmt_core::binding::session::RuntimeMode> {
        tmt_core::binding::session::RuntimeMode::new(MODE_DEFAULT).ok()
    }
}

impl crate::runtime::lifecycle::LifecycleObservation for ClaudeObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn driver_state(
        &self,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        // Only a continued conversation keeps usage, and only as reported.
        let usage = matches!(
            self.transition,
            SessionTransition::Resumed | SessionTransition::Forked
        )
        .then_some(self.context_tokens)
        .flatten()
        .and_then(|tokens| crate::runtime::driver_state::Usage::new(tokens, None, now_ms));
        crate::runtime::driver_state::after_start(self.model.as_deref(), usage, previous)
    }
    fn starting(&self) -> bool {
        self.starting
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        _: crate::runtime::lifecycle::HostEvidence,
        _: bool,
    ) -> Option<BindingSessionState> {
        ClaudeObservation::propose(self, current, process, previous)
    }
}

#[cfg(test)]
mod tests;
