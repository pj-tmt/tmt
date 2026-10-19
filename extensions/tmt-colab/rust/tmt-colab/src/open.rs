//! CLI flag and interaction inputs for the shared browser opener, and the hand-off to Remote's
//! launch seam so an unpaired browser connects when a page opens.
use crate::door;
use std::time::{Duration, Instant};
use tmt_colab::short_links;
pub use tmt_invoke::open::{Flag, Outcome};

pub fn flag(args: &clap::ArgMatches) -> Flag {
    match (args.get_flag("open"), args.get_flag("no-open")) {
        (_, true) => Flag::NoOpen,
        (true, false) => Flag::Open,
        _ => Flag::Unset,
    }
}

/// How an explicit `tmt colab open` request decides: it opens even without a terminal or with the
/// open setting off, but, like every browser handoff, never under an agent (`TMT_AGENT`) unless
/// `--open` asks for it. `--no-open` always wins.
pub fn explicit_flag(args: &clap::ArgMatches, agent: bool) -> Flag {
    explicit(flag(args), agent)
}

fn explicit(flag: Flag, agent: bool) -> Flag {
    match flag {
        Flag::Unset if agent => Flag::NoOpen,
        Flag::Unset => Flag::Open,
        other => other,
    }
}

pub fn open_link(link: &str, flag: Flag, setting: bool, json: bool) -> Outcome {
    open_through_remote(
        link,
        flag,
        setting,
        json,
        tmt_cli_style::Interaction::detect(false).stdout,
    )
}

/// A committed page opens for the owner, independently of the agent's output mode.
/// Keep the shared flag, display, CI, opener discovery and launch policy; only the
/// terminal/JSON interaction gates do not apply to this creation effect.
pub fn open_created_link(link: &str, flag: Flag, setting: bool) -> Outcome {
    open_through_remote(link, flag, setting, false, true)
}

/// How long Remote may take to ready an offer and start the platform opener.
const LAUNCH_DEADLINE: Duration = Duration::from_secs(15);

/// Colab decides whether to open, with the shared flag, setting and environment policy. Remote
/// owns the launch: it opens the short target and connects an unpaired browser, and its launch
/// URL never reaches Colab. A Remote without the seam leaves today's plain opener.
fn open_through_remote(
    link: &str,
    flag: Flag,
    setting: bool,
    json: bool,
    interactive: bool,
) -> Outcome {
    let env = tmt_invoke::open::Env::current(interactive);
    if matches!(
        tmt_invoke::open::decide(flag, setting, json, &env),
        tmt_invoke::open::Decision::Skip(_)
    ) {
        // Suppressed opening creates no offer, so Remote is not called.
        return Outcome::Skipped;
    }
    let answer = match short_target(link) {
        // Colab already applied the policy; `--open` keeps Remote from deciding a second time.
        Some(target) => interpret(door::remote_launch(
            target,
            "--open",
            Instant::now() + LAUNCH_DEADLINE,
        )),
        None => Answer::Absent,
    };
    match answer {
        Answer::Opened => Outcome::Opened,
        Answer::Skipped => Outcome::Skipped,
        Answer::NoOpener => Outcome::NoOpener,
        Answer::Failed(why) => Outcome::Failed(why),
        Answer::Absent => tmt_invoke::open::open_link(link, flag, setting, json, interactive),
    }
}

/// The canonical short target of a Colab link: `/colab/` or `/p/<id>`. Anything else is never
/// handed to Remote.
fn short_target(link: &str) -> Option<&str> {
    let (_, rest) = link.split_once("://")?;
    let path = &rest[rest.find('/')?..];
    (path == "/colab/"
        || path
            .strip_prefix("/p/")
            .is_some_and(short_links::valid_prefix))
    .then_some(path)
}

/// What Remote's `launch` seam said.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Opened,
    Skipped,
    NoOpener,
    /// Remote answered with a reason; the browser is not opened a second time.
    Failed(String),
    /// No seam: an older Remote, an unknown version or state, or no answer to read.
    Absent,
}

fn interpret(launch: door::Launch) -> Answer {
    match launch {
        door::Launch::Unreachable => Answer::Absent,
        door::Launch::NoAnswer => Answer::Failed("Remote did not answer in time".to_owned()),
        door::Launch::Ran { code, stdout } => read_answer(code, &stdout),
    }
}

/// Success is exactly `{version:1,link,opened,state}`; a failure is `{version:1,error:{code,message}}`
/// with a nonzero exit. Anything else, including a usage error, means the seam is absent.
fn read_answer(code: Option<u32>, stdout: &str) -> Answer {
    let Ok(serde_json::Value::Object(value)) = serde_json::from_str(stdout.trim()) else {
        return Answer::Absent;
    };
    if value.get("version") != Some(&serde_json::json!(1)) {
        return Answer::Absent;
    }
    if code == Some(0) {
        let (Some(opened), Some(state)) = (
            value.get("opened").and_then(serde_json::Value::as_bool),
            value.get("state").and_then(serde_json::Value::as_str),
        ) else {
            return Answer::Absent;
        };
        if value.len() != 4 || !value.get("link").is_some_and(serde_json::Value::is_string) {
            return Answer::Absent;
        }
        return match (state, opened) {
            ("opened", true) => Answer::Opened,
            ("skipped", false) => Answer::Skipped,
            ("no-opener", false) => Answer::NoOpener,
            ("failed", false) => Answer::Failed("Remote could not start the browser".to_owned()),
            _ => Answer::Absent,
        };
    }
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .map_or(Answer::Absent, |message| {
            let message: String = message
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            Answer::Failed(message.trim_end_matches('.').to_owned())
        })
}

/// The `open` row value for a link, and the warning to print when the opener failed.
pub fn describe(outcome: &Outcome, link: &str) -> (String, Option<String>) {
    match outcome {
        Outcome::Opened => (format!("opened in your browser: {link}"), None),
        Outcome::Skipped | Outcome::NoOpener => (link.to_owned(), None),
        Outcome::Failed(why) => (
            link.to_owned(),
            // Sentences, not a parenthesis; the shared printer omits the final period.
            Some(format!(
                "Could not open the browser. {}. Open the link above yourself",
                why.trim().trim_end_matches('.')
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_open_reads_as_sentences_without_a_doubled_period() {
        for why in [
            "Remote could not start the browser",
            "Remote could not start the browser.",
        ] {
            let (row, warning) = describe(
                &Outcome::Failed(why.to_owned()),
                "http://127.0.0.1:1/colab/",
            );
            assert_eq!(row, "http://127.0.0.1:1/colab/");
            assert_eq!(
                warning.as_deref(),
                Some(
                    "Could not open the browser. Remote could not start the browser. Open the link above yourself"
                )
            );
        }
    }

    #[test]
    fn only_the_canonical_short_target_reaches_remote() {
        let id = "12345678-0000-4000-8000-000000000001";
        for (link, target) in [
            ("http://127.0.0.1:53253/colab/", Some("/colab/")),
            ("http://127.0.0.1:53253/p/12345678", Some("/p/12345678")),
            (
                &format!("http://127.0.0.1:53253/p/{id}"),
                Some(&format!("/p/{id}")[..]),
            ),
            ("http://127.0.0.1:53253/", None),
            ("http://127.0.0.1:53253/p/1234", None),
            ("http://127.0.0.1:53253/p/12345678?x=1", None),
            ("http://127.0.0.1:53253/p/12345678#t=x", None),
            ("http://127.0.0.1:53253/r/abcdefghijklmnop/x/colab/", None),
            ("http://127.0.0.1:53253/pair#auto=CODE", None),
            ("not a link", None),
        ] {
            assert_eq!(short_target(link), target, "{link}");
        }
    }

    #[test]
    fn remotes_answer_is_read_strictly_and_everything_else_is_version_skew() {
        let ok = |state: &str, opened: bool| {
            format!(
                r#"{{"version":1,"link":"http://127.0.0.1:1/p/12345678","opened":{opened},"state":"{state}"}}"#
            )
        };
        assert_eq!(read_answer(Some(0), &ok("opened", true)), Answer::Opened);
        assert_eq!(read_answer(Some(0), &ok("skipped", false)), Answer::Skipped);
        assert_eq!(
            read_answer(Some(0), &ok("no-opener", false)),
            Answer::NoOpener
        );
        assert!(matches!(
            read_answer(Some(0), &ok("failed", false)),
            Answer::Failed(_)
        ));
        // A state that disagrees with `opened`, an unknown state or version, extra or missing
        // fields, and non-JSON are all treated as an absent seam.
        for body in [
            ok("opened", false),
            ok("launched", true),
            ok("opened", true).replace("\"version\":1", "\"version\":2"),
            ok("opened", true).replace("\"link\":\"http://127.0.0.1:1/p/12345678\",", ""),
            ok("opened", true).replace("}", ",\"code\":\"x\"}"),
            "not json".to_owned(),
            String::new(),
        ] {
            assert_eq!(read_answer(Some(0), &body), Answer::Absent, "{body}");
        }
        let error = r#"{"version":1,"error":{"code":"REMOTE_PAIRING_BUSY","message":"A manual pairing is still open. Finish or cancel it first.\nsecond line"}}"#;
        assert_eq!(
            read_answer(Some(1), error),
            Answer::Failed("A manual pairing is still open. Finish or cancel it first".to_owned())
        );
        // A usage error prints nothing on stdout; a signal leaves no exit code.
        assert_eq!(read_answer(Some(2), ""), Answer::Absent);
        assert_eq!(
            read_answer(None, error),
            Answer::Failed("A manual pairing is still open. Finish or cancel it first".to_owned())
        );
        assert_eq!(
            read_answer(Some(1), r#"{"error":{"message":"x"}}"#),
            Answer::Absent
        );
    }

    #[test]
    fn an_explicit_open_prints_the_link_under_an_agent_unless_open_is_passed() {
        assert_eq!(explicit(Flag::Unset, true), Flag::NoOpen);
        assert_eq!(explicit(Flag::Open, true), Flag::Open);
        assert_eq!(explicit(Flag::NoOpen, true), Flag::NoOpen);
    }

    #[test]
    fn an_explicit_open_outside_an_agent_opens_unless_no_open_is_passed() {
        assert_eq!(explicit(Flag::Unset, false), Flag::Open);
        assert_eq!(explicit(Flag::Open, false), Flag::Open);
        assert_eq!(explicit(Flag::NoOpen, false), Flag::NoOpen);
    }
}
