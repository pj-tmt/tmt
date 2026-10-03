use super::*;
use crate::{
    drivers::{DriverEntry, Registry},
    skill_installation::ProviderEnvironment,
    test_support::TestDirectory,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
use tmt_core::binding::session::{
    BindingSessionState, ProviderSessionId, RememberedSession, RuntimeLiveness, RuntimeMode,
    RuntimeState, SessionPreferences,
};
use tmt_core::endpoint::ProcessIncarnation;

const SCRIPT: &str = "#!/bin/sh\nif [ \"$3\" = resume ]; then cat > \"$0.request\"; fi\necho \"$3\" >> \"$0.calls\"\ncat \"$(dirname \"$0\")/$3\"\n";
struct Fixture(TestDirectory);
impl Fixture {
    fn new(claims: bool) -> Self {
        let this = Self(TestDirectory::new());
        let caps = json!({"ok": {"protocols": [1], "kind": "runtime", "name": "custom", "version": "0.1.0",
            "ops": ["locations", "resume"], "executables": ["agent", "agent-alt"], "claims": claims, "env": ["AGENT_HOME"],
            "hooks": {"format": "sessionHooksJson", "fields": {"event": "/event", "session": "/session", "model": "/model", "transcript": "/transcript", "turn": "/turn"},
            "events": [{"name": "Start", "effect": "start", "by": "/source", "values": {"startup": "started", "clear": "cleared"}},
                {"name": "End", "effect": "end", "by": "/reason", "values": {"other": "ended", "clear": "cleared"}},
                {"name": "Prompt", "effect": "working"}, {"name": "Stop", "effect": "idle"}]}}});
        this.answer("capabilities", caps);
        this.answer("locations", json!({"ok": {"configDirs": [this.0.path], "skills": this.0.path.join("skills"), "hookSettings": this.0.path.join("settings.json")}}));
        this.answer(
            "resume",
            json!({"ok": {"argv": ["agent-alt", "--resume", "opaque-session"]}}),
        );
        this.publish(SCRIPT);
        this.approve();
        this
    }
    fn executable(&self) -> std::path::PathBuf {
        self.0.path.join("driver")
    }
    fn answer(&self, op: &str, value: Value) {
        fs::write(self.0.path.join(op), value.to_string()).unwrap();
    }
    fn publish(&self, script: &str) {
        // A separate writer publishes a closed inode, avoiding inherited exec-busy descriptors.
        let mut writer = Command::new("/bin/sh")
            .args([
                "-c",
                "cat > \"$1.new\" && chmod 755 \"$1.new\" && mv \"$1.new\" \"$1\"",
                "writer",
            ])
            .arg(self.executable())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(writer.wait().unwrap().success());
    }
    fn approve(&self) {
        registry::approve(&self.0.path, &self.executable(), &UnixCommandRunner).unwrap();
    }
    fn registry(&self) -> Registry {
        Registry::builtin()
            .with_approved(&self.0.path, Path::new("/unused/tmt"))
            .unwrap()
    }
    fn session(&self) -> RememberedSession {
        RememberedSession {
            harness: HarnessId::new("custom").unwrap(),
            provider_session: ProviderSessionId::new("opaque-session").unwrap(),
            mode: RuntimeMode::new("default").unwrap(),
            state: driver_state::after_start(Some("reported-model"), None, None),
            stale_at_ms: None,
            resume_pending_at_ms: None,
        }
    }
}

#[test]
fn approved_seam_does_not_change_production_visibility_or_send_budget() {
    let fixture = Fixture::new(true);
    let drivers = fixture.registry();
    assert_eq!(drivers.entries().last().unwrap().name(), "custom");
    // These are the constructors still used by production run/hooks/setup in B1.
    assert!(
        Registry::builtin()
            .entries()
            .all(|entry| entry.name() != "custom")
    );
    assert!(Registry::builtin().find("custom").is_none());
    assert!(RuntimeRegistry::default().claim("agent".as_ref()).is_none());
    assert!(
        RuntimeRegistry::first_party()
            .claim("agent".as_ref())
            .is_none()
    );
    assert!(
        drivers.find("custom").is_none(),
        "builtin projection remains unchanged"
    );
    let runtime = RuntimeRegistry::from_drivers(&drivers);
    assert_eq!(
        runtime.claim("/bin/agent-alt".as_ref()),
        Some(fixture.session().harness)
    );
    assert_eq!(
        runtime.maximum_send_duration(),
        RuntimeRegistry::first_party().maximum_send_duration()
    );
    let DriverEntry::Approved(approved) = drivers.entries().last().unwrap() else {
        panic!("approved entry")
    };
    assert_eq!(
        ExternalRuntime(approved.clone()).maximum_send_duration(),
        Duration::ZERO
    );
    let mut only_external = RuntimeRegistry::default();
    approved.register(&mut only_external).unwrap();
    assert_eq!(
        only_external.maximum_send_duration(),
        Duration::ZERO,
        "the trait's 30-second default must not apply"
    );
    assert!(only_external.channel(&fixture.session().harness).is_none());
    assert_eq!(
        drivers.entries().last().unwrap().hue(),
        crate::drivers::Hue::Neutral
    );
    assert_eq!(
        drivers.entries().last().unwrap().executables(),
        vec!["agent", "agent-alt"]
    );
    let environment = ProviderEnvironment::from_parts(&fixture.0.path, &fixture.0.path, vec![], []);
    assert_eq!(
        drivers
            .entries()
            .last()
            .unwrap()
            .locations(&environment)
            .unwrap()
            .skills,
        fixture.0.path.join("skills")
    );
}

#[test]
fn resume_validates_argv_and_passes_only_reported_model_without_changing_session() {
    let fixture = Fixture::new(false);
    let mut runtime = RuntimeRegistry::from_drivers(&fixture.registry());
    assert!(runtime.claim("agent".as_ref()).is_none());
    let session = fixture.session();
    assert_eq!(
        runtime.resume(&session),
        ActionResult::Completed(RuntimeCommand {
            executable: "agent-alt".into(),
            args: vec!["--resume".into(), "opaque-session".into()]
        })
    );
    let request: Value =
        serde_json::from_slice(&fs::read(fixture.executable().with_extension("request")).unwrap())
            .unwrap();
    assert_eq!(request["session"], "opaque-session");
    assert_eq!(request["model"], "reported-model");
    assert!(request["deadlineMs"].as_u64().unwrap() > 0);
    fixture.answer("resume", json!({"ok": {"argv": ["sh", "-c", "unsafe"]}}));
    assert_eq!(
        runtime.resume(&session),
        ActionResult::Failed(RuntimeError::DriverRefused)
    );
    assert_eq!(session, fixture.session());
}

#[test]
fn changed_and_missing_approvals_keep_sessions_until_withdrawn() {
    let fixture = Fixture::new(true);
    let mut preferences = SessionPreferences {
        preferred_harness: Some(fixture.session().harness.clone()),
        remembered: Some(fixture.session()),
    };
    let before = preferences.clone();
    fixture.publish(&format!("{SCRIPT}\n# changed\n"));
    let mut changed = RuntimeRegistry::from_drivers(&fixture.registry());
    assert!(changed.unavailable(&fixture.session().harness));
    assert!(changed.claim("agent".as_ref()).is_none());
    assert!(changed.lifecycle(&fixture.session().harness).is_none());
    assert_eq!(changed.reconcile(&mut preferences), None);
    assert_eq!(preferences, before);
    assert_eq!(
        changed.resume(preferences.remembered.as_ref().unwrap()),
        ActionResult::Failed(RuntimeError::DriverUnavailable)
    );
    fs::remove_file(fixture.executable()).unwrap();
    let missing = RuntimeRegistry::from_drivers(&fixture.registry());
    assert!(missing.unavailable(&fixture.session().harness));
    assert_eq!(missing.reconcile(&mut preferences), None);
    assert_eq!(preferences, before);
    fixture.publish(SCRIPT);
    fixture.approve();
    let mut restored = RuntimeRegistry::from_drivers(&fixture.registry());
    assert!(!restored.unavailable(&fixture.session().harness));
    assert!(matches!(
        restored.resume(preferences.remembered.as_ref().unwrap()),
        ActionResult::Completed(_)
    ));
    registry::remove(&fixture.0.path, "custom").unwrap();
    let removed = RuntimeRegistry::from_drivers(&fixture.registry());
    assert!(removed.reconcile(&mut preferences).is_some());
    assert!(preferences.remembered.is_none());
}

#[test]
fn declarative_hooks_share_session_policy_and_never_spawn_a_driver() {
    let fixture = Fixture::new(true);
    let runtime = RuntimeRegistry::from_drivers(&fixture.registry());
    let lifecycle = runtime.lifecycle(&fixture.session().harness).unwrap();
    fs::write(fixture.executable().with_extension("calls"), "").unwrap();
    let payload = |event: &str, session: &str, extra: Value| {
        let mut value = json!({"event": event, "session": session});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::to_vec(&value).unwrap()
    };
    let process = ProcessIncarnation::new(42, "start").unwrap();
    let host = lifecycle.host_evidence().unwrap();
    let start = lifecycle
        .decode(&payload(
            "Start",
            "one",
            json!({"source": "startup", "model": "m1"}),
        ))
        .unwrap();
    assert!(start.starting());
    let state = start.driver_state(None, 1).unwrap();
    assert_eq!(lifecycle.state_model(&state).as_deref(), Some("m1"));
    let current = start
        .propose(
            &BindingSessionState::default(),
            &process,
            RuntimeLiveness::Unknown,
            host,
            false,
        )
        .unwrap();
    let clear_start = lifecycle
        .decode(&payload("Start", "two", json!({"source": "clear"})))
        .unwrap();
    assert!(
        clear_start
            .propose(&current, &process, RuntimeLiveness::Alive, host, false)
            .is_none()
    );
    let end = lifecycle
        .decode(&payload("End", "one", json!({"reason": "clear"})))
        .unwrap();
    let switching = end
        .propose(&current, &process, RuntimeLiveness::Alive, host, false)
        .unwrap();
    assert_eq!(switching.state, RuntimeState::Unknown);
    let next = clear_start
        .propose(&switching, &process, RuntimeLiveness::Alive, host, false)
        .unwrap();
    assert_eq!(next.key.unwrap().provider_session.unwrap().as_str(), "two");
    let working = payload("Prompt", "two", json!({"turn": "turn-2"}));
    assert_eq!(lifecycle.decode_prompt(&working).unwrap().as_str(), "two");
    assert_eq!(
        lifecycle
            .decode_activity(&working)
            .unwrap()
            .turn
            .unwrap()
            .as_str(),
        "turn-2"
    );
    let stop = payload("Stop", "two", json!({"transcript": "/tmp/transcript"}));
    assert!(lifecycle.decode_turn(&stop).is_some());
    assert!(lifecycle.decode(b"{bad}").is_none());
    assert!(
        lifecycle
            .decode(&payload("Future", "two", json!({})))
            .is_none()
    );
    assert_eq!(
        fs::read_to_string(fixture.executable().with_extension("calls")).unwrap(),
        ""
    );
}
