//! The Remote door address, learned only through Remote's public CLI (`tmt remote status --json`).
//! Never Remote's private state: any failure, a missing command or a stopped door is `None`.
use std::time::{Duration, Instant};
use tmt_invoke::Request;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Door(String);
/// What `tmt remote status --json` told us.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Running(Door),
    /// Remote answered that no door runs; it may remember the last port.
    Stopped(Option<u64>),
    /// No answer: missing command, failure, timeout or an unrecognized document.
    Unknown,
}
impl Door {
    /// The running door, if any.
    pub fn discover() -> Option<Self> {
        match Self::lookup() {
            Lookup::Running(door) => Some(door),
            _ => None,
        }
    }
    pub fn lookup() -> Lookup {
        Self::run().map_or(Lookup::Unknown, |text| Self::interpret(&text))
    }
    /// One bounded call of the public CLI: its deadline, output cap and process cleanup belong
    /// to `tmt_invoke`. A missing command, failure, deadline or cap is no answer.
    fn run() -> Option<String> {
        let executable = tmt_invoke::invoking_tmt().ok()?;
        let args = ["remote".into(), "status".into(), "--json".into()];
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
        let origin = value["origin"].as_str()?;
        let path = value["path"].as_str()?;
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
        .then(|| Self(format!("{origin}{path}")))
    }
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
    /// A copyable link for a mount-relative path such as `x/colab/#space=...`.
    pub fn url(&self, relative: &str) -> String {
        format!("{}/{}", self.0, relative)
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
}
