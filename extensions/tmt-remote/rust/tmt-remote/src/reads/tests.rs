use super::*;
use crate::core::tests::executable_fixture;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fake public `tmt`: it logs each argv line and prints `<argv joined by _>.out`, exiting with
/// `<argv joined by _>.exit` when that file exists. Tests write those files, never the script.
struct Fake(PathBuf);
impl Fake {
    fn new() -> Self {
        let root = PathBuf::from(format!(
            "/tmp/tmt-4325-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        executable_fixture::write_executable(
            &root.join("tmt"),
            &format!(
                "cd '{0}'\nkey=$(printf '%s_' \"$@\")\necho \"$*\" >> calls\n\
                 [ -f \"$key.sleep\" ] && /bin/sleep \"$(cat \"$key.sleep\")\"\n\
                 [ -f \"$key.out\" ] && cat \"$key.out\"\n\
                 [ -f \"$key.exit\" ] && exit \"$(cat \"$key.exit\")\"\nexit 0\n",
                root.display()
            ),
        )
        .unwrap();
        Self(root)
    }
    fn reads(&self) -> Reads {
        Reads::new(self.0.join("tmt"), Arc::new(AtomicBool::new(false)))
    }
    fn answer(&self, argv: &str, output: &str) {
        let key: String = argv.split(' ').map(|word| format!("{word}_")).collect();
        fs::write(self.0.join(format!("{key}.out")), output).unwrap();
    }
    fn set(&self, argv: &str, extension: &str, value: &str) {
        let key: String = argv.split(' ').map(|word| format!("{word}_")).collect();
        fs::write(self.0.join(format!("{key}.{extension}")), value).unwrap();
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.0.join("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

const LS: &str = "ops squad ls --json";
const CRON: &str = "ops squad cron ls --json";
const DIGEST: &str = "digest usage report --since 24h --json";

fn body(reads: &Reads, target: Target) -> Value {
    serde_json::from_slice(&reads.body(target)).unwrap()
}

/// One squad as `tmt ops squad ls --json` prints it, including everything the projection
/// must not forward: the pane address and cwd, room IDs, unlisted fields and a message path.
fn board() -> Value {
    let row = |id: &str, name: &str, task: &str| {
        json!({
            "id": id, "name": name, "lifetime": "saved", "presence": "active",
            "pane": {"id": "%10", "target": "tmt-core:2.1", "cwd": "/Users/secret/work",
                     "location": {"host": "tmux", "session": "tmt-core", "window": "2"}},
            "activity": {"state": "working", "sinceMs": 1700000000000u64, "lastActivityMs": 5},
            "state": "working", "pending": "approve the plan", "failed": [],
            "fields": {"task": task, "next": "then merge", "model": "sol",
                       "link": "https://github.com/pj-tmt/tmt/issues/4325",
                       "secret": "SECRET-FIELD", "cwd": "/Users/secret/work"},
            "staleness": {"state": "fresh", "reasons": ["x"]},
            "colors": {"state": "working"}, "annotation": "ANNOTATION",
            "waitingOnYou": [{"id": "r1"}, {"id": "r2"}],
            "digest": {"active": true, "digestUntilMs": 9, "remainingMs": 1000, "heldCount": 2},
        })
    };
    json!({
        "squads": [{
            "squad": {
                "name": "tmt-core", "roomId": "ROOM-SECRET", "layout": "team",
                "lead": row("00000000-0000-4000-8000-000000000001", "tmt-lead", "lead task"),
                "idleAlert": {"enabled": true, "afterMs": 1800000},
                "attention": {"state": "normal", "waiting": 1, "blocked": 0},
            },
            "columns": [{"field": "member"}], "lines": [[{"field": "member"}]],
            "sections": [{"title": "Members", "rows": [
                row("00000000-0000-4000-8000-000000000002", "core-3", "plan\u{202e}\nnow"),
            ]}],
        }],
        "you": {"id": "YOU-SECRET", "name": "ben", "source": "recorded"},
    })
}
fn cron() -> Value {
    json!({"action": "ls", "jobs": [{
        "id": "c1", "squad": "tmt-core", "roomId": "ROOM-SECRET", "ownerId": "OWNER-SECRET",
        "message": "Hourly pass\nfollow /Users/secret/runbook.md SECOND-SECRET",
        "schedule": {"kind": "cron", "zone": "Asia/Tokyo", "expression": "35 * * * *"},
        "state": "on", "owner": "tmt-core-pm", "scheduleText": "cron 35 * * * *",
        "nextMs": [1792636500000u64, 1792640100000u64], "warnings": [],
    }]})
}

#[test]
fn routes_accept_exactly_the_named_reads_and_no_query_or_other_path() {
    assert_eq!(Target::route("ops"), Some(Target::Ops));
    for (segment, range) in [
        ("24h", Range::Day),
        ("7d", Range::Week),
        ("30d", Range::Month),
    ] {
        assert_eq!(
            Target::route(&format!("digest/{segment}")),
            Some(Target::Digest(range))
        );
    }
    for unrouted in [
        "",
        "shell",
        "limits",
        "ops/",
        "ops/x",
        "opsx",
        "OPS",
        "-/ops",
        "digest",
        "digest/",
        "digest/1h",
        "digest/24H",
        "digest/7d/",
        "digest/7d/x",
        "digest//7d",
        "digest?since=7d",
        "digest/7d?x=1",
        "ops?x=1",
    ] {
        assert_eq!(Target::route(unrouted), None, "{unrouted}");
    }
}

#[test]
fn the_ops_board_is_a_projection_that_forwards_no_pane_cwd_room_or_unlisted_field() {
    let fake = Fake::new();
    fake.answer(LS, &board().to_string());
    fake.answer(CRON, &cron().to_string());
    let reads = fake.reads();
    let wire = reads.body(Target::Ops);
    let text = String::from_utf8(wire.to_vec()).unwrap();
    for secret in [
        "SECRET",
        "/Users/",
        "%10",
        "tmt-core:2.1",
        "tmux",
        "lastActivityMs",
        "ANNOTATION",
        "roomId",
        "ROOM",
        "OWNER",
        "YOU-",
        "runbook",
        "follow",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["version"], 1);
    assert_eq!(document["state"], "ok");
    assert!(document["observedAtMs"].as_u64().unwrap() > 0);
    assert_eq!(document["truncated"], false);
    assert_eq!(
        document["squads"],
        json!([{"name": "tmt-core", "layout": "team",
                "lead": {"id": "00000000-0000-4000-8000-000000000001", "name": "tmt-lead"},
                "idleAlertMs": 1800000,
                "attention": {"state": "normal", "waiting": 1, "blocked": 0}}])
    );
    assert_eq!(document["agents"].as_array().unwrap().len(), 2);
    assert_eq!(
        document["agents"][0],
        json!({"id": "00000000-0000-4000-8000-000000000001", "name": "tmt-lead",
               "squad": "tmt-core", "role": "lead", "presence": "active",
               "activity": {"state": "working", "sinceMs": 1700000000000u64},
               "state": "working", "stateTone": "working", "task": "lead task",
               "next": "then merge", "model": "sol",
               "link": "https://github.com/pj-tmt/tmt/issues/4325",
               "pending": "approve the plan", "waitingOnYou": 2,
               "digest": {"active": true, "remainingMs": 1000, "heldCount": 2},
               "staleness": "fresh"})
    );
    // Control and bidi characters never reach the browser as text.
    assert_eq!(document["agents"][1]["role"], "member");
    assert_eq!(document["agents"][1]["task"], "plan now");
    assert_eq!(
        document["timers"],
        json!([{"id": "c1", "squad": "tmt-core", "owner": "tmt-core-pm", "state": "on",
                "scheduleText": "cron 35 * * * *", "nextMs": 1792636500000u64,
                "what": "Hourly pass"}])
    );
}

#[test]
fn an_unusable_cron_read_leaves_timers_null_and_never_an_empty_list() {
    let fake = Fake::new();
    fake.answer(LS, &board().to_string());
    fake.set(CRON, "exit", "1");
    let document = body(&fake.reads(), Target::Ops);
    assert_eq!(document["state"], "ok");
    assert!(document["timers"].is_null());
    assert_eq!(document["agents"].as_array().unwrap().len(), 2);
}

#[test]
fn text_fields_are_cut_to_their_limits_and_links_must_be_https_without_spaces() {
    let fake = Fake::new();
    let mut ls = board();
    let fields = &mut ls["squads"][0]["sections"][0]["rows"][0]["fields"];
    fields["task"] = json!("字".repeat(500));
    fields["link"] = json!("http://insecure.example/x");
    ls["squads"][0]["sections"][0]["rows"][0]["state"] = json!("x".repeat(500));
    fake.answer(LS, &ls.to_string());
    let agent = &body(&fake.reads(), Target::Ops)["agents"][1];
    assert_eq!(
        agent["task"].as_str().unwrap().chars().count(),
        limits::READ_OPS_TEXT
    );
    assert_eq!(
        agent["state"].as_str().unwrap().chars().count(),
        limits::READ_OPS_STATE
    );
    assert!(agent.get("link").is_none(), "http link forwarded");
    let mut ls = board();
    ls["squads"][0]["sections"][0]["rows"][0]["fields"]["link"] = json!("https://a b");
    let fake = Fake::new();
    fake.answer(LS, &ls.to_string());
    assert!(
        body(&fake.reads(), Target::Ops)["agents"][1]
            .get("link")
            .is_none()
    );
}

#[test]
fn the_board_is_capped_by_squads_agents_and_timers_and_says_so() {
    let fake = Fake::new();
    let one = board()["squads"][0].clone();
    let mut many = Vec::new();
    for index in 0..limits::READ_OPS_SQUADS + 3 {
        let mut squad = one.clone();
        squad["squad"]["name"] = json!(format!("s{index}"));
        many.push(squad);
    }
    fake.answer(LS, &json!({"squads": many}).to_string());
    let jobs: Vec<Value> = (0..limits::READ_OPS_TIMERS + 3)
        .map(|index| {
            let mut job = cron()["jobs"][0].clone();
            job["id"] = json!(format!("c{index}"));
            job
        })
        .collect();
    fake.answer(CRON, &json!({"jobs": jobs}).to_string());
    let document = body(&fake.reads(), Target::Ops);
    assert_eq!(
        document["squads"].as_array().unwrap().len(),
        limits::READ_OPS_SQUADS
    );
    assert_eq!(
        document["timers"].as_array().unwrap().len(),
        limits::READ_OPS_TIMERS
    );
    assert!(document["agents"].as_array().unwrap().len() <= limits::READ_OPS_AGENTS);
    assert_eq!(document["truncated"], true);
}

#[test]
fn one_large_squad_is_cut_at_the_agent_limit() {
    let fake = Fake::new();
    let mut ls = board();
    let row = ls["squads"][0]["sections"][0]["rows"][0].clone();
    let rows: Vec<Value> = (0..limits::READ_OPS_AGENTS + 20)
        .map(|index| {
            let mut row = row.clone();
            row["id"] = json!(format!("00000000-0000-4000-8000-{index:012}"));
            row["name"] = json!(format!("m{index}"));
            row
        })
        .collect();
    ls["squads"][0]["sections"][0]["rows"] = json!(rows);
    fake.answer(LS, &ls.to_string());
    let document = body(&fake.reads(), Target::Ops);
    assert_eq!(
        document["agents"].as_array().unwrap().len(),
        limits::READ_OPS_AGENTS
    );
    assert_eq!(document["truncated"], true);
}

#[test]
fn digest_forwards_the_usage_document_unchanged_under_report() {
    let fake = Fake::new();
    let report = json!({"version": 1, "observedAtMs": 5, "since": "24h", "window": null,
        "usage": {"seats": [], "unavailable": [], "lines": []}, "limits": [], "exchange": null});
    fake.answer(DIGEST, &report.to_string());
    let document = body(&fake.reads(), Target::Digest(Range::Day));
    assert_eq!(document["state"], "ok");
    assert_eq!(document["since"], "24h");
    assert_eq!(document["report"], report);
    assert_eq!(fake.calls(), [DIGEST]);
}

#[test]
fn digest_reads_the_requested_range_and_rejects_a_document_for_another() {
    let fake = Fake::new();
    let week = "digest usage report --since 7d --json";
    fake.answer(week, &json!({"version": 1, "since": "7d"}).to_string());
    let reads = fake.reads();
    assert_eq!(body(&reads, Target::Digest(Range::Week))["since"], "7d");
    fake.answer(
        "digest usage report --since 30d --json",
        &json!({"version": 1, "since": "24h"}).to_string(),
    );
    let document = body(&reads, Target::Digest(Range::Month));
    assert_eq!(document["state"], "unavailable");
    assert_eq!(document["reason"], "read_failed");
}

#[test]
fn every_failure_maps_to_one_closed_reason_and_never_to_an_empty_ok() {
    let fake = Fake::new();
    let reads = fake.reads();
    // Absent extension: a fixed argv can only be a usage error when the command is absent.
    fake.answer(
        DIGEST,
        r#"{"error":{"code":"USAGE_ERROR","message":"unrecognized subcommand"}}"#,
    );
    fake.set(DIGEST, "exit", "1");
    assert_eq!(
        body(&reads, Target::Digest(Range::Day))["reason"],
        "not_installed"
    );
    // A structured failure, a bad exit with no JSON, and a wrong version are all read_failed.
    let week = "digest usage report --since 7d --json";
    fake.answer(
        week,
        r#"{"error":{"code":"ROSTER_UNAVAILABLE","message":"x"}}"#,
    );
    fake.set(week, "exit", "1");
    assert_eq!(
        body(&reads, Target::Digest(Range::Week))["reason"],
        "read_failed"
    );
    let month = "digest usage report --since 30d --json";
    fake.answer(month, "not json");
    assert_eq!(
        body(&reads, Target::Digest(Range::Month))["reason"],
        "read_failed"
    );
    fake.answer(LS, r#"{"version":2}"#);
    assert_eq!(body(&reads, Target::Ops)["reason"], "read_failed");
    for document in [
        body(&reads, Target::Digest(Range::Day)),
        body(&reads, Target::Ops),
    ] {
        assert_eq!(document["version"], 1);
        assert_eq!(document["state"], "unavailable");
        assert!(document.get("agents").is_none() && document.get("report").is_none());
    }
}

#[test]
fn a_slow_read_is_cut_at_the_deadline_and_a_missing_executable_is_core_unavailable() {
    let fake = Fake::new();
    fake.answer(DIGEST, &json!({"version": 1, "since": "24h"}).to_string());
    fake.set(DIGEST, "sleep", "30");
    let reads = fake.reads().with_wait(Duration::from_millis(600));
    let began = Instant::now();
    assert_eq!(
        body(&reads, Target::Digest(Range::Day))["reason"],
        "timeout"
    );
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    let missing = Reads::new(
        PathBuf::from("/tmp/tmt-4325-does-not-exist"),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        body(&missing, Target::Digest(Range::Day))["reason"],
        "core_unavailable"
    );
}

#[test]
fn an_oversized_output_is_too_large_not_truncated() {
    let fake = Fake::new();
    let big = json!({"version": 1, "since": "24h",
        "pad": "x".repeat(limits::READ_DIGEST_BYTES + 1)});
    fake.answer(DIGEST, &big.to_string());
    let document = body(&fake.reads(), Target::Digest(Range::Day));
    assert_eq!(document["state"], "unavailable");
    assert_eq!(document["reason"], "too_large");
    // Past the subprocess capture bound the read is cut, not parsed.
    let huge = json!({"version": 1, "since": "7d",
        "pad": "x".repeat(limits::READ_SUBPROCESS_BYTES + 1)});
    fake.answer("digest usage report --since 7d --json", &huge.to_string());
    let document = body(&fake.reads(), Target::Digest(Range::Week));
    assert_eq!(document["reason"], "too_large");
}

#[test]
fn concurrent_requests_share_one_read_and_a_fresh_answer_is_reused() {
    let fake = Fake::new();
    fake.answer(DIGEST, &json!({"version": 1, "since": "24h"}).to_string());
    fake.set(DIGEST, "sleep", "1");
    let reads = Arc::new(fake.reads());
    let answers: Vec<_> = (0..8)
        .map(|_| {
            let reads = Arc::clone(&reads);
            thread::spawn(move || reads.body(Target::Digest(Range::Day)))
        })
        .collect();
    let answers: Vec<_> = answers.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(answers.iter().all(|answer| answer == &answers[0]));
    assert_eq!(fake.calls().len(), 1, "{:?}", fake.calls());
    // Inside the fresh window another request costs nothing; a different key is separate.
    reads.body(Target::Digest(Range::Day));
    assert_eq!(fake.calls().len(), 1);
    fake.answer(
        "digest usage report --since 7d --json",
        &json!({"version": 1, "since": "7d"}).to_string(),
    );
    reads.body(Target::Digest(Range::Week));
    assert_eq!(fake.calls().len(), 2);
}

#[test]
fn nothing_runs_without_a_request_and_a_stale_answer_is_read_again() {
    let fake = Fake::new();
    fake.answer(DIGEST, &json!({"version": 1, "since": "24h"}).to_string());
    let reads = fake.reads();
    thread::sleep(Duration::from_millis(200));
    assert!(fake.calls().is_empty(), "a read ran without a request");
    reads.body(Target::Digest(Range::Day));
    // Age the answer past its window instead of sleeping through it.
    reads
        .lock()
        .get_mut(&Target::Digest(Range::Day))
        .unwrap()
        .fresh
        .as_mut()
        .unwrap()
        .0 = Instant::now() - limits::READ_FRESH - Duration::from_millis(1);
    reads.body(Target::Digest(Range::Day));
    assert_eq!(fake.calls().len(), 2);
}

#[test]
fn a_failed_read_is_also_reused_so_a_broken_command_is_not_hammered() {
    let fake = Fake::new();
    fake.answer(DIGEST, "not json");
    let reads = fake.reads();
    for _ in 0..5 {
        assert_eq!(
            body(&reads, Target::Digest(Range::Day))["reason"],
            "read_failed"
        );
    }
    assert_eq!(fake.calls().len(), 1);
}

fn vectors() -> Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../contracts/owner-reads-v1.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn the_shared_vectors_are_what_the_door_answers_and_leak_nothing() {
    let vectors = vectors();
    let forbidden: Vec<&str> = vectors["forbiddenInAnswers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|word| word.as_str().unwrap())
        .collect();
    assert!(!forbidden.is_empty());
    for case in vectors["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let fake = Fake::new();
        for (argv, result) in case["commands"].as_object().unwrap() {
            let stdout = match &result["stdout"] {
                Value::String(raw) => raw.clone(),
                document => document.to_string(),
            };
            fake.answer(argv, &stdout);
            fake.set(argv, "exit", &result["exit"].to_string());
        }
        let target = Target::route(case["target"].as_str().unwrap())
            .unwrap_or_else(|| panic!("{name}: target is not a route"));
        let wire = fake.reads().body(target);
        let text = String::from_utf8(wire.to_vec()).unwrap();
        for word in &forbidden {
            assert!(!text.contains(word), "{name}: {word} leaked: {text}");
        }
        let mut answer: Value = serde_json::from_str(&text).unwrap();
        assert!(answer["observedAtMs"].as_u64().unwrap() > 0, "{name}");
        answer.as_object_mut().unwrap().remove("observedAtMs");
        assert_eq!(answer, case["expect"], "{name}");
    }
}

#[test]
fn the_vector_reason_set_is_the_code_set() {
    // An exhaustive match: a new reason fails to compile here until the vector names it.
    let all = |reason: Reason| match reason {
        Reason::NotInstalled
        | Reason::CoreUnavailable
        | Reason::Timeout
        | Reason::ReadFailed
        | Reason::TooLarge => reason.code(),
    };
    let codes: Vec<&str> = [
        Reason::NotInstalled,
        Reason::CoreUnavailable,
        Reason::Timeout,
        Reason::ReadFailed,
        Reason::TooLarge,
    ]
    .into_iter()
    .map(all)
    .collect();
    let vectors = vectors();
    let named: Vec<&str> = vectors["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| reason.as_str().unwrap())
        .collect();
    assert_eq!(codes, named);
}

#[test]
fn the_vector_bounds_are_the_door_bounds() {
    let vectors = vectors();
    let bounds = &vectors["bounds"];
    assert_eq!(bounds["freshMs"], limits::READ_FRESH.as_millis() as u64);
    assert_eq!(bounds["waitMs"], limits::READ_WAIT.as_millis() as u64);
    assert_eq!(
        bounds["subprocessStdoutBytes"],
        limits::READ_SUBPROCESS_BYTES
    );
    assert_eq!(bounds["opsBytes"], limits::READ_OPS_BYTES);
    assert_eq!(bounds["digestBytes"], limits::READ_DIGEST_BYTES);
    assert_eq!(bounds["opsSquads"], limits::READ_OPS_SQUADS);
    assert_eq!(bounds["opsAgents"], limits::READ_OPS_AGENTS);
    assert_eq!(bounds["opsTimers"], limits::READ_OPS_TIMERS);
    assert_eq!(bounds["text"]["name"], limits::READ_OPS_NAME);
    assert_eq!(bounds["text"]["state"], limits::READ_OPS_STATE);
    assert_eq!(bounds["text"]["text"], limits::READ_OPS_TEXT);
    assert_eq!(bounds["text"]["link"], limits::READ_OPS_LINK);
    assert_eq!(bounds["text"]["what"], limits::READ_OPS_WHAT);
}
