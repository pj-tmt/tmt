use super::*;
use serde_json::Value;

fn records(buffer: &Mutex<Vec<u8>>) -> Vec<Value> {
    let bytes = buffer.lock().unwrap();
    std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn opt_in_values_preserve_file_paths() {
    for value in [None, Some(OsStr::new("")), Some(OsStr::new("0"))] {
        assert!(matches!(destination(value), Destination::Off));
        assert!(Trace::open(destination(value)).is_none());
    }
    assert!(matches!(
        destination(Some(OsStr::new("1"))),
        Destination::Stderr
    ));
    let Destination::File(path) = destination(Some(OsStr::new("my trace.jsonl"))) else {
        panic!()
    };
    assert_eq!(path, std::path::PathBuf::from("my trace.jsonl"));
}

#[test]
fn disabled_stage_never_samples_the_clock_and_preserves_result() {
    let work = std::cell::Cell::new(0);
    let result = measured(
        None,
        "Squad::list",
        || {
            work.set(work.get() + 1);
            Err::<(), _>("unavailable")
        },
        || panic!("off must not read clock"),
    );
    assert_eq!(result, Err("unavailable"));
    assert_eq!(work.get(), 1);
}

#[test]
fn stage_records_have_stable_shape_units_escaping_and_error_status() {
    let (trace, buffer) = Trace::buffer();
    let load = trace.load(Some("quoted\"\nname"), 7);
    load.emit("stage", "Squad::list", Duration::from_micros(1234), true);
    let result = measure(Some(&load), "Config::load", || {
        Err::<(), _>("secret error body")
    });
    assert!(result.is_err());
    let records = records(&buffer);
    assert_eq!(records.len(), 2);
    let mut expected = json!({"version":1,"pid":std::process::id(),"load_id":1,"generation":7,
        "tab":"quoted\"\nname","event":"stage","stage":"Squad::list","duration_us":1234,
        "elapsed_us":0,"status":"ok","deferred_pending":false});
    expected["elapsed_us"] = records[0]["elapsed_us"].clone();
    assert_eq!(records[0], expected);
    assert_eq!(records[1]["stage"], "Config::load");
    assert_eq!(records[1]["status"], "error");
    assert!(records[1]["duration_us"].is_u64());
    assert!(
        !String::from_utf8(buffer.lock().unwrap().clone())
            .unwrap()
            .contains("secret")
    );
}

#[test]
fn file_sink_appends_private_complete_records_and_rejects_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = std::env::temp_dir().join(format!(
        "ops-timing-{}-{}",
        std::process::id(),
        crate::id::new_v4().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("trace");
    for _ in 0..2 {
        let trace = Trace::open(Destination::File(path.clone())).unwrap();
        trace
            .startup()
            .emit("stage", "startup.Config::load", Duration::ZERO, true);
    }
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(records(&Mutex::new(bytes.clone())).len(), 2);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = root.join("link");
    symlink(&path, &link).unwrap();
    assert!(Trace::open(Destination::File(link)).is_none());
    assert!(Trace::open(Destination::File(root.clone())).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn milestones_require_accepted_current_snapshot_and_are_emitted_once() {
    let (trace, buffer) = Trace::buffer();
    let mut ui = Ui::new(trace.clone());
    ui.drawn(Some("product"), true, false); // Retained view alone is insufficient.
    ui.drawn(Some("product"), true, true); // A stored display is its own milestone.
    ui.drawn(Some("product"), true, true);
    let mut load = trace.load(Some("product"), 2);
    load.partial = true;
    load.pending = true;
    ui.snapshot(Some(load.clone()), false); // Failed/not-current snapshot.
    ui.drawn(Some("product"), true, false);
    ui.snapshot(Some(load.clone()), true);
    ui.drawn(Some("infra"), true, false); // Switched before draw.
    ui.snapshot(Some(load.clone()), true);
    ui.drawn(Some("product"), false, false); // Still loading.
    ui.snapshot(Some(load), true);
    ui.drawn(Some("product"), true, false);
    ui.drawn(Some("product"), true, true);
    let records = records(&buffer);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["event"], "first_frame");
    assert_eq!(records[1]["event"], "cached_board");
    assert_eq!(records[1]["tab"], "product");
    assert_eq!(records[2]["event"], "fresh_board");
    assert_eq!(records[2]["generation"], 2);
    assert_eq!(records[2]["status"], "partial");
    assert_eq!(records[2]["deferred_pending"], true);
}

#[test]
fn sink_failure_disables_further_writes_without_changing_work_results() {
    let path = std::env::temp_dir().join(format!(
        "ops-timing-readonly-{}",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(&path, b"preserved").unwrap();
    let trace = Trace::new(Sink::File(File::open(&path).unwrap()));
    let load = trace.load(Some("product"), 0);
    for _ in 0..2 {
        assert_eq!(
            measure(Some(&load), "Config::load", || Ok::<_, ()>(42)),
            Ok(42)
        );
        assert!(matches!(*trace.0.sink.lock().unwrap(), Sink::Disabled));
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"preserved");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn worker_and_ui_records_share_one_complete_line_writer() {
    let (trace, buffer) = Trace::buffer();
    std::thread::scope(|scope| {
        for generation in 0..2 {
            let trace = trace.clone();
            scope.spawn(move || {
                let load = trace.load(Some("product"), generation);
                for _ in 0..10 {
                    load.emit("stage", "Squad::list", Duration::ZERO, true);
                }
            });
        }
    });
    let records = records(&buffer);
    assert_eq!(records.len(), 20);
    for generation in 0..2 {
        assert_eq!(
            records
                .iter()
                .filter(|record| record["generation"] == generation)
                .count(),
            10
        );
    }
}
