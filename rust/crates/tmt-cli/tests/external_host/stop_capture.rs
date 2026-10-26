//! Real Stop stdin and blocking Digest feedback in an owned external-host world.
use super::*;
use tmt_core::binding::BindingRepository;

fn ok(output: (i32, String, String)) -> String {
    assert_eq!((output.0, output.2.as_str()), (0, ""), "hook: {output:?}");
    output.1
}

#[test]
fn a_blocking_digest_keeps_both_claude_finals_in_causal_order_for_both_hook_orders() {
    blocking_digest(
        "claude",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::Normal,
    );
}

#[test]
fn a_blocking_digest_keeps_the_answer_to_text_folded_into_a_running_claude_turn() {
    blocking_digest(
        "claude",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::FoldedInput,
    );
}

#[test]
fn a_blocking_digest_keeps_both_claude_finals_after_a_compaction_inside_the_turn() {
    blocking_digest(
        "claude",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::CompactedMidTurn,
    );
}

#[test]
fn a_blocking_digest_keeps_both_codex_finals_after_a_compaction_inside_the_turn() {
    blocking_digest(
        "codex",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::CompactedMidTurn,
    );
}

#[test]
fn a_blocking_digest_keeps_both_codex_finals_in_causal_order_for_both_hook_orders() {
    blocking_digest(
        "codex",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::Normal,
    );
}

#[test]
fn a_blocking_digest_keeps_the_agy_final_before_its_held_input_for_both_hook_orders() {
    blocking_digest(
        "agy",
        true,
        &[(false, false), (true, true)],
        CaptureScenario::Normal,
    );
}

#[test]
fn canceled_and_subagent_agy_stops_grant_no_final_or_digest_input() {
    for scenario in [CaptureScenario::Canceled, CaptureScenario::Subagent] {
        blocking_digest("agy", true, &[(false, false), (true, true)], scenario);
    }
}

#[test]
fn digest_does_not_enable_provider_capture_when_answer_hooks_are_not_installed() {
    blocking_digest(
        "agy",
        false,
        &[(false, false), (true, false), (true, true)],
        CaptureScenario::Normal,
    );
    blocking_digest(
        "claude",
        false,
        &[(false, false), (true, false), (true, true)],
        CaptureScenario::Normal,
    );
    blocking_digest(
        "codex",
        false,
        &[(false, false), (true, false), (true, true)],
        CaptureScenario::Normal,
    );
}

#[test]
fn an_unbound_capture_still_hands_off_a_verified_digest_launch() {
    for driver in ["agy", "claude", "codex"] {
        blocking_digest(
            driver,
            true,
            &[(false, false)],
            CaptureScenario::UnboundOnce,
        );
    }
}

#[test]
fn a_duplicate_stop_captured_by_the_observer_still_hands_off_digest() {
    for driver in ["agy", "claude", "codex"] {
        blocking_digest(driver, true, &[(true, false)], CaptureScenario::Normal);
    }
}

#[test]
fn a_current_capture_failure_holds_digest_before_publishing_its_input() {
    for driver in ["agy", "claude", "codex"] {
        blocking_digest(
            driver,
            true,
            &[(false, false)],
            CaptureScenario::StorageFailure,
        );
    }
}

#[test]
fn a_stop_without_a_new_activity_transition_still_records_consented_usage() {
    register();
    for (driver, tree, source, tokens) in [
        (
            "claude",
            ".claude/projects/-workspace",
            include_str!("../../../tmt-adapters/src/runtime/fixtures/claude-usage-sequence.jsonl"),
            355113,
        ),
        (
            "codex",
            ".codex/sessions/2026/09/29",
            include_str!("../../../tmt-adapters/src/runtime/fixtures/codex-token-count.jsonl"),
            146577,
        ),
    ] {
        let fixture = Fixture::delivering(&format!("capture-usage-{driver}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let home = fixture.root.join("home");
        let transcript = home.join(tree).join("s-1.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(
            &transcript,
            format!(
                "{{\"type\":\"user\"}}\n{}\n",
                source.lines().next().unwrap()
            ),
        )
        .unwrap();
        let settings = home.join(if driver == "claude" {
            ".claude/settings.json"
        } else {
            ".codex/hooks.json"
        });
        let entry = tmt_adapters::runtime::hook_protocol::command_entry(driver, "/stable/tmt");
        fs::write(
            settings,
            serde_json::json!({"hooks":{"Stop":[entry]}}).to_string(),
        )
        .unwrap();
        let mut provider = SteppedProvider::start(&fixture, driver, "usage-steps");
        let event = |name| {
            if driver == "claude" {
                claude_event(name)
            } else {
                codex_event(name, None)
            }
        };
        ok(provider.hook(event("SessionStart")));
        // No prompt was admitted, so this Stop cannot produce a captured final.
        // It must still refresh usage, as must an ordinary repeated Stop.
        let mut stop = event("Stop");
        stop["transcript_path"] = transcript.to_string_lossy().into_owned().into();
        stop["last_assistant_message"] = "not an admitted final".into();
        for _ in 0..2 {
            ok(provider.hook(stop.clone()));
            let list = fixture.json(&["ls", "--json"]);
            assert_eq!(
                worker(&list)["resume"]["usage"]["tokens"],
                tokens,
                "{list:#}"
            );
        }
        let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM recent_responses", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

fn observer(
    provider: &mut SteppedProvider,
    payload: Value,
    capture: bool,
    scenario: CaptureScenario,
) {
    if !capture {
        fs::write(
            provider
                .steps
                .join(provider.next.to_string())
                .with_extension("activity"),
            "",
        )
        .unwrap();
    }
    let output = provider.hook(payload);
    if scenario == CaptureScenario::Subagent {
        assert_eq!(output.0, 0, "{output:?}");
        assert_eq!(output.1.trim(), "{}", "{output:?}");
        assert_eq!(
            output.2.trim(),
            "tmt: lifecycle context unavailable; continuing without context.",
            "{output:?}"
        );
    } else {
        ok(output);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureScenario {
    Normal,
    Canceled,
    Subagent,
    UnboundOnce,
    StorageFailure,
    /// Ben's text is typed into a turn another prompt started and a tool finishes
    /// before the Stop (#5268); no prompt ever announces it.
    FoldedInput,
    /// The provider compacts its context inside the turn, before the Stop (#5268).
    CompactedMidTurn,
}

fn blocking_digest(
    driver: &'static str,
    capture: bool,
    orders: &[(bool, bool)],
    scenario: CaptureScenario,
) {
    register();
    // Agy's continuation lifecycle still needs provider evidence (#5166). Its
    // one-Stop rows prove the retained final before Digest; never invent B's authority.
    let continued_final = driver != "agy";
    let unbound_once = scenario == CaptureScenario::UnboundOnce;
    let event = |name| match driver {
        "claude" => claude_event(name),
        "codex" => codex_event(name, (name != "SessionStart").then_some("capture-turn")),
        "agy" => match name {
            "SessionStart" => serde_json::json!({"conversationId":"s-1"}),
            "UserPromptSubmit" => serde_json::json!({"conversationId":"s-1","invocationNum":1}),
            "Stop" => {
                serde_json::json!({"conversationId":"s-1","terminationReason":"NO_TOOL_CALL","executionNum":1,"fullyIdle":true})
            }
            _ => panic!("unsupported agy fixture event {name}"),
        },
        _ => panic!("unsupported capture fixture driver {driver}"),
    };
    let final_field = if driver == "agy" {
        "finalModelOutput"
    } else {
        "last_assistant_message"
    };
    for &(observer_first, activity_only) in orders {
        let fixture = Fixture::delivering(&format!(
            "capture-{driver}-{capture}-{observer_first}-{activity_only}-{scenario:?}"
        ));
        if unbound_once {
            // The first capture sees no caller pane. Digest's independent,
            // later launch verification sees the real owned pane again.
            let script = DRIVER.replace(
                "  caller)\n",
                r#"  caller)
    if [ -f "$d/capture-unbound-once" ]; then
      rm "$d/capture-unbound-once"
      echo capture-unbound >> "$d/calls"
      printf '{"ok":{"pane":null}}'
      exit 0
    fi
"#,
            );
            tmt_test_support::write_executable(&fixture.driver, script.as_bytes(), 0o755).unwrap();
        }
        let record = fixture.approve();
        bind(&fixture, record);
        let agent = worker(&fixture.json(&["ls", "--json"]))["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let person = fixture.json(&["identity", "create", "ben", "--json"])["identity"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        fixture.json(&[
            "identity",
            "meta",
            "set",
            "user",
            "true",
            "--identity",
            "ben",
            "--json",
        ]);
        fixture.json(&["identity", "create", "core-1", "--json"]);
        if capture {
            let settings = fixture.root.join(match driver {
                "claude" => "home/.claude/settings.json",
                "codex" => "home/.codex/hooks.json",
                "agy" => "home/.gemini/config/hooks.json",
                _ => unreachable!(),
            });
            fs::create_dir_all(settings.parent().unwrap()).unwrap();
            let hooks = if driver == "agy" {
                let observer =
                    tmt_adapters::runtime::hook_protocol::flat_command_entry(driver, "/stable/tmt");
                serde_json::json!({"tmt":{"Stop":[observer]}})
            } else {
                let observer =
                    tmt_adapters::runtime::hook_protocol::command_entry(driver, "/stable/tmt");
                serde_json::json!({"hooks":{"Stop":[observer]}})
            };
            fs::write(settings, hooks.to_string()).unwrap();
        }
        let mut provider = SteppedProvider::start(&fixture, driver, "capture-steps");
        ok(provider.hook(event("SessionStart")));
        // This stand-in has no run wrapper. The test owns its live parent and
        // records that exact native incarnation as the launch owner, as a run
        // fixture would; Digest still verifies it through the public hook.
        let owner = observe_runtime_process(
            &UnixCommandRunner,
            u64::from(std::process::id()),
            deadline(),
        )
        .unwrap();
        let ProcessObservation::Live(owner) = owner else {
            panic!("test owner is not live");
        };
        let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
        storage
            .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|rows| {
                let binding = rows.entry_by_id(&agent)?.unwrap().binding.unwrap();
                let mut state = binding.session.clone();
                state.launch_owner = Some(owner.clone());
                assert!(rows.set_session_state(&binding.id, &binding.session, &state)?);
                Ok(())
            })
            .unwrap();
        storage.close().unwrap();
        let now = tmt_adapters::request_runtime::wall_time_ms();
        let policy = passthrough::api(
            &fixture,
            "ok",
            "digest.policy.set",
            serde_json::json!({"identityId":agent,"ownerIdentityId":person,"setterIdentityId":person,"expectedRevision":0,"untilMs":now+60_000}),
        );
        assert_eq!(policy["active"], true, "{policy:#}");
        let folded = scenario == CaptureScenario::FoldedInput;
        if folded {
            let mut earlier = event("UserPromptSubmit");
            earlier["prompt"] = "A request that began the turn".into();
            ok(provider.hook(earlier));
        }
        let sent = fixture.json(&[
            "talk",
            "worker",
            "Ben question",
            "--passthrough",
            "--identity",
            "ben",
            "--detach",
            "--json",
        ]);
        let request = sent["requestId"].as_str().unwrap();
        let mut prompt = event("UserPromptSubmit");
        prompt[if driver == "agy" {
            "lastUserInput"
        } else {
            "prompt"
        }] = "Ben question".into();
        if folded {
            ok(provider.hook(event("PostToolUse")));
        } else {
            ok(provider.hook(prompt));
        }
        if scenario == CaptureScenario::CompactedMidTurn {
            let mut compacted = event("SessionStart");
            compacted["source"] = "compact".into();
            ok(provider.hook(compacted));
        }
        let held = fixture.json(&[
            "talk",
            "worker",
            "Core decision item",
            "--identity",
            "core-1",
            "--detach",
            "--json",
        ]);
        assert_eq!(held["digest"], true, "{held:#}");
        let mut first = event("Stop");
        if driver != "agy" {
            first["stop_hook_active"] = false.into();
        }
        first[final_field] = "A: answer to Ben".into();
        match scenario {
            CaptureScenario::Canceled => first["terminationReason"] = "USER_CANCELED".into(),
            CaptureScenario::Subagent => first["parentConversationId"] = "parent-session".into(),
            _ => (),
        }
        let before_stop = passthrough::api(
            &fixture,
            "ok",
            "responses.timeline",
            serde_json::json!({"identityId":agent}),
        );
        if observer_first {
            if activity_only {
                fs::write(
                    provider
                        .steps
                        .join(provider.next.to_string())
                        .with_extension("activity"),
                    "",
                )
                .unwrap();
            }
            observer(&mut provider, first.clone(), capture, scenario);
        }
        if unbound_once {
            fs::write(fixture.root.join("driver/capture-unbound-once"), "").unwrap();
        }
        if scenario == CaptureScenario::StorageFailure {
            let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
            db.execute_batch("CREATE TRIGGER refuse_final BEFORE INSERT ON recent_responses BEGIN SELECT RAISE(ABORT, 'capture fixture failure'); END;").unwrap();
        }
        let feedback = ok(provider.digest_hook(first.clone()));
        if matches!(
            scenario,
            CaptureScenario::Canceled | CaptureScenario::Subagent
        ) {
            assert_eq!(
                feedback.trim(),
                "{}",
                "no Digest continuation for {scenario:?}"
            );
            if !observer_first || activity_only {
                observer(&mut provider, first.clone(), capture, scenario);
            }
            let after = passthrough::api(
                &fixture,
                "ok",
                "responses.timeline",
                serde_json::json!({"identityId":agent}),
            );
            assert_eq!(after["responses"], serde_json::json!([]), "{after:#}");
            assert_eq!(
                after["entries"], before_stop["entries"],
                "no final or invented Digest input: {after:#}"
            );
            let shown = passthrough::api(
                &fixture,
                "ok",
                "requests.show",
                serde_json::json!({"requestId":request}),
            );
            assert_eq!(
                shown["passthrough"]["state"],
                if scenario == CaptureScenario::Canceled {
                    "no_answer"
                } else {
                    "bound"
                },
                "{shown:#}"
            );
            let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
            let (batches, open): (i64, i64) = db
                .query_row(
                    "SELECT count(*),sum(closed_ms IS NULL) FROM capture_batches",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                (batches, open),
                (
                    1,
                    if scenario == CaptureScenario::Canceled {
                        0
                    } else {
                        1
                    }
                ),
                "{scenario:?}"
            );
            continue;
        }
        if scenario == CaptureScenario::StorageFailure {
            assert!(
                (feedback.is_empty() || (driver == "agy" && feedback.trim() == "{}")),
                "failed capture must precede Digest publication"
            );
            let timeline = passthrough::api(
                &fixture,
                "ok",
                "responses.timeline",
                serde_json::json!({"identityId":agent}),
            );
            let entries = timeline["entries"].as_array().unwrap();
            assert!(
                entries.iter().any(|entry| entry["kind"] == "failure"
                    && entry["failure"]["code"] == "storage_error"),
                "{timeline:#}"
            );
            assert!(
                !entries
                    .iter()
                    .any(|entry| entry["kind"] == "input" && entry["text"] == "Core decision item"),
                "{timeline:#}"
            );
            continue;
        }
        assert!(
            !(feedback.is_empty() || (driver == "agy" && feedback.trim() == "{}")),
            "a verified Digest launch must still hand off the held item"
        );
        let feedback: Value = serde_json::from_str(&feedback).unwrap();
        assert_eq!(
            feedback["decision"],
            if driver == "agy" { "continue" } else { "block" },
            "{feedback:#}"
        );
        assert!(
            feedback["reason"]
                .as_str()
                .unwrap()
                .contains("Core decision item")
        );
        if unbound_once {
            assert!(fixture.calls().iter().any(|call| call == "capture-unbound"));
            let timeline = passthrough::api(
                &fixture,
                "ok",
                "responses.timeline",
                serde_json::json!({"identityId":agent}),
            );
            assert_eq!(timeline["responses"], serde_json::json!([]), "{timeline:#}");
            assert!(
                timeline["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["kind"] == "input" && entry["text"] == "Core decision item"),
                "{timeline:#}"
            );
            continue;
        }
        if !observer_first {
            observer(&mut provider, first.clone(), capture, scenario);
        }
        if continued_final {
            let mut second = event("Stop");
            second["stop_hook_active"] = true.into();
            second[final_field] = "B: answer to core-1".into();
            ok(provider.digest_hook(second.clone()));
            if activity_only {
                fs::write(
                    provider
                        .steps
                        .join(provider.next.to_string())
                        .with_extension("activity"),
                    "",
                )
                .unwrap();
                observer(&mut provider, second.clone(), capture, scenario);
            }
            observer(&mut provider, second, capture, scenario);
        }
        let timeline = passthrough::api(
            &fixture,
            "ok",
            "responses.timeline",
            serde_json::json!({"identityId":agent}),
        );
        if !capture {
            assert_eq!(timeline["responses"], serde_json::json!([]), "{timeline:#}");
            assert!(
                timeline["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|e| e["kind"] == "input"),
                "{timeline:#}"
            );
            continue;
        }
        let entries = timeline["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("{timeline:#}"));
        assert_eq!(
            entries
                .iter()
                .map(|e| e["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            if continued_final {
                vec!["input", "final", "input", "final"]
            } else {
                vec!["input", "final", "input"]
            },
            "{timeline:#}"
        );
        assert_eq!(entries[0]["requestId"], request);
        assert_eq!(entries[2]["text"], "Core decision item");
        assert_eq!(entries[2]["sender"]["name"], "core-1");
        assert!(
            entries
                .iter()
                .all(|e| e["inputBatchId"] == entries[0]["inputBatchId"])
        );
        let responses = timeline["responses"].as_array().unwrap();
        assert_eq!(
            responses.len(),
            if continued_final { 2 } else { 1 },
            "{timeline:#}"
        );
        assert_eq!(responses[0]["provider"], driver);
        assert_eq!(responses[0]["text"], "A: answer to Ben");
        assert_eq!(responses[0]["requestId"], request);
        if continued_final {
            assert_eq!(responses[1]["provider"], driver);
            assert_eq!(responses[1]["text"], "B: answer to core-1");
            assert_ne!(responses[0]["responseId"], responses[1]["responseId"]);
            assert!(responses[1].get("requestId").is_none());
            assert_eq!(entries[3]["acknowledgements"], serde_json::json!([]));
        }
        assert_eq!(entries[1]["acknowledgements"][0]["identityId"], person);
        assert_eq!(entries[1]["acknowledgements"][0]["requestId"], request);
        let shown = passthrough::api(
            &fixture,
            "ok",
            "requests.show",
            serde_json::json!({"requestId":request}),
        );
        assert_eq!(
            shown["passthrough"]["answer"]["text"], "A: answer to Ben",
            "{shown:#}"
        );
        assert_eq!(
            shown["passthrough"]["inputBatchId"],
            entries[0]["inputBatchId"]
        );
        observer(&mut provider, first, capture, scenario);
        let after = passthrough::api(
            &fixture,
            "ok",
            "responses.timeline",
            serde_json::json!({"identityId":agent}),
        );
        assert_eq!(
            after["entries"], timeline["entries"],
            "duplicate ordinary Stop changes nothing"
        );
    }
}
