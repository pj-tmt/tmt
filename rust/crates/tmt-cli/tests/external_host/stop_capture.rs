//! Real Stop stdin and blocking Digest feedback in an owned external-host world.
use super::*;
use tmt_core::binding::BindingRepository;

fn ok(output: (i32, String, String)) -> String {
    assert_eq!((output.0, output.2.as_str()), (0, ""), "hook: {output:?}");
    output.1
}

#[test]
fn a_blocking_digest_keeps_both_claude_finals_in_causal_order_for_both_hook_orders() {
    blocking_digest("claude", true);
}

#[test]
fn a_blocking_digest_keeps_both_codex_finals_in_causal_order_for_both_hook_orders() {
    blocking_digest("codex", true);
}

#[test]
fn digest_does_not_enable_provider_capture_when_answer_hooks_are_not_installed() {
    blocking_digest("claude", false);
    blocking_digest("codex", false);
}

fn observer(provider: &mut SteppedProvider, payload: Value, capture: bool) {
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
    ok(provider.hook(payload));
}

fn blocking_digest(driver: &'static str, capture: bool) {
    register();
    let event = |name| {
        if driver == "claude" {
            claude_event(name)
        } else {
            codex_event(name, (name != "SessionStart").then_some("capture-turn"))
        }
    };
    for (observer_first, activity_only) in [(false, false), (true, false), (true, true)] {
        let fixture = Fixture::delivering(&format!(
            "capture-{driver}-{capture}-{observer_first}-{activity_only}"
        ));
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
            fs::create_dir_all(fixture.root.join(format!("home/.{driver}"))).unwrap();
            let observer =
                tmt_adapters::runtime::hook_protocol::command_entry(driver, "/stable/tmt");
            fs::write(
                fixture.root.join(if driver == "claude" {
                    "home/.claude/settings.json"
                } else {
                    "home/.codex/hooks.json"
                }),
                serde_json::json!({"hooks":{"Stop":[observer]}}).to_string(),
            )
            .unwrap();
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
        prompt["prompt"] = "Ben question".into();
        ok(provider.hook(prompt));
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
        first["stop_hook_active"] = false.into();
        first["last_assistant_message"] = "A: answer to Ben".into();
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
            observer(&mut provider, first.clone(), capture);
        }
        let feedback = ok(provider.digest_hook(first.clone()));
        let feedback: Value = serde_json::from_str(&feedback).unwrap();
        assert_eq!(feedback["decision"], "block", "{feedback:#}");
        assert!(
            feedback["reason"]
                .as_str()
                .unwrap()
                .contains("Core decision item")
        );
        if !observer_first {
            observer(&mut provider, first.clone(), capture);
        }
        let mut second = event("Stop");
        second["stop_hook_active"] = true.into();
        second["last_assistant_message"] = "B: answer to core-1".into();
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
            observer(&mut provider, second.clone(), capture);
        }
        observer(&mut provider, second, capture);
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
            ["input", "final", "input", "final"],
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
        assert_eq!(responses.len(), 2, "{timeline:#}");
        assert_eq!(responses[0]["text"], "A: answer to Ben");
        assert_eq!(responses[1]["text"], "B: answer to core-1");
        assert_ne!(responses[0]["responseId"], responses[1]["responseId"]);
        assert_eq!(responses[0]["requestId"], request);
        assert!(responses[1].get("requestId").is_none());
        assert_eq!(entries[1]["acknowledgements"][0]["identityId"], person);
        assert_eq!(entries[1]["acknowledgements"][0]["requestId"], request);
        assert_eq!(entries[3]["acknowledgements"], serde_json::json!([]));
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
        observer(&mut provider, first, capture);
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
