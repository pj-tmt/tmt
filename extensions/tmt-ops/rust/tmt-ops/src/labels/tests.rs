//! Reader and action seam against the supplier's frozen vectors and a fake extension.
use super::*;
use crate::test_support::write_ready_executable;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const VECTORS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tmt-digest/contracts/vectors/status-v1.json"
);
const NAMESPACE: &str = "digest";

fn vectors() -> Value {
    serde_json::from_slice(&fs::read(VECTORS).expect("the supplier's vectors")).unwrap()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ops-labels-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// A stand-in for `tmt` that answers `digest status --json` from `status.json`
    /// after running `before`, and logs every invocation to `calls`.
    fn fake(&self, before: &str) -> Core {
        let script = self.path("tmt");
        write_ready_executable(
            &script,
            &format!(
                "#!/bin/sh\necho \"$$ $*\" >> '{dir}/calls'\n{before}\n\
                 [ \"$1 $2 $3\" = 'digest status --json' ] || exit 2\n\
                 cat '{dir}/status.json'\n",
                dir = self.0.display()
            ),
        );
        Core::at(script)
    }

    fn status(&self, document: &Value) {
        fs::write(self.path("status.json"), document.to_string()).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.path("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn quick() -> Timing {
    Timing {
        every: Duration::from_millis(20),
        deadline: Duration::from_secs(5),
        keep: Duration::from_millis(200),
        slowest: Duration::from_millis(80),
    }
}

fn reader(core: &Core, timing: Timing) -> (Reader, mpsc::Receiver<Supplied>) {
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(core, vec![NAMESPACE.into()], timing, move |supplied| {
        sender.send(supplied).is_ok()
    })
    .expect("a configured source");
    (reader, received)
}

fn next(received: &mpsc::Receiver<Supplied>) -> Supplied {
    received
        .recv_timeout(Duration::from_secs(20))
        .expect("a delivery")
}

#[test]
fn every_status_snapshot_reads_into_rows_exactly_as_supplied() {
    let vectors = vectors();
    let snapshots = vectors["status"].as_array().unwrap();
    assert_eq!(snapshots.len(), 3);
    for snapshot in snapshots {
        let response = &snapshot["response"];
        let rows = parse(response).unwrap_or_else(|| panic!("{} reads", snapshot["name"]));
        let members = response["members"].as_array().unwrap();
        assert_eq!(rows.len(), members.len(), "{}", snapshot["name"]);
        for member in members {
            let shown = &rows[member["identityId"].as_str().unwrap()];
            let supplied = member["labels"].as_array().unwrap();
            assert_eq!(shown.len(), supplied.len());
            for (label, supplied) in shown.iter().zip(supplied) {
                assert_eq!(label.text, supplied["text"].as_str().unwrap());
                assert_eq!(label.role.name(), supplied["colorClass"].as_str().unwrap());
            }
        }
    }
}

#[test]
fn malformed_documents_are_unavailable_and_malformed_rows_are_left_out() {
    let good =
        json!({"identityId":"a","labels":[{"text":"Auto","colorClass":"text","action":null}]});
    let document = |version: Value, members: Value| json!({"version":version,"members":members});
    assert!(parse(&document(json!(1), json!([good]))).is_some());
    for unavailable in [
        document(json!(2), json!([good])),
        document(json!("1"), json!([good])),
        json!({"members":[good]}),
        json!({"version":1}),
        document(json!(1), json!({"a":1})),
        json!([]),
    ] {
        assert!(parse(&unavailable).is_none(), "{unavailable}");
    }
    let label = |text: Value, class: &str, action: Value| json!({"identityId":"bad","labels":[{"text":text,"colorClass":class,"action":action}]});
    let rows = parse(&document(
        json!(1),
        json!([
            good,
            label(json!("Auto"), "red", Value::Null),
            label(json!("Auto"), "accent", Value::Null),
            label(json!(""), "text", Value::Null),
            label(json!("Aut\u{1b}[31mo"), "text", Value::Null),
            label(json!("x".repeat(49)), "text", Value::Null),
            label(json!(7), "text", Value::Null),
            json!({"identityId":"four","labels":[
                {"text":"a","colorClass":"text","action":null},
                {"text":"b","colorClass":"text","action":null},
                {"text":"c","colorClass":"text","action":null},
                {"text":"d","colorClass":"text","action":null}]}),
            json!({"labels":[]}),
            json!({"identityId":"nolabels"}),
        ]),
    ))
    .unwrap();
    assert_eq!(rows.keys().collect::<Vec<_>>(), ["a"]);
}

#[test]
fn unknown_additive_fields_are_ignored() {
    let mut document = vectors()["status"][1]["response"].clone();
    document["future"] = json!({"x": 1});
    document["members"][0]["future"] = json!(true);
    document["members"][0]["labels"][0]["future"] = json!("later");
    assert_eq!(
        parse(&document).unwrap(),
        parse(&vectors()["status"][1]["response"]).unwrap()
    );
}

#[test]
fn a_ready_source_is_delivered_once_and_never_read_twice_at_the_same_time() {
    let scratch = Scratch::new();
    scratch.status(&vectors()["status"][0]["response"]);
    let core = scratch.fake(&format!(
        "echo start >> '{0}/trace'; sleep 0.05; echo end >> '{0}/trace'",
        scratch.0.display()
    ));
    let (reader, received) = reader(&core, quick());
    let supplied = next(&received);
    let mixed = parse(&vectors()["status"][0]["response"]).unwrap();
    let id = mixed.keys().next().unwrap();
    assert_eq!(
        supplied
            .of(id)
            .map(|(source, _)| source)
            .collect::<Vec<_>>(),
        vec![NAMESPACE; mixed[id].len()]
    );
    eventually("repeated reads", || scratch.calls().len() >= 3);
    // The answer did not change, so nothing more is delivered.
    assert!(received.try_recv().is_err());
    drop(reader);
    let trace = fs::read_to_string(scratch.path("trace")).unwrap();
    let lines: Vec<_> = trace.lines().collect();
    assert!(
        lines
            .chunks(2)
            .all(|pair| pair[0] == "start" && pair.get(1).is_none_or(|end| *end == "end")),
        "reads overlapped: {lines:?}"
    );
}

#[test]
fn a_changed_answer_is_delivered_again() {
    let scratch = Scratch::new();
    scratch.status(&vectors()["status"][1]["response"]);
    let core = scratch.fake("");
    let (_reader, received) = reader(&core, quick());
    let first = next(&received);
    scratch.status(&vectors()["status"][2]["response"]);
    let second = next(&received);
    assert_ne!(first, second);
    assert_eq!(second, Supplied(vec![(NAMESPACE.into(), Rows::new())]));
}

#[test]
fn a_slow_source_is_cut_at_the_deadline_and_delivers_nothing() {
    let scratch = Scratch::new();
    scratch.status(&vectors()["status"][0]["response"]);
    let core = scratch.fake("sleep 30");
    let timing = Timing {
        deadline: Duration::from_millis(100),
        ..quick()
    };
    let (reader, received) = reader(&core, timing);
    eventually("a retry after the deadline", || scratch.calls().len() >= 2);
    assert!(received.try_recv().is_err(), "a cut read showed labels");
    let pids: Vec<i32> = scratch
        .calls()
        .iter()
        .filter_map(|call| call.split(' ').next()?.parse().ok())
        .collect();
    drop(reader);
    for pid in pids {
        eventually("the cut read's process to be gone", || {
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err()
        });
    }
}

#[test]
fn failing_and_absent_sources_stay_unavailable_without_error() {
    let scratch = Scratch::new();
    // Exits non-zero with no document, then with a structured error.
    let failing = scratch.fake("exit 1");
    let (reader_one, received) = reader(&failing, quick());
    eventually("retries", || scratch.calls().len() >= 2);
    assert!(received.try_recv().is_err());
    drop(reader_one);
    scratch.status(&json!({"error":{"code":"SQUAD_CORE_UNAVAILABLE","message":"no"}}));
    let erroring = scratch.fake("");
    let (reader_two, received) = reader(&erroring, quick());
    std::thread::sleep(Duration::from_millis(150));
    assert!(received.try_recv().is_err());
    drop(reader_two);
    let missing = Core::at(scratch.path("absent"));
    let (reader_three, received) = reader(&missing, quick());
    std::thread::sleep(Duration::from_millis(150));
    assert!(received.try_recv().is_err());
    drop(reader_three);
}

#[test]
fn failures_back_off_and_the_last_good_answer_stands_in_only_briefly() {
    let scratch = Scratch::new();
    scratch.status(&vectors()["status"][1]["response"]);
    let core = scratch.fake(&format!("[ -e '{0}/down' ] && exit 1", scratch.0.display()));
    let timing = Timing {
        keep: Duration::from_millis(250),
        ..quick()
    };
    let (_reader, received) = reader(&core, timing);
    let good = next(&received);
    assert!(
        good.of(parse(&vectors()["status"][1]["response"])
            .unwrap()
            .keys()
            .next()
            .unwrap())
            .next()
            .is_some()
    );
    fs::write(scratch.path("down"), "").unwrap();
    let gone = next(&received);
    assert_eq!(
        gone,
        Supplied::default(),
        "stale labels outlived their keep"
    );
    let before = scratch.calls().len();
    std::thread::sleep(Duration::from_millis(400));
    // 80 ms at the slowest, so a few more reads, not one per 20 ms tick.
    assert!(scratch.calls().len() - before <= 8, "{:?}", scratch.calls());
}

#[test]
fn no_configured_source_starts_no_reader() {
    let scratch = Scratch::new();
    assert!(Reader::spawn(&scratch.fake(""), Vec::new(), quick(), |_| true).is_none());
    assert!(scratch.calls().is_empty());
}

#[test]
fn dropping_the_reader_stops_the_thread() {
    let scratch = Scratch::new();
    scratch.status(&vectors()["status"][1]["response"]);
    let (reader, received) = reader(&scratch.fake(""), quick());
    next(&received);
    drop(reader);
    let calls = scratch.calls().len();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(scratch.calls().len(), calls);
}

#[test]
fn actions_are_not_this_readers_business() {
    let label = |action: Value| {
        json!({"version":1,"members":[{"identityId":"a","labels":[
            {"text":"Auto","colorClass":"text","action":action}]}]})
    };
    for action in [
        Value::Null,
        json!({"kind":"shell","argv":"rm -rf /"}),
        json!("x"),
    ] {
        let rows = parse(&label(action)).unwrap();
        assert_eq!(rows["a"].len(), 1);
    }
}
