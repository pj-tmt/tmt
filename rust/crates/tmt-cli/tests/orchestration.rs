//! Real CLI/API parity and read preservation in an owned world, without tmux.
#![cfg(unix)]
mod support;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tmt_adapters::storage::Storage;
use tmt_core::identity::{Lifetime, create_or_resolve};
const NOW: u64 = 1_700_000_000_000;
const PLAN: &str = "11111111-1111-4111-8111-111111111111";
const SHIFT: &str = "22222222-2222-4222-8222-222222222222";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-orchestration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn db(&self) -> PathBuf {
        support::state_dir(&self.root).join("tmt.db")
    }
    fn run(&mut self, args: &[&str], input: Option<Value>) -> Output {
        let mut command = support::command(&self.root, args);
        let stdin = if let Some(input) = input {
            let path = self.root.join("input.json");
            fs::write(&path, input.to_string()).unwrap();
            Stdio::from(fs::File::open(path).unwrap())
        } else {
            Stdio::null()
        };
        self.child = Some(
            command
                .env("TZ", "Asia/Tokyo")
                .stdin(stdin)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let output = support::wait(&mut self.child, Duration::from_secs(15))
            .wait_with_output()
            .unwrap();
        assert!(output.stdout.len() < 131_072);
        output
    }
    fn json(&mut self, args: &[&str], input: Option<Value>) -> Value {
        let o = self.run(args, input);
        assert!(
            o.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(o.stderr.is_empty());
        serde_json::from_slice(&o.stdout).unwrap()
    }
    fn seed(&self) -> String {
        let mut storage = Storage::open(self.db()).unwrap();
        let id = create_or_resolve(&mut storage, "Night-1", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        storage.close().unwrap();
        let db = Connection::open(self.db()).unwrap();
        db.execute(
            "INSERT INTO orchestration_settings VALUES(1,4,'Asia/Tokyo')",
            [],
        )
        .unwrap();
        for (id, name, priority, fields) in [
            (PLAN, "Quiet", 1, r#"{"state":"sleep","sending":"leads"}"#),
            (SHIFT, "Shift", 2, r#"{"state":"awake"}"#),
        ] {
            db.execute("INSERT INTO orchestration_plans(id,name,revision,priority,enabled,schedule,fields) VALUES(?1,?2,1,?3,1,'{\"kind\":\"always\"}',?4)",params![id,name,priority,fields]).unwrap();
            db.execute(
                "INSERT INTO orchestration_plan_targets VALUES(?1,0,'{\"kind\":\"all\"}')",
                [id],
            )
            .unwrap();
        }
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn cursor(path: &std::path::Path) -> i64 {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap()
}
#[test]
fn help_and_capabilities_create_no_database_then_empty_read_explains_base() {
    let mut f = Fixture::new();
    let o = f.run(&["orch", "--help"], None);
    assert!(o.status.success());
    assert!(String::from_utf8(o.stdout).unwrap().contains("preview"));
    assert!(!f.db().exists());
    let c = f.json(
        &["api"],
        Some(json!({"version":1,"operation":"capabilities","input":{}})),
    );
    assert!(c.to_string().contains("orchestration.read"));
    assert!(!f.db().exists());
    let o = f.run(&["orch"], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(text.contains("Base only"));
    assert!(text.contains("slice 2"));
    let empty = f.json(&["orch", "--json"], None);
    assert_eq!(empty["configurationRevision"], 0);
    assert!(empty["plans"].as_array().unwrap().is_empty());
    assert!(empty["members"].as_array().unwrap().is_empty());
}
#[test]
fn real_seeded_cli_and_api_share_effective_sources_and_reads_preserve_cursor() {
    let mut f = Fixture::new();
    let id = f.seed();
    let before = cursor(&f.db());
    let mut cli = f.json(
        &[
            "orch",
            "--at",
            "2023-11-14T22:13:20Z",
            "--member",
            "NIGHT",
            "--json",
        ],
        None,
    );
    let mut api=f.json(&["api"],Some(json!({"version":1,"operation":"orchestration.read","input":{"view":"overview","atMs":NOW,"memberQuery":"NIGHT"}})));
    assert!(cli["observedAtMs"].as_u64().unwrap() > NOW);
    assert!(api["observedAtMs"].as_u64().unwrap() > NOW);
    cli.as_object_mut().unwrap().remove("observedAtMs");
    api.as_object_mut().unwrap().remove("observedAtMs");
    assert_eq!(cli, api);
    let m = &cli["members"][0];
    assert_eq!(m["identityId"], id);
    assert_eq!(m["effective"]["state"]["value"], "awake");
    assert_eq!(m["effective"]["receiving"]["value"]["mode"], "direct");
    assert_eq!(m["effective"]["routine"]["value"], "run");
    assert_eq!(m["effective"]["sending"]["value"], "leads");
    assert_eq!(m["effective"]["sending"]["source"]["planId"], PLAN);
    assert_eq!(m["effective"]["state"]["source"]["planId"], SHIFT);
    assert!(m["jobs"].is_null());
    let why = f.json(&["orch", "why", &id, "--json"], None);
    assert!(why["items"].as_array().unwrap().is_empty());
    assert_eq!(why["coverage"]["liveProducers"], false);
    assert_eq!(cursor(&f.db()), before);
}
#[test]
fn preview_is_in_memory_and_bad_candidate_or_future_schema_preserves_state() {
    let mut f = Fixture::new();
    f.seed();
    let before = cursor(&f.db());
    let path = f.root.join("plan.json");
    let candidate = json!({"id":SHIFT,"name":"Shift","revision":2,"priority":2,"enabled":true,"targets":[{"kind":"all"}],"schedule":{"kind":"always"},"fields":{"state":"focus"}});
    fs::write(&path, candidate.to_string()).unwrap();
    let p = f.json(
        &[
            "orch",
            "preview",
            "--file",
            path.to_str().unwrap(),
            "--at",
            "2023-11-14T22:13:20Z",
            "--json",
        ],
        None,
    );
    assert_eq!(
        p["before"]["members"][0]["effective"]["state"]["value"],
        "awake"
    );
    assert_eq!(
        p["after"]["members"][0]["effective"]["receiving"]["value"]["mode"],
        "urgent"
    );
    assert!(!p["changes"].as_array().unwrap().is_empty());
    assert_eq!(cursor(&f.db()), before);
    fs::write(&path, r#"{"id":"duplicate","id":"duplicate"}"#).unwrap();
    let bad = f.run(
        &[
            "orch",
            "preview",
            "--file",
            path.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    assert!(!bad.status.success());
    assert_eq!(cursor(&f.db()), before);
    let db = Connection::open(f.db()).unwrap();
    db.execute(
        "INSERT INTO _migrations(version,name,applied_at) VALUES(74,'future','fixture')",
        [],
    )
    .unwrap();
    drop(db);
    let bad = f.run(&["orch", "--json"], None);
    assert!(!bad.status.success());
    assert!(bad.stderr.is_empty());
    assert_eq!(cursor(&f.db()), before);
}
#[test]
fn why_reads_actual_transactional_events_and_preserves_uncertainty() {
    use tmt_core::orchestration::*;
    let mut f = Fixture::new();
    let id = f.seed();
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let mut storage = Storage::open(f.db()).unwrap();
    for (action, outcome) in [
        (OrchDecisionAction::Block, OrchDecisionOutcome::Held),
        (OrchDecisionAction::Delay, OrchDecisionOutcome::Held),
        (OrchDecisionAction::Send, OrchDecisionOutcome::Uncertain),
        (OrchDecisionAction::JobRun, OrchDecisionOutcome::Completed),
    ] {
        storage
            .append_orchestration_decision(&OrchDecision {
                recorded_at_ms: now,
                subject_kind: OrchSubjectKind::Identity,
                subject_id: id.clone(),
                actor_id: None,
                recipient_id: Some(id.clone()),
                action,
                outcome,
                reason: "fixture_transition".into(),
                configuration_revision: 4,
                time_zone: "Asia/Tokyo".into(),
                sources: vec![],
            })
            .unwrap();
    }
    storage.close().unwrap();
    let before = cursor(&f.db());
    let p = f.json(&["orch", "why", &id, "--limit", "2", "--json"], None);
    assert_eq!(p["items"][0]["action"], "jobRun");
    assert_eq!(p["items"][1]["outcome"], "uncertain");
    let cursor_value = p["nextBefore"].as_u64().unwrap().to_string();
    let p = f.json(
        &["orch", "why", &id, "--before", &cursor_value, "--json"],
        None,
    );
    assert_eq!(p["items"].as_array().unwrap().len(), 2);
    assert_eq!(p["items"][0]["action"], "delay");
    assert_eq!(p["items"][1]["action"], "block");
    assert_eq!(cursor(&f.db()), before);
}
#[test]
fn plan_filters_keep_competing_effects_and_uuid_selection_never_falls_back_to_names() {
    let mut f = Fixture::new();
    f.seed();
    let selected = f.json(&["orch", "--plan", "Quiet", "--json"], None);
    assert_eq!(selected["selectedPlanId"], PLAN);
    assert_eq!(
        selected["members"][0]["effective"]["state"]["value"],
        "awake"
    );
    assert_eq!(selected["timeline"][0]["planIds"], json!([PLAN]));
    let db = Connection::open(f.db()).unwrap();
    db.execute("INSERT INTO orchestration_plans(id,name,revision,priority,enabled,schedule,fields) VALUES('44444444-4444-4444-8444-444444444444',?1,1,3,1,'{\"kind\":\"always\"}','{}')",[SHIFT]).unwrap();
    db.execute("INSERT INTO orchestration_plan_targets VALUES('44444444-4444-4444-8444-444444444444',0,'{\"kind\":\"all\"}')",[]).unwrap();
    drop(db);
    let p = f.json(&["orch", "plan", "show", SHIFT, "--json"], None);
    assert_eq!(p["plans"].as_array().unwrap().len(), 1);
    assert_eq!(p["plans"][0]["name"], "Shift");
    let missing = f.run(
        &[
            "orch",
            "plan",
            "show",
            "55555555-5555-4555-8555-555555555555",
            "--json",
        ],
        None,
    );
    assert_eq!(missing.status.code(), Some(3));
    assert!(missing.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&missing.stdout).unwrap()["error"]["code"],
        "ORCH_PLAN_NOT_FOUND"
    );
}
