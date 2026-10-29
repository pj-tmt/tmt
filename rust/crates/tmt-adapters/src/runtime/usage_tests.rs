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
use std::{fs, io::Write, path::Path};
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

/// A Stop that carries no final text, for the transcript fallback.
fn stop_without_message(transcript: Option<&Path>) -> Vec<u8> {
    let mut payload: Value = serde_json::from_slice(&stop(transcript)).unwrap();
    payload
        .as_object_mut()
        .unwrap()
        .remove("last_assistant_message");
    payload.to_string().into_bytes()
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
            // total 146577, in a 258400 window.
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
    driver.lifecycle.turn_state(
        &turn,
        &environment,
        previous,
        NOW,
        std::time::Instant::now() + std::time::Duration::from_secs(2),
    )
}

#[test]
fn real_provider_lines_record_usage_and_keep_the_model() {
    for driver in drivers() {
        let home = TestDirectory::new();
        let model = driver_state::after_start(Some("model-a"), None, None, 1).unwrap();
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
fn foreign_only_initial_sources_do_not_invent_usage_but_claude_keeps_a_zero_cursor() {
    let drivers = drivers();
    let home = TestDirectory::new();
    for foreign in [
        r#"{"type":"summary","summary":"foreign format"}"#.to_owned(),
        edited(drivers[0].line, |value| {
            value["type"] = json!("something-new")
        }),
    ] {
        let state = turn(&drivers[0], &home.path, &[format!("{foreign}\n")], None).unwrap();
        assert!(
            drivers[0].lifecycle.state_usage(&state).is_none(),
            "foreign records report no usage"
        );
        let counter = drivers[0].lifecycle.state_consumption(&state).unwrap();
        assert_eq!(
            (
                counter.input_tokens,
                counter.cached_input_tokens,
                counter.output_tokens
            ),
            (0, 0, 0)
        );
        assert!(counter.complete && !counter.gap);
        assert!(counter.delta_by_model.as_ref().unwrap().is_empty());
    }
    // Codex's cumulative format still requires an actual token_count baseline.
    for foreign in [
        r#"{"type":"summary","summary":"foreign format"}"#.to_owned(),
        edited(drivers[1].line, |value| {
            value["type"] = json!("something-new")
        }),
    ] {
        assert_eq!(
            turn(&drivers[1], &home.path, &[format!("{foreign}\n")], None),
            None
        );
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
    // Reasoning is part of the total Codex reports, as its status card shows.
    assert_eq!(
        codex::transcript_usage(&reasoning),
        Some((146_577, Some(258_400)))
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
            value["payload"]["info"]["last_token_usage"]["total_tokens"] = json!(-1)
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
            .turn_state(
                &turn,
                &environment,
                None,
                NOW,
                std::time::Instant::now() + std::time::Duration::from_secs(2)
            )
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
                final_text: Some(tmt_core::responses::FinalText::new("<redacted>").unwrap()),
                interrupted: false,
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
        // A Codex interrupt is a turn end for activity only: its transcript
        // and final text are dropped. Claude has no such event.
        let interrupt = String::from_utf8(stop(Some(Path::new("/t.jsonl"))))
            .unwrap()
            .replace("\"Stop\"", "\"Interrupt\"");
        let decoded = driver.lifecycle.decode_turn(interrupt.as_bytes());
        if driver.tree.starts_with(".codex") {
            assert_eq!(
                decoded,
                Some(TurnEnd {
                    session: tmt_core::binding::session::ProviderSessionId::new(SESSION).unwrap(),
                    transcript: None,
                    final_text: None,
                    interrupted: true,
                })
            );
        } else {
            assert!(decoded.is_none());
        }
    }
}

#[test]
fn claude_custom_config_root_admits_only_its_own_transcripts() {
    let home = TestDirectory::new();
    let custom = home.path.join("custom-claude");
    let environment = ProviderEnvironment::from_parts(
        &home.path,
        &home.path,
        Vec::new(),
        [("CLAUDE_CONFIG_DIR", custom.clone())],
    );
    for (root, accepted) in [(custom, true), (home.path.join(".claude"), false)] {
        let transcript = root.join("projects/workspace/session.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(&transcript, CLAUDE_LINE).unwrap();
        let turn = claude::ClaudeLifecycle
            .decode_turn(&stop(Some(&transcript)))
            .unwrap();
        let state = claude::ClaudeLifecycle.turn_state(
            &turn,
            &environment,
            None,
            NOW,
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        );
        assert_eq!(state.is_some(), accepted);
        if let Some(state) = state {
            assert_eq!(
                claude::ClaudeLifecycle.state_usage(&state),
                Some(drivers()[0].expected)
            );
        }
    }
}

#[test]
fn an_idle_claude_agent_stores_the_same_state_on_every_tick() {
    let home = TestDirectory::new();
    let environment = ProviderEnvironment::from_parts(
        &home.path,
        &home.path,
        Vec::new(),
        Vec::<(&str, _)>::new(),
    );
    let transcript = home.path.join(".claude/projects/workspace/session.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(&transcript, CLAUDE_LINE).unwrap();
    let turn = claude::ClaudeLifecycle
        .decode_turn(&stop(Some(&transcript)))
        .unwrap();
    let read = |previous: Option<&DriverState>, now| {
        claude::ClaudeLifecycle
            .turn_state(
                &turn,
                &environment,
                previous,
                now,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap()
    };
    let first = read(None, NOW);
    let second = read(Some(&first), NOW + 5_000);
    let third = read(Some(&second), NOW + 10_000);
    // The transcript did not grow: nothing about the stored state is new.
    assert_eq!(second, first);
    assert_eq!(third, first);
    assert_eq!(
        claude::ClaudeLifecycle
            .state_usage(&third)
            .unwrap()
            .observed_at_ms,
        NOW
    );
}

#[test]
fn claude_cache_windows_follow_each_seats_root_and_refresh_without_counting_tokens() {
    let home = TestDirectory::new();
    let custom = home.path.join("custom-claude");
    let cache = |used| {
        json!({
            "oauthAccount": {"accountUuid":"same-root-account"},
            "cachedUsageUtilization": {
                "accountUuid":"same-root-account", "fetchedAtMs": NOW - 86_400_000,
                "utilization": {"seven_day": {"utilization": used, "resets_at":"2026-05-31T20:26:40+00:00"}}
            }
        })
    };
    fs::write(home.path.join(".claude.json"), cache(25).to_string()).unwrap();
    fs::create_dir_all(&custom).unwrap();
    fs::write(custom.join(".claude.json"), cache(78).to_string()).unwrap();
    for (root, vars, expected) in [
        (home.path.join(".claude"), vec![], 25.0),
        (
            custom.clone(),
            vec![("CLAUDE_CONFIG_DIR", custom.clone())],
            78.0,
        ),
    ] {
        let transcript = root.join("projects/workspace/session.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(
            &transcript,
            include_str!("fixtures/claude-usage-sequence.jsonl"),
        )
        .unwrap();
        let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), vars);
        let turn = claude::ClaudeLifecycle
            .decode_turn(&stop(Some(&transcript)))
            .unwrap();
        let read = |previous, now| {
            claude::ClaudeLifecycle
                .turn_state(
                    &turn,
                    &environment,
                    previous,
                    now,
                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                )
                .unwrap()
        };
        let state = read(None, NOW);
        let before = driver_state::state_consumption(&state).unwrap().value;
        let limits = before.rate_limits.as_ref().unwrap();
        assert_eq!(limits.provider, "claude");
        assert_eq!(limits.observed_at_ms, NOW - 86_400_000);
        assert_eq!(limits.windows[0].used_percent, expected);
        let repeated = driver_state::state_consumption(&read(Some(&state), NOW + 1))
            .unwrap()
            .value;
        assert_eq!(repeated, before);
        let mut appended = fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        writeln!(
            appended,
            "{}",
            json!({"type":"assistant","requestId":"new-request","message":{
                "id":"new-message","model":"claude-sonnet-5-5",
                "usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":4}
            }})
        )
        .unwrap();
        drop(appended);
        let counted = read(Some(&state), NOW + 2);
        let before = driver_state::state_consumption(&counted).unwrap().value;
        assert_eq!(before.input_tokens, 14);
        assert!(!before.delta_by_model.as_ref().unwrap().is_empty());
        let file = if expected == 25.0 {
            home.path.join(".claude.json")
        } else {
            custom.join(".claude.json")
        };
        fs::remove_file(&file).unwrap();
        let after = driver_state::state_consumption(&read(Some(&counted), NOW + 3))
            .unwrap()
            .value;
        assert_eq!(after.rate_limits, None);
        assert_eq!(after.sequence, before.sequence + 1);
        assert_eq!(after.observed_at_ms, NOW + 3);
        assert_eq!(after.input_tokens, before.input_tokens);
        assert!(after.delta_by_model.as_ref().unwrap().is_empty());
    }
}

#[test]
fn a_skipped_or_failed_allowance_read_keeps_the_stored_limits_and_a_later_read_replaces_them() {
    use std::time::Duration;
    let home = TestDirectory::new();
    let config = home.path.join(".claude.json");
    let cache = |used: u64, fetched: u64| {
        json!({
            "oauthAccount": {"accountUuid":"account-a"},
            "cachedUsageUtilization": {
                "accountUuid":"account-a", "fetchedAtMs": fetched,
                "utilization": {"seven_day": {"utilization": used, "resets_at":"2026-05-31T20:26:40+00:00"}}
            }
        })
    };
    fs::write(&config, cache(25, NOW - 1_000).to_string()).unwrap();
    let transcript = home.path.join(".claude/projects/workspace/session.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(
        &transcript,
        include_str!("fixtures/claude-usage-sequence.jsonl"),
    )
    .unwrap();
    let environment = ProviderEnvironment::from_parts(
        &home.path,
        &home.path,
        Vec::new(),
        Vec::<(&str, _)>::new(),
    );
    let turn = claude::ClaudeLifecycle
        .decode_turn(&stop(Some(&transcript)))
        .unwrap();
    let observe = |previous: Option<&DriverState>, now: u64, left: Duration| {
        let state = claude::ClaudeLifecycle
            .turn_state(
                &turn,
                &environment,
                previous,
                now,
                std::time::Instant::now() + left,
            )
            .unwrap();
        let used = driver_state::state_consumption(&state)
            .unwrap()
            .value
            .rate_limits
            .map(|limits| limits.windows[0].used_percent);
        (state, used)
    };
    let two_seconds = Duration::from_secs(2);
    let (stored, used) = observe(None, NOW, two_seconds);
    assert_eq!(used, Some(25.0));

    // A turn with no time left skips the read; that is not an observation of no limits.
    let (skipped, used) = observe(Some(&stored), NOW + 1, Duration::ZERO);
    assert_eq!(used, Some(25.0));

    // A rotated transcript starts the counter anew and cannot carry the limits; they stay.
    fs::write(&transcript, "").unwrap();
    let (rotated, used) = observe(Some(&skipped), NOW + 2, Duration::ZERO);
    assert_eq!(used, Some(25.0));
    assert!(driver_state::state_consumption(&rotated).unwrap().value.gap);

    // A config that cannot be read observed nothing either.
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    let (_, used) = observe(Some(&rotated), NOW + 3, two_seconds);
    assert_eq!(used, Some(25.0));

    // A read that looked replaces them, and a root with no config at all really has none.
    fs::remove_dir(&config).unwrap();
    fs::write(&config, cache(31, NOW - 500).to_string()).unwrap();
    let (replaced, used) = observe(Some(&rotated), NOW + 4, two_seconds);
    assert_eq!(used, Some(31.0));
    fs::remove_file(&config).unwrap();
    let (_, used) = observe(Some(&replaced), NOW + 5, two_seconds);
    assert_eq!(used, None);
}

#[test]
fn empty_claude_source_baselines_without_inventing_context_usage() {
    let driver = drivers().into_iter().next().unwrap();
    let home = TestDirectory::new();
    let state = turn(&driver, &home.path, &[String::new()], None).unwrap();
    assert_eq!(driver.lifecycle.state_usage(&state), None);
    let consumption = driver.lifecycle.state_consumption(&state).unwrap();
    assert_eq!(consumption.input_tokens, 0);
    assert_eq!(consumption.output_tokens, 0);
    assert!(consumption.complete && !consumption.gap);
}

#[test]
fn claude_cache_windows_survive_the_normal_model_activity_and_cursor_state() {
    use tmt_core::{
        binding::session::{
            ProviderSessionId,
            activity::{Event, EventKind},
        },
        endpoint::ProcessIncarnation,
    };
    let home = TestDirectory::new();
    fs::write(
        home.path.join(".claude.json"),
        json!({
            "oauthAccount":{"accountUuid":"account-a"},
            "cachedUsageUtilization":{"accountUuid":"account-a","fetchedAtMs":NOW-1000,
                "utilization":{
                    "five_hour":{"utilization":12.5,"resets_at":"2026-05-28T22:26:40+00:00"},
                    "seven_day":{"utilization":78,"resets_at":"2026-05-31T20:26:40+00:00"}
                }}
        })
        .to_string(),
    )
    .unwrap();
    let transcript = home.path.join(".claude/projects/workspace/session.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(
        &transcript,
        include_str!("fixtures/claude-usage-sequence.jsonl"),
    )
    .unwrap();
    let start = driver_state::after_start(Some("claude-opus-5-5"), None, None, NOW - 2).unwrap();
    let active = driver_state::after_activity(
        Some(&start),
        &ProviderSessionId::new(SESSION).unwrap(),
        &ProcessIncarnation::new(12345, "ps-v1:2026-05-28T20:00:00Z").unwrap(),
        &Event {
            kind: EventKind::TurnStarted,
            turn: None,
        },
        None,
        NOW - 1,
    )
    .unwrap();
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let turn = claude::ClaudeLifecycle
        .decode_turn(&stop(Some(&transcript)))
        .unwrap();
    let state = claude::ClaudeLifecycle
        .turn_state(
            &turn,
            &environment,
            Some(&active),
            NOW,
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .unwrap();
    let consumption = driver_state::state_consumption(&state).expect("normal consumption state");
    assert_eq!(
        consumption
            .value
            .rate_limits
            .as_ref()
            .expect("both cached windows fit the normal state")
            .windows
            .len(),
        2
    );
    assert!(driver_state::state_activity(&state).is_some());
    assert_eq!(
        driver_state::state_model(&state).as_deref(),
        Some("claude-opus-5-5")
    );
    // The account is never part of this state (it is capped at 1024 bytes); the sampled reading
    // names it from the root (#5274).
    assert!(
        consumption
            .value
            .rate_limits
            .as_ref()
            .unwrap()
            .account
            .is_none()
    );
    let sampled = claude::ClaudeLifecycle
        .sampled_consumption(&state, &environment)
        .unwrap();
    let account = sampled
        .rate_limits
        .as_ref()
        .unwrap()
        .account
        .clone()
        .unwrap();
    assert_eq!(account.label, "claude");
    assert!(sampled.valid());
    let mut bare = sampled.clone();
    bare.rate_limits.as_mut().unwrap().account = None;
    assert_eq!(
        Some(bare),
        claude::ClaudeLifecycle.state_consumption(&state)
    );
}

#[test]
fn recent_final_text_uses_the_existing_provider_turn_observation() {
    let directory = TestDirectory::new();
    let environment =
        ProviderEnvironment::from_parts(&directory.path, &directory.path, Vec::new(), []);
    let root = directory.path.join(".claude/projects/-workspace");
    fs::create_dir_all(&root).unwrap();
    let transcript = root.join("session.jsonl");
    let main = json!({"type":"assistant","sessionId":SESSION,"message":{"model":"claude","usage":{"input_tokens":1},"content":[{"type":"text","text":"first\r\nline"},{"type":"thinking","thinking":"not output"},{"type":"tool_use","name":"private tool"},{"type":"text","text":"last"}]}});
    let mut sidechain = main.clone();
    sidechain["isSidechain"] = json!(true);
    sidechain["message"]["content"][0]["text"] = json!("not main");
    fs::write(&transcript, format!("{main}\n{sidechain}\n")).unwrap();
    let turn = claude::ClaudeLifecycle
        .decode_turn(&stop_without_message(Some(&transcript)))
        .unwrap();
    let read = |turn: &TurnEnd| {
        claude::ClaudeLifecycle.turn_observation(
            turn,
            &environment,
            None,
            NOW,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
    };
    let observation = read(&turn);
    assert_eq!(
        observation.final_text.unwrap().text(),
        "first\r\nline\nlast"
    );
    let state = format!("{:?}", observation.state);
    assert!(!state.contains("first"));
    assert!(!state.contains("private tool"));
    let mut tool_only = main.clone();
    tool_only["message"]["content"] = json!([{"type":"tool_use","name":"private tool"}]);
    fs::write(&transcript, format!("{main}\n{tool_only}\n")).unwrap();
    assert!(
        read(&turn).final_text.is_none(),
        "a tool-only latest assistant does not resurrect the older text"
    );
    let mut foreign = main.clone();
    foreign["sessionId"] = json!("another-session");
    fs::write(&transcript, format!("{foreign}\n")).unwrap();
    assert!(read(&turn).final_text.is_none());
    fs::write(&transcript, format!("{main}\n")).unwrap();
    let outside = directory.path.join("outside.jsonl");
    fs::copy(&transcript, &outside).unwrap();
    let outside_turn = claude::ClaudeLifecycle
        .decode_turn(&stop_without_message(Some(&outside)))
        .unwrap();
    assert!(
        read(&outside_turn).final_text.is_none(),
        "same provider-tree admission as usage"
    );

    let codex_turn = codex::CodexLifecycle.decode_turn(&stop(None)).unwrap();
    let observation = codex::CodexLifecycle.turn_observation(
        &codex_turn,
        &environment,
        None,
        NOW,
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    );
    assert_eq!(observation.final_text.unwrap().text(), "<redacted>");
    assert!(!format!("{codex_turn:?}").contains("<redacted>"));
    assert!(
        codex_turn.transcript.is_none(),
        "Codex final text requires no transcript read"
    );
    let mut no_text: Value = serde_json::from_slice(&stop(None)).unwrap();
    no_text
        .as_object_mut()
        .unwrap()
        .remove("last_assistant_message");
    assert!(
        codex::CodexLifecycle
            .decode_turn(no_text.to_string().as_bytes())
            .unwrap()
            .final_text
            .is_none()
    );
    no_text["hook_event_name"] = json!("SubagentStop");
    assert!(
        codex::CodexLifecycle
            .decode_turn(no_text.to_string().as_bytes())
            .is_none()
    );
}

/// Claude Code flushes a turn's final assistant record to the transcript a
/// little after it fires Stop (#4165), so the Stop payload's own text is the
/// final text and the transcript is only the fallback.
#[test]
fn a_claude_stop_payload_text_is_the_final_text_before_the_transcript_has_it() {
    let directory = TestDirectory::new();
    let environment =
        ProviderEnvironment::from_parts(&directory.path, &directory.path, Vec::new(), []);
    let root = directory.path.join(".claude/projects/-workspace");
    fs::create_dir_all(&root).unwrap();
    let transcript = root.join("session.jsonl");
    let assistant = |content: Value| json!({"type":"assistant","sessionId":SESSION,"message":{"model":"claude","usage":{"input_tokens":1},"content":content}});
    // The race: the newest assistant record on disk is still the tool call.
    let tool_use = assistant(json!([{"type":"tool_use","name":"private tool"}]));
    fs::write(&transcript, format!("{tool_use}\n")).unwrap();
    let read = |payload: Vec<u8>| {
        let turn = claude::ClaudeLifecycle.decode_turn(&payload).unwrap();
        claude::ClaudeLifecycle
            .turn_observation(
                &turn,
                &environment,
                None,
                NOW,
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .final_text
    };
    let with_text = |text: Value| {
        let mut payload: Value = serde_json::from_slice(&stop(Some(&transcript))).unwrap();
        payload["last_assistant_message"] = text;
        payload.to_string().into_bytes()
    };
    assert_eq!(
        read(with_text(json!("the answer\r\nin full")))
            .unwrap()
            .text(),
        "the answer\r\nin full"
    );
    // Without a usable payload text the transcript decides, as before.
    for unusable in [
        json!(""),
        json!({"unexpected":"shape"}),
        json!(7),
        Value::Null,
    ] {
        assert!(read(with_text(unusable.clone())).is_none(), "{unusable}");
    }
    assert!(read(stop_without_message(Some(&transcript))).is_none());
    let text = assistant(json!([{"type":"text","text":"from the transcript"}]));
    fs::write(&transcript, format!("{tool_use}\n{text}\n")).unwrap();
    assert_eq!(
        read(stop_without_message(Some(&transcript)))
            .unwrap()
            .text(),
        "from the transcript"
    );
    // The payload wins when both exist, and needs no transcript at all.
    assert_eq!(read(with_text(json!("payload"))).unwrap().text(), "payload");
    let mut bare: Value = serde_json::from_slice(&stop(None)).unwrap();
    bare["last_assistant_message"] = json!("no transcript");
    assert_eq!(
        read(bare.to_string().into_bytes()).unwrap().text(),
        "no transcript"
    );
}

#[test]
fn malformed_final_text_does_not_discard_other_codex_lifecycle_evidence() {
    let mut input: Value = serde_json::from_slice(&stop(None)).unwrap();
    input["last_assistant_message"] = json!({"unexpected": "shape"});
    let turn = codex::decode_turn(input.to_string().as_bytes())
        .expect("Stop still admits lifecycle evidence");
    assert!(turn.final_text.is_none());
    input["hook_event_name"] = json!("SessionStart");
    input["source"] = json!("startup");
    assert!(codex::decode_hook(input.to_string().as_bytes()).is_some());
}

/// The sessions a Codex session started count toward its seat (#3828): the
/// sampler counts them, a hook-driven read carries them, and the state still
/// fits the driver's opaque bound.
#[test]
fn codex_counts_the_sessions_it_started_and_a_hook_read_keeps_them() {
    use super::consumption::family::{Input, Stored};
    let home = TestDirectory::new();
    let day = home.path.join(".codex/sessions/2026/10/19");
    fs::create_dir_all(&day).unwrap();
    let header = |id: &str, parent: Option<&str>| {
        format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":id,"parent_thread_id":parent,
                "base_instructions":"x".repeat(20_000)}})
        )
    };
    let usage = |input: u64, output: u64| {
        format!(
            "{}\n",
            json!({"type":"event_msg","payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":input,"output_tokens":output,
                    "cached_input_tokens":0,"total_tokens":input+output},
                "last_token_usage":{"total_tokens":input+output},
                "model_context_window":258_400}}})
        )
    };
    let main = day.join(format!("rollout-2026-10-19T09-00-00-{SESSION}.jsonl"));
    fs::write(&main, header(SESSION, None) + &usage(1_000, 100)).unwrap();
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let lifecycle = &codex::CodexLifecycle;
    let turn = lifecycle.decode_turn(&stop(Some(&main))).unwrap();
    let session = turn.session.clone();
    let deadline = || std::time::Instant::now() + std::time::Duration::from_secs(5);
    let tick = |stored: Option<Stored>, now: u64| {
        lifecycle
            .family_tick(
                &turn,
                &session,
                &environment,
                Input {
                    stored,
                    corrupt: false,
                    others: Default::default(),
                },
                now,
                deadline(),
            )
            .unwrap()
    };
    let counted = |state: &DriverState| {
        let consumption = lifecycle.state_consumption(state).unwrap();
        (consumption.input_tokens, consumption.output_tokens)
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // First sample: nothing started yet.
    let first = tick(None, now);
    let state = lifecycle
        .turn_state_with_family(
            &turn,
            &environment,
            None,
            Some(&first.contribution),
            now,
            deadline(),
        )
        .unwrap();
    assert_eq!(counted(&state), (1_000, 100));
    let stored = Stored::after(None, &first.write);
    // A subagent starts and works.
    let child = day.join("rollout-2026-10-19T09-10-00-00000000-0000-4000-8000-000000000001.jsonl");
    fs::write(
        &child,
        header("00000000-0000-4000-8000-000000000001", Some(SESSION)) + &usage(300, 30),
    )
    .unwrap();
    let second = tick(Some(stored.clone()), now + 5_000);
    let state = lifecycle
        .turn_state_with_family(
            &turn,
            &environment,
            Some(&state),
            Some(&second.contribution),
            now + 5_000,
            deadline(),
        )
        .unwrap();
    assert_eq!(counted(&state), (1_300, 130));
    // The session works on its own: a hook-driven read keeps the family.
    fs::write(
        &main,
        header(SESSION, None) + &usage(1_000, 100) + &usage(1_200, 120),
    )
    .unwrap();
    let state = lifecycle
        .turn_state(&turn, &environment, Some(&state), now + 6_000, deadline())
        .unwrap();
    assert_eq!(counted(&state), (1_500, 150));
}

/// A Claude compaction appends its boundary and summary to the same session's
/// transcript, and its `SessionStart` (`source: compact`) arrives through the
/// start hook. An agent that then stays idle is observed: the sampler's next
/// caught-up read is a complete zero observation, not an unknown gap (#5122).
#[test]
fn an_idle_claude_after_a_compaction_reads_as_observed_zero() {
    use super::lifecycle::LifecycleObservation;
    let home = TestDirectory::new();
    let directory = home.path.join(".claude/projects/-workspace");
    fs::create_dir_all(&directory).unwrap();
    let transcript = directory.join("session.jsonl");
    fs::write(
        &transcript,
        include_str!("fixtures/claude-usage-sequence.jsonl"),
    )
    .unwrap();
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let turn = claude::ClaudeLifecycle
        .decode_turn(&stop(Some(&transcript)))
        .unwrap();
    let sample = |previous: &DriverState, now: u64| {
        claude::ClaudeLifecycle
            .turn_state(
                &turn,
                &environment,
                Some(previous),
                now,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap()
    };
    let consumption = |state: &DriverState| driver_state::state_consumption(state).unwrap().value;
    let start = driver_state::after_start(Some("claude-opus-5-5"), None, None, NOW - 1).unwrap();
    let caught_up = sample(&start, NOW);
    let before = consumption(&caught_up);
    // Claude writes the compaction as appended records of the same session.
    let mut appended = fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    for record in [
        json!({"type":"system","subtype":"compact_boundary","sessionId":SESSION,
            "compactMetadata":{"trigger":"auto","preTokens":180_000}}),
        json!({"type":"user","isCompactSummary":true,"sessionId":SESSION,
            "message":{"role":"user","content":"<summary>"}}),
    ] {
        writeln!(appended, "{record}").unwrap();
    }
    drop(appended);
    let compacted = claude::decode_hook(
        json!({"session_id":SESSION,"hook_event_name":"SessionStart","source":"compact",
            "cwd":"/workspace","transcript_path":transcript})
        .to_string()
        .as_bytes(),
    )
    .unwrap()
    .driver_state(Some(&caught_up), NOW + 1_000)
    .unwrap();
    let idle = sample(&compacted, NOW + 5_000);
    let observed = consumption(&idle);
    assert!(
        observed.complete && !observed.gap,
        "an idle agent after a compaction is observed: {observed:?}"
    );
    assert_eq!(
        observed.epoch, before.epoch,
        "a compaction is no new baseline"
    );
    assert_eq!(
        (observed.input_tokens, observed.output_tokens),
        (before.input_tokens, before.output_tokens)
    );
    // Later idle ticks write nothing new (#5019).
    assert_eq!(sample(&idle, NOW + 10_000), idle);
    // Real growth after the compaction still counts its tokens.
    let mut appended = fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    writeln!(
        appended,
        "{}",
        json!({"type":"assistant","requestId":"after-compaction","message":{
            "id":"after-compaction","model":"claude-opus-5-5",
            "usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":4}
        }})
    )
    .unwrap();
    drop(appended);
    let grown = consumption(&sample(&idle, NOW + 15_000));
    assert!(grown.complete && !grown.gap);
    assert_eq!(grown.input_tokens, observed.input_tokens + 14);
    assert_eq!(grown.output_tokens, observed.output_tokens + 2);
}

/// A Claude clear starts a new conversation in a new transcript file. The
/// counter starts again there: the first read is a new baseline, and the next
/// caught-up read makes the agent observed while it stays idle (#5122).
#[test]
fn a_claude_clear_starts_a_new_baseline_that_is_observed_while_idle() {
    use super::lifecycle::LifecycleObservation;
    let home = TestDirectory::new();
    let directory = home.path.join(".claude/projects/-workspace");
    fs::create_dir_all(&directory).unwrap();
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let sample = |transcript: &Path, previous: &DriverState, now: u64| {
        let turn = claude::ClaudeLifecycle
            .decode_turn(&stop(Some(transcript)))
            .unwrap();
        claude::ClaudeLifecycle
            .turn_state(
                &turn,
                &environment,
                Some(previous),
                now,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap()
    };
    let consumption = |state: &DriverState| driver_state::state_consumption(state).unwrap().value;
    let source = include_str!("fixtures/claude-usage-sequence.jsonl");
    let old = directory.join("old.jsonl");
    fs::write(&old, source).unwrap();
    let start = driver_state::after_start(Some("claude-opus-5-5"), None, None, NOW - 1).unwrap();
    let before = sample(&old, &sample(&old, &start, NOW), NOW + 1);
    assert!(consumption(&before).complete);
    let fresh = directory.join("fresh.jsonl");
    fs::write(&fresh, source).unwrap();
    let cleared = claude::decode_hook(
        json!({"session_id":SESSION,"hook_event_name":"SessionStart","source":"clear",
            "cwd":"/workspace"})
        .to_string()
        .as_bytes(),
    )
    .unwrap()
    .driver_state(Some(&before), NOW + 2)
    .unwrap();
    assert_eq!(driver_state::state_consumption(&cleared), None);
    let first = sample(&fresh, &cleared, NOW + 5_000);
    assert_ne!(consumption(&first).epoch, consumption(&before).epoch);
    assert_eq!(
        consumption(&first).input_tokens,
        0,
        "no history is replayed"
    );
    let idle = sample(&fresh, &first, NOW + 10_000);
    let observed = consumption(&idle);
    assert!(observed.complete && !observed.gap, "{observed:?}");
    assert_eq!(sample(&fresh, &idle, NOW + 15_000), idle);
}

/// A Codex compaction goes on in the same rollout, so it keeps the counter and
/// an idle agent stays observed (#5122).
#[test]
fn an_idle_codex_after_a_compaction_stays_observed() {
    use super::lifecycle::LifecycleObservation;
    let driver = &drivers()[1];
    let home = TestDirectory::new();
    let directory = home.path.join(driver.tree);
    fs::create_dir_all(&directory).unwrap();
    let rollout = directory.join("session.jsonl");
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let turn = codex::CodexLifecycle
        .decode_turn(&stop(Some(&rollout)))
        .unwrap();
    let sample = |previous: &DriverState, now: u64| {
        codex::CodexLifecycle
            .turn_state(
                &turn,
                &environment,
                Some(previous),
                now,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
            )
            .unwrap()
    };
    let consumption = |state: &DriverState| driver_state::state_consumption(state).unwrap().value;
    let line = |total: u64| {
        edited(driver.line, |value| {
            let usage = value
                .pointer_mut("/payload/info/total_token_usage")
                .unwrap();
            usage["output_tokens"] = json!(total);
            usage["total_tokens"] = json!(usage["input_tokens"].as_u64().unwrap() + total);
        }) + "\n"
    };
    fs::write(&rollout, line(1)).unwrap();
    let start = driver_state::after_start(Some("gpt-5.5"), None, None, NOW - 1).unwrap();
    let first = sample(&start, NOW);
    fs::write(&rollout, line(1) + &line(2)).unwrap();
    let working = sample(&first, NOW + 5_000);
    let before = consumption(&working);
    assert!(before.complete && !before.gap, "{before:?}");
    let compacted = codex::decode_hook(
        json!({"session_id":SESSION,"hook_event_name":"SessionStart","source":"compact",
            "cwd":"/workspace"})
        .to_string()
        .as_bytes(),
    )
    .unwrap()
    .driver_state(Some(&working), NOW + 6_000)
    .unwrap();
    let idle = sample(&compacted, NOW + 10_000);
    assert_eq!(consumption(&idle), before, "a compaction keeps the counter");
    assert_eq!(sample(&idle, NOW + 15_000), idle);
}

/// A Codex fresh launch, clear or new thread begins a new baseline in its
/// rollout. Through the real hook path the first read may be a gap; the next
/// idle read is a complete zero observation and later ones write nothing (#5122).
#[test]
fn an_idle_codex_after_a_new_start_is_observed() {
    use super::lifecycle::LifecycleObservation;
    let driver = &drivers()[1];
    let mut failures = Vec::new();
    for source in [None, Some("clear"), Some("startup")] {
        let home = TestDirectory::new();
        let directory = home.path.join(driver.tree);
        fs::create_dir_all(&directory).unwrap();
        let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
        let sample = |rollout: &Path, previous: &DriverState, now: u64| {
            let turn = codex::CodexLifecycle
                .decode_turn(&stop(Some(rollout)))
                .unwrap();
            codex::CodexLifecycle
                .turn_state(
                    &turn,
                    &environment,
                    Some(previous),
                    now,
                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                )
                .unwrap()
        };
        let consumption =
            |state: &DriverState| driver_state::state_consumption(state).unwrap().value;
        let line = format!("{}\n", driver.line.trim_end());
        let start = driver_state::after_start(Some("gpt-5.5"), None, None, NOW - 1).unwrap();
        let rollout = directory.join("new.jsonl");
        let started = match source {
            None => start,
            Some(source) => {
                let old = directory.join("old.jsonl");
                fs::write(&old, &line).unwrap();
                let before = sample(&old, &start, NOW);
                codex::decode_hook(
                    json!({"session_id":SESSION,"hook_event_name":"SessionStart",
                        "source":source,"cwd":"/workspace"})
                    .to_string()
                    .as_bytes(),
                )
                .unwrap()
                .driver_state(Some(&before), NOW + 1)
                .unwrap()
            }
        };
        fs::write(&rollout, &line).unwrap();
        let first = sample(&rollout, &started, NOW + 5_000);
        let idle = sample(&rollout, &first, NOW + 10_000);
        let observed = consumption(&idle);
        if !observed.complete || observed.gap {
            failures.push(format!("{source:?}: {observed:?}"));
            continue;
        }
        assert_eq!(observed.epoch, consumption(&first).epoch, "{source:?}");
        assert_eq!(sample(&rollout, &idle, NOW + 15_000), idle, "{source:?}");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
