//! A Digest Stop that refuses still prints no continuation, and names why on its launch's
//! session-learning row (#4871). Real Stop stdin through the public hook in an owned
//! external-host world.
use super::*;
use tmt_adapters::runtime::hook_protocol::HookLaunch;
use tmt_core::{
    binding::{BindingRepository, session::launch::LaunchStatus},
    endpoint::ProcessIncarnation,
};

/// A Codex agent with a Digest due, a launch owner and an admitted launch row.
struct DueDigest {
    fixture: Fixture,
    codex: SteppedProvider,
    agent: String,
    launch: HookLaunch,
}

impl DueDigest {
    /// `owner` is the launch owner `tmt run` recorded; `due` holds one item for the agent.
    fn start(name: &str, owner: ProcessIncarnation, due: bool, capture: bool) -> Self {
        register();
        let fixture = Fixture::delivering(name);
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
            fs::create_dir_all(fixture.root.join("home/.codex")).unwrap();
            let observer =
                tmt_adapters::runtime::hook_protocol::command_entry("codex", "/stable/tmt");
            fs::write(
                fixture.root.join("home/.codex/hooks.json"),
                serde_json::json!({"hooks":{"Stop":[observer]}}).to_string(),
            )
            .unwrap();
        }
        let mut codex = SteppedProvider::start(&fixture, "codex", "digest-refusal-steps");
        ok(codex.hook(codex_event("SessionStart", None)));
        let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
        let binding = storage
            .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|rows| {
                let binding = rows.entry_by_id(&agent)?.unwrap().binding.unwrap();
                let mut state = binding.session.clone();
                state.launch_owner = Some(owner.clone());
                assert!(rows.set_session_state(&binding.id, &binding.session, &state)?);
                let binding = rows.entry_by_id(&agent)?.unwrap().binding.unwrap();
                let now = tmt_adapters::request_runtime::wall_time_ms();
                let driver = tmt_core::binding::session::HarnessId::new("codex").unwrap();
                assert!(rows.begin_launch_observation(&binding, &owner, &driver, now)?);
                let observed = rows.launch_observation(&binding.id, &owner)?.unwrap();
                let mut admitted = observed.clone();
                admitted.status = LaunchStatus::Admitted;
                admitted.diagnostic = None;
                assert!(rows.set_launch_observation(&observed, &admitted)?);
                Ok(binding)
            })
            .unwrap();
        storage.close().unwrap();
        if due {
            let now = tmt_adapters::request_runtime::wall_time_ms();
            let policy = passthrough::api(
                &fixture,
                "ok",
                "digest.policy.set",
                serde_json::json!({"identityId":agent,"ownerIdentityId":person,"setterIdentityId":person,"expectedRevision":0,"untilMs":now+60_000}),
            );
            assert_eq!(policy["active"], true, "{policy:#}");
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
        }
        let launch = HookLaunch {
            identity_id: agent.clone(),
            binding_id: binding.id,
            owner_pid: owner.pid(),
            owner_start: owner.start_identity().into(),
        };
        Self {
            fixture,
            codex,
            agent,
            launch,
        }
    }

    /// One Digest Stop with the launch's observation in its environment, as `tmt run` sets it.
    fn stop(&mut self, turn: &str) -> String {
        let mut payload = codex_event("Stop", Some(turn));
        payload["stop_hook_active"] = false.into();
        payload["last_assistant_message"] = format!("answer {turn}").into();
        let step = self.codex.steps.join(self.codex.next.to_string());
        fs::write(step.with_extension("digest"), "").unwrap();
        let observation = serde_json::to_string(&self.launch).unwrap();
        ok(self.codex.hook_with_env(
            payload,
            &[("TMT_LAUNCH_OBSERVATION", observation.as_bytes())],
        ))
    }

    /// The public `tmt ls --json` session block of the agent.
    fn session(&self) -> Value {
        worker(&self.fixture.json(&["ls", "--json"]))["session"].clone()
    }

    fn set_binding(
        &self,
        change: impl FnOnce(&mut tmt_core::binding::session::BindingSessionState),
    ) {
        let mut storage = Storage::open(self.fixture.state().join("tmt.db")).unwrap();
        storage
            .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|rows| {
                let binding = rows.entry_by_id(&self.agent)?.unwrap().binding.unwrap();
                let mut state = binding.session.clone();
                change(&mut state);
                assert!(rows.set_session_state(&binding.id, &binding.session, &state)?);
                Ok(())
            })
            .unwrap();
        storage.close().unwrap();
    }
}

fn ok(output: (i32, String, String)) -> String {
    assert_eq!((output.0, output.2.as_str()), (0, ""), "hook: {output:?}");
    output.1
}

fn live_owner() -> ProcessIncarnation {
    let ProcessObservation::Live(owner) = observe_runtime_process(
        &UnixCommandRunner,
        u64::from(std::process::id()),
        deadline(),
    )
    .unwrap() else {
        panic!("test owner is not live");
    };
    owner
}

/// The same process with a start identity one second off, as a re-read `ps lstart` that
/// rounded differently would give.
fn one_second_off(owner: &ProcessIncarnation) -> ProcessIncarnation {
    let start = owner.start_identity();
    let time = start
        .split(' ')
        .find(|word| word.len() == 8 && word.as_bytes()[2] == b':')
        .expect("ps-v1 start identity has a clock time");
    let seconds: u8 = time[6..].parse().unwrap();
    let shifted = format!(
        "{}{:02}",
        &time[..6],
        if seconds < 59 {
            seconds + 1
        } else {
            seconds - 1
        }
    );
    ProcessIncarnation::new(owner.pid(), &start.replacen(time, &shifted, 1)).unwrap()
}

fn assert_refused(digest: &mut DueDigest, turn: &str, token: &str) {
    let output = digest.stop(turn);
    assert!(
        !output.contains("decision"),
        "a refused Digest Stop never continues: {output}"
    );
    let session = digest.session();
    assert_eq!(session["learning"]["detail"], token, "{session:#}");
    assert_ne!(
        session["unbound"], true,
        "a Digest Stop reason never marks the launch unbound: {session:#}"
    );
}

#[test]
fn a_digest_stop_whose_owner_start_is_one_second_off_names_the_owner_check() {
    let owner = one_second_off(&live_owner());
    let mut digest = DueDigest::start("digest-refusal-owner", owner, true, false);
    assert_refused(&mut digest, "a", "digest_stop:launch:owner");
    let learning = &digest.session()["learning"];
    assert_eq!(learning["step"], "provider_verification");
    assert_eq!(learning["reason"], "worker_error");
}

#[test]
fn a_digest_stop_for_an_ended_binding_names_the_binding_check() {
    let mut digest = DueDigest::start("digest-refusal-ended", live_owner(), true, false);
    digest.set_binding(|state| state.state = tmt_core::binding::session::RuntimeState::Ended);
    assert_refused(&mut digest, "a", "digest_stop:launch:binding");
}

#[test]
fn a_failed_capture_is_named_until_a_digest_stop_delivers() {
    let mut digest = DueDigest::start("digest-refusal-capture", live_owner(), true, true);
    let db = rusqlite::Connection::open(digest.fixture.state().join("tmt.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_final BEFORE INSERT ON recent_responses BEGIN SELECT RAISE(ABORT, 'capture fixture failure'); END;").unwrap();
    ok(digest
        .codex
        .hook(codex_event("UserPromptSubmit", Some("a"))));
    assert_refused(&mut digest, "a", "digest_stop:capture_failed");
    assert_eq!(
        digest.session()["learning"]["step"],
        "storage",
        "the capture is a storage step"
    );
    // Other hook runs on the admitted launch keep the reason.
    ok(digest
        .codex
        .hook(codex_event("UserPromptSubmit", Some("b"))));
    assert_eq!(
        digest.session()["learning"]["detail"],
        "digest_stop:capture_failed"
    );
    db.execute_batch("DROP TRIGGER refuse_final;").unwrap();
    let delivered = digest.stop("b");
    assert!(delivered.contains("decision"), "{delivered}");
    let session = digest.session();
    assert_eq!(session["learning"], Value::Null, "{session:#}");
}

#[test]
fn a_digest_stop_with_nothing_due_names_nothing() {
    let mut digest = DueDigest::start("digest-refusal-nothing-due", live_owner(), false, false);
    let output = digest.stop("a");
    assert!(!output.contains("decision"), "{output}");
    let session = digest.session();
    assert_eq!(session["learning"], Value::Null, "{session:#}");
}
