//! Provider switches use public CLI commands in an owned external host world.
use super::move_session::{destination, remembered, start};
use super::*;
const SESSION: &str = "4f8c2a7e-0d3b-4c61-9a52-7b1e6d0c9f13";

fn switch_provider(fixture: &Fixture, provider: &str, session: &str, source: &str) {
    let executable = fixture.root.join("bin").join(provider);
    if executable.exists() {
        fs::remove_file(&executable).unwrap();
    }
    let payload = serde_json::json!({"hook_event_name":"SessionStart", "session_id":session, "source":source});
    let cli = fixture.command_in(Some("f2"), &[]).get_program().to_owned();
    let native = fixture.root.join("switch-native").join(provider);
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    if !native.exists() {
        std::os::unix::fs::symlink("/bin/sh", &native).unwrap();
    }
    // A shebang script is `sh` to native comm. The provider-named shell
    // reuses SteppedProvider's native ancestry shape and keeps the launch PID.
    let body = format!(
        "printf '%s' '{payload}' | CLAUDE_PID=$$ '{}' __hook {provider} > switch.hook.out 2> switch.hook.err\n{AGENT}",
        cli.to_string_lossy()
    );
    let body = body.replace('\'', "'\\''");
    tmt_test_support::write_executable(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > switch.args\nexec '{}' -c '{body}'\n",
            native.display()
        )
        .as_bytes(),
        0o755,
    )
    .unwrap();
}

/// Both driver directions use the public command. A real receipt answers the
/// handoff; a missing answer and an already-gone provider use notebook fallback.
#[test]
fn driver_switch_keeps_identity_history_and_hands_off_once_before_launch() {
    register();
    for (provider, target) in [("claude", "codex"), ("codex", "claude")] {
        for disposition in ["answers", "times-out", "gone"] {
            let fixture = Fixture::delivering(&format!("switch-{provider}-{disposition}"));
            destination(&fixture);
            fs::write(fixture.root.join("driver/agent"), "ok").unwrap();
            let (old, process) = start(&fixture, provider, fixture.approve());
            let mut old = Some(old);
            let before = remembered(&fixture);
            let notes = fixture
                .state()
                .join("notes")
                .join(&before.entry.identity.id);
            fs::create_dir_all(&notes).unwrap();
            fs::write(
                notes.join("notes.md"),
                "Current work: preserve the UUID. Next: verify the switch.",
            )
            .unwrap();
            if disposition == "gone" {
                drop(old.take());
                assert_eq!(process.seen(), Seen::Ended);
            }
            switch_provider(
                &fixture,
                target,
                "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
                "startup",
            );
            let mut command = fixture.command_in(
                Some("f2"),
                &[
                    "resume",
                    "--driver",
                    target,
                    "--model",
                    "fixture-model",
                    "--no-channel",
                    "worker",
                ],
            );
            command.env(
                "PATH",
                format!(
                    "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                    fixture.root.join("bin").display()
                ),
            );
            let mut launched = Launched::start(&fixture, command);
            if disposition != "gone" {
                wait_until("one handoff request is submitted", || {
                    !fixture.requests("prompted").is_empty()
                        || launched
                            .child
                            .as_mut()
                            .unwrap()
                            .try_wait()
                            .unwrap()
                            .is_some()
                });
                let prompts = fixture.requests("prompted");
                assert_eq!(
                    prompts.len(),
                    1,
                    "{}",
                    fs::read_to_string(fixture.root.join("launched.err")).unwrap()
                );
                assert_eq!(process.seen(), Seen::Live);
                assert!(
                    !fixture.root.join("agent.pid").exists(),
                    "no new agent before handoff completes"
                );
                if disposition == "answers" {
                    let text = prompts[0]["text"].as_str().unwrap();
                    let reply = text
                        .lines()
                        .find(|line| line.starts_with("tmt reply "))
                        .expect("receipt-bound handoff");
                    let words = reply.split_whitespace().collect::<Vec<_>>();
                    let receipt = words.iter().position(|word| *word == "--receipt").unwrap();
                    fs::write(
                        notes.join("notes.md"),
                        "Saved handoff: continue the exact identity.",
                    )
                    .unwrap();
                    let result = old.as_mut().unwrap().command(
                        &[],
                        &[
                            "reply",
                            words[2],
                            "--receipt",
                            words[receipt + 1],
                            "--message",
                            "Notes saved",
                        ],
                    );
                    assert_eq!(result.0, 0, "{result:?}");
                }
            }
            // A timeout is the approved 30 s wait, so this scenario's readiness
            // bound includes it. There is no sleep, request retry or product knob.
            let until = std::time::Instant::now() + Duration::from_secs(40);
            while !fixture.root.join("agent.pid").exists() {
                if launched
                    .child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < until,
                    "switch readiness timed out"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                fixture.root.join("agent.pid").exists(),
                "{}",
                fs::read_to_string(fixture.root.join("launched.err")).unwrap()
            );
            launched.adopt("agent.pid", false);
            assert_eq!(
                fs::read_to_string(fixture.root.join("switch.hook.err")).unwrap(),
                "",
                "{provider}->{target} {disposition}"
            );
            assert_eq!(process.seen(), Seen::Ended);
            wait_until("the new exact session is committed", || {
                let stored = remembered(&fixture);
                stored
                    .preferences
                    .remembered
                    .as_ref()
                    .is_some_and(|session| {
                        session.harness.as_str() == target
                            && session.provider_session.as_str()
                                == "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
                    })
            });
            let after = remembered(&fixture);
            assert_eq!(after.entry.identity, before.entry.identity);
            assert_eq!(after.entry.binding.as_ref().unwrap().pane_id, "fake-2");
            let retained = after
                .preferences
                .retained
                .iter()
                .find(|record| record.session.harness.as_str() == provider)
                .unwrap();
            assert_eq!(retained.session.provider_session.as_str(), SESSION);
            let mut switched_back = after.preferences.clone();
            switched_back.launched(&tmt_core::binding::session::HarnessId::new(provider).unwrap());
            assert_eq!(
                switched_back.remembered.unwrap().provider_session.as_str(),
                SESSION
            );
            let args = fs::read(fixture.root.join("switch.args")).unwrap();
            let args = args
                .split(|byte| *byte == 0)
                .filter(|word| !word.is_empty())
                .map(|word| String::from_utf8(word.to_vec()).unwrap())
                .collect::<Vec<_>>();
            assert!(args.iter().any(|word| word == "fixture-model"));
            let context = args.last().unwrap();
            assert!(context.contains("untrusted context"));
            assert!(context.contains(if disposition == "answers" {
                "Saved handoff:"
            } else {
                "Current work:"
            }));
            assert!(!context.contains("--receipt"));
            assert_eq!(
                fixture.requests("prompted").len(),
                usize::from(disposition != "gone")
            );
            let (status, stderr) = launched.finish();
            assert_eq!(status, Some(7), "{stderr}");
            // The old destination is now this command's own pane and its
            // agent has ended. A public switch back must resume the old record,
            // with no extra --move and no settings borrowed from the new driver.
            fs::remove_file(fixture.root.join("agent.pid")).unwrap();
            fs::remove_file(fixture.root.join("release")).unwrap();
            switch_provider(&fixture, provider, SESSION, "resume");
            let mut command = fixture.command_in(
                Some("f2"),
                &["resume", "--driver", provider, "--no-channel", "worker"],
            );
            command.env(
                "PATH",
                format!(
                    "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                    fixture.root.join("bin").display()
                ),
            );
            let mut back = Launched::start(&fixture, command);
            wait_until("the retained driver launches or refuses", || {
                fixture.root.join("agent.pid").exists()
                    || back.child.as_mut().unwrap().try_wait().unwrap().is_some()
            });
            assert!(
                fixture.root.join("agent.pid").exists(),
                "{}",
                fs::read_to_string(fixture.root.join("launched.err")).unwrap()
            );
            back.adopt("agent.pid", false);
            assert_eq!(
                fs::read_to_string(fixture.root.join("switch.hook.err")).unwrap(),
                ""
            );
            let back_args = fs::read(fixture.root.join("switch.args")).unwrap();
            let back_args = back_args
                .split(|byte| *byte == 0)
                .filter(|word| !word.is_empty())
                .map(|word| String::from_utf8(word.to_vec()).unwrap())
                .collect::<Vec<_>>();
            let flag = if provider == "claude" {
                "--resume"
            } else {
                "resume"
            };
            assert!(back_args.iter().any(|word| word == flag));
            assert!(back_args.iter().any(|word| word == SESSION));
            assert!(!back_args.iter().any(|word| word == "fixture-model"));
            wait_until("the original exact session is current again", || {
                remembered(&fixture)
                    .preferences
                    .remembered
                    .as_ref()
                    .is_some_and(|session| {
                        session.harness.as_str() == provider
                            && session.provider_session.as_str() == SESSION
                    })
            });
            assert_eq!(remembered(&fixture).entry.identity, before.entry.identity);
            assert_eq!(
                fixture.requests("prompted").len(),
                usize::from(disposition != "gone")
            );
            let (status, stderr) = back.finish();
            assert_eq!(status, Some(7), "{stderr}");
            drop(old);
            assert_drivers_ended(&fixture);
        }
    }
}

/// Unsupported targets are refused before the optional handoff or any stop.
#[test]
fn unsupported_switch_keeps_the_live_agent_and_all_resume_records_unchanged() {
    register();
    let fixture = Fixture::delivering("switch-unsupported");
    destination(&fixture);
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let before = remembered(&fixture);
    for target in ["unknown-driver", "gemini"] {
        let output = fixture
            .command_in(
                Some("f2"),
                &["resume", "--driver", target, "--no-channel", "worker"],
            )
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("The selected driver cannot switch sessions"),
            "{stderr}"
        );
        assert!(
            stderr.contains("No new agent was launched")
                || stderr.contains("no new agent was launched"),
            "{stderr}"
        );
        let after = remembered(&fixture);
        assert_eq!(after.entry, before.entry);
        assert_eq!(after.preferences, before.preferences);
        assert_eq!(process.seen(), Seen::Live);
        assert!(fixture.requests("prompted").is_empty());
        assert!(!fixture.root.join("agent.pid").exists());
    }
    drop(old);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}
