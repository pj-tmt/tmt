//! The Remote door address, learned only through Remote's public CLI (`tmt remote status --json`).
//! Never Remote's private state: any failure, a missing command or a stopped door is `None`.
use std::time::{Duration, Instant};
use tmt_invoke::Request;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BYTES: usize = 16 * 1024;

/// One bounded call of Remote's public CLI through the invoking core: its deadline, output cap and
/// process cleanup belong to `tmt_invoke`. A missing command, failure, deadline or cap is no answer.
fn remote_json(words: &[&str]) -> Option<String> {
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
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())?
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
        remote_json(&["status", "--json"]).map_or(Lookup::Unknown, |text| Self::interpret(&text))
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
    /// The use-time line when no Remote door can be reached (#1575's wording).
    pub const INSTALL_HINT: &str =
        "Browser access needs the Remote extension: tmt extension install remote --yes";
    /// What a human is told beside a mount-relative path when no door runs: how to get a full link.
    pub fn hint(lookup: &Lookup, relative: &str) -> String {
        match lookup {
            Lookup::Running(door) => door.url(relative),
            Lookup::Stopped(Some(port)) => format!(
                "{relative} (start tmt remote serve (last door port {port}) to get a full link)"
            ),
            _ => format!("{relative} (start tmt remote serve to get a full link)"),
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
    fn the_hint_names_the_link_or_how_to_get_one() {
        let door = Door::parse(r#"{"running":true,"origin":"http://127.0.0.1:1","path":"/r/ab"}"#)
            .unwrap();
        assert_eq!(
            Door::hint(&Lookup::Running(door), "x/colab/"),
            "http://127.0.0.1:1/r/ab/x/colab/"
        );
        assert_eq!(
            Door::hint(&Lookup::Stopped(Some(7)), "p"),
            "p (start tmt remote serve (last door port 7) to get a full link)"
        );
        for lookup in [Lookup::Stopped(None), Lookup::Unknown] {
            assert_eq!(
                Door::hint(&lookup, "p"),
                "p (start tmt remote serve to get a full link)"
            );
        }
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
}
