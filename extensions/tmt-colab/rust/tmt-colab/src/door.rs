//! The Remote door address, learned only through Remote's public CLI (`tmt remote status --json`).
//! Never Remote's private state: any failure, a missing command or a stopped door is `None`.
use std::time::{Duration, Instant};
use tmt_invoke::Request;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BYTES: usize = 16 * 1024;

/// One bounded call of Remote's public CLI through the invoking core: its deadline, output cap and
/// process cleanup belong to `tmt_invoke`. A missing executable, deadline or output cap is no
/// answer; a command that ran returns whether it succeeded and what it printed.
fn remote_call(words: &[&str]) -> Option<(bool, String)> {
    let executable = tmt_invoke::invoking_tmt().ok()?;
    let args: Vec<_> = std::iter::once("remote")
        .chain(words.iter().copied())
        .map(Into::into)
        .collect();
    let output = tmt_invoke::invoke(
        Request {
            program: &executable,
            args: &args,
            input: &[],
            deadline: Instant::now() + TIMEOUT,
            max_stream_bytes: MAX_BYTES,
            launch: Default::default(),
        },
        None,
    )
    .ok()?;
    Some((
        output.status.success(),
        String::from_utf8(output.stdout).ok()?,
    ))
}
/// One creation snapshot reused for recipient provenance and link presentation.
pub struct CreationObservation {
    pub lookup: Lookup,
    pub machine_id: Option<String>,
}
pub fn creation_observation() -> CreationObservation {
    let result = remote_call(&["status", "--machine", "--json"]);
    result
        .and_then(|(ok, reply)| ok.then(|| creation_observation_from_reply(&reply)))
        .unwrap_or(CreationObservation {
            lookup: Lookup::Unknown,
            machine_id: None,
        })
}
fn creation_observation_from_reply(reply: &str) -> CreationObservation {
    let unknown = || CreationObservation {
        lookup: Lookup::Unknown,
        machine_id: None,
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Stopped {
        running: bool,
        #[serde(deserialize_with = "required_port")]
        last_port: Option<u16>,
    }
    fn required_port<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u16>, D::Error> {
        serde::Deserialize::deserialize(deserializer)
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Running {
        running: bool,
        origin: String,
        path: String,
        machine_id: String,
    }
    // Typed documents reject missing, unknown and duplicate fields before interpretation.
    if let Ok(stopped) = serde_json::from_str::<Stopped>(reply)
        && !stopped.running
        && stopped.last_port != Some(0)
    {
        return CreationObservation {
            lookup: Lookup::Stopped(stopped.last_port.map(u64::from)),
            machine_id: None,
        };
    }
    let parsed = (|| {
        let running = serde_json::from_str::<Running>(reply).ok()?;
        if !running.running {
            return None;
        }
        let origin = &running.origin;
        let port = origin
            .strip_prefix("http://127.0.0.1:")?
            .parse::<u16>()
            .ok()?;
        if port == 0 || origin != &format!("http://127.0.0.1:{port}") {
            return None;
        }
        let prefix = running.path.strip_prefix("/r/")?;
        if prefix.len() != 16
            || !prefix
                .bytes()
                .all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
        {
            return None;
        }
        let door = Door::from_parts(origin, &running.path)?;
        tmt_colab_model::values::generated_id(&running.machine_id).ok()?;
        Some(CreationObservation {
            lookup: Lookup::Running(door),
            machine_id: Some(running.machine_id),
        })
    })();
    parsed.unwrap_or_else(unknown)
}
/// The answer of a call that succeeded; anything else is no answer.
fn remote_json(words: &[&str]) -> Option<String> {
    remote_call(words).and_then(|(ok, text)| ok.then_some(text))
}

/// A Remote error envelope, `{"error":{"code","message"}}`, shown as Remote wrote it.
#[derive(Debug, PartialEq, Eq)]
pub struct RemoteFailure {
    pub code: String,
    pub message: String,
}
impl RemoteFailure {
    pub fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        Some(Self {
            code: value["error"]["code"].as_str()?.to_owned(),
            message: value["error"]["message"].as_str()?.to_owned(),
        })
    }
    /// What a person reads: one shared wording for a serve that predates `status` and `stop`,
    /// else Remote's own message.
    pub fn text(&self) -> &str {
        if self.code == "REMOTE_SERVE_OUTDATED" {
            Door::OUTDATED
        } else {
            &self.message
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Door {
    origin: String,
    /// `/r/<prefix>`.
    path: String,
}
/// What `tmt remote status --json` told us.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Running(Door),
    /// Remote answered that no door runs; it may remember the last port.
    Stopped(Option<u64>),
    /// Remote ran and answered with an error envelope (for example `REMOTE_SERVE_OUTDATED`). It is
    /// shown as it is, and nothing is started in its place.
    Failed(RemoteFailure),
    /// No answer: missing command, failure, timeout or an unrecognized document.
    Unknown,
}
/// Whether any device is paired with the door, from `tmt remote devices --json`.
#[derive(Debug, PartialEq, Eq)]
pub enum Pairing {
    /// This many devices are paired and not revoked.
    Paired(usize),
    Unpaired,
    /// No usable answer: a missing command, a failure, a timeout or an unrecognized document.
    Unknown,
}
impl Pairing {
    /// The explicit next step while a door runs and no browser is known to be paired.
    pub fn step(&self) -> Option<&'static str> {
        match self {
            Self::Paired(_) => None,
            Self::Unpaired => Some("pair for page access: tmt remote pair"),
            Self::Unknown => Some("if this browser is new, pair for page access: tmt remote pair"),
        }
    }
    /// `{"devices":[{"revoked":false,...},...]}`: revoked devices no longer count.
    pub fn lookup() -> Self {
        remote_json(&["devices", "--json"]).map_or(Self::Unknown, |text| Self::interpret(&text))
    }
    pub fn interpret(json: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let Some(devices) = value["devices"].as_array() else {
            return Self::Unknown;
        };
        match devices.iter().filter(|d| d["revoked"] != true).count() {
            0 => Self::Unpaired,
            n => Self::Paired(n),
        }
    }
}
impl Door {
    pub fn lookup() -> Lookup {
        match remote_call(&["status", "--json"]) {
            Some((true, text)) => Self::interpret(&text),
            // A non-zero exit still prints Remote's error envelope on stdout.
            Some((false, text)) => {
                RemoteFailure::parse(&text).map_or(Lookup::Unknown, Lookup::Failed)
            }
            None => Lookup::Unknown,
        }
    }
    /// `{"running":false,"lastPort":53253|null}` is a stopped door; anything else unusable is unknown.
    pub fn interpret(json: &str) -> Lookup {
        if let Some(door) = Self::parse(json) {
            return Lookup::Running(door);
        }
        match serde_json::from_str::<serde_json::Value>(json) {
            Ok(value) if value["running"] == false => Lookup::Stopped(value["lastPort"].as_u64()),
            _ => Lookup::Unknown,
        }
    }
    /// `{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/<prefix>"}`; unknown fields are ignored.
    pub fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        if value["running"] != true {
            return None;
        }
        Self::from_parts(value["origin"].as_str()?, value["path"].as_str()?)
    }
    /// The first line of `tmt remote serve --json`: `{"state":"ready","address":"<origin>/r/<prefix>",...}`.
    pub fn from_descriptor(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        if value["state"] != "ready" {
            return None;
        }
        let address = value["address"].as_str()?;
        let at = address.find("/r/")?;
        Self::from_parts(&address[..at], &address[at..])
    }
    fn from_parts(origin: &str, path: &str) -> Option<Self> {
        let authority = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))?;
        let prefix = path.strip_prefix("/r/")?;
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']');
        (!authority.is_empty()
            && authority.chars().all(plain)
            && !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
        .then(|| Self {
            origin: origin.into(),
            path: path.into(),
        })
    }
    /// The one wording for a running Remote serve that predates `status` and `stop`
    /// (`REMOTE_SERVE_OUTDATED`): it cannot be stopped from here, and a second door must not start.
    pub const OUTDATED: &str = "The running Remote serve is older than this Colab. Stop it with Ctrl-C in its terminal, then run tmt colab serve.";
    /// The use-time line when no Remote door can be reached (#1575's wording).
    pub const INSTALL_HINT: &str =
        "Browser access needs the Remote extension: tmt extension install remote --yes";
    /// What `tmt colab stop` says about a door it did not start: Remote's own command stops it.
    pub const ATTACHED_STOP_NOTE: &str = "Remote is still running; stop it with tmt remote stop";
    /// What a human is told for a mount-relative path: the full link while a door runs, else the
    /// path and why there is no full link. Never a manual `tmt remote serve`: `tmt colab serve`
    /// starts the door itself.
    pub fn hint(lookup: &Lookup, relative: &str) -> String {
        match lookup {
            Lookup::Running(door) => door.url(relative),
            Lookup::Stopped(_) => format!("{relative} (run tmt colab serve to get a full link)"),
            Lookup::Failed(failure) => format!("{relative} ({})", failure.text()),
            Lookup::Unknown => format!("{relative} ({})", Self::INSTALL_HINT),
        }
    }
    /// `http://127.0.0.1:<port>`, the loopback authority a browser opens.
    pub fn origin(&self) -> &str {
        &self.origin
    }
    /// A copyable link for a mount-relative path such as `x/colab/#space=...`.
    pub fn url(&self, relative: &str) -> String {
        format!("{}{}/{}", self.origin, self.path, relative)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn creation_observation_requires_exact_complete_documents() {
        use super::creation_observation_from_reply as parse;
        for (reply, port) in [
            (r#"{"running":false,"lastPort":null}"#, None),
            (r#"{"running":false,"lastPort":53253}"#, Some(53253)),
        ] {
            let observed = parse(reply);
            assert_eq!(observed.lookup, Lookup::Stopped(port));
            assert_eq!(observed.machine_id, None);
        }
        let running = r#"{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345","machineId":"40000000-0000-4000-8000-000000000001"}"#;
        let observed = parse(running);
        assert!(matches!(observed.lookup, Lookup::Running(_)));
        assert_eq!(
            observed.machine_id.as_deref(),
            Some("40000000-0000-4000-8000-000000000001")
        );
        for bad in [
            r#"{"running":false,"unexpected":1}"#.to_owned(),
            r#"{"running":false}"#.to_owned(),
            r#"{"running":false,"running":false,"lastPort":null}"#.to_owned(),
            r#"{"running":false,"lastPort":null,"lastPort":null}"#.to_owned(),
            r#"{"running":false,"lastPort":0}"#.to_owned(),
            r#"{"running":false,"lastPort":65536}"#.to_owned(),
            r#"{"running":false,"lastPort":null,"extra":1}"#.to_owned(),
            running.replace("true,", "true,\"running\":true,"),
            running.replace(
                "\"machineId\":",
                "\"machineId\":\"40000000-0000-4000-8000-000000000001\",\"machineId\":",
            ),
            running.replace("abcdefghijkl2345", "abcdefghijkl2340"),
            running.replace("abcdefghijkl2345", "abc"),
            running.replace(":53253", ":053253"),
            running.replace("40000000-0000-4000", "40000000-0000-1000"),
            r#"{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345"}"#
                .to_owned(),
        ] {
            let observed = parse(&bad);
            assert_eq!(observed.lookup, Lookup::Unknown, "{bad}");
            assert_eq!(observed.machine_id, None, "{bad}");
        }
    }
    #[test]
    fn the_hint_names_the_link_or_the_reason_there_is_none() {
        let door = Door::parse(r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/ab"}"#)
            .unwrap();
        assert_eq!(
            Door::hint(&Lookup::Running(door), "x/colab/"),
            "http://127.0.0.1:1/r/ab/x/colab/"
        );
        for stopped in [Lookup::Stopped(Some(7)), Lookup::Stopped(None)] {
            assert_eq!(
                Door::hint(&stopped, "p"),
                "p (run tmt colab serve to get a full link)"
            );
        }
        assert_eq!(
            Door::hint(&Lookup::Unknown, "p"),
            format!("p ({})", Door::INSTALL_HINT)
        );
    }
    use super::{Door, Lookup};
    #[test]
    fn interprets_running_stopped_and_unusable_answers() {
        let running =
            r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/ab","future":[1]}"#;
        assert!(matches!(Door::interpret(running), Lookup::Running(_)));
        assert_eq!(
            Door::interpret(r#"{"running":false,"lastPort":53253}"#),
            Lookup::Stopped(Some(53253))
        );
        assert_eq!(
            Door::interpret(r#"{"running":false,"lastPort":null}"#),
            Lookup::Stopped(None)
        );
        assert_eq!(
            Door::interpret(r#"{"running":false}"#),
            Lookup::Stopped(None)
        );
        for unusable in [
            "",
            r#"{"error":{"code":"REMOTE_X","message":"m"}}"#,
            r#"{"running":true}"#,
        ] {
            assert_eq!(Door::interpret(unusable), Lookup::Unknown, "{unusable}");
        }
    }
    #[test]
    fn accepts_only_a_running_door_with_a_plain_origin_and_prefix() {
        let ok = r#"{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/3e2c69f7"}"#;
        assert_eq!(
            Door::parse(ok).unwrap().url("x/colab/#a=b"),
            "http://127.0.0.1:53253/r/3e2c69f7/x/colab/#a=b"
        );
        for bad in [
            "",
            "{}",
            "not json",
            r#"{"running":false,"origin":"http://127.0.0.1:1","path":"/r/ab"}"#,
            r#"{"running":"true","origin":"http://127.0.0.1:1","path":"/r/ab"}"#,
            r#"{"running":true,"origin":"ftp://127.0.0.1:1","path":"/r/ab"}"#,
            r#"{"running":true,"origin":"http://user@127.0.0.1:1","path":"/r/ab"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1/x","path":"/r/ab"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1?q","path":"/r/ab"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/x/ab"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/a/b"}"#,
            r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/AB"}"#,
        ] {
            assert_eq!(Door::parse(bad), None, "{bad}");
        }
    }
    #[test]
    fn reads_the_address_of_a_ready_serve_descriptor() {
        let ready = r#"{"profile":"local-v1","state":"ready","address":"http://127.0.0.1:53253/r/3e2c69f7","machineId":"m"}"#;
        assert_eq!(
            Door::from_descriptor(ready).unwrap().url("x/colab/"),
            "http://127.0.0.1:53253/r/3e2c69f7/x/colab/"
        );
        for bad in [
            "",
            "{}",
            r#"{"state":"starting","address":"http://127.0.0.1:1/r/ab"}"#,
            r#"{"state":"ready","address":"http://127.0.0.1:1"}"#,
            r#"{"state":"ready","address":"http://127.0.0.1:1/x/ab"}"#,
            r#"{"state":"ready","address":"http://127.0.0.1:1/r/A"}"#,
        ] {
            assert_eq!(Door::from_descriptor(bad), None, "{bad}");
        }
    }
    #[test]
    fn counts_only_devices_that_are_not_revoked() {
        use super::Pairing;
        assert_eq!(
            Pairing::interpret(r#"{"devices":[{"revoked":false},{"revoked":true},{"name":"x"}]}"#),
            Pairing::Paired(2)
        );
        assert_eq!(
            Pairing::interpret(r#"{"devices":[{"revoked":true}]}"#),
            Pairing::Unpaired
        );
        assert_eq!(Pairing::interpret(r#"{"devices":[]}"#), Pairing::Unpaired);
        for unusable in ["", "{}", r#"{"devices":null}"#, r#"{"error":{}}"#] {
            assert_eq!(Pairing::interpret(unusable), Pairing::Unknown, "{unusable}");
        }
    }
    #[test]
    fn a_remote_error_envelope_is_a_failure_with_one_wording_for_an_outdated_serve() {
        use super::RemoteFailure;
        let outdated =
            RemoteFailure::parse(r#"{"error":{"code":"REMOTE_SERVE_OUTDATED","message":"older"}}"#)
                .unwrap();
        assert_eq!(outdated.text(), Door::OUTDATED);
        let other =
            RemoteFailure::parse(r#"{"error":{"code":"REMOTE_X","message":"Remote's words"}}"#)
                .unwrap();
        assert_eq!(other.text(), "Remote's words");
        for not_an_envelope in [
            "",
            "{}",
            r#"{"error":{"code":"X"}}"#,
            "not json",
            r#"{"running":false}"#,
        ] {
            assert_eq!(
                RemoteFailure::parse(not_an_envelope),
                None,
                "{not_an_envelope}"
            );
        }
        assert_eq!(
            Door::hint(&Lookup::Failed(outdated), "p"),
            format!("p ({})", Door::OUTDATED)
        );
    }
}
