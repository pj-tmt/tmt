//! A hand resume completes the existing identity binding, never a new identity.
use super::*;

fn event(provider: &str, session: &str) -> Value {
    let mut value = if provider == "codex" {
        codex_event("SessionStart", None)
    } else {
        serde_json::json!({"hook_event_name":"SessionStart", "session_id":session,
            "source":"resume", "cwd":"/src", "model":"fixture-model"})
    };
    value["session_id"] = session.into();
    value["source"] = "resume".into();
    value
}

fn stored(fixture: &Fixture) -> tmt_adapters::storage::IdentityContextSnapshot {
    super::move_session::remembered(fixture)
}

#[test]
fn a_hand_resume_keeps_the_binding_and_learns_the_new_session() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(&format!("hand-resume-{provider}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut old = SteppedProvider::start(&fixture, provider, "old-provider");
        let output = old.hook(event(provider, "old-session"));
        assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
        let before = stored(&fixture);
        drop(old);
        let mut resumed = SteppedProvider::start(&fixture, provider, "resumed-provider");
        let process = live_provider(&resumed);
        let output = resumed.hook(event(provider, "resumed-session"));
        assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
        let after = stored(&fixture);
        assert_eq!(after.entry.identity, before.entry.identity);
        let binding = after.entry.binding.unwrap();
        assert_eq!(binding.id, before.entry.binding.unwrap().id);
        let key = binding.session.key.unwrap();
        assert_eq!(key.incarnation, process.incarnation);
        assert_eq!(key.provider_session.unwrap().as_str(), "resumed-session");
        assert_eq!(
            after
                .preferences
                .remembered
                .unwrap()
                .provider_session
                .as_str(),
            "resumed-session"
        );
        assert_eq!(
            worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
            false
        );
    }
}

#[test]
fn another_driver_cannot_replace_the_old_record_after_its_process_exits() {
    register();
    for (provider, other) in [("claude", "codex"), ("codex", "claude")] {
        let fixture = Fixture::new(&format!("hand-resume-other-{provider}"));
        let record = fixture.approve();
        bind(&fixture, record);
        let mut old = SteppedProvider::start(&fixture, provider, "old-provider");
        let output = old.hook(event(provider, "old-session"));
        assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
        let before = stored(&fixture);
        drop(old);
        let mut wrong = SteppedProvider::start(&fixture, other, "wrong-provider");
        let output = wrong.hook(event(other, "wrong-session"));
        assert_eq!(output.0, 0);
        assert!(output.1.is_empty(), "{output:?}");
        let after = stored(&fixture);
        assert_eq!(after.preferences, before.preferences);
        assert_eq!(after.entry.identity, before.entry.identity);
        assert_eq!(
            worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
            true
        );
    }
}

#[test]
fn a_live_or_stopped_old_session_refuses_and_a_later_verified_start_clears_the_reason() {
    register();
    for provider in ["claude", "codex"] {
        for stopped in [false, true] {
            let fixture = Fixture::new(&format!("hand-resume-busy-{provider}-{stopped}"));
            bind(&fixture, fixture.approve());
            let mut old = SteppedProvider::start(&fixture, provider, "old-provider");
            let output = old.hook(event(provider, "old-session"));
            assert_eq!(output.2, "");
            let before = stored(&fixture);
            let owned = live_provider(&old);
            if stopped {
                nix::sys::signal::kill(owned.pid(), nix::sys::signal::Signal::SIGSTOP).unwrap();
                wait_until("the exact old provider is stopped", || {
                    owned.seen() == Seen::Stopped
                });
            }
            let mut resumed = SteppedProvider::start(&fixture, provider, "new-provider");
            let output = resumed.hook(event(provider, "new-session"));
            assert_eq!(output.0, 0);
            assert!(output.1.is_empty(), "{output:?}");
            let refused = stored(&fixture);
            assert_eq!(refused.preferences, before.preferences);
            let observation = refused
                .entry
                .binding
                .as_ref()
                .unwrap()
                .session
                .start_refusal
                .as_ref()
                .unwrap();
            assert_eq!(observation.harness.as_str(), provider);
            assert_eq!(observation.session.as_str(), "new-session");
            assert_eq!(
                observation.reason.as_str(),
                if stopped {
                    "previous_session_unknown"
                } else {
                    "previous_session_live"
                }
            );
            assert_eq!(
                worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
                true
            );
            if stopped {
                nix::sys::signal::kill(owned.pid(), nix::sys::signal::Signal::SIGCONT).unwrap();
            }
            drop(old);
            let output = resumed.hook(event(provider, "new-session"));
            assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
            let admitted = stored(&fixture);
            assert!(
                admitted
                    .entry
                    .binding
                    .unwrap()
                    .session
                    .start_refusal
                    .is_none()
            );
            assert_eq!(
                worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
                false
            );
        }
    }
}

#[test]
fn a_nested_start_has_a_reason_and_cannot_replace_the_main_provider_record() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(&format!("hand-resume-nested-{provider}"));
        bind(&fixture, fixture.approve());
        let before = stored(&fixture);
        let mut nested =
            SteppedProvider::start_nested(&fixture, provider, "nested-provider", &[provider]);
        let output = nested.hook(event(provider, "nested-session"));
        assert_eq!(output.0, 0);
        assert!(output.1.is_empty());
        let after = stored(&fixture);
        assert_eq!(after.preferences, before.preferences);
        assert_eq!(after.entry.identity, before.entry.identity);
        let binding = after.entry.binding.unwrap();
        assert!(binding.session.key.is_none());
        assert_eq!(
            binding.session.start_refusal.unwrap().reason.as_str(),
            "private_process_unverified"
        );
        assert_eq!(
            worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
            true
        );
    }
}

#[test]
fn an_ended_session_can_be_replaced_while_its_old_process_is_still_alive() {
    register();
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(&format!("hand-resume-ended-{provider}"));
        bind(&fixture, fixture.approve());
        let mut old = SteppedProvider::start(&fixture, provider, "old-provider");
        assert_eq!(old.hook(event(provider, "old-session")).2, "");
        let before = stored(&fixture);
        let old_process = live_provider(&old);
        let mut end = event(provider, "old-session");
        end["hook_event_name"] = "SessionEnd".into();
        end["reason"] = "other".into();
        assert_eq!(old.hook(end).2, "");
        assert_eq!(old_process.seen(), Seen::Live);
        let mut resumed = SteppedProvider::start(&fixture, provider, "new-provider");
        let process = live_provider(&resumed);
        let output = resumed.hook(event(provider, "new-session"));
        assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
        let after = stored(&fixture);
        assert_eq!(after.entry.identity, before.entry.identity);
        let binding = after.entry.binding.unwrap();
        assert_eq!(binding.id, before.entry.binding.unwrap().id);
        assert_eq!(
            binding.session.key.unwrap().incarnation,
            process.incarnation
        );
        assert!(binding.session.start_refusal.is_none());
        assert_eq!(old_process.seen(), Seen::Live, "adoption sends no signal");
    }
}

// Reuse the captured index/header grammar, never a transcript search or recent-file guess.
fn codex_locator(fixture: &Fixture) -> (String, String) {
    let captured: Value = serde_json::from_str(include_str!(
        "../../../tmt-adapters/src/runtime/fixtures/codex-caller-session.json"
    ))
    .unwrap();
    let home = fixture.root.join("codex-locator");
    let rollout = home.join("sessions/root.jsonl");
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    fs::write(&rollout, format!("{}\n", captured["header"])).unwrap();
    let connection = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
    let definitions = captured["index"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            format!(
                "{} {} {} {} DEFAULT {}",
                column["name"].as_str().unwrap(),
                column["type"].as_str().unwrap(),
                if column["notNull"] == 1 {
                    "NOT NULL"
                } else {
                    ""
                },
                if column["primaryKey"] == 1 {
                    "PRIMARY KEY"
                } else {
                    ""
                },
                if column["type"] == "TEXT" { "''" } else { "0" }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    connection
        .execute_batch(&format!("CREATE TABLE threads ({definitions});"))
        .unwrap();
    let session = captured["environment"]["CODEX_THREAD_ID"]
        .as_str()
        .unwrap()
        .to_owned();
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path, source) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                session,
                rollout.to_str().unwrap(),
                captured["index"]["row"]["source"].as_str().unwrap()
            ],
        )
        .unwrap();
    (session, home.to_str().unwrap().to_owned())
}

#[test]
fn explicit_binds_in_the_verified_provider_learn_its_session_without_a_hook() {
    register();
    for provider in ["claude", "codex"] {
        for args in [
            vec!["name", "worker", "--json"],
            vec!["this", "worker", "--json"],
            vec!["add", "f1", "worker", "--json"],
        ] {
            let fixture = Fixture::new(&format!("hand-resume-explicit-{provider}-{}", args[0]));
            bind(&fixture, fixture.approve());
            let before = stored(&fixture);
            let mut agent = SteppedProvider::start(&fixture, provider, "hand-started-provider");
            let process = live_provider(&agent);
            let pid = process.incarnation.pid().to_string();
            let (session, home) = codex_locator(&fixture);
            let environment = if provider == "claude" {
                vec![
                    ("CLAUDE_CODE_SESSION_ID", session.as_str()),
                    ("CLAUDE_PID", pid.as_str()),
                ]
            } else {
                vec![
                    ("CODEX_THREAD_ID", session.as_str()),
                    ("CODEX_HOME", home.as_str()),
                    ("TMT_CALLER_SESSION_DEBUG", "1"),
                ]
            };
            let output = agent.command(&environment, &args);
            assert_eq!(output.0, 0, "{provider} {args:?}: {output:?}");
            let document: Value = serde_json::from_str(&output.1).unwrap();
            assert_eq!(
                document["session"]["unbound"], false,
                "{provider} {args:?}: {output:?}"
            );
            let after = stored(&fixture);
            assert_eq!(after.entry.identity, before.entry.identity);
            let binding = after.entry.binding.unwrap();
            assert_eq!(binding.id, before.entry.binding.unwrap().id);
            let key = binding.session.key.unwrap();
            assert_eq!(key.incarnation, process.incarnation);
            assert_eq!(key.provider_session.unwrap().as_str(), session);
        }
    }
}

#[test]
fn add_from_elsewhere_never_borrows_the_callers_session_and_explains_the_next_step() {
    register();
    let fixture = Fixture::new("hand-resume-add-elsewhere");
    super::move_session::destination(&fixture);
    let _target = SteppedProvider::start_in(&fixture, "claude", "target-provider", "f1", true);
    fs::write(
        fixture.root.join("driver/pid"),
        _target.child.id().to_string(),
    )
    .unwrap();
    fs::write(
        fixture.root.join("driver/shell"),
        _target.child.id().to_string(),
    )
    .unwrap();
    bind(&fixture, fixture.approve());
    let before = stored(&fixture);
    let mut caller = SteppedProvider::start_in(&fixture, "claude", "caller-provider", "f2", true);
    fs::write(
        fixture.root.join("driver/pid-2"),
        caller.child.id().to_string(),
    )
    .unwrap();
    let process = live_provider(&caller);
    let pid = process.incarnation.pid().to_string();
    let output = caller.command(
        &[
            (
                "CLAUDE_CODE_SESSION_ID",
                "4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13",
            ),
            ("CLAUDE_PID", &pid),
        ],
        &["add", "f1", "worker", "--json"],
    );
    assert_eq!(output.0, 0, "{output:?}");
    let document: Value = serde_json::from_str(&output.1).unwrap();
    let warning = document["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["code"] == "SESSION_NOT_RECORDED")
        .unwrap();
    assert!(
        warning["reason"]
            .as_str()
            .unwrap()
            .contains("session was not recorded")
    );
    assert!(warning["fix"].as_str().unwrap().contains("tmt setup"));
    let after = stored(&fixture);
    assert_eq!(after.preferences, before.preferences);
    assert_eq!(
        after.entry.binding.unwrap().session.key,
        before.entry.binding.unwrap().session.key
    );
}
