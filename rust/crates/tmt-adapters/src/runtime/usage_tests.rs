//! Turn-end usage (#519) through each first-party driver's lifecycle, as the
//! hook worker calls it. `claude-assistant-usage.jsonl` and
//! `codex-token-count.jsonl` are real lines (see `fixtures/README.md`); every
//! other line here is assembled from them.

use super::{
    driver_state::{self, Usage},
    lifecycle::{RuntimeLifecycle, TurnEnd},
    transcript::TAIL_LIMIT,
};
use crate::{
    drivers::{claude, codex},
    skill_installation::ProviderEnvironment,
    test_support::TestDirectory,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tmt_core::binding::session::DriverState;

const CLAUDE_LINE: &str = include_str!("fixtures/claude-assistant-usage.jsonl");
const CODEX_LINE: &str = include_str!("fixtures/codex-token-count.jsonl");
const NOW: u64 = 1_780_000_000_000;
const SESSION: &str = "0199a213-81c0-7800-8aa1-bbab2a035a53";

fn stop(transcript: Option<&Path>) -> Vec<u8> {
    json!({
        "session_id": SESSION,
        "transcript_path": transcript,
        "cwd": "/workspace",
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "<redacted>",
    })
    .to_string()
    .into_bytes()
}

fn edited(line: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(line).unwrap();
    edit(&mut value);
    value.to_string()
}

struct Driver {
    lifecycle: &'static dyn RuntimeLifecycle,
    /// The transcript tree under the test HOME.
    tree: &'static str,
    line: &'static str,
    /// A count in `line` the reading depends on.
    count: &'static str,
    expected: Usage,
}

fn drivers() -> [Driver; 2] {
    [
        Driver {
            lifecycle: &claude::ClaudeLifecycle,
            tree: ".claude/projects/-workspace",
            line: CLAUDE_LINE,
            count: "/message/usage/input_tokens",
            // input 2 + cache read 192684 + cache creation 2978.
            expected: Usage::new(195_664, None, NOW).unwrap(),
        },
        Driver {
            lifecycle: &codex::CodexLifecycle,
            tree: ".codex/sessions/2026/09/29",
            line: CODEX_LINE,
            count: "/payload/info/last_token_usage/total_tokens",
            // total 146577 - reasoning 0, in a 258400 window.
            expected: Usage::new(146_577, Some(258_400), NOW).unwrap(),
        },
    ]
}

/// Writes `lines` as a transcript in the driver's tree and runs a turn end.
fn turn(
    driver: &Driver,
    home: &Path,
    lines: &[String],
    previous: Option<&DriverState>,
) -> Option<DriverState> {
    let directory = home.join(driver.tree);
    fs::create_dir_all(&directory).unwrap();
    let transcript = directory.join("session.jsonl");
    fs::write(&transcript, lines.concat()).unwrap();
    read(driver, home, Some(&transcript), previous)
}

fn read(
    driver: &Driver,
    home: &Path,
    transcript: Option<&Path>,
    previous: Option<&DriverState>,
) -> Option<DriverState> {
    let turn = driver.lifecycle.decode_turn(&stop(transcript)).unwrap();
    let environment = ProviderEnvironment::from_parts(home, home, Vec::new(), []);
    driver
        .lifecycle
        .turn_state(&turn, &environment, previous, NOW)
}

#[test]
fn real_provider_lines_record_usage_and_keep_the_model() {
    for driver in drivers() {
        let home = TestDirectory::new();
        let model = driver_state::after_start(Some("model-a"), None, None).unwrap();
        let state = turn(&driver, &home.path, &[driver.line.into()], Some(&model)).unwrap();
        assert_eq!(driver.lifecycle.state_usage(&state), Some(driver.expected));
        assert_eq!(
            driver.lifecycle.state_model(&state).as_deref(),
            Some("model-a")
        );
        // The newest usable line wins over older ones and trailing noise.
        let older = edited(driver.line, |value| {
            *value.pointer_mut(driver.count).unwrap() = json!(1);
        }) + "\n";
        let lines = [older, driver.line.into(), "{\"type\":\"user\"}\n".into()];
        let state = turn(&driver, &home.path, &lines, None).unwrap();
        assert_eq!(driver.lifecycle.state_usage(&state), Some(driver.expected));
    }
}

#[test]
fn unreadable_transcripts_write_nothing() {
    for driver in drivers() {
        let home = TestDirectory::new();
        let malformed = |line: &str| vec![format!("{line}\n")];
        for lines in [
            malformed("not json"),
            malformed(r#"{"type":"summary","summary":"foreign format"}"#),
            malformed(&edited(driver.line, |value| {
                value["type"] = json!("something-new")
            })),
            vec![String::new()],
            // A final line larger than the whole read window.
            vec![
                driver.line.into(),
                format!("{{\"pad\":\"{}\"}}\n", "x".repeat(TAIL_LIMIT as usize)),
            ],
        ] {
            assert_eq!(turn(&driver, &home.path, &lines, None), None);
        }
        let missing = home.path.join(driver.tree).join("missing.jsonl");
        assert_eq!(read(&driver, &home.path, Some(&missing), None), None);
        assert_eq!(read(&driver, &home.path, None, None), None);
        // A path outside the driver's own tree is never read.
        let outside = home.path.join("elsewhere.jsonl");
        fs::write(&outside, driver.line).unwrap();
        assert_eq!(read(&driver, &home.path, Some(&outside), None), None);
    }
}

#[test]
fn claude_reads_top_level_usage_of_the_main_conversation_only() {
    let driver = &drivers()[0];
    // The real line's one iteration equals its top level; when they differ,
    // the top level is the conversation's usage.
    let line: Value = serde_json::from_str(CLAUDE_LINE).unwrap();
    assert_eq!(
        line["message"]["usage"]["iterations"][0]["cache_read_input_tokens"],
        line["message"]["usage"]["cache_read_input_tokens"]
    );
    let diverged = edited(CLAUDE_LINE, |value| {
        value["message"]["usage"]["iterations"][0]["cache_read_input_tokens"] = json!(7);
    });
    assert_eq!(claude::transcript_usage(&diverged), Some(195_664));
    for skipped in [
        edited(CLAUDE_LINE, |value| value["isSidechain"] = json!(true)),
        edited(CLAUDE_LINE, |value| {
            value["message"]["model"] = json!("<synthetic>")
        }),
        edited(CLAUDE_LINE, |value| {
            value["message"]["usage"]["input_tokens"] = json!(-1)
        }),
        edited(CLAUDE_LINE, |value| {
            value["message"]["usage"]
                .as_object_mut()
                .unwrap()
                .remove("input_tokens");
        }),
    ] {
        assert_eq!(claude::transcript_usage(&skipped), None, "{skipped}");
    }
    let uncached = edited(CLAUDE_LINE, |value| {
        let usage = value["message"]["usage"].as_object_mut().unwrap();
        usage.remove("cache_read_input_tokens");
        usage.remove("cache_creation_input_tokens");
    });
    assert_eq!(claude::transcript_usage(&uncached), Some(2));
    let home = TestDirectory::new();
    assert_eq!(
        turn(driver, &home.path, &[CLAUDE_LINE.into()], None)
            .and_then(|state| driver_state::state_usage(&state)),
        Some(driver.expected)
    );
}

#[test]
fn codex_reads_the_last_token_count_and_its_window() {
    // An event without info (rate limits only) is skipped for an older one.
    let empty = edited(CODEX_LINE, |value| value["payload"]["info"] = Value::Null);
    let home = TestDirectory::new();
    let driver = &drivers()[1];
    let state = turn(
        driver,
        &home.path,
        &[CODEX_LINE.into(), format!("{empty}\n")],
        None,
    )
    .unwrap();
    assert_eq!(driver_state::state_usage(&state), Some(driver.expected));
    let reasoning = edited(CODEX_LINE, |value| {
        value["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(577);
    });
    assert_eq!(
        codex::transcript_usage(&reasoning),
        Some((146_000, Some(258_400)))
    );
    let windowless = edited(CODEX_LINE, |value| {
        value["payload"]["info"]
            .as_object_mut()
            .unwrap()
            .remove("model_context_window");
    });
    assert_eq!(codex::transcript_usage(&windowless), Some((146_577, None)));
    for skipped in [
        edited(CODEX_LINE, |value| {
            value["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(146_578)
        }),
        edited(CODEX_LINE, |value| {
            value["payload"]["type"] = json!("agent_message")
        }),
        edited(CODEX_LINE, |value| {
            value["payload"]["info"]["model_context_window"] = json!("large")
        }),
    ] {
        assert_eq!(codex::transcript_usage(&skipped), None, "{skipped}");
    }

    // CODEX_HOME moves the tree the driver trusts.
    let custom = home.path.join("custom-codex");
    let transcript = custom.join("sessions/rollout.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(&transcript, CODEX_LINE).unwrap();
    let environment = ProviderEnvironment::from_parts(
        &home.path,
        &home.path,
        Vec::new(),
        [("CODEX_HOME", custom)],
    );
    let turn = driver
        .lifecycle
        .decode_turn(&stop(Some(&transcript)))
        .unwrap();
    assert!(
        driver
            .lifecycle
            .turn_state(&turn, &environment, None, NOW)
            .is_some()
    );
}

#[test]
fn only_a_turn_end_is_decoded_as_one() {
    for driver in drivers() {
        let decoded = driver
            .lifecycle
            .decode_turn(&stop(Some(Path::new("/t.jsonl"))));
        assert_eq!(
            decoded,
            Some(TurnEnd {
                session: tmt_core::binding::session::ProviderSessionId::new(SESSION).unwrap(),
                transcript: Some("/t.jsonl".into()),
            })
        );
        // A null path is still a turn end, with nothing to read.
        assert_eq!(
            driver
                .lifecycle
                .decode_turn(&stop(None))
                .unwrap()
                .transcript,
            None
        );
        // It is not a lifecycle event, and other events are not turn ends.
        assert!(driver.lifecycle.decode(&stop(None)).is_none());
        for other in ["SubagentStop", "SessionStart", "UserPromptSubmit"] {
            let payload = String::from_utf8(stop(None))
                .unwrap()
                .replace("\"Stop\"", &format!("\"{other}\""));
            assert!(driver.lifecycle.decode_turn(payload.as_bytes()).is_none());
        }
    }
}
