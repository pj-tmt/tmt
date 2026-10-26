//! Where a member's terminal stream is. Core publishes it (`terminal.stream`); a client never
//! derives a path, and a member with no stream shows no Terminal tab.
use std::{path::PathBuf, time::Duration};

/// How long the read may take before the tab says it could not ask.
const WITHIN: Duration = Duration::from_secs(3);
/// Core's refusal for a member that is not on the built-in host, has no binding, or has ended.
const UNAVAILABLE: &str = "TERMINAL_STREAM_UNAVAILABLE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Address {
    pub socket: PathBuf,
    pub pane: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Located {
    Stream(Address),
    /// Not a built-in-host member, or one whose program has ended: no tab.
    NoStream,
    /// Core could not answer; the words it gave, to be tried again.
    Failed(String),
}

/// One `terminal.stream` read of `identity`.
pub(in crate::board) fn locate(core: &crate::core::Core, identity: &str) -> Located {
    if !crate::board::recent_responses::canonical_identity(identity) {
        return Located::NoStream;
    }
    match core.api_within(
        "terminal.stream",
        serde_json::json!({"identityId": identity}),
        WITHIN,
    ) {
        Ok(value) => from_value(identity, &value),
        Err(error) if error.code == UNAVAILABLE => Located::NoStream,
        Err(error) => Located::Failed(crate::board::notes::single_line(&error.message)),
    }
}

/// The answer, checked: it must name this identity, the built-in host and an absolute socket.
fn from_value(identity: &str, value: &serde_json::Value) -> Located {
    let address = (|| {
        if value["identityId"].as_str()? != identity || value["host"].as_str()? != "pty" {
            return None;
        }
        let socket = PathBuf::from(value["socket"].as_str()?);
        let pane = value["pane"].as_str()?.to_owned();
        (socket.is_absolute() && !pane.is_empty()).then_some(Address { socket, pane })
    })();
    address.map_or_else(
        || Located::Failed("the answer was not a terminal stream".into()),
        Located::Stream,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn an_answer_for_this_member_on_the_built_in_host_is_its_address() {
        let answer =
            json!({"identityId": ID, "host": "pty", "socket": "/data/pty/p1.sock", "pane": "p1"});
        assert_eq!(
            from_value(ID, &answer),
            Located::Stream(Address {
                socket: "/data/pty/p1.sock".into(),
                pane: "p1".into()
            })
        );
    }

    #[test]
    fn an_answer_that_is_not_for_this_member_or_host_or_path_is_refused() {
        for answer in [
            json!({"identityId": "other", "host": "pty", "socket": "/s", "pane": "p"}),
            json!({"identityId": ID, "host": "tmux", "socket": "/s", "pane": "p"}),
            json!({"identityId": ID, "host": "pty", "socket": "relative/s", "pane": "p"}),
            json!({"identityId": ID, "host": "pty", "socket": "/s", "pane": ""}),
            json!({"identityId": ID, "host": "pty", "socket": "/s"}),
            json!("text"),
        ] {
            assert!(
                matches!(from_value(ID, &answer), Located::Failed(_)),
                "{answer}"
            );
        }
    }
}
