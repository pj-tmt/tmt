//! Core's side: turn what a driver printed into checked values. Output over
//! the operation's bound, JSON that isn't one envelope, or a value outside
//! the declared grammar is a failure; nothing is repaired or guessed.

use crate::{
    Op, PROTOCOL,
    grammar::{self, Grammar, GrammarError},
    wire::*,
};
use serde::de::DeserializeOwned;
use std::{collections::BTreeSet, fmt};

const MESSAGE_MAX: usize = 512;
const VERSION_MAX: usize = 64;
const PANES_MAX: usize = 4096;
const CWD_MAX: usize = 4096;
const COMMAND_MAX: usize = 256;
const NAME_MAX: usize = 256;
const START_TIME_MAX: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLarge {
        op: Op,
        limit: usize,
    },
    Json {
        op: Op,
        reason: String,
    },
    Invalid {
        op: Op,
        reason: String,
    },
    Grammar(GrammarError),
    /// The driver speaks no protocol this core does.
    Protocol(Vec<u32>),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { op, limit } => {
                write!(output, "{} printed more than {limit} bytes", op.as_str())
            }
            Self::Json { op, reason } => {
                write!(output, "{} printed invalid JSON: {reason}", op.as_str())
            }
            Self::Invalid { op, reason } => write!(output, "{}: {reason}", op.as_str()),
            Self::Grammar(error) => write!(output, "capabilities: {error}"),
            Self::Protocol(offered) => write!(
                output,
                "the driver speaks protocol {offered:?}; this tmt speaks {PROTOCOL}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A successful answer to one operation, checked against the grammar.
pub trait Answer: DeserializeOwned {
    const OP: Op;
    fn check(&self, grammar: &Grammar) -> Result<(), String>;
}

/// The driver's answer: its value, or the error it reported.
pub fn decode<T: Answer>(
    grammar: &Grammar,
    output: &[u8],
) -> Result<Result<T, DriverError>, DecodeError> {
    match envelope::<T>(T::OP, output)? {
        Response::Ok(value) => {
            value
                .check(grammar)
                .map_err(|reason| DecodeError::Invalid { op: T::OP, reason })?;
            Ok(Ok(value))
        }
        Response::Error(error) => Ok(Err(error)),
    }
}

/// A driver's `capabilities`, and the grammar they declare. A driver that
/// can't answer `capabilities` isn't usable, so an error answer fails too.
pub fn decode_capabilities(output: &[u8]) -> Result<(Capabilities, Grammar), DecodeError> {
    let op = Op::Capabilities;
    let invalid = |reason: &str| DecodeError::Invalid {
        op,
        reason: reason.into(),
    };
    let capabilities = match envelope::<Capabilities>(op, output)? {
        Response::Ok(capabilities) => capabilities,
        Response::Error(error) => {
            return Err(invalid(&format!("the driver refused: {}", error.message)));
        }
    };
    if !capabilities.protocols.contains(&PROTOCOL) {
        return Err(DecodeError::Protocol(capabilities.protocols));
    }
    if capabilities.kind != "host" {
        return Err(invalid("kind must be host"));
    }
    if capabilities.version.is_empty() || !grammar::plain(&capabilities.version, VERSION_MAX) {
        return Err(invalid("version must be 1-64 bytes of text"));
    }
    let grammar = Grammar::from_capabilities(&capabilities).map_err(DecodeError::Grammar)?;
    Ok((capabilities, grammar))
}

fn envelope<T: DeserializeOwned>(op: Op, output: &[u8]) -> Result<Response<T>, DecodeError> {
    let limit = op.bounds().max_output_bytes;
    if output.len() > limit {
        return Err(DecodeError::TooLarge { op, limit });
    }
    let response: Response<T> =
        serde_json::from_slice(output).map_err(|error| DecodeError::Json {
            op,
            reason: error.to_string(),
        })?;
    if let Response::Error(error) = &response
        && !grammar::plain(&error.message, MESSAGE_MAX)
    {
        return Err(DecodeError::Invalid {
            op,
            reason: "the error message must be up to 512 bytes of text".into(),
        });
    }
    Ok(response)
}

fn require(ok: bool, reason: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(reason.to_owned()) }
}

fn optional(value: &Option<String>, max: usize) -> bool {
    value
        .as_deref()
        .is_none_or(|value| grammar::plain(value, max))
}

fn check_pane(pane: &Pane, grammar: &Grammar) -> Result<(), String> {
    require(
        grammar.is_pane_id(&pane.id),
        "a pane ID is outside the declared syntax",
    )?;
    require(
        pane.target
            .as_deref()
            .is_none_or(|target| grammar.is_target(target)),
        "a pane target is outside the declared syntax",
    )?;
    require(
        pane.cwd
            .as_deref()
            .is_none_or(|cwd| cwd.starts_with('/') && grammar::plain(cwd, CWD_MAX)),
        "a pane cwd must be an absolute path of text",
    )?;
    require(
        grammar::plain(&pane.command, COMMAND_MAX),
        "a pane command must be text",
    )?;
    require(grammar::pid(pane.pane_pid), "a pane pid is out of range")?;
    require(
        optional(&pane.suggested_name, NAME_MAX),
        "a suggested name must be text",
    )?;
    if let Some(marker) = &pane.marker {
        check_marker(marker)?;
    }
    Ok(())
}

fn check_marker(marker: &Marker) -> Result<(), String> {
    require(
        !marker.name.is_empty()
            && grammar::plain(&marker.name, NAME_MAX)
            && !marker.canonical_name.is_empty()
            && grammar::plain(&marker.canonical_name, NAME_MAX),
        "a marker name must be text",
    )?;
    require(
        [&marker.identity_id, &marker.binding_id, &marker.server_id]
            .into_iter()
            .all(|id| grammar::uuid_shaped(id)),
        "a marker ID is not a UUID",
    )?;
    require(
        grammar::pid(marker.pane_pid),
        "a marker pid is out of range",
    )
}

fn check_panes(panes: &[Pane], grammar: &Grammar) -> Result<(), String> {
    require(panes.len() <= PANES_MAX, "more than 4096 panes")?;
    let mut seen = BTreeSet::new();
    for pane in panes {
        check_pane(pane, grammar)?;
        require(seen.insert(pane.id.as_str()), "a pane is listed twice")?;
    }
    Ok(())
}

/// Whether every pane is one of those asked about; `None` asked for all.
pub(crate) fn within(panes: &[Pane], requested: Option<&[String]>) -> bool {
    requested.is_none_or(|requested| panes.iter().all(|pane| requested.contains(&pane.id)))
}

impl SnapshotResponse {
    /// A snapshot limited to some panes reports only those.
    pub fn within(&self, requested: Option<&[String]>) -> bool {
        within(&self.panes, requested)
    }
}

impl ProbeResponse {
    pub fn within(&self, requested: &[String]) -> bool {
        match self {
            Self::Live { panes } => within(panes, Some(requested)),
            Self::Dead | Self::Unknown => true,
        }
    }
}

impl Answer for CallerResponse {
    const OP: Op = Op::Caller;
    fn check(&self, grammar: &Grammar) -> Result<(), String> {
        let Some(pane) = &self.pane else {
            return Ok(());
        };
        require(
            grammar.is_pane_id(&pane.id),
            "the caller pane ID is outside the declared syntax",
        )?;
        require(
            grammar::socket(&pane.socket),
            "the caller socket must be an absolute path",
        )?;
        require(
            grammar::pid(pane.shell_pid),
            "the caller shell pid is out of range",
        )
    }
}

impl Answer for ServerResponse {
    const OP: Op = Op::Server;
    fn check(&self, _: &Grammar) -> Result<(), String> {
        let Some(server) = &self.server else {
            return Ok(());
        };
        require(
            grammar::socket(&server.socket),
            "the server socket must be an absolute path",
        )?;
        require(grammar::pid(server.pid), "the server pid is out of range")?;
        require(
            !server.start_time.is_empty() && grammar::plain(&server.start_time, START_TIME_MAX),
            "the server start time must be 1-64 bytes of text",
        )
    }
}

impl Answer for ResolveTargetResponse {
    const OP: Op = Op::ResolveTarget;
    fn check(&self, grammar: &Grammar) -> Result<(), String> {
        require(
            self.pane_id
                .as_deref()
                .is_none_or(|id| grammar.is_pane_id(id)),
            "the resolved pane ID is outside the declared syntax",
        )
    }
}

impl Answer for SnapshotResponse {
    const OP: Op = Op::Snapshot;
    fn check(&self, grammar: &Grammar) -> Result<(), String> {
        check_panes(&self.panes, grammar)
    }
}

impl Answer for ProbeResponse {
    const OP: Op = Op::Probe;
    fn check(&self, grammar: &Grammar) -> Result<(), String> {
        match self {
            Self::Live { panes } => check_panes(panes, grammar),
            Self::Dead | Self::Unknown => Ok(()),
        }
    }
}

impl Answer for ClearResponse {
    const OP: Op = Op::Clear;
    fn check(&self, _: &Grammar) -> Result<(), String> {
        Ok(())
    }
}

impl Answer for CaptureResponse {
    const OP: Op = Op::Capture;
    /// Captured text is terminal output; core escapes it where it shows it.
    fn check(&self, _: &Grammar) -> Result<(), String> {
        Ok(())
    }
}

/// The answer to `publish`, `input` or `focus`: `{"ok": {}}`, or the
/// driver's error.
pub fn decode_done(op: Op, output: &[u8]) -> Result<Result<(), DriverError>, DecodeError> {
    Ok(match envelope::<Empty>(op, output)? {
        Response::Ok(Empty {}) => Ok(()),
        Response::Error(error) => Err(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::tests::herdr_like;
    use serde_json::json;

    fn grammar() -> Grammar {
        Grammar::from_capabilities(&herdr_like()).unwrap()
    }

    fn bytes(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    fn pane(id: &str) -> serde_json::Value {
        json!({
            "id": id, "target": "w1:p2", "cwd": "/src", "command": "claude",
            "panePid": 42, "suggestedName": null,
            "marker": {
                "name": "Ann", "canonicalName": "ann",
                "identityId": "0f8fad5b-d9cb-469f-a165-70867728950e",
                "bindingId": "1f8fad5b-d9cb-469f-a165-70867728950e",
                "serverId": "2f8fad5b-d9cb-469f-a165-70867728950e",
                "panePid": 42
            }
        })
    }

    #[test]
    fn capabilities_decode_with_their_grammar() {
        let output = bytes(json!({"ok": herdr_like()}));
        let (capabilities, grammar) = decode_capabilities(&output).unwrap();
        assert_eq!(capabilities.name, "herdr");
        assert!(grammar.is_pane_id("term_1"));
        let mut other = herdr_like();
        other.protocols = vec![2];
        assert_eq!(
            decode_capabilities(&bytes(json!({"ok": other}))),
            Err(DecodeError::Protocol(vec![2]))
        );
        let mut runtime = herdr_like();
        runtime.kind = "runtime".into();
        assert!(decode_capabilities(&bytes(json!({"ok": runtime}))).is_err());
        let refused = bytes(json!({"error": {"code": "failed", "message": "no"}}));
        assert!(decode_capabilities(&refused).is_err());
    }

    #[test]
    fn a_snapshot_is_checked_pane_by_pane() {
        let good = bytes(json!({"ok": {"panes": [pane("term_1"), pane("term_2")]}}));
        let snapshot: SnapshotResponse = decode(&grammar(), &good).unwrap().unwrap();
        assert_eq!(snapshot.panes.len(), 2);
        assert!(snapshot.within(Some(&["term_1".into(), "term_2".into()])));
        assert!(
            !snapshot.within(Some(&["term_1".into()])),
            "an unasked pane"
        );
        assert!(snapshot.within(None));

        let bad = |edit: fn(&mut serde_json::Value)| {
            let mut value = pane("term_1");
            edit(&mut value);
            decode::<SnapshotResponse>(&grammar(), &bytes(json!({"ok": {"panes": [value]}})))
        };
        for edit in [
            (|p: &mut serde_json::Value| p["id"] = json!("%1")) as fn(&mut serde_json::Value),
            |p| p["target"] = json!("1.2"),
            |p| p["cwd"] = json!("relative"),
            |p| p["command"] = json!("cl\u{1b}aude"),
            |p| p["panePid"] = json!(0),
            |p| p["marker"]["bindingId"] = json!("not-a-uuid"),
            |p| p["marker"]["name"] = json!(""),
        ] {
            assert!(
                matches!(
                    bad(edit),
                    Err(DecodeError::Invalid {
                        op: Op::Snapshot,
                        ..
                    })
                ),
                "{:?}",
                bad(edit)
            );
        }
        let twice = bytes(json!({"ok": {"panes": [pane("term_1"), pane("term_1")]}}));
        assert!(decode::<SnapshotResponse>(&grammar(), &twice).is_err());
    }

    #[test]
    fn output_over_the_bound_or_not_one_envelope_fails() {
        let big = vec![b' '; 4097];
        assert_eq!(
            decode::<ClearResponse>(&grammar(), &big),
            Err(DecodeError::TooLarge {
                op: Op::Clear,
                limit: 4096
            })
        );
        for output in [
            &b"cleared"[..],
            br#"{"ok": {"cleared": true}} {"ok": {}}"#,
            br#"{"ok": {"cleared": true}, "error": {"code": "failed", "message": ""}}"#,
            br#"{"ok": {}}"#,
        ] {
            assert!(
                matches!(
                    decode::<ClearResponse>(&grammar(), output),
                    Err(DecodeError::Json { .. })
                ),
                "{}",
                String::from_utf8_lossy(output)
            );
        }
    }

    #[test]
    fn a_driver_error_is_its_answer_when_its_message_is_text() {
        let output = bytes(json!({"error": {"code": "unsupported", "message": "no input"}}));
        assert_eq!(
            decode_done(Op::Input, &output),
            Ok(Err(DriverError::new(ErrorCode::Unsupported, "no input")))
        );
        let noisy = bytes(json!({"error": {"code": "failed", "message": "\u{1b}]52;c;x\u{7}"}}));
        assert!(decode::<ClearResponse>(&grammar(), &noisy).is_err());
    }

    #[test]
    fn caller_server_and_target_answers_are_checked() {
        let caller = |pane: serde_json::Value| {
            decode::<CallerResponse>(&grammar(), &bytes(json!({"ok": {"pane": pane}})))
        };
        assert!(caller(json!(null)).is_ok());
        assert!(caller(json!({"id": "term_1", "socket": "/h.sock", "shellPid": 7})).is_ok());
        assert!(caller(json!({"id": "term_1", "socket": "h.sock", "shellPid": 7})).is_err());
        let server = |value: serde_json::Value| {
            decode::<ServerResponse>(&grammar(), &bytes(json!({"ok": {"server": value}})))
        };
        assert!(server(json!({"socket": "/h", "pid": 9, "startTime": "Mon 10:00"})).is_ok());
        assert!(server(json!({"socket": "/h", "pid": 9, "startTime": ""})).is_err());
        let target = |id: &str| {
            decode::<ResolveTargetResponse>(&grammar(), &bytes(json!({"ok": {"paneId": id}})))
        };
        assert!(target("term_5").is_ok() && target("w1:p1").is_err());
        let probe = bytes(json!({"ok": {"state": "live", "panes": [pane("term_1")]}}));
        let probe: ProbeResponse = decode(&grammar(), &probe).unwrap().unwrap();
        assert!(probe.within(&["term_1".into()]) && !probe.within(&[]));
    }
}
