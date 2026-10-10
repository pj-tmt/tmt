//! First-party driver state, the resume details a driver keeps for its
//! remembered session:
//! - version 1, `{"model": <slug>}`: the model a starting provider hook reported;
//! - version 2, `{"model"?: <slug>, "usage": {...}}`: also the context usage the
//!   driver last read from its own provider's transcript (#519).
//!
//! Version 3 adds main-turn activity scoped to the provider session and process.
//! Without activity, legacy model/usage documents retain versions 1 and 2.
//! Version 4 adds optional cumulative consumption and its bounded source cursor.
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

/// Whether this document version is one the first-party drivers read.
pub fn reads(version: u16) -> bool {
    matches!(
        version,
        MODEL_STATE_VERSION
            | USAGE_STATE_VERSION
            | ACTIVITY_STATE_VERSION
            | CONSUMPTION_STATE_VERSION
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
        let known = usize::from(consumption.is_some())
            + usize::from(model.is_some())
            + usize::from(usage.is_some())
            + usize::from(activity.is_some());
        let valid = match state.version() {
            MODEL_STATE_VERSION => {
                model.is_some() && usage.is_none() && activity.is_none() && consumption.is_none()
            }
            USAGE_STATE_VERSION => usage.is_some() && activity.is_none() && consumption.is_none(),
            ACTIVITY_STATE_VERSION => activity.is_some() && consumption.is_none(),
            CONSUMPTION_STATE_VERSION => consumption.is_some(),
            _ => false,
        };
        (valid && fields.len() == known).then_some(Self {
            model,
            usage,
            activity,
            consumption,
        })
    }

    fn write(self) -> Option<DriverState> {
        let mut fields = Map::new();
        if let Some(model) = self.model {
            fields.insert("model".into(), json!(model));
        }
        let mut version = match self.usage {
            Some(usage) => {
                fields.insert("usage".into(), usage.document());
                USAGE_STATE_VERSION
            }
            None if fields.is_empty() && self.activity.is_none() && self.consumption.is_none() => {
                return None;
            }
            None => MODEL_STATE_VERSION,
        };
        if let Some(activity) = self.activity {
            version = ACTIVITY_STATE_VERSION;
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
        if let Some(mut consumption) = self.consumption {
            fields.insert("consumption".into(), consumption.document());
            let document = Value::Object(fields.clone()).to_string();
            if document.len() <= DriverState::MAXIMUM_BYTES {
                return DriverState::new(CONSUMPTION_STATE_VERSION, &document).ok();
            }
            // Optional evidence goes newest first, so a later addition never
            // evicts what earlier documents already carried: limits, then
            // attribution, then the cache-write counter.
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
pub fn after_start(
    reported: Option<&str>,
    usage: Option<Usage>,
    previous: Option<&DriverState>,
) -> Option<DriverState> {
    let model = reported
        .filter(|model| valid_model(model))
        .map(str::to_owned)
        .or_else(|| previous.and_then(state_model));
    Document {
        model,
        usage,
        activity: None,
        consumption: None,
    }
    .write()
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

pub fn after_activity(
    previous: Option<&DriverState>,
    session: &ProviderSessionId,
    process: &ProcessIncarnation,
    event: &Event,
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
    document.write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::limits::MAX_JS_SAFE_INTEGER;

    fn usage(tokens: u64) -> Usage {
        Usage::new(tokens, Some(258_400), 1_780_000_000_000).unwrap()
    }

    #[test]
    fn activity_round_trips_without_transferring_through_session_start() {
        let session = ProviderSessionId::new("s").unwrap();
        let process = ProcessIncarnation::new(42, "birth").unwrap();
        let previous = after_start(Some("opus"), Some(usage(10)), None).unwrap();
        let event = Event {
            phase: ActivityPhase::Working,
            turn: Some(ProviderSessionId::new("turn").unwrap()),
        };
        let active = after_activity(Some(&previous), &session, &process, &event, 100).unwrap();
        assert_eq!(active.version(), 3);
        assert_eq!(state_model(&active).as_deref(), Some("opus"));
        assert_eq!(state_usage(&active), Some(usage(10)));
        assert!(state_activity(&active).unwrap().matches(&session, &process));
        assert!(after_activity(Some(&active), &session, &process, &event, 101).is_none());
        let updated = after_observation(Some(usage(12)), None, Some(&active)).unwrap();
        assert_eq!(state_activity(&updated), state_activity(&active));
        let resumed = after_start(None, None, Some(&active)).unwrap();
        assert_eq!(resumed.version(), 1);
        assert!(state_activity(&resumed).is_none());
        let mut bad: Value = serde_json::from_str(active.document()).unwrap();
        bad["activity"]["sinceMs"] = 101.into();
        assert!(state_activity(&DriverState::new(3, &bad.to_string()).unwrap()).is_none());
    }

    #[test]
    fn a_reported_model_replaces_and_an_absent_one_keeps_the_previous() {
        let first = after_start(Some("claude-opus-5"), None, None).unwrap();
        assert_eq!(first.version(), MODEL_STATE_VERSION);
        assert_eq!(first.document(), r#"{"model":"claude-opus-5"}"#);
        assert_eq!(state_model(&first).as_deref(), Some("claude-opus-5"));
        assert_eq!(after_start(None, None, Some(&first)), Some(first.clone()));
        let second = after_start(Some("claude-sonnet-4-5[1m]"), None, Some(&first)).unwrap();
        assert_eq!(
            state_model(&second).as_deref(),
            Some("claude-sonnet-4-5[1m]")
        );
        assert_eq!(after_start(None, None, None), None);
    }

    #[test]
    fn a_turn_records_usage_and_keeps_the_model_until_the_context_changes() {
        let model = after_start(Some("gpt-5.2-codex"), None, None).unwrap();
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
        assert_eq!(after_start(None, None, Some(&later)), Some(model.clone()));
        // A resumed conversation may report its own usage.
        let resumed = after_start(None, Some(usage(182_340)), Some(&later)).unwrap();
        assert_eq!(state_usage(&resumed), Some(usage(182_340)));
        assert_eq!(state_model(&resumed).as_deref(), Some("gpt-5.2-codex"));

        // Usage alone, with no model ever reported, is kept too.
        let bare = after_observation(Some(usage(10)), None, None).unwrap();
        assert_eq!(bare.version(), USAGE_STATE_VERSION);
        assert_eq!(state_model(&bare), None);
        assert_eq!(after_start(None, None, Some(&bare)), None);
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
        let kept = after_start(Some("gpt-5.2-codex"), None, None).unwrap();
        for model in [
            "",
            "--dangerously-skip-permissions",
            "a b",
            "x\n",
            &"m".repeat(129),
        ] {
            assert_eq!(
                after_start(Some(model), None, Some(&kept)),
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
            assert_eq!(after_start(None, None, Some(&state)), None, "{document}");
        }
        assert!(reads(1) && reads(2) && reads(3) && reads(4) && !reads(5));
    }
}
