//! Antigravity (`agy`): its descriptor, file locations, runtime and hook wire
//! mapping. Payloads describe observations, never bindings.

pub use crate::runtime::hook_protocol::{CONTEXT_LIMIT, HOOK_INPUT_LIMIT, HOOK_TIMEOUT_SECONDS};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use tmt_core::binding::session::{
    BindingSessionState, ObservedSessionKey, ProviderSessionId, RuntimeLiveness, RuntimeState,
    SessionTransition,
};
use tmt_core::endpoint::ProcessIncarnation;

const NAME: &str = tmt_core::driver::descriptor::AGY.name;

/// Antigravity has one runtime mode.
pub const MODE_DEFAULT: &str = "default";

const ACTIVITY_EVENTS: &[&str] = &[];

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::AGY,
    env: &["ANTIGRAVITY_APP_DATA_DIR"],
    locate,
    runtime: Some(super::Runtime {
        caller_marked: || {
            ["ANTIGRAVITY_AGENT", "ANTIGRAVITY_CONVERSATION_ID"]
                .iter()
                .any(|name| std::env::var_os(name).is_some())
        },
        driver: || Box::new(AgyRuntime),
        lifecycle: || Box::new(AgyLifecycle),
        channel: None,
        identify_caller: None,
    }),
};

fn config_root(environment: &crate::skill_installation::ProviderEnvironment) -> PathBuf {
    environment.home().join(".gemini/config")
}

fn app_data_dir(environment: &crate::skill_installation::ProviderEnvironment) -> PathBuf {
    environment.var("ANTIGRAVITY_APP_DATA_DIR").map_or_else(
        || environment.home().join(".gemini/antigravity-cli"),
        |path| environment.resolve(path),
    )
}

fn brain_root(environment: &crate::skill_installation::ProviderEnvironment) -> PathBuf {
    app_data_dir(environment).join("brain")
}

fn transcript_relative_path(session: &str) -> PathBuf {
    Path::new(session).join(".system_generated/logs/transcript.jsonl")
}

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let config = config_root(environment);
    let mut config_dirs = vec![config.clone()];
    if environment.var("ANTIGRAVITY_APP_DATA_DIR").is_some() {
        let app_data = app_data_dir(environment);
        if !config_dirs.contains(&app_data) {
            config_dirs.push(app_data);
        }
    }
    super::Locations {
        skills: config.join("skills"),
        config_dirs,
        legacy_skills: Vec::new(),
        hook_settings: Some(config.join("hooks.json")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgyObservation {
    pub session: ProviderSessionId,
    /// The model the provider reported for this event, when concrete.
    pub model: Option<String>,
    pub transition: SessionTransition,
    pub starting: bool,
}

impl AgyObservation {
    /// Called only with a fresh driver-verified `agy` process in the verified
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
                next.state = RuntimeState::Unknown;
            }
            return Some(next);
        }
        if let Some(previous) = &current.key {
            if previous.incarnation != *incarnation {
                if previous_liveness != RuntimeLiveness::Gone
                    && !(previous_liveness == RuntimeLiveness::Alive
                        && current.last_transition == Some(SessionTransition::Ended))
                {
                    return None;
                }
            } else {
                if current.state == RuntimeState::Ended {
                    return None;
                }
                if current.state == RuntimeState::Unknown
                    && current.last_transition == Some(SessionTransition::Ended)
                {
                    return current.admit(key, self.transition, RuntimeLiveness::Alive);
                }
                if previous.provider_session.is_none()
                    && let Some(next) = current.name_launch(key.clone(), self.transition)
                {
                    return Some(next);
                }
                match self.transition {
                    SessionTransition::Compacted => {
                        return current.transition(&key, self.transition, None);
                    }
                    SessionTransition::Cleared | SessionTransition::Resumed
                        if previous.provider_session != key.provider_session =>
                    {
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
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(default, rename = "modelName")]
    model_name: Option<serde_json::Value>,
    #[serde(default, rename = "transcriptPath")]
    transcript_path: Option<String>,
    #[serde(default, rename = "transcript_path")]
    legacy_transcript_path: Option<String>,
    #[serde(default, rename = "lastUserInput")]
    last_user_input: Option<String>,
    #[serde(default, rename = "parentConversationId")]
    parent_conversation_id: Option<String>,
    #[serde(default, rename = "isBattleMode")]
    is_battle_mode: bool,
    #[serde(default, rename = "invocationNum")]
    invocation_num: Option<u64>,
    #[serde(default, rename = "terminationReason")]
    termination_reason: Option<String>,
    #[serde(default, rename = "finalModelOutput")]
    final_model_output: Option<serde_json::Value>,
    #[serde(default, rename = "toolCall", deserialize_with = "present")]
    tool_call: bool,
    #[serde(default, rename = "modelOutput", deserialize_with = "present")]
    model_output: bool,
    #[serde(default, rename = "modelThinking", deserialize_with = "present")]
    model_thinking: bool,
    #[serde(default, rename = "executionNum", deserialize_with = "present")]
    execution_num: bool,
    #[serde(default, rename = "fullyIdle")]
    fully_idle: Option<bool>,
}

fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    serde::de::Deserialize::deserialize(deserializer).map(|_: serde::de::IgnoredAny| true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookInputError {
    TooLarge,
    Invalid,
    Unsupported,
}

fn parse_main_payload(bytes: &[u8]) -> Result<(Payload, ProviderSessionId), HookInputError> {
    if bytes.len() > HOOK_INPUT_LIMIT {
        return Err(HookInputError::TooLarge);
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| HookInputError::Invalid)?;
    let session =
        ProviderSessionId::new(&payload.conversation_id).map_err(|_| HookInputError::Invalid)?;
    if payload.is_battle_mode
        || payload
            .parent_conversation_id
            .as_deref()
            .is_some_and(|parent| !parent.is_empty())
    {
        return Err(HookInputError::Unsupported);
    }
    Ok((payload, session))
}

fn concrete_model(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .filter(|model| {
            !model.is_empty()
                && *model != "auto"
                && crate::runtime::driver_state::valid_model(model)
        })
        .map(str::to_owned)
}

impl Payload {
    fn is_session_start(&self) -> bool {
        self.invocation_num.is_none()
            && !self.tool_call
            && !self.model_output
            && !self.model_thinking
            && self.termination_reason.is_none()
            && !self.execution_num
            && self.fully_idle.is_none()
    }

    fn pre_invocation_num(&self) -> Option<u64> {
        let num = self.invocation_num?;
        (num >= 1
            && !self.tool_call
            && !self.model_output
            && !self.model_thinking
            && self.termination_reason.is_none()
            && !self.execution_num
            && self.fully_idle.is_none())
        .then_some(num)
    }

    fn is_stop(&self) -> bool {
        (self.termination_reason.is_some() || self.execution_num || self.fully_idle.is_some())
            && self.invocation_num.is_none()
            && !self.tool_call
            && !self.model_output
            && !self.model_thinking
    }
}

/// Decode `SessionStart` (or first-invocation `PreInvocation` for resumed sessions).
pub fn decode_hook(bytes: &[u8]) -> Result<AgyObservation, HookInputError> {
    let (payload, session) = parse_main_payload(bytes)?;
    let transition = if payload.is_session_start() {
        SessionTransition::Started
    } else if payload.pre_invocation_num() == Some(1) {
        SessionTransition::Resumed
    } else {
        return Err(HookInputError::Unsupported);
    };
    Ok(AgyObservation {
        session,
        model: concrete_model(payload.model_name.as_ref()),
        transition,
        starting: true,
    })
}

/// `Stop`: the main agent's turn ended. Subagent stops are rejected.
pub fn decode_turn(bytes: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
    let (payload, session) = parse_main_payload(bytes).ok()?;
    payload.is_stop().then_some(())?;
    let interrupted = payload.termination_reason.as_deref() == Some("USER_CANCELED");
    let transcript = if interrupted {
        None
    } else {
        payload
            .transcript_path
            .or(payload.legacy_transcript_path)
            .map(Into::into)
    };
    let final_text = if interrupted {
        None
    } else {
        payload
            .final_model_output
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .and_then(tmt_core::responses::FinalText::new)
    };
    Some(crate::runtime::lifecycle::TurnEnd {
        session,
        transcript,
        final_text,
        interrupted,
    })
}

/// The context usage of one `agy` transcript line: a `PLANNER_RESPONSE` step's
/// `input_tokens` plus `cache_read_tokens`.
pub fn transcript_usage(line: &str) -> Option<u64> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    if entry.get("type")?.as_str()? != "PLANNER_RESPONSE"
        || entry.get("source")?.as_str()? != "MODEL"
    {
        return None;
    }
    let cache = match entry.get("cache_read_tokens") {
        None => 0,
        Some(value) => value.as_u64()?,
    };
    entry.get("input_tokens")?.as_u64()?.checked_add(cache)
}

/// Outer `Some` marks the newest `PLANNER_RESPONSE` model record even when it
/// carries only tool calls: tool-only turns must not resurrect an older answer.
pub fn transcript_final_text(line: &str) -> Option<Option<tmt_core::responses::FinalText>> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    if entry.get("type")?.as_str()? != "PLANNER_RESPONSE"
        || entry.get("source")?.as_str()? != "MODEL"
    {
        return None;
    }
    let text = entry
        .get("content")
        .and_then(serde_json::Value::as_str)
        .and_then(tmt_core::responses::FinalText::new);
    Some(text)
}

pub fn encode_context(context: &str) -> Option<String> {
    if context.is_empty() || context.len() > CONTEXT_LIMIT {
        return None;
    }
    Some(json!({"injectSteps": [{"systemMessage": {"systemMessage": context}}]}).to_string())
}

pub fn encode_prompt_context(context: &str) -> Option<String> {
    if context.is_empty() || context.len() > CONTEXT_LIMIT {
        return None;
    }
    Some(json!({"injectSteps": [{"ephemeralMessage": context}]}).to_string())
}

mod caller_session {
    use crate::runtime::lifecycle::{CallerSession, CallerSessionRefusal};
    use std::ffi::OsStr;

    pub(super) fn coordinates(session: Option<&OsStr>) -> Option<CallerSession> {
        let session = session?.to_str()?;
        let id = uuid::Uuid::parse_str(session).ok()?;
        if id.to_string() != session {
            return None;
        }
        Some(CallerSession {
            session: tmt_core::binding::session::ProviderSessionId::new(session).ok()?,
            runtime_pid: None,
        })
    }

    pub(super) fn verify_root(
        coordinates: &CallerSession,
        environment: &crate::skill_installation::ProviderEnvironment,
    ) -> Result<(), CallerSessionRefusal> {
        let db_path = super::app_data_dir(environment).join("conversation_summaries.db");
        if let Ok(metadata) = std::fs::symlink_metadata(&db_path) {
            if !metadata.file_type().is_file() {
                return Err(CallerSessionRefusal::IndexUnavailable);
            }
            if let Ok(connection) = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ) {
                let _ = connection.busy_timeout(std::time::Duration::from_millis(100));
                let row: Result<Option<(String, i64)>, _> = connection
                    .query_row(
                        "SELECT COALESCE(parent_conversation_id, ''), COALESCE(nesting_depth, 0) FROM conversation_summaries WHERE conversation_id = ?1 LIMIT 1",
                        [coordinates.session.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    });
                if let Ok(Some((parent_id, depth))) = row
                    && (!parent_id.is_empty() || depth > 0)
                {
                    return Err(CallerSessionRefusal::NotRoot);
                }
            }
        }
        Ok(())
    }
}

struct AgyRuntime;

impl tmt_core::driver::Driver for AgyRuntime {
    type Target = tmt_core::binding::BindingEntry;
    type Error = crate::runtime::RuntimeError;
    type Launch = crate::runtime::RuntimeCommand;

    fn claims(&self, command: &str) -> Option<tmt_core::binding::session::HarnessId> {
        crate::runtime::claim_named(command, NAME)
    }

    fn send(
        &mut self,
        _target: &Self::Target,
        _message: &str,
    ) -> tmt_core::driver::ActionResult<
        tmt_core::driver::DeliveryAcceptance,
        tmt_core::driver::SendFailure<Self::Error>,
    > {
        tmt_core::driver::ActionResult::Unsupported
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
        let mut args = vec!["--conversation".into(), resume.session.as_str().into()];
        if let Some(model) = model {
            args.extend(["--model".into(), model.into()]);
        }
        ActionResult::Completed(crate::runtime::RuntimeCommand {
            executable: NAME.into(),
            args,
        })
    }
}

pub struct AgyLifecycle;

impl crate::runtime::lifecycle::RuntimeLifecycle for AgyLifecycle {
    fn prepare_launch(
        &self,
        command: &mut crate::runtime::RuntimeCommand,
        state_directory: &Path,
        _exact_resume: bool,
    ) {
        let mut writable = false;
        let mut words = command.args.iter().peekable();
        while let Some(arg) = words.next() {
            let Some(word) = arg.to_str() else {
                continue;
            };
            if word == "--" {
                break;
            }
            if word == "--add-dir" {
                if words
                    .next()
                    .is_some_and(|dir| Path::new(dir) == state_directory)
                {
                    writable = true;
                }
            } else if let Some(dir) = word.strip_prefix("--add-dir=") {
                if Path::new(dir) == state_directory {
                    writable = true;
                }
            } else if matches!(
                word,
                "--agent"
                    | "--conversation"
                    | "-r"
                    | "--directory"
                    | "--experiment"
                    | "--local-Cascade-server-port"
                    | "--max-turns"
                    | "--model"
                    | "-m"
                    | "--output-format"
                    | "--permission-mode"
                    | "--system-prompt"
                    | "--system-prompt-file"
            ) {
                words.next();
            } else if !word.starts_with('-') {
                if matches!(
                    word,
                    "agent"
                        | "agents"
                        | "changelog"
                        | "help"
                        | "install"
                        | "mcp"
                        | "mic-serve"
                        | "models"
                        | "plugin"
                        | "plugins"
                        | "remote-control"
                        | "update"
                ) {
                    return;
                }
                break;
            }
        }
        if !writable {
            let boundary = command
                .args
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or(command.args.len());
            command.args.splice(
                boundary..boundary,
                ["--add-dir".into(), state_directory.as_os_str().to_owned()],
            );
        }
    }

    fn launch_settings(
        &self,
        args: &[std::ffi::OsString],
    ) -> crate::runtime::launch_preset::LaunchSettings {
        crate::runtime::launch_preset::flags(args, &["--model", "-m"], &[], false)
    }

    fn resume_settings(
        &self,
        command: &mut crate::runtime::RuntimeCommand,
        settings: &crate::runtime::launch_preset::LaunchSettings,
    ) -> bool {
        if !settings.valid() || settings.effort.is_some() {
            return false;
        }
        if let Some(model) = &settings.model {
            if let Some(index) = command
                .args
                .iter()
                .position(|arg| arg == "--model" || arg == "-m")
            {
                command.args.drain(index..index + 2);
            }
            command.args.extend(["--model".into(), model.into()]);
        }
        true
    }

    fn initial_message(&self, command: &mut crate::runtime::RuntimeCommand, message: &str) -> bool {
        command.args.extend(["--".into(), message.into()]);
        true
    }

    fn caller_session(&self) -> Option<crate::runtime::lifecycle::CallerSession> {
        caller_session::coordinates(std::env::var_os("ANTIGRAVITY_CONVERSATION_ID").as_deref())
    }

    fn caller_session_observation(
        &self,
        coordinates: &crate::runtime::lifecycle::CallerSession,
        environment: &crate::skill_installation::ProviderEnvironment,
        _deadline: std::time::Instant,
    ) -> Result<
        Box<dyn crate::runtime::lifecycle::LifecycleObservation>,
        crate::runtime::lifecycle::CallerSessionRefusal,
    > {
        caller_session::verify_root(coordinates, environment)?;
        Ok(Box::new(AgyObservation {
            session: coordinates.session.clone(),
            model: None,
            transition: SessionTransition::Started,
            starting: true,
        }))
    }

    fn observe_main_caller(
        &self,
        runner: &dyn crate::process::CommandRunner,
        caller: u64,
        pane: u64,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        crate::runtime::evidence::observe_main_in_pane(&runner, caller, pane, deadline, NAME)
    }

    fn decode_digest_turn(&self, payload: &[u8]) -> Option<ProviderSessionId> {
        let (parsed, session) = parse_main_payload(payload).ok()?;
        (parsed.is_stop()
            && parsed.termination_reason.as_deref() != Some("USER_CANCELED")
            && parsed.fully_idle != Some(false))
        .then_some(session)
    }

    fn encode_digest_turn(&self, digest: &str) -> Option<String> {
        (!digest.is_empty() && digest.len() <= crate::digest::DIGEST_BYTES)
            .then(|| json!({"decision": "continue", "reason": digest}).to_string())
    }

    fn encode_hook_notice(&self, message: &str, decision: Option<&str>) -> Option<String> {
        crate::runtime::hook_notices::system_message(message, decision)
    }

    fn empty_hook_output(&self) -> &'static str {
        "{}"
    }

    fn reads_state(&self, version: u16) -> bool {
        crate::runtime::driver_state::reads(version)
    }

    fn state_model(&self, state: &tmt_core::binding::session::DriverState) -> Option<String> {
        crate::runtime::driver_state::state_model(state)
    }

    fn state_observed_model(
        &self,
        state: &tmt_core::binding::session::DriverState,
    ) -> Option<crate::runtime::driver_state::ObservedModel> {
        crate::runtime::driver_state::state_observed_model(state)
    }

    fn state_usage(
        &self,
        state: &tmt_core::binding::session::DriverState,
    ) -> Option<crate::runtime::driver_state::Usage> {
        crate::runtime::driver_state::state_usage(state)
    }

    fn decode_activity(
        &self,
        bytes: &[u8],
    ) -> Option<crate::runtime::hook_protocol::ActivityObservation> {
        use tmt_core::binding::session::activity::{Event, EventKind};
        let (payload, _) = parse_main_payload(bytes).ok()?;
        let kind = if let Some(num) = payload.pre_invocation_num() {
            if num == 1 {
                EventKind::TurnStarted
            } else {
                EventKind::ToolFinished
            }
        } else if payload.is_stop() {
            EventKind::TurnEnded
        } else {
            return None;
        };
        Some(crate::runtime::hook_protocol::ActivityObservation {
            event: Event { kind, turn: None },
            model: concrete_model(payload.model_name.as_ref()),
        })
    }

    fn activity_events(&self) -> &'static [&'static str] {
        ACTIVITY_EVENTS
    }

    fn activity_state(
        &self,
        observation: &crate::runtime::hook_protocol::ActivityObservation,
        session: &ProviderSessionId,
        process: &ProcessIncarnation,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        crate::runtime::driver_state::after_activity(
            previous,
            session,
            process,
            &observation.event,
            observation.model.as_deref(),
            now_ms,
        )
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
        let (payload, session) = parse_main_payload(bytes).ok()?;
        payload.pre_invocation_num().is_some().then_some(session)
    }

    fn decode_prompt_text(&self, bytes: &[u8]) -> Option<String> {
        let (payload, _) = parse_main_payload(bytes).ok()?;
        (payload.pre_invocation_num() == Some(1))
            .then_some(payload.last_user_input)
            .flatten()
            .filter(|prompt| !prompt.is_empty())
    }

    fn encode_prompt_context(&self, text: &str) -> Option<String> {
        encode_prompt_context(text)
    }

    fn encode_context(&self, text: &str) -> Option<String> {
        encode_context(text)
    }

    fn decode_turn(&self, payload: &[u8]) -> Option<crate::runtime::lifecycle::TurnEnd> {
        decode_turn(payload)
    }

    fn consumption_locator(
        &self,
        payload: &[u8],
        environment: &crate::skill_installation::ProviderEnvironment,
    ) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let path = value
            .get("transcriptPath")
            .or_else(|| value.get("transcript_path"))
            .and_then(serde_json::Value::as_str)?;
        crate::runtime::sampling::locator(&brain_root(environment), Path::new(path))
    }

    fn sampling_turn(
        &self,
        session: &ProviderSessionId,
        locator: Option<&str>,
        environment: &crate::skill_installation::ProviderEnvironment,
        _deadline: std::time::Instant,
    ) -> Option<crate::runtime::lifecycle::TurnEnd> {
        let root = brain_root(environment);
        if let Some(locator) = locator
            && let Some(turn) = crate::runtime::sampling::located(&root, locator, session)
        {
            return Some(turn);
        }
        let default_locator = transcript_relative_path(session.as_str());
        crate::runtime::sampling::located(&root, default_locator.to_str()?, session)
    }

    fn session_presence(
        &self,
        session: &ProviderSessionId,
        environment: &crate::skill_installation::ProviderEnvironment,
        deadline: std::time::Instant,
    ) -> crate::runtime::session_presence::SessionPresence {
        crate::runtime::session_presence::probe_exact(
            &brain_root(environment),
            &transcript_relative_path(session.as_str()),
            session,
            deadline,
        )
    }

    fn turn_state(
        &self,
        turn: &crate::runtime::lifecycle::TurnEnd,
        environment: &crate::skill_installation::ProviderEnvironment,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
        deadline: std::time::Instant,
    ) -> Option<tmt_core::binding::session::DriverState> {
        self.turn_observation(turn, environment, previous, now_ms, deadline)
            .state
    }

    fn turn_observation(
        &self,
        turn: &crate::runtime::lifecycle::TurnEnd,
        environment: &crate::skill_installation::ProviderEnvironment,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
        deadline: std::time::Instant,
    ) -> crate::runtime::lifecycle::TurnObservation {
        let root = brain_root(environment);
        let fallback = root.join(transcript_relative_path(turn.session.as_str()));
        let path = turn.transcript.as_deref().unwrap_or(&fallback);
        let (usage, final_text) =
            crate::runtime::transcript::latest_pair(&root, path, transcript_usage, |line| {
                transcript_final_text(line)
            });
        let usage =
            usage.and_then(|tokens| crate::runtime::driver_state::Usage::new(tokens, None, now_ms));
        let previous_consumption =
            previous.and_then(crate::runtime::driver_state::state_consumption);
        let consumption = crate::runtime::consumption::agy(
            &root,
            path,
            previous_consumption.as_ref(),
            now_ms,
            deadline,
        );
        crate::runtime::lifecycle::TurnObservation {
            state: crate::runtime::driver_state::after_observation(usage, consumption, previous),
            final_text: turn.final_text.clone().or(final_text.flatten()),
        }
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
        runner: &dyn crate::process::CommandRunner,
        caller: u64,
        pane: u64,
        deadline: std::time::Instant,
    ) -> Option<ProcessIncarnation> {
        crate::runtime::evidence::observe_main_in_pane(&runner, caller, pane, deadline, NAME)
    }

    fn movable_process(
        &self,
        runner: &dyn crate::process::CommandRunner,
        binding: &tmt_core::binding::Binding,
        _preferences: &tmt_core::binding::session::SessionPreferences,
        deadline: std::time::Instant,
    ) -> Option<tmt_core::endpoint::ProcessIncarnation> {
        crate::runtime::evidence::movable_process(
            &runner,
            &binding.session.key.as_ref()?.incarnation,
            binding.pane_pid,
            deadline,
            NAME,
        )
    }

    fn mode(
        &self,
        _: crate::runtime::lifecycle::HostEvidence,
    ) -> Option<tmt_core::binding::session::RuntimeMode> {
        tmt_core::binding::session::RuntimeMode::new(MODE_DEFAULT).ok()
    }
}

impl crate::runtime::lifecycle::LifecycleObservation for AgyObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn driver_state(
        &self,
        previous: Option<&tmt_core::binding::session::DriverState>,
        now_ms: u64,
    ) -> Option<tmt_core::binding::session::DriverState> {
        crate::runtime::driver_state::after_start(self.model.as_deref(), None, previous, now_ms)
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
        _: crate::runtime::lifecycle::HostEvidence,
        _: bool,
    ) -> Option<BindingSessionState> {
        AgyObservation::propose(self, current, process, previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::lifecycle::RuntimeLifecycle;
    use tmt_core::binding::session::activity::EventKind;
    use tmt_core::driver::{ActionResult, Driver, HarnessResume};

    const SESSION: &str = "50fec1a2-581c-4adc-924d-15c31cef4da4";

    #[test]
    fn session_start_and_pre_invocation_and_stop_decode_cleanly() {
        let start = json!({
            "conversationId": SESSION,
            "modelName": "claude-opus-4-6",
            "transcriptPath": "/tmp/brain/50fec1a2-581c-4adc-924d-15c31cef4da4/.system_generated/logs/transcript.jsonl"
        })
        .to_string();
        let obs = decode_hook(start.as_bytes()).unwrap();
        assert_eq!(obs.session.as_str(), SESSION);
        assert_eq!(obs.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(obs.transition, SessionTransition::Started);
        assert!(obs.starting);

        let pre1 = json!({
            "conversationId": SESSION,
            "invocationNum": 1,
            "modelName": "auto",
            "lastUserInput": "hello world"
        })
        .to_string();
        assert_eq!(
            AgyLifecycle
                .decode_prompt(pre1.as_bytes())
                .unwrap()
                .as_str(),
            SESSION
        );
        assert_eq!(
            AgyLifecycle.decode_prompt_text(pre1.as_bytes()).as_deref(),
            Some("hello world")
        );
        let act1 = AgyLifecycle.decode_activity(pre1.as_bytes()).unwrap();
        assert_eq!(act1.event.kind, EventKind::TurnStarted);
        assert_eq!(act1.model, None);
        let resume_obs = decode_hook(pre1.as_bytes()).unwrap();
        assert_eq!(resume_obs.transition, SessionTransition::Resumed);

        let pre2 = json!({
            "conversationId": SESSION,
            "invocationNum": 2,
            "modelName": "gemini-2.5-pro",
            "lastUserInput": "hello world"
        })
        .to_string();
        assert_eq!(AgyLifecycle.decode_prompt_text(pre2.as_bytes()), None);
        let act2 = AgyLifecycle.decode_activity(pre2.as_bytes()).unwrap();
        assert_eq!(act2.event.kind, EventKind::ToolFinished);
        assert_eq!(act2.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(
            decode_hook(pre2.as_bytes()),
            Err(HookInputError::Unsupported)
        );

        let stop = json!({
            "conversationId": SESSION,
            "terminationReason": "NO_TOOL_CALL",
            "executionNum": 1,
            "fullyIdle": true,
            "finalModelOutput": "Done!"
        })
        .to_string();
        let turn = decode_turn(stop.as_bytes()).unwrap();
        assert_eq!(turn.session.as_str(), SESSION);
        assert!(!turn.interrupted);
        assert_eq!(
            turn.final_text,
            tmt_core::responses::FinalText::new("Done!")
        );
        assert_eq!(
            AgyLifecycle
                .decode_digest_turn(stop.as_bytes())
                .unwrap()
                .as_str(),
            SESSION
        );

        let canceled = json!({
            "conversationId": SESSION,
            "terminationReason": "USER_CANCELED",
            "executionNum": 1,
            "fullyIdle": true
        })
        .to_string();
        let canceled_turn = decode_turn(canceled.as_bytes()).unwrap();
        assert!(canceled_turn.interrupted);
        assert_eq!(AgyLifecycle.decode_digest_turn(canceled.as_bytes()), None);
    }

    #[test]
    fn subagent_and_battle_mode_payloads_are_rejected() {
        let subagent = json!({
            "conversationId": SESSION,
            "parentConversationId": "11111111-1111-4111-8111-111111111111",
            "invocationNum": 1
        })
        .to_string();
        assert!(decode_hook(subagent.as_bytes()).is_err());
        assert!(AgyLifecycle.decode_prompt(subagent.as_bytes()).is_none());
        assert!(AgyLifecycle.decode_activity(subagent.as_bytes()).is_none());

        let battle = json!({
            "conversationId": SESSION,
            "isBattleMode": true
        })
        .to_string();
        assert!(decode_hook(battle.as_bytes()).is_err());
    }

    #[test]
    fn transcript_usage_and_final_text_parse_planner_responses() {
        let line = json!({
            "step_index": 5,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "status": "DONE",
            "content": "Final answer",
            "input_tokens": 1200,
            "cache_read_tokens": 800,
            "output_tokens": 50
        })
        .to_string();
        assert_eq!(transcript_usage(&line), Some(2000));
        assert_eq!(
            transcript_final_text(&line).flatten(),
            tmt_core::responses::FinalText::new("Final answer")
        );
    }

    #[test]
    fn resume_builds_conversation_flag_and_model() {
        let session = ProviderSessionId::new(SESSION).unwrap();
        let harness = tmt_core::binding::session::HarnessId::new(NAME).unwrap();
        let mode = tmt_core::binding::session::RuntimeMode::new(MODE_DEFAULT).unwrap();
        let ActionResult::Completed(cmd) = AgyRuntime.resume(HarnessResume {
            start: tmt_core::driver::HarnessStart {
                harness: &harness,
                context: None,
            },
            mode: &mode,
            session: &session,
            state: None,
        }) else {
            panic!("expected resume command");
        };
        assert_eq!(cmd.executable, "agy");
        assert_eq!(cmd.args, ["--conversation", SESSION]);
    }
}
