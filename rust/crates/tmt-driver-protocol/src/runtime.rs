//! A runtime driver's checked declaration, and the hook decoding core applies
//! with it. Hooks run on every agent turn, so core reads them from the
//! declaration instead of starting the driver: this module is that decoder.

use crate::{
    MAX_HOOK_PAYLOAD_BYTES, Op,
    grammar::{self, GrammarError},
    wire::{HookEffect, HookEventDeclaration, Hooks, RuntimeCapabilities, Transition},
};
use serde_json::Value;
use std::collections::BTreeSet;

const EXECUTABLES_MAX: usize = 4;
const EXECUTABLE_MAX: usize = 64;
const EVENTS_MAX: usize = 8;
const EVENT_NAME_MAX: usize = 64;
const VALUES_MAX: usize = 16;
const VALUE_MAX: usize = 64;
const POINTER_MAX: usize = 128;
const SESSION_MAX: usize = 256;
const MODEL_MAX: usize = 256;
pub(crate) const PATH_MAX: usize = 1024;

/// The hook layout the built-in Claude and Codex hooks use.
const SESSION_HOOKS_JSON: &str = "sessionHooksJson";

/// A runtime driver's validated declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDeclaration {
    name: String,
    ops: BTreeSet<Op>,
    executables: Vec<String>,
    env: Vec<String>,
    session_env: Option<String>,
    hooks: Option<Hooks>,
}

/// What one provider hook reported, decoded from its payload. Values are
/// checked against the protocol's text rules; what the event does to a
/// binding is core's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookObservation {
    /// The declared event's name.
    pub event: String,
    pub effect: HookEffect,
    /// For `start` and `end` events.
    pub transition: Option<Transition>,
    pub session: String,
    pub model: Option<String>,
    pub transcript: Option<String>,
    pub turn: Option<String>,
}

impl RuntimeDeclaration {
    /// Checks everything but the protocol, kind and version, which
    /// [`decode_runtime_capabilities`](crate::decode_runtime_capabilities)
    /// checks for every driver kind.
    pub fn new(capabilities: &RuntimeCapabilities) -> Result<Self, GrammarError> {
        if !tmt_host_grammar::valid_name(&capabilities.name) {
            return Err(GrammarError::new("name must be [a-z][a-z0-9-]{0,31}"));
        }
        // Names this protocol doesn't know are a later addition's; ignore them.
        let ops: BTreeSet<Op> = capabilities
            .ops
            .iter()
            .filter_map(|name| Op::parse(name))
            .filter(|op| op.is_runtime())
            .collect();
        if !ops.contains(&Op::Locations) {
            return Err(GrammarError::new("ops must list locations"));
        }
        check_executables(&capabilities.executables)?;
        if capabilities.env.len() > 4 {
            return Err(GrammarError::new("env names at most 4 variables"));
        }
        for variable in capabilities
            .env
            .iter()
            .chain(capabilities.session_env.as_ref())
        {
            if !grammar::valid_env_name(variable) {
                return Err(GrammarError::new(format!(
                    "{variable:?} must be [A-Z][A-Z0-9_]* and not TMT_*"
                )));
            }
        }
        if let Some(hooks) = &capabilities.hooks {
            check_hooks(hooks)?;
        }
        if ops.contains(&Op::Usage) {
            let reports_turns = capabilities.hooks.as_ref().is_some_and(|hooks| {
                hooks.fields.transcript.is_some()
                    && hooks
                        .events
                        .iter()
                        .any(|event| event.effect == HookEffect::Idle)
            });
            if !reports_turns {
                return Err(GrammarError::new(
                    "usage needs hooks with an idle event and a transcript field",
                ));
            }
        }
        Ok(Self {
            name: capabilities.name.clone(),
            ops,
            executables: capabilities.executables.clone(),
            env: capabilities.env.clone(),
            session_env: capabilities.session_env.clone(),
            hooks: capabilities.hooks.clone(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the driver implements a runtime operation.
    pub fn supports(&self, op: Op) -> bool {
        self.ops.contains(&op)
    }

    pub fn executables(&self) -> &[String] {
        &self.executables
    }

    /// The variables `locations` reads.
    pub fn env(&self) -> &[String] {
        &self.env
    }

    pub fn session_env(&self) -> Option<&str> {
        self.session_env.as_deref()
    }

    pub fn hooks(&self) -> Option<&Hooks> {
        self.hooks.as_ref()
    }

    /// Whether a pane's command is this agent: its last path component is a
    /// declared executable. Pure: no process or file is consulted.
    pub fn claims(&self, command: &str) -> bool {
        let base = command.rsplit('/').next().unwrap_or(command);
        self.executables.iter().any(|executable| executable == base)
    }

    /// Decodes one provider hook's payload. `None` means core ignores it and
    /// changes nothing: no hooks are declared, the payload is over its bound
    /// or not one JSON object, no event matches, or a required value is
    /// missing or invalid. An invalid optional value reads as absent.
    pub fn decode_hook(&self, payload: &[u8]) -> Option<HookObservation> {
        let hooks = self.hooks.as_ref()?;
        if payload.len() > MAX_HOOK_PAYLOAD_BYTES {
            return None;
        }
        let payload: Value = serde_json::from_slice(payload).ok()?;
        payload.as_object()?;
        let text = |pointer: &str| payload.pointer(pointer).and_then(Value::as_str);
        let name = text(&hooks.fields.event)?;
        let event = hooks.events.iter().find(|event| event.name == name)?;
        let session = text(&hooks.fields.session).filter(|value| session_id(value))?;
        let transition = match event.effect {
            HookEffect::Start | HookEffect::End => {
                let by = text(event.by.as_deref()?)?;
                Some(*event.values.get(by)?)
            }
            HookEffect::Working | HookEffect::Idle => None,
        };
        let optional = |pointer: &Option<String>, valid: fn(&str) -> bool| {
            pointer
                .as_deref()
                .and_then(text)
                .filter(|value| valid(value))
                .map(str::to_owned)
        };
        Some(HookObservation {
            event: event.name.clone(),
            effect: event.effect,
            transition,
            session: session.to_owned(),
            model: optional(&hooks.fields.model, |value| {
                !value.is_empty() && grammar::plain(value, MODEL_MAX)
            }),
            transcript: optional(&hooks.fields.transcript, absolute_path),
            turn: optional(&hooks.fields.turn, session_id),
        })
    }
}

/// A provider session ID: 1–256 bytes, not blank, no control characters.
pub(crate) fn session_id(value: &str) -> bool {
    !value.trim().is_empty() && grammar::plain(value, SESSION_MAX)
}

/// An absolute path of text without `.` or `..` components.
pub(crate) fn absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && grammar::plain(value, PATH_MAX)
        && !value
            .split('/')
            .any(|component| component == "." || component == "..")
}

fn check_executables(executables: &[String]) -> Result<(), GrammarError> {
    if !(1..=EXECUTABLES_MAX).contains(&executables.len()) {
        return Err(GrammarError::new("executables names 1 to 4 commands"));
    }
    let mut seen = BTreeSet::new();
    for executable in executables {
        let valid = (1..=EXECUTABLE_MAX).contains(&executable.len())
            && executable.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && executable
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c));
        if !valid {
            return Err(GrammarError::new(format!(
                "executable {executable:?} must be 1-64 of [a-z0-9._-], starting with a letter or digit"
            )));
        }
        if !seen.insert(executable) {
            return Err(GrammarError::new(format!(
                "executable {executable:?} is listed twice"
            )));
        }
    }
    Ok(())
}

fn check_hooks(hooks: &Hooks) -> Result<(), GrammarError> {
    if hooks.format != SESSION_HOOKS_JSON {
        return Err(GrammarError::new("hooks.format must be sessionHooksJson"));
    }
    let fields = &hooks.fields;
    for pointer in [&fields.event, &fields.session]
        .into_iter()
        .chain(&fields.model)
        .chain(&fields.transcript)
        .chain(&fields.turn)
    {
        check_pointer(pointer)?;
    }
    if !(1..=EVENTS_MAX).contains(&hooks.events.len()) {
        return Err(GrammarError::new("hooks declare 1 to 8 events"));
    }
    let mut names = BTreeSet::new();
    for event in &hooks.events {
        check_event(event)?;
        if !names.insert(&event.name) {
            return Err(GrammarError::new(format!(
                "hook event {:?} is declared twice",
                event.name
            )));
        }
    }
    Ok(())
}

fn check_event(event: &HookEventDeclaration) -> Result<(), GrammarError> {
    let named = (1..=EVENT_NAME_MAX).contains(&event.name.len())
        && event
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !named {
        return Err(GrammarError::new(format!(
            "hook event name {:?} must be 1-64 of [A-Za-z0-9_]",
            event.name
        )));
    }
    let allowed: &[Transition] = match event.effect {
        HookEffect::Start => &[
            Transition::Started,
            Transition::Resumed,
            Transition::Cleared,
            Transition::Compacted,
            Transition::Forked,
        ],
        HookEffect::End => &[Transition::Cleared, Transition::Resumed, Transition::Ended],
        HookEffect::Working | HookEffect::Idle => {
            if event.by.is_some() || !event.values.is_empty() {
                return Err(GrammarError::new(format!(
                    "hook event {:?} takes no by or values",
                    event.name
                )));
            }
            return Ok(());
        }
    };
    let Some(by) = &event.by else {
        return Err(GrammarError::new(format!(
            "hook event {:?} needs by and values",
            event.name
        )));
    };
    check_pointer(by)?;
    if !(1..=VALUES_MAX).contains(&event.values.len()) {
        return Err(GrammarError::new(format!(
            "hook event {:?} maps 1 to 16 values",
            event.name
        )));
    }
    for (value, transition) in &event.values {
        if value.is_empty() || !grammar::plain(value, VALUE_MAX) {
            return Err(GrammarError::new(format!(
                "hook event {:?} has a value that isn't 1-64 bytes of text",
                event.name
            )));
        }
        if !allowed.contains(transition) {
            return Err(GrammarError::new(format!(
                "hook event {:?} can't report {transition:?}",
                event.name
            )));
        }
    }
    Ok(())
}

/// An RFC 6901 JSON Pointer to a member: it starts with `/`, and `~` only
/// begins the escapes `~0` and `~1`.
fn check_pointer(pointer: &str) -> Result<(), GrammarError> {
    let mut chars = pointer.chars().peekable();
    let mut escapes_valid = true;
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            escapes_valid = false;
        }
    }
    if pointer.starts_with('/') && grammar::plain(pointer, POINTER_MAX) && escapes_valid {
        Ok(())
    } else {
        Err(GrammarError::new(format!(
            "{pointer:?} must be a JSON Pointer of up to 128 bytes"
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wire::HookFields;
    use serde_json::json;

    /// A declaration shaped like the built-in Claude driver's hooks.
    pub(crate) fn claude_like() -> RuntimeCapabilities {
        serde_json::from_value(json!({
            "protocols": [1], "kind": "runtime", "name": "kimi", "version": "0.1.0",
            "ops": ["locations", "resume", "usage"],
            "executables": ["kimi", "kimi-cli"],
            "env": ["KIMI_HOME"],
            "sessionEnv": "KIMI_SESSION_ID",
            "hooks": {
                "format": "sessionHooksJson",
                "fields": {"event": "/hook_event_name", "session": "/session_id",
                           "model": "/model", "transcript": "/transcript_path",
                           "turn": "/turn_id"},
                "events": [
                    {"name": "SessionStart", "effect": "start", "by": "/source",
                     "values": {"startup": "started", "resume": "resumed", "clear": "cleared",
                                "compact": "compacted", "fork": "forked"}},
                    {"name": "SessionEnd", "effect": "end", "by": "/reason",
                     "values": {"clear": "cleared", "resume": "resumed", "logout": "ended"}},
                    {"name": "UserPromptSubmit", "effect": "working"},
                    {"name": "Stop", "effect": "idle"}
                ]
            }
        }))
        .unwrap()
    }

    fn declaration() -> RuntimeDeclaration {
        RuntimeDeclaration::new(&claude_like()).unwrap()
    }

    fn with(
        edit: impl FnOnce(&mut RuntimeCapabilities),
    ) -> Result<RuntimeDeclaration, GrammarError> {
        let mut capabilities = claude_like();
        edit(&mut capabilities);
        RuntimeDeclaration::new(&capabilities)
    }

    fn hooks(capabilities: &mut RuntimeCapabilities) -> &mut Hooks {
        capabilities.hooks.as_mut().unwrap()
    }

    #[test]
    fn a_minimal_declaration_needs_only_locations_and_an_executable() {
        let minimal: RuntimeCapabilities = serde_json::from_value(json!({
            "protocols": [1], "kind": "runtime", "name": "pi", "version": "1",
            "ops": ["locations", "later-op"], "executables": ["pi"]
        }))
        .unwrap();
        let declaration = RuntimeDeclaration::new(&minimal).unwrap();
        assert!(declaration.supports(Op::Locations) && !declaration.supports(Op::Resume));
        assert_eq!(declaration.session_env(), None);
        assert_eq!(
            declaration.decode_hook(br#"{"hook_event_name": "Stop"}"#),
            None
        );
    }

    #[test]
    fn unsafe_or_incoherent_declarations_are_refused() {
        type Edit = fn(&mut RuntimeCapabilities);
        let cases: [(&str, Edit); 16] = [
            ("name", |c| c.name = "Kimi".into()),
            ("no locations", |c| c.ops = vec!["resume".into()]),
            ("no executable", |c| c.executables.clear()),
            ("a path", |c| c.executables = vec!["/bin/kimi".into()]),
            ("a shell word", |c| c.executables = vec!["kimi;rm".into()]),
            ("twice", |c| {
                c.executables = vec!["kimi".into(), "kimi".into()]
            }),
            ("five", |c| {
                c.executables = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
            }),
            ("env", |c| c.env = vec!["TMT_IDENTITY".into()]),
            ("session env", |c| c.session_env = Some("lower".into())),
            ("format", |c| hooks(c).format = "toml".into()),
            ("pointer", |c| hooks(c).fields.session = "session_id".into()),
            ("escape", |c| hooks(c).fields.event = "/a~2b".into()),
            ("working with values", |c| {
                hooks(c).events[2].by = Some("/source".into());
            }),
            ("start without by", |c| hooks(c).events[0].by = None),
            ("end cannot start", |c| {
                hooks(c).events[1]
                    .values
                    .insert("startup".into(), Transition::Started);
            }),
            ("usage without turn ends", |c| {
                hooks(c)
                    .events
                    .retain(|event| event.effect != HookEffect::Idle);
            }),
        ];
        for (case, edit) in cases {
            assert!(with(edit).is_err(), "{case}");
        }
        assert!(
            with(|c| {
                c.ops.retain(|op| op != "usage");
                hooks(c).fields.transcript = None;
            })
            .is_ok(),
            "without usage, no transcript is needed"
        );
        assert!(
            with(|c| {
                let stop = hooks(c).events[3].clone();
                hooks(c).events.push(stop);
            })
            .is_err(),
            "an event declared twice"
        );
        let host_ops = with(|c| c.ops = vec!["snapshot".into(), "locations".into()]).unwrap();
        assert!(
            !host_ops.supports(Op::Snapshot) && host_ops.supports(Op::Locations),
            "a host op is no runtime op"
        );
    }

    #[test]
    fn claims_match_the_last_path_component_exactly() {
        let declaration = declaration();
        for command in ["kimi", "/usr/local/bin/kimi", "kimi-cli"] {
            assert!(declaration.claims(command), "{command}");
        }
        for command in ["kimix", "/opt/kimi/bin/claude", "", "Kimi"] {
            assert!(!declaration.claims(command), "{command}");
        }
    }

    fn payload(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn hooks_decode_by_the_declared_pointers_and_values() {
        let declaration = declaration();
        let start = declaration
            .decode_hook(&payload(json!({
                "hook_event_name": "SessionStart", "session_id": "s-1",
                "source": "resume", "model": "kimi-k2", "transcript_path": "/t/s-1.jsonl"
            })))
            .unwrap();
        assert_eq!(
            start,
            HookObservation {
                event: "SessionStart".into(),
                effect: HookEffect::Start,
                transition: Some(Transition::Resumed),
                session: "s-1".into(),
                model: Some("kimi-k2".into()),
                transcript: Some("/t/s-1.jsonl".into()),
                turn: None,
            }
        );
        let idle = declaration
            .decode_hook(&payload(json!({
                "hook_event_name": "Stop", "session_id": "s-1", "turn_id": "t-9",
                "transcript_path": "relative.jsonl", "model": "bad\u{7}"
            })))
            .unwrap();
        assert_eq!(
            (idle.effect, idle.transition, idle.turn.as_deref()),
            (HookEffect::Idle, None, Some("t-9"))
        );
        assert_eq!(
            (idle.transcript, idle.model),
            (None, None),
            "an invalid optional value reads as absent"
        );
    }

    #[test]
    fn a_payload_core_cannot_read_changes_nothing() {
        let declaration = declaration();
        let ignored = |value: serde_json::Value| declaration.decode_hook(&payload(value));
        assert_eq!(
            ignored(json!({"hook_event_name": "Notification", "session_id": "s"})),
            None
        );
        assert_eq!(
            ignored(json!({"hook_event_name": "Stop"})),
            None,
            "no session"
        );
        assert_eq!(
            ignored(json!({"hook_event_name": "Stop", "session_id": " "})),
            None
        );
        assert_eq!(
            ignored(json!({"hook_event_name": "Stop", "session_id": 7})),
            None
        );
        assert_eq!(
            ignored(json!({"hook_event_name": "SessionEnd", "session_id": "s", "reason": "crash"})),
            None,
            "an unmapped value"
        );
        assert_eq!(
            ignored(json!({"hook_event_name": "SessionStart", "session_id": "s"})),
            None,
            "a start without its value"
        );
        assert_eq!(ignored(json!(["Stop"])), None, "not an object");
        assert_eq!(declaration.decode_hook(b"{"), None);
        let mut big = payload(json!({"hook_event_name": "Stop", "session_id": "s", "pad": ""}));
        big.splice(
            big.len() - 2..big.len() - 2,
            std::iter::repeat_n(b'x', MAX_HOOK_PAYLOAD_BYTES),
        );
        assert_eq!(declaration.decode_hook(&big), None, "over 64 KiB");
    }

    #[test]
    fn pointers_follow_rfc_6901_escapes() {
        let mut capabilities = claude_like();
        hooks(&mut capabilities).fields = HookFields {
            event: "/hook~1event".into(),
            session: "/ids/session~0id".into(),
            model: None,
            transcript: Some("/transcript".into()),
            turn: None,
        };
        let declaration = RuntimeDeclaration::new(&capabilities).unwrap();
        let observed = declaration
            .decode_hook(&payload(json!({
                "hook/event": "Stop", "ids": {"session~id": "s-2"}
            })))
            .unwrap();
        assert_eq!(observed.session, "s-2");
    }

    #[test]
    fn paths_are_absolute_without_dot_components() {
        assert!(absolute_path("/Users/ann/.kimi/sessions"));
        for path in ["relative", "/", "/a/../b", "/a/./b", "/a\u{1b}b"] {
            assert!(!absolute_path(path), "{path}");
        }
        assert!(!absolute_path(&format!("/{}", "a".repeat(PATH_MAX))));
    }
}
