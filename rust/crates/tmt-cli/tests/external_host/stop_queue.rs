//! Durable Stop handoff and real finite-worker crash/recovery boundaries.
use super::stop_capture::{install_capture_hook, ok};
use super::*;

#[test]
fn stop_returns_under_contention_and_recovers_real_flusher_crashes_before_and_after_main_commit() {
    register();
    for after_commit in [false, true] {
        let fixture = Fixture::delivering(&format!("capture-crash-{after_commit}"));
        bind(&fixture, fixture.approve());
        install_capture_hook(&fixture, "claude");
        let mut provider = SteppedProvider::start(&fixture, "claude", "writer-collision-steps");
        ok(provider.hook(claude_event("SessionStart")));
        let mut prompt = claude_event("UserPromptSubmit");
        prompt["prompt"] = "question".into();
        ok(provider.hook(prompt));
        let agent = worker(&fixture.json(&["ls", "--json"]))["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let ProcessObservation::Live(owner) = observe_runtime_process(
            &UnixCommandRunner,
            u64::from(std::process::id()),
            deadline(),
        )
        .unwrap() else {
            panic!("live launch owner")
        };
        let launch =
            serde_json::to_string(&record_admitted_launch(&fixture, &agent, "claude", &owner))
                .unwrap();
        let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut stop = claude_event("Stop");
        stop["last_assistant_message"] = "durable while Digest holds SQLite".into();
        let output = provider.hook_command(stop, &[("TMT_LAUNCH_OBSERVATION", launch.as_bytes())]);
        // Adopt the owned worker before any behavior assertion can unwind the test.
        let log = fs::read_dir(fixture.state())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("hook-flush-")
            })
            .unwrap();
        let pid = fs::read_to_string(&log)
            .unwrap()
            .trim()
            .strip_prefix("flusher_pid=")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let ProcessObservation::Live(incarnation) =
            observe_runtime_process(&UnixCommandRunner, pid, deadline()).unwrap()
        else {
            panic!("flusher waits for the owned main writer")
        };
        struct Flusher(Owned);
        impl Drop for Flusher {
            fn drop(&mut self) {
                self.0.kill();
                wait_until("owned flusher cleanup", || self.0.ended());
            }
        }
        let owned = Flusher(Owned {
            incarnation,
            group: true,
        });
        ok(output);
        // The hook returned while THIS transaction still owns the main writer.
        // This proves handoff, without a timing threshold, retry or larger budget.
        let finals = || {
            db.query_row("SELECT count(*) FROM recent_responses", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        assert_eq!(finals(), 0);
        let queue = rusqlite::Connection::open_with_flags(
            fixture.state().join("hook-writes.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .unwrap();
        let pending = || {
            queue
                .query_row("SELECT count(*) FROM hook_writes", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        };
        assert_eq!(pending(), 1);
        if after_commit {
            // Block only queue acknowledgment, not the main capture transaction.
            queue.execute_batch("BEGIN IMMEDIATE").unwrap();
            db.execute_batch("ROLLBACK").unwrap();
            wait_until("the final and receipt commit before acknowledgment", || {
                finals() == 1
            });
            let receipts: i64 = db
                .query_row(
                    "SELECT count(*) FROM hook_capture_receipts WHERE outcome='committed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(receipts, 1);
            assert_eq!(pending(), 1);
        }
        owned.0.kill();
        wait_until(
            "the crashed flusher releases its process and advisory lock",
            || owned.0.ended(),
        );
        if after_commit {
            queue.execute_batch("ROLLBACK").unwrap();
        } else {
            assert_eq!(finals(), 0);
            db.execute_batch("ROLLBACK").unwrap();
        }
        assert_eq!(
            pending(),
            1,
            "the crash cannot acknowledge uncommitted queue work"
        );
        // The killed worker's known log is retained by the product. Classify and
        // remove exactly this fixture-owned crash artifact before the next drain.
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!("flusher_pid={pid}\n")
        );
        fs::remove_file(log).unwrap();
        // A later ordinary writer, without another Stop, recovers the accepted
        // final and starts the same finite worker for follow-up/acknowledgment.
        fixture.json(&[
            "role",
            "set",
            "recovery writer",
            "--identity",
            "worker",
            "--json",
        ]);
        wait_flushed(&fixture.state());
        assert_eq!(finals(), 1);
        assert_eq!(pending(), 0);
        let role = fixture.json(&["role", "show", "--identity", "worker", "--json"]);
        assert_eq!(role["role"]["content"], "recovery writer");
        let mut continuation = claude_event("Stop");
        continuation["stop_hook_active"] = true.into();
        continuation["last_assistant_message"] = "a distinct continued final".into();
        ok(provider.hook(continuation));
        wait_flushed(&fixture.state());
        let mut query = db
            .prepare("SELECT text FROM recent_responses ORDER BY sequence")
            .unwrap();
        let texts = query
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            texts,
            vec![
                "durable while Digest holds SQLite",
                "a distinct continued final"
            ]
        );
        let failures: i64 = db
            .query_row(
                "SELECT count(*) FROM capture_entries WHERE kind='failure'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(failures, 0);
        assert_eq!(pending(), 0);
    }
}

#[test]
fn an_admitted_prompt_without_text_still_opens_a_turn_batch() {
    register();
    let fixture = Fixture::delivering("prompt-without-text-batch");
    bind(&fixture, fixture.approve());
    install_capture_hook(&fixture, "claude");
    let mut provider = SteppedProvider::start(&fixture, "claude", "no-text-prompt-steps");
    ok(provider.hook(claude_event("SessionStart")));
    ok(provider.hook(claude_event("UserPromptSubmit")));
    let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
    let open: i64 = db
        .query_row(
            "SELECT count(*) FROM capture_batches WHERE closed_ms IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        open, 1,
        "an admitted no-text prompt needs its own durable turn fence"
    );
}

#[test]
fn a_delayed_no_start_diagnostic_cannot_reappear_after_a_new_prompt() {
    register();
    let fixture = Fixture::delivering("delayed-stop-diagnostic");
    bind(&fixture, fixture.approve());
    install_capture_hook(&fixture, "claude");
    let mut provider = SteppedProvider::start(&fixture, "claude", "delayed-diagnostic-steps");
    ok(provider.hook(claude_event("SessionStart")));
    let agent = worker(&fixture.json(&["ls", "--json"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let ProcessObservation::Live(owner) = observe_runtime_process(
        &UnixCommandRunner,
        u64::from(std::process::id()),
        deadline(),
    )
    .unwrap() else {
        panic!("live fixture owner")
    };
    let launch =
        serde_json::to_string(&record_admitted_launch(&fixture, &agent, "claude", &owner)).unwrap();
    let queue = tmt_adapters::storage::hook_queue::HookQueue::open(
        &fixture.state().join("tmt.db"),
        Duration::ZERO,
    )
    .unwrap();
    let guard = queue.claim().unwrap();
    let mut stop = claude_event("Stop");
    stop["last_assistant_message"] = "unadmitted text stays dropped".into();
    ok(provider.hook_command(stop, &[("TMT_LAUNCH_OBSERVATION", launch.as_bytes())]));
    // The owned lock holds back only the optional flusher. A later prompt's
    // ordinary writer ingests the old event before creating its fresh input batch.
    let mut prompt = claude_event("UserPromptSubmit");
    prompt["prompt"] = "new question".into();
    ok(provider.hook_with_env(prompt, &[("TMT_LAUNCH_OBSERVATION", launch.as_bytes())]));
    drop(guard);
    fixture.json(&[
        "role",
        "set",
        "recover delayed diagnostic",
        "--identity",
        "worker",
        "--json",
    ]);
    wait_flushed(&fixture.state());
    let listed = fixture.json(&["ls", "--json"]);
    assert!(
        worker(&listed)["session"]["learning"].is_null(),
        "{listed:#}"
    );
    let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM recent_responses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM hook_capture_receipts WHERE outcome='no_admitted_start'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let snapshot = Storage::context_by_identity(
        &fixture.state().join("tmt.db"),
        &agent,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .unwrap()
    .unwrap();
    let state = snapshot.preferences.remembered.unwrap().state.unwrap();
    assert_eq!(
        tmt_adapters::runtime::driver_state::state_activity(&state)
            .unwrap()
            .phase,
        tmt_core::binding::session::activity::ActivityPhase::Working
    );
}
