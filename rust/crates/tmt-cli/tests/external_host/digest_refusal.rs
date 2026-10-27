//! A Digest Stop that refuses still prints no continuation, and names why on its launch's
//! session-learning row (#4871). Real Stop stdin through the public hook in an owned
//! external-host world.
use super::*;
use tmt_adapters::runtime::hook_protocol::HookLaunch;
use tmt_core::{binding::BindingRepository, endpoint::ProcessIncarnation};

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
        let launch = record_admitted_launch(&fixture, &agent, "codex", &owner);
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
    let queue = rusqlite::Connection::open(digest.fixture.state().join("hook-writes.db")).unwrap();
    queue.execute_batch("CREATE TRIGGER refuse_capture BEFORE INSERT ON hook_writes BEGIN SELECT RAISE(ABORT, 'queue append fixture failure'); END;").unwrap();
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
    queue.execute_batch("DROP TRIGGER refuse_capture;").unwrap();
    let delivered = digest.stop("b");
    assert!(delivered.contains("decision"), "{delivered}");
    let session = digest.session();
    assert_eq!(session["learning"], Value::Null, "{session:#}");
}

#[test]
fn a_malformed_queued_capture_is_quarantined_and_does_not_block_digest() {
    use tmt_adapters::storage::hook_queue::HookQueue;
    let mut digest = DueDigest::start("digest-invalid-queue", live_owner(), true, false);
    let queue = HookQueue::open(&digest.fixture.state().join("tmt.db"), Duration::ZERO).unwrap();
    let _guard = queue.claim().unwrap();
    let db = rusqlite::Connection::open(digest.fixture.state().join("hook-writes.db")).unwrap();
    db.execute("INSERT INTO hook_writes(event_id,identity_id,kind,payload,enqueued_ms) VALUES('bad',?,'capture','{}',0)",[&digest.agent]).unwrap();
    let delivered = digest.stop("a");
    assert!(
        delivered.contains("decision"),
        "Digest should deliver after quarantine: {delivered}"
    );
    let receipt: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM invalid_hook_writes WHERE event_id='bad')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        receipt,
        "the malformed row leaves a durable failure receipt"
    );
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_writes WHERE event_id='bad')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn a_digest_stop_with_nothing_due_names_nothing() {
    let mut digest = DueDigest::start("digest-refusal-nothing-due", live_owner(), false, false);
    let output = digest.stop("a");
    assert!(!output.contains("decision"), "{output}");
    let session = digest.session();
    assert_eq!(session["learning"], Value::Null, "{session:#}");
}

impl DueDigest {
    /// Configuration is fixture setup; the actual held request still enters through public talk.
    fn hold_summary(&self) -> String {
        use tmt_adapters::orchestration::{DigestReceiving, DigestReceivingSettings};
        let mut storage = Storage::open(self.fixture.state().join("tmt.db")).unwrap();
        storage
            .sync_digest_receiving(
                0,
                DigestReceivingSettings {
                    enabled: true,
                    default_receiving: DigestReceiving::Auto {},
                    members: vec![],
                },
            )
            .unwrap();
        storage.close().unwrap();
        let db = rusqlite::Connection::open(self.fixture.state().join("tmt.db")).unwrap();
        db.execute("INSERT INTO orchestration_settings VALUES(1,1,'UTC')", [])
            .unwrap();
        let plan = tmt_core::operation::new_operation_id();
        db.execute("INSERT INTO orchestration_plans VALUES(?1,'Quiet',1,1,1,'{\"kind\":\"always\"}','{\"state\":\"sleep\"}')",[&plan]).unwrap();
        db.execute(
            "INSERT INTO orchestration_plan_targets VALUES(?1,0,'{\"kind\":\"all\"}')",
            [&plan],
        )
        .unwrap();
        let now = tmt_adapters::request_runtime::wall_time_ms();
        let policy = passthrough::api(
            &self.fixture,
            "ok",
            "digest.policy.set",
            serde_json::json!({"identityId":self.agent,"ownerIdentityId":self.agent,"setterIdentityId":self.agent,"expectedRevision":0,"untilMs":now+120000}),
        );
        assert_eq!(policy["active"], true, "{policy:#}");
        let held = self.fixture.json(&[
            "talk",
            "worker",
            "secret body belongs only in inbox",
            "--identity",
            "core-1",
            "--detach",
            "--json",
        ]);
        assert_eq!(held["digest"], true, "{held:#}");
        held["requestId"].as_str().unwrap().into()
    }
    fn release_summary(&self) {
        rusqlite::Connection::open(self.fixture.state().join("tmt.db"))
            .unwrap()
            .execute(
                "UPDATE orchestration_plans SET fields='{\"state\":\"awake\"}',revision=2",
                [],
            )
            .unwrap();
    }
    fn assert_terminal_summary(&self, request: &str) {
        let db = rusqlite::Connection::open(self.fixture.state().join("tmt.db")).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM orchestration_pending_wakes",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM capture_entries WHERE checklist_id IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let evidence:(String,String,i64,i64,bool)=db.query_row("SELECT state,projection,sealed_count,summary_urgent_count,confirmed_at_ms IS NOT NULL FROM focus_checklists",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            evidence,
            ("delivered".into(), "incoming_summary".into(), 1, 0, true)
        );
        assert!(
            db.query_row(
                "SELECT response_submitted_at_ms IS NULL FROM request_attempts WHERE request_id=?",
                [request],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
    }
}
#[test]
fn a_released_summary_uses_the_public_stop_handoff_without_bodies_capture_or_reoffer() {
    let mut digest = DueDigest::start("digest-summary-stop", live_owner(), false, false);
    let request = digest.hold_summary();
    digest.release_summary();
    let output = digest.stop("summary");
    let handoff: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(handoff["decision"], "block", "{output}");
    assert_eq!(
        handoff["reason"], "you have 1 messages, 0 urgent: tmt inbox",
        "{output}"
    );
    digest.assert_terminal_summary(&request);
    let again = digest.stop("summary-finished");
    assert!(
        !again.contains("decision"),
        "terminal summary must never reoffer: {again}"
    );
}
#[test]
fn the_public_wake_step_keeps_sleep_pending_then_uses_the_existing_host_send() {
    let mut digest = DueDigest::start("digest-summary-host", live_owner(), false, false);
    let request = digest.hold_summary();
    ok(digest
        .codex
        .hook(codex_event("UserPromptSubmit", Some("ready"))));
    ok(digest.codex.hook(codex_event("Stop", Some("ready"))));
    let blocked = passthrough::api(
        &digest.fixture,
        "ok",
        "digest.wake.step",
        serde_json::json!({"budgetMs":1000,"limit":1}),
    );
    assert_eq!(blocked["state"], "unavailable", "{blocked:#}");
    digest.release_summary();
    // Complete the prior bounded UUID pass, then the next step sees the released seat.
    let mut result = Value::Null;
    for _ in 0..2 {
        result = passthrough::api(
            &digest.fixture,
            "ok",
            "digest.wake.step",
            serde_json::json!({"budgetMs":5000,"limit":1}),
        );
        if result["state"] == "delivered" {
            break;
        }
    }
    assert_eq!(result["schemaVersion"], 1);
    assert_eq!(result["state"], "delivered", "{result:#}");
    assert_eq!(result["identityId"], digest.agent);
    assert_eq!(result["incomingCount"], 1);
    assert_eq!(result["urgentCount"], 0);
    let prompted = digest.fixture.requests("prompted");
    assert_eq!(prompted.len(), 1, "{prompted:#?}");
    assert_eq!(
        prompted[0]["text"],
        "you have 1 messages, 0 urgent: tmt inbox"
    );
    digest.assert_terminal_summary(&request);
}

#[test]
fn a_uuid_pinned_flush_defers_a_busy_summary_even_with_when_busy_then_delivers_at_idle() {
    let mut digest = DueDigest::start("digest-summary-pinned", live_owner(), false, false);
    let request = digest.hold_summary();
    digest.release_summary();
    ok(digest
        .codex
        .hook(codex_event("UserPromptSubmit", Some("busy"))));
    let busy = passthrough::api(
        &digest.fixture,
        "ok",
        "digest.checklist.flush",
        serde_json::json!({"identityId":digest.agent,"whenBusy":true}),
    );
    assert_eq!(busy["state"], "not_idle", "{busy:#}");
    let db = rusqlite::Connection::open(digest.fixture.state().join("tmt.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM orchestration_wake_clock", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0,
        "busy input consumes no admission"
    );
    ok(digest.codex.hook(codex_event("Stop", Some("busy"))));
    let idle = passthrough::api(
        &digest.fixture,
        "ok",
        "digest.checklist.flush",
        serde_json::json!({"identityId":digest.agent,"whenBusy":true}),
    );
    assert_eq!(idle["state"], "delivered", "{idle:#}");
    assert_eq!(idle["identityId"], digest.agent);
    let prompted = digest.fixture.requests("prompted");
    assert_eq!(prompted.len(), 1);
    assert_eq!(
        prompted[0]["text"],
        "you have 1 messages, 0 urgent: tmt inbox"
    );
    digest.assert_terminal_summary(&request);
}
