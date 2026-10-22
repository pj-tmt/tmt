//! A member pane's visible screen, read through `pane.screen` and drawn read-only. The
//! text is another program's output, parsed by `tmt_cli_style::foreign`.
use tmt_cli_style::foreign::{Row, parse, width};

/// What the pane read last said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Observation {
    /// Nothing read yet.
    #[default]
    Pending,
    /// The identity has no pane, or the pane is gone.
    Gone,
    /// The host could not give the screen, with the reason it gave.
    Unavailable(String),
    Screen(Screen),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Screen {
    pub rows: Vec<Row>,
    /// Cells wide the widest row is.
    pub width: usize,
    pub read_ms: u64,
}

/// One `pane.screen` read of `identity`. A pane that is gone and a screen the host would not
/// give are told apart; a malformed answer counts as the second.
pub(super) fn read(core: &crate::core::Core, identity: &str) -> Observation {
    if !super::recent_responses::canonical_identity(identity) {
        return Observation::Unavailable("not an agent identity".into());
    }
    match core.api_within(
        "pane.screen",
        serde_json::json!({"identityId": identity}),
        std::time::Duration::from_secs(3),
    ) {
        Ok(value) => from_value(identity, &value, crate::status::now_ms()),
        Err(error) => match error.code.as_str() {
            "PANE_NOT_FOUND" | "NAME_NOT_FOUND" => Observation::Gone,
            _ => Observation::Unavailable(super::notes::single_line(&error.message)),
        },
    }
}

fn from_value(identity: &str, value: &serde_json::Value, read_ms: u64) -> Observation {
    let screen = (|| {
        if value["identityId"].as_str()? != identity
            || !matches!(value["format"].as_str()?, "ansi" | "plain")
        {
            return None;
        }
        let rows = parse(value["text"].as_str()?);
        let width = rows.iter().map(width).max().unwrap_or(0);
        Some(Screen {
            rows,
            width,
            read_ms,
        })
    })();
    screen.map_or_else(
        || Observation::Unavailable("the answer was not a pane screen".into()),
        Observation::Screen,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_a_screen_only_for_the_asked_identity_and_a_known_format() {
        let id = "11111111-1111-4111-8111-111111111111";
        let answer = |id: &str, format: &str| serde_json::json!({"identityId": id, "host": "tmux", "format": format, "text": "\u{1b}[31mhi\u{1b}[0m\nthere"});
        let Observation::Screen(screen) = from_value(id, &answer(id, "ansi"), 7) else {
            panic!("a screen");
        };
        assert_eq!((screen.rows.len(), screen.width, screen.read_ms), (2, 5, 7));
        for bad in [
            answer("22222222-2222-4222-8222-222222222222", "ansi"),
            answer(id, "html"),
            serde_json::json!({"identityId": id, "format": "ansi"}),
        ] {
            assert!(matches!(
                from_value(id, &bad, 0),
                Observation::Unavailable(_)
            ));
        }
    }
}
