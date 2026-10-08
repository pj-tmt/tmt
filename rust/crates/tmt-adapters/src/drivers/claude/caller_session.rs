//! Documented tool locators; native PID equality and ancestry admit the root.
use crate::runtime::lifecycle::CallerSession;
use std::ffi::OsStr;

pub(super) fn coordinates(session: Option<&OsStr>, pid: Option<&OsStr>) -> Option<CallerSession> {
    let session = session?.to_str()?;
    let id = uuid::Uuid::parse_str(session).ok()?;
    if id.to_string() != session {
        return None;
    }
    let pid = pid?.to_str()?.parse::<u32>().ok().filter(|pid| *pid > 0)?;
    Some(CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new(session).ok()?,
        runtime_pid: Some(pid),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_claude_env_shape_and_changes_are_fail_closed() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../runtime/fixtures/claude-caller-session.json"
        ))
        .unwrap();
        let session = OsStr::new(
            fixture["environment"]["CLAUDE_CODE_SESSION_ID"]
                .as_str()
                .unwrap(),
        );
        let pid = OsStr::new(fixture["environment"]["CLAUDE_PID"].as_str().unwrap());
        let coordinates = coordinates(Some(session), Some(pid)).unwrap();
        assert_eq!(coordinates.runtime_pid, Some(12345));
        assert_eq!(coordinates.session.as_str(), session.to_str().unwrap());
        for changed in [
            None,
            Some(OsStr::new("0")),
            Some(OsStr::new("-1")),
            Some(OsStr::new("abc")),
            Some(OsStr::new("4294967296")),
        ] {
            assert!(super::coordinates(Some(session), changed).is_none());
        }
        for changed in [
            None,
            Some(OsStr::new("agent-123")),
            Some(OsStr::new("50FEC1A2-581C-4ADC-924D-15C31CEF4DA4")),
        ] {
            assert!(super::coordinates(changed, Some(pid)).is_none());
        }
    }
}
