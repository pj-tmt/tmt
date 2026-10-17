//! First-party driver state, the resume details a driver keeps for its
//! remembered session:
//! - version 1, `{"model": <slug>}`: the model a starting provider hook reported;
//! - version 2, `{"model"?: <slug>, "usage": {...}}`: also the context usage the
//!   driver last read from its own provider's transcript (#519).
//!
//! Version 3 adds main-turn activity scoped to the provider session and process.
//! Without activity, legacy model/usage documents retain versions 1 and 2.
//! Version 4 adds optional cumulative consumption and its bounded source cursor.
//! Version 5 adds the last observed model: the model an admitted provider event
//! reported. It is written only while present and goes first under the bound.
//! Older documents retain their original versions/bytes; optional counters never
//! expand the core's 1 KiB opaque-state bound.
//! The resume launch model is never inferred from transcripts, arguments or files;
//! consumption attribution separately reads the model on accepted provider records.

use serde_json::{Map, Value, json};
use tmt_core::{
    binding::session::{
        DriverState, ProviderSessionId,
        activity::{Activity, ActivityPhase, Event},
    },
    endpoint::ProcessIncarnation,
    limits::is_valid_js_safe_integer,
};

pub const MODEL_STATE_VERSION: u16 = 1;
pub const USAGE_STATE_VERSION: u16 = 2;
pub const ACTIVITY_STATE_VERSION: u16 = 3;
pub const CONSUMPTION_STATE_VERSION: u16 = 4;
pub const OBSERVED_STATE_VERSION: u16 = 5;

/// Whether this document version is one the first-party drivers read.
pub fn reads(version: u16) -> bool {
    matches!(
        version,
        MODEL_STATE_VERSION
            | USAGE_STATE_VERSION
            | ACTIVITY_STATE_VERSION
            | CONSUMPTION_STATE_VERSION
            | OBSERVED_STATE_VERSION
    )
}

/// Context-window usage as a driver read it: the tokens the provider's next
/// request re-sends, and the window when the provider states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub tokens: u64,
    pub window_tokens: Option<u64>,
    pub observed_at_ms: u64,
}

impl Usage {
    pub fn new(tokens: u64, window_tokens: Option<u64>, observed_at_ms: u64) -> Option<Self> {
        (is_valid_js_safe_integer(tokens)
            && window_tokens.is_none_or(|window| window > 0 && is_valid_js_safe_integer(window))
            && observed_at_ms > 0
            && is_valid_js_safe_integer(observed_at_ms))
        .then_some(Self {
            tokens,
            window_tokens,
            observed_at_ms,
        })
    }

    /// The projection `resume.usage` and the stored document share.
    pub fn document(self) -> Value {
        let mut usage = Map::new();
        usage.insert("tokens".into(), json!(self.tokens));
        if let Some(window) = self.window_tokens {
            usage.insert("windowTokens".into(), json!(window));
        }
        usage.insert("observedAtMs".into(), json!(self.observed_at_ms));
        Value::Object(usage)
    }

    fn read(value: &Value) -> Option<Self> {
        let usage = value.as_object()?;
        if usage
            .keys()
            .any(|key| !["tokens", "windowTokens", "observedAtMs"].contains(&key.as_str()))
        {
            return None;
        }
        let window = match usage.get("windowTokens") {
            None => None,
            Some(window) => Some(window.as_u64()?),
        };
        Self::new(
            usage.get("tokens")?.as_u64()?,
            window,
            usage.get("observedAtMs")?.as_u64()?,
        )
    }
}

/// The last model an admitted provider event reported, and when TMT accepted
/// it. It is a recorded fact, never proof of the current model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedModel {
    pub model: String,
    pub observed_at_ms: u64,
}

impl ObservedModel {
    fn new(model: &str, observed_at_ms: u64) -> Option<Self> {
        (valid_model(model) && observed_at_ms > 0 && is_valid_js_safe_integer(observed_at_ms)).then(
            || Self {
                model: model.to_owned(),
                observed_at_ms,
            },
        )
    }

    /// The projection `resume.observedModel` and the stored document share.
    pub fn document(&self) -> Value {
        json!({"model": self.model, "observedAtMs": self.observed_at_ms})
    }

    fn read(value: &Value) -> Option<Self> {
        let fields = value.as_object()?;
        if fields.len() != 2 {
            return None;
        }
        Self::new(
            fields.get("model")?.as_str()?,
            fields.get("observedAtMs")?.as_u64()?,
        )
    }
}

/// A provider model slug, safe to pass as one argv value: bounded, no
/// whitespace or controls, and never option-like.
pub(super) fn valid_model(model: &str) -> bool {
    (1..=128).contains(&model.len())
        && !model.starts_with('-')
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/[]@".contains(&byte))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Document {
    model: Option<String>,
    usage: Option<Usage>,
    activity: Option<Activity>,
    consumption: Option<super::consumption::State>,
    observed: Option<ObservedModel>,
}

impl Document {
    /// A readable document, or `None`. Every field must be valid: one bad
    /// field makes the whole document unreadable, never partly used.
    fn read(state: &DriverState) -> Option<Self> {
        let fields: Map<String, Value> = serde_json::from_str(state.document()).ok()?;
        let model = match fields.get("model") {
            None => None,
            Some(Value::String(model)) if valid_model(model) => Some(model.clone()),
            Some(_) => return None,
        };
        let usage = match fields.get("usage") {
            None => None,
            Some(usage) => Some(Usage::read(usage)?),
        };
        let activity = match fields.get("activity") {
            None => None,
            Some(value) => Some(read_activity(value)?),
        };
        let consumption = match fields.get("consumption") {
            None => None,
            Some(value) => Some(super::consumption::State::read(value)?),
        };
        let observed = match fields.get("observed") {
            None => None,
            Some(value) => Some(ObservedModel::read(value)?),
        };
        let known = usize::from(consumption.is_some())
            + usize::from(model.is_some())
            + usize::from(usage.is_some())
            + usize::from(activity.is_some())
            + usize::from(observed.is_some());
        let valid = match state.version() {
            MODEL_STATE_VERSION => {
                model.is_some() && usage.is_none() && activity.is_none() && consumption.is_none()
            }
            USAGE_STATE_VERSION => usage.is_some() && activity.is_none() && consumption.is_none(),
            ACTIVITY_STATE_VERSION => activity.is_some() && consumption.is_none(),
            CONSUMPTION_STATE_VERSION => consumption.is_some(),
            OBSERVED_STATE_VERSION => observed.is_some(),
            _ => false,
        } && (state.version() == OBSERVED_STATE_VERSION || observed.is_none());
        (valid && fields.len() == known).then_some(Self {
            model,
            usage,
            activity,
            consumption,
            observed,
        })
    }

    fn write(mut self) -> Option<DriverState> {
        // The observed model is the newest optional evidence, so it goes first
        // when the bound bites: the document then takes its legacy version.
        if let Some(observed) = self.observed.take() {
            let mut fields = self.legacy_fields();
            fields.insert("observed".into(), observed.document());
            if let Some(consumption) = &self.consumption {
                fields.insert("consumption".into(), consumption.document());
            }
            let document = Value::Object(fields).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(OBSERVED_STATE_VERSION, &document).ok();
            }
        }
        let mut fields = self.legacy_fields();
        let mut version = if self.usage.is_some() {
            USAGE_STATE_VERSION
        } else if fields.is_empty() && self.consumption.is_none() {
            return None;
        } else {
            MODEL_STATE_VERSION
        };
        if self.activity.is_some() {
            version = ACTIVITY_STATE_VERSION;
        }
        if let Some(mut consumption) = self.consumption {
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            // Optional evidence goes newest first, so a later addition never
            // evicts what earlier documents already carried: credits and tier,
            // then limits, then attribution, then the cache-write counter.
            consumption.value.credits = None;
            consumption.value.service_tier = None;
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            consumption.value.rate_limits = None;
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            consumption.value.delta_by_model = None;
            consumption.value.model_id = None;
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            consumption.value.cache_write_tokens = None;
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            // Optional counters must not crowd out existing context/activity
            // evidence. Missing consumption is explicit to public readers.
            fields.remove("consumption");
            if fields.is_empty() {
                return None;
            }
        }
        DriverState::new(version, &Value::Object(fields).to_string()).ok()
    }

    /// The fields every version up to 3 carries, in their stored shape.
    fn legacy_fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        if let Some(model) = &self.model {
            fields.insert("model".into(), json!(model));
        }
        if let Some(usage) = self.usage {
            fields.insert("usage".into(), usage.document());
        }
        if let Some(activity) = &self.activity {
            fields.insert(
                "activity".into(),
                json!({
                    "session": activity.session.as_str(), "pid": activity.process.pid(),
                    "start": activity.process.start_identity(), "state": activity.phase.as_str(),
                    "turn": activity.turn.as_ref().map(ProviderSessionId::as_str),
                    "sinceMs": activity.since_ms, "lastActivityMs": activity.last_activity_ms,
                }),
            );
        }
        fields
    }
}

/// The model stored in a readable first-party state document, if any.
pub fn state_model(state: &DriverState) -> Option<String> {
    Document::read(state)?.model
}

/// The usage stored in a readable first-party state document, if any.
pub fn state_usage(state: &DriverState) -> Option<Usage> {
    Document::read(state)?.usage
}

/// The state after a starting event. A reported model replaces the stored
/// one; without one the previous readable model stays, and nothing is
/// guessed. Usage is what the start itself reported (a resumed Claude
/// conversation reports it), otherwise none: the context changed.
/// The observed model is only what this start reported: the kept resume model
/// is not new evidence, so without a report the observation is cleared.
pub fn after_start(
    reported: Option<&str>,
    usage: Option<Usage>,
    previous: Option<&DriverState>,
    now_ms: u64,
) -> Option<DriverState> {
    let reported = reported.filter(|model| valid_model(model));
    Document {
        model: reported
            .map(str::to_owned)
            .or_else(|| previous.and_then(state_model)),
        usage,
        activity: None,
        consumption: None,
        observed: reported.and_then(|model| ObservedModel::new(model, now_ms)),
    }
    .write()
}

/// The last observed model in a readable first-party state document, if any.
pub fn state_observed_model(state: &DriverState) -> Option<ObservedModel> {
    Document::read(state)?.observed
}

/// Internal cursor evidence is kept opaque to core and public projections.
pub fn state_consumption(state: &DriverState) -> Option<super::consumption::State> {
    Document::read(state)?.consumption
}

/// Context and consumption observations share the existing hook transaction.
pub fn after_observation(
    usage: Option<Usage>,
    mut consumption: Option<super::consumption::State>,
    previous: Option<&DriverState>,
) -> Option<DriverState> {
    if usage.is_none() && consumption.is_none() {
        return None;
    }
    let mut document = previous.and_then(Document::read).unwrap_or_default();
    if previous.is_some() && document.consumption.is_none() {
        // Legacy/counterless state supplies no measurement continuity. Keep
        // its model/context/activity, and make the new counter's gap explicit.
        if let Some(consumption) = consumption.as_mut() {
            consumption.value.gap = true;
            consumption.value.complete = false;
        }
    }
    if let Some(usage) = usage {
        document.usage = Some(usage);
    }
    // A failed consumption read removes the optional counter rather than
    // claiming continuity with stale evidence on the next successful read.
    document.consumption = consumption;
    document.write()
}

fn read_activity(value: &Value) -> Option<Activity> {
    let fields = value.as_object()?;
    if fields.len() != 7 {
        return None;
    }
    let phase = match value["state"].as_str()? {
        "working" => ActivityPhase::Working,
        "waiting" => ActivityPhase::Waiting,
        "idle" => ActivityPhase::Idle,
        _ => return None,
    };
    let since = value["sinceMs"].as_u64()?;
    let last = value["lastActivityMs"].as_u64()?;
    if since == 0 || since > last || !is_valid_js_safe_integer(last) {
        return None;
    }
    Some(Activity {
        session: ProviderSessionId::new(value["session"].as_str()?).ok()?,
        process: ProcessIncarnation::new(value["pid"].as_u64()?, value["start"].as_str()?).ok()?,
        phase,
        turn: match fields.get("turn")? {
            Value::Null => None,
            value => Some(ProviderSessionId::new(value.as_str()?).ok()?),
        },
        since_ms: since,
        last_activity_ms: last,
    })
}

pub fn state_activity(state: &DriverState) -> Option<Activity> {
    Document::read(state)?.activity
}

/// The state after an activity event, or `None` when `Activity::observe`
/// rejects it. Only an admitted transition records the model it reported; an
/// event without a valid one keeps the previous observation and its time.
pub fn after_activity(
    previous: Option<&DriverState>,
    session: &ProviderSessionId,
    process: &ProcessIncarnation,
    event: &Event,
    model: Option<&str>,
    now: u64,
) -> Option<DriverState> {
    let mut document = previous.and_then(Document::read).unwrap_or_default();
    document.activity = Some(Activity::observe(
        document.activity.as_ref(),
        session,
        process,
        event,
        now,
    )?);
    if let Some(observed) = model.and_then(|model| ObservedModel::new(model, now)) {
        document.observed = Some(observed);
    }
    document.write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::limits::MAX_JS_SAFE_INTEGER;

    fn usage(tokens: u64) -> Usage {
        Usage::new(tokens, Some(258_400), 1_780_000_000_000).unwrap()
    }

    /// A model-only document as every version before 5 wrote it.
    fn legacy(model: &str) -> DriverState {
        DriverState::new(MODEL_STATE_VERSION, &json!({ "model": model }).to_string()).unwrap()
    }

    #[test]
    fn activity_round_trips_without_transferring_through_session_start() {
        let session = ProviderSessionId::new("s").unwrap();
        let process = ProcessIncarnation::new(42, "birth").unwrap();
        let previous = after_start(Some("opus"), Some(usage(10)), None, 50).unwrap();
        let event = Event {
            kind: tmt_core::binding::session::activity::EventKind::TurnStarted,
            turn: Some(ProviderSessionId::new("turn").unwrap()),
        };
        let active =
            after_activity(Some(&previous), &session, &process, &event, None, 100).unwrap();
        assert_eq!(active.version(), OBSERVED_STATE_VERSION);
        assert_eq!(
            state_observed_model(&active),
            state_observed_model(&previous)
        );
        assert_eq!(state_model(&active).as_deref(), Some("opus"));
        assert_eq!(state_usage(&active), Some(usage(10)));
        assert!(state_activity(&active).unwrap().matches(&session, &process));
        assert!(after_activity(Some(&active), &session, &process, &event, None, 101).is_none());
        let waiting = after_activity(
            Some(&active),
            &session,
            &process,
            &Event {
                kind: tmt_core::binding::session::activity::EventKind::ApprovalPrompt,
                turn: event.turn.clone(),
            },
            None,
            102,
        )
        .unwrap();
        assert_eq!(
            state_activity(&waiting).unwrap().phase,
            ActivityPhase::Waiting
        );
        assert_eq!(state_model(&waiting), state_model(&active));
        assert_eq!(state_usage(&waiting), state_usage(&active));
        assert_eq!(waiting.version(), active.version());
        let cleared = after_activity(
            Some(&waiting),
            &session,
            &process,
            &Event {
                kind: tmt_core::binding::session::activity::EventKind::ToolFinished,
                turn: event.turn.clone(),
            },
            None,
            103,
        )
        .unwrap();
        assert_eq!(
            state_activity(&cleared).unwrap().phase,
            ActivityPhase::Working
        );
        assert_eq!(state_model(&cleared), state_model(&active));
        assert_eq!(state_usage(&cleared), state_usage(&active));
        let updated = after_observation(Some(usage(12)), None, Some(&active)).unwrap();
        assert_eq!(state_activity(&updated), state_activity(&active));
        let resumed = after_start(None, None, Some(&active), 102).unwrap();
        assert_eq!(resumed.version(), 1);
        assert!(state_activity(&resumed).is_none());
        let mut bad: Value = serde_json::from_str(active.document()).unwrap();
        bad["activity"]["sinceMs"] = 101.into();
        assert!(state_activity(&DriverState::new(5, &bad.to_string()).unwrap()).is_none());
    }

    #[test]
    fn a_reported_model_replaces_and_an_absent_one_keeps_the_previous() {
        let first = after_start(Some("claude-opus-5"), None, None, 7).unwrap();
        assert_eq!(first.version(), OBSERVED_STATE_VERSION);
        assert_eq!(
            first.document(),
            r#"{"model":"claude-opus-5","observed":{"model":"claude-opus-5","observedAtMs":7}}"#
        );
        assert_eq!(state_model(&first).as_deref(), Some("claude-opus-5"));
        // The kept resume model is not a new observation: the start clears it,
        // and the document returns to its legacy bytes.
        assert_eq!(
            after_start(None, None, Some(&first), 8),
            Some(legacy("claude-opus-5"))
        );
        assert_eq!(
            after_start(None, None, Some(&legacy("claude-opus-5")), 8),
            Some(legacy("claude-opus-5"))
        );
        let second = after_start(Some("claude-sonnet-4-5[1m]"), None, Some(&first), 9).unwrap();
        assert_eq!(
            state_model(&second).as_deref(),
            Some("claude-sonnet-4-5[1m]")
        );
        assert_eq!(after_start(None, None, None, 9), None);
    }

    #[test]
    fn a_turn_records_usage_and_keeps_the_model_until_the_context_changes() {
        let model = legacy("gpt-5.2-codex");
        let turned = after_observation(Some(usage(146_577)), None, Some(&model)).unwrap();
        assert_eq!(turned.version(), USAGE_STATE_VERSION);
        assert_eq!(
            turned.document(),
            r#"{"model":"gpt-5.2-codex","usage":{"tokens":146577,"windowTokens":258400,"observedAtMs":1780000000000}}"#
        );
        assert_eq!(state_model(&turned).as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(state_usage(&turned), Some(usage(146_577)));
        let later = after_observation(Some(usage(150_000)), None, Some(&turned)).unwrap();
        assert_eq!(state_usage(&later), Some(usage(150_000)));

        // A new, cleared or compacted context drops usage; the model stays
        // and the document returns to version 1, byte for byte.
        assert_eq!(
            after_start(None, None, Some(&later), 1),
            Some(model.clone())
        );
        // A resumed conversation may report its own usage.
        let resumed = after_start(None, Some(usage(182_340)), Some(&later), 1).unwrap();
        assert_eq!(state_usage(&resumed), Some(usage(182_340)));
        assert_eq!(state_model(&resumed).as_deref(), Some("gpt-5.2-codex"));

        // Usage alone, with no model ever reported, is kept too.
        let bare = after_observation(Some(usage(10)), None, None).unwrap();
        assert_eq!(bare.version(), USAGE_STATE_VERSION);
        assert_eq!(state_model(&bare), None);
        assert_eq!(after_start(None, None, Some(&bare), 1), None);
    }

    #[test]
    fn usage_is_bounded() {
        assert!(Usage::new(0, None, 1).is_some());
        assert!(Usage::new(MAX_JS_SAFE_INTEGER, Some(MAX_JS_SAFE_INTEGER), 1).is_some());
        assert!(Usage::new(MAX_JS_SAFE_INTEGER + 1, None, 1).is_none());
        assert!(Usage::new(1, Some(0), 1).is_none());
        assert!(Usage::new(1, None, 0).is_none());
    }

    #[test]
    fn unsafe_models_and_unreadable_documents_are_never_used() {
        let kept = legacy("gpt-5.2-codex");
        for model in [
            "",
            "--dangerously-skip-permissions",
            "a b",
            "x\n",
            &"m".repeat(129),
        ] {
            assert_eq!(
                after_start(Some(model), None, Some(&kept), 1),
                Some(kept.clone()),
                "{model:?}"
            );
        }
        let usage = r#""usage":{"tokens":1,"observedAtMs":2}"#;
        for (version, document) in [
            (3, r#"{"model":"opus"}"#.to_owned()),
            (1, r#"{"model":"opus","effort":"high"}"#.to_owned()),
            (1, r#"{"model":7}"#.to_owned()),
            (1, r#"{"model":"-x"}"#.to_owned()),
            (1, "not json".to_owned()),
            (1, format!("{{\"model\":\"opus\",{usage}}}")),
            (2, r#"{"model":"opus"}"#.to_owned()),
            (2, format!("{{\"model\":\"-x\",{usage}}}")),
            (2, r#"{"usage":{"tokens":-1,"observedAtMs":2}}"#.to_owned()),
            (2, r#"{"usage":{"tokens":1}}"#.to_owned()),
            (
                2,
                r#"{"usage":{"tokens":1,"observedAtMs":2,"cost":3}}"#.to_owned(),
            ),
            (
                2,
                r#"{"usage":{"tokens":1,"windowTokens":0,"observedAtMs":2}}"#.to_owned(),
            ),
            (2, format!("{{{usage},\"extra\":1}}")),
        ] {
            let state = DriverState::new(version, &document).unwrap();
            assert_eq!(state_model(&state), None, "{document}");
            assert_eq!(state_usage(&state), None, "{document}");
            assert_eq!(after_start(None, None, Some(&state), 1), None, "{document}");
        }
        assert!(reads(1) && reads(2) && reads(3) && reads(4) && reads(5) && !reads(6));
    }

    fn working(turn: &str) -> Event {
        Event {
            kind: tmt_core::binding::session::activity::EventKind::TurnStarted,
            turn: Some(ProviderSessionId::new(turn).unwrap()),
        }
    }

    fn idle(turn: &str) -> Event {
        Event {
            kind: tmt_core::binding::session::activity::EventKind::TurnEnded,
            turn: Some(ProviderSessionId::new(turn).unwrap()),
        }
    }

    fn observed(state: &DriverState) -> Option<(String, u64)> {
        state_observed_model(state).map(|observed| (observed.model, observed.observed_at_ms))
    }

    #[test]
    fn only_an_admitted_event_records_its_model_and_a_rejected_one_keeps_it() {
        let session = ProviderSessionId::new("s").unwrap();
        let process = ProcessIncarnation::new(42, "birth").unwrap();
        let other_session = ProviderSessionId::new("other").unwrap();
        let other_process = ProcessIncarnation::new(42, "rebirth").unwrap();
        let start = after_start(Some("m0"), None, None, 10).unwrap();
        assert_eq!(observed(&start), Some(("m0".into(), 10)));

        let a = after_activity(
            Some(&start),
            &session,
            &process,
            &working("a"),
            Some("m1"),
            20,
        )
        .unwrap();
        assert_eq!(observed(&a), Some(("m1".into(), 20)));
        let b =
            after_activity(Some(&a), &session, &process, &working("b"), Some("m2"), 30).unwrap();
        assert_eq!(observed(&b), Some(("m2".into(), 30)));
        // Every rejected event returns no state, so the stored model stays.
        for (event, session, process, now) in [
            (working("b"), &session, &process, 31),    // duplicate prompt
            (idle("a"), &session, &process, 31),       // late end of an older turn
            (idle("b"), &session, &process, 29),       // clock rollback
            (idle("b"), &other_session, &process, 31), // replaced session
            (idle("b"), &session, &other_process, 31), // replaced process
        ] {
            assert_eq!(
                after_activity(Some(&b), session, process, &event, Some("late"), now),
                None
            );
        }
        assert_eq!(
            after_activity(
                Some(&start),
                &session,
                &process,
                &idle("a"),
                Some("late"),
                21
            ),
            None,
            "an end with no admitted start"
        );

        let ended =
            after_activity(Some(&b), &session, &process, &idle("b"), Some("m3"), 40).unwrap();
        assert_eq!(observed(&ended), Some(("m3".into(), 40)));
        assert_eq!(
            after_activity(Some(&ended), &session, &process, &idle("b"), Some("m4"), 41),
            None,
            "a Stop after the interrupt is a duplicate"
        );
        // An admitted event without a valid model applies its activity and
        // keeps the previous observation with its own time.
        for model in [None, Some("-x"), Some("a b"), Some("")] {
            let next =
                after_activity(Some(&ended), &session, &process, &working("c"), model, 50).unwrap();
            assert_eq!(state_activity(&next).unwrap().since_ms, 50);
            assert_eq!(observed(&next), Some(("m3".into(), 40)), "{model:?}");
        }
        // Usage-only observations neither set nor clear it.
        let sampled = after_observation(Some(usage(5)), None, Some(&ended)).unwrap();
        assert_eq!(observed(&sampled), Some(("m3".into(), 40)));
        assert_eq!(state_usage(&sampled), Some(usage(5)));
        // Turn events never change the resume model.
        for state in [&a, &b, &ended, &sampled] {
            assert_eq!(state_model(state).as_deref(), Some("m0"));
        }
    }

    #[test]
    fn a_start_sets_the_observed_model_only_from_its_own_report() {
        let session = ProviderSessionId::new("s").unwrap();
        let process = ProcessIncarnation::new(42, "birth").unwrap();
        let start = after_start(Some("m0"), None, None, 10).unwrap();
        let turned = after_activity(
            Some(&start),
            &session,
            &process,
            &working("a"),
            Some("m1"),
            20,
        )
        .unwrap();
        let restarted = after_start(Some("m2"), None, Some(&turned), 30).unwrap();
        assert_eq!(observed(&restarted), Some(("m2".into(), 30)));
        // No report, or an unsafe one: the kept resume model is not evidence.
        for reported in [None, Some("-x")] {
            let next = after_start(reported, None, Some(&turned), 30).unwrap();
            assert_eq!(observed(&next), None, "{reported:?}");
            assert_eq!(state_model(&next).as_deref(), Some("m0"));
            assert_eq!(next, legacy("m0"));
        }
        // A start time that cannot be stored records no observation.
        assert_eq!(after_start(Some("m2"), None, None, 0), Some(legacy("m2")));
    }

    #[test]
    fn the_observed_model_has_an_exact_shape_and_can_stand_alone() {
        let alone = Document {
            observed: ObservedModel::new("m", 1),
            ..Document::default()
        }
        .write()
        .unwrap();
        assert_eq!(alone.version(), OBSERVED_STATE_VERSION);
        assert_eq!(
            alone.document(),
            r#"{"observed":{"model":"m","observedAtMs":1}}"#
        );
        assert_eq!(observed(&alone), Some(("m".into(), 1)));
        assert!(ObservedModel::new("m", MAX_JS_SAFE_INTEGER).is_some());
        assert!(ObservedModel::new("m", MAX_JS_SAFE_INTEGER + 1).is_none());
        assert!(ObservedModel::new("m", 0).is_none());

        let unsafe_time = (MAX_JS_SAFE_INTEGER + 1).to_string();
        for (version, observed_value) in [
            (5, r#"{"model":"m"}"#),
            (5, r#"{"observedAtMs":1}"#),
            (5, r#"{"model":"m","observedAtMs":1,"event":"Stop"}"#),
            (5, r#"{"model":7,"observedAtMs":1}"#),
            (5, r#"{"model":"-x","observedAtMs":1}"#),
            (5, r#"{"model":"m","observedAtMs":0}"#),
            (5, r#"{"model":"m","observedAtMs":"1"}"#),
            (
                5,
                &format!(r#"{{"model":"m","observedAtMs":{unsafe_time}}}"#),
            ),
            (5, r#""m""#),
            (5, "null"),
            (1, r#"{"model":"m","observedAtMs":1}"#),
            (3, r#"{"model":"m","observedAtMs":1}"#),
            (4, r#"{"model":"m","observedAtMs":1}"#),
        ] {
            let document = format!(r#"{{"model":"opus","observed":{observed_value}}}"#);
            let state = DriverState::new(version, &document).unwrap();
            assert_eq!(state_model(&state), None, "{version} {document}");
            assert_eq!(state_observed_model(&state), None, "{version} {document}");
        }
        // Version 5 needs the field.
        assert_eq!(
            state_model(&DriverState::new(5, r#"{"model":"opus"}"#).unwrap()),
            None
        );
    }

    #[test]
    fn older_versions_round_trip_byte_for_byte() {
        let session = ProviderSessionId::new("s").unwrap();
        let process = ProcessIncarnation::new(42, "birth").unwrap();
        let v1 = legacy("opus");
        let v2 = after_observation(Some(usage(3)), None, Some(&v1)).unwrap();
        let v3 = after_activity(Some(&v2), &session, &process, &working("a"), None, 9).unwrap();
        for (state, version) in [(&v1, 1), (&v2, 2), (&v3, 3)] {
            assert_eq!(state.version(), version);
            let rewritten = Document::read(state).unwrap().write().unwrap();
            assert_eq!(&rewritten, state);
        }
    }

    #[test]
    fn the_observed_model_goes_first_and_the_legacy_version_returns() {
        let model = "m".repeat(128);
        let session = ProviderSessionId::new(&"\"".repeat(128)).unwrap();
        let turn = "t".repeat(256);
        let mut evicted = false;
        let mut kept = false;
        for length in (1..=256).rev() {
            let process = ProcessIncarnation::new(42, &"\\".repeat(length)).unwrap();
            let previous = after_start(Some(&model), Some(usage(7)), None, 10).unwrap();
            let Some(next) = after_activity(
                Some(&previous),
                &session,
                &process,
                &working(&turn),
                Some(&model),
                20,
            ) else {
                continue; // activity alone exceeds the bound, as before v5
            };
            let without = after_activity(
                Some(&after_observation(Some(usage(7)), None, Some(&legacy(&model))).unwrap()),
                &session,
                &process,
                &working(&turn),
                None,
                20,
            )
            .unwrap();
            assert!(next.document().len() <= DriverState::MAXIMUM_BYTES);
            assert_eq!(state_activity(&next), state_activity(&without));
            assert_eq!(state_usage(&next), Some(usage(7)));
            assert_eq!(state_model(&next), Some(model.clone()));
            if state_observed_model(&next).is_none() {
                // Evicted: the same version and bytes as a document that never had it.
                evicted = true;
                assert_eq!(next, without);
                assert_eq!(next.version(), ACTIVITY_STATE_VERSION);
                let with = format!(
                    r#"{},"observed":{}}}"#,
                    without.document().strip_suffix('}').unwrap(),
                    ObservedModel::new(&model, 20).unwrap().document()
                );
                assert!(with.len() > DriverState::MAXIMUM_BYTES);
            } else {
                kept = true;
                assert_eq!(next.version(), OBSERVED_STATE_VERSION);
            }
        }
        assert!(evicted && kept, "the escaped IDs cross the bound");
    }
}
