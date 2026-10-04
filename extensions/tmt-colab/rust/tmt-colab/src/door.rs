//! The Remote door address, learned only through Remote's public CLI (`tmt remote status --json`).
//! Never Remote's private state: any failure, a missing command or a stopped door is `None`.
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BYTES: u64 = 16 * 1024;

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
    fn run() -> Option<String> {
        let tmt = tmt_invoke::invoking_tmt().ok()?;
        let mut child = Command::new(tmt)
            .args(["remote", "status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.by_ref().take(MAX_BYTES).read_to_string(&mut text);
            let _ = tx.send(text);
        });
        let text = rx.recv_timeout(TIMEOUT).ok();
        let status = match child.try_wait() {
            Ok(Some(status)) => Some(status),
            _ => {
                let _ = child.kill();
                child.wait().ok()
            }
        };
        let text = text?;
        status?.success().then_some(text)
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
    /// A copyable link for a mount-relative path such as `x/colab/#space=...`.
    pub fn url(&self, relative: &str) -> String {
        format!("{}/{}", self.0, relative)
    }
}

#[cfg(test)]
mod tests {
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
