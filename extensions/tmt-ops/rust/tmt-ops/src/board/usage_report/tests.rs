//! Reader demand, failure retention and lifecycle. Acquisition admission and
//! batching use public Core-shaped fixtures in acquire/tests.rs.
use super::{Delivery, Reader, Report, Since, State, Timing, View};
use crate::{core::Core, test_support::write_ready_executable};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

fn empty(range: Since) -> Report {
    Report {
        since: range,
        observed_at_ms: 1,
        window_from_ms: None,
        seats: vec![],
        unavailable: vec![],
        exchange: None,
        warnings: vec![],
    }
}
#[test]
fn failed_refresh_retains_evidence_and_exposes_staleness_only_for_its_range() {
    let mut state = State::default();
    assert!(matches!(state.view_for(Since::Day), View::Loading));
    state.apply(Delivery {
        since: Since::Day,
        result: Ok(empty(Since::Day)),
    });
    state.apply(Delivery {
        since: Since::Day,
        result: Err("down".into()),
    });
    assert!(matches!(state.view_for(Since::Day), View::Report(_)));
    assert_eq!(state.refresh_error(Since::Day), Some("down"));
    assert!(matches!(state.view_for(Since::Week), View::Loading));
    assert_eq!(state.refresh_error(Since::Week), None);
    state.apply(Delivery {
        since: Since::Week,
        result: Err("week down".into()),
    });
    assert!(matches!(
        state.view_for(Since::Week),
        View::Failed("week down")
    ));
    state.apply(Delivery {
        since: Since::Day,
        result: Ok(empty(Since::Day)),
    });
    assert_eq!(state.refresh_error(Since::Day), None);
}

pub(super) struct Scratch(pub(super) std::path::PathBuf);
impl Scratch {
    pub(super) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ops-usage-report-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    /// A stand-in for `tmt` that logs each call and prints `report.json`, or fails.
    fn core(&self) -> Core {
        let script = self.0.join("tmt");
        write_ready_executable(
            &script,
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{dir}/calls'\n\
                 if [ -f '{dir}/fail' ]; then echo '{{\"error\":{{\"code\":\"X\",\"message\":\"supplier down\"}}}}'; exit 1; fi\n\
                 cat '{dir}/report.json'\n",
                dir = self.0.display()
            ),
        );
        Core::at(script)
    }
    pub(super) fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.0.join("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn quick() -> Timing {
    Timing {
        every: Duration::from_millis(30),
        deadline: Duration::from_secs(10),
        slowest: Duration::from_millis(120),
    }
}

fn next(received: &mpsc::Receiver<Delivery>) -> Delivery {
    received
        .recv_timeout(Duration::from_secs(20))
        .expect("a delivery")
}

#[test]
fn the_reader_reads_only_while_wanted_and_for_the_wanted_window() {
    let scratch = Scratch::new();
    std::fs::write(
        scratch.0.join("report.json"),
        json!({"sinceMs":1,"observedAtMs":2,"rows":[]}).to_string(),
    )
    .unwrap();
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(&scratch.core(), quick(), move |d| sender.send(d).is_ok());
    let wants = reader.wants();
    // Nothing is read before the panel is wanted.
    std::thread::sleep(Duration::from_millis(150));
    assert!(scratch.calls().is_empty());
    wants.set(Some(super::Request::new(crate::config::TokRange::Day, [])));
    let first = next(&received);
    assert_eq!((first.since, first.result.is_ok()), (Since::Day, true));
    assert_eq!(scratch.calls()[0], "x flow --since 1d --bucket 1d --json");
    // It repeats while wanted, and a new window reads at once for that window.
    let again = next(&received);
    assert_eq!(again.since, Since::Day);
    wants.set(Some(super::Request::new(crate::config::TokRange::Week, [])));
    loop {
        let delivery = next(&received);
        if delivery.since == Since::Week {
            break;
        }
    }
    assert!(
        scratch
            .calls()
            .iter()
            .any(|call| call == "x flow --since 7d --bucket 7d --json")
    );
    // Leaving the panel stops the reads.
    wants.set(None);
    std::thread::sleep(Duration::from_millis(150));
    while received.try_recv().is_ok() {}
    let settled = scratch.calls().len();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        scratch.calls().len(),
        settled,
        "no read while the panel is off screen"
    );
    drop(reader);
}

#[test]
fn a_failing_supplier_is_delivered_as_a_message_and_a_malformed_document_is_refused() {
    let scratch = Scratch::new();
    std::fs::write(scratch.0.join("fail"), "").unwrap();
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(&scratch.core(), quick(), move |d| sender.send(d).is_ok());
    reader
        .wants()
        .set(Some(super::Request::new(crate::config::TokRange::Week, [])));
    let failed = next(&received);
    assert_eq!(failed.since, Since::Week);
    assert_eq!(failed.result.unwrap_err(), "supplier down");
    std::fs::remove_file(scratch.0.join("fail")).unwrap();
    std::fs::write(scratch.0.join("report.json"), "{\"version\":2}").unwrap();
    loop {
        let delivery = next(&received);
        if let Err(message) = delivery.result
            && message != "supplier down"
        {
            assert!(message.contains("Malformed"), "{message}");
            break;
        }
    }
    drop(reader);
}

fn reached(path: &std::path::Path) {
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(std::time::Instant::now() < end, "read reached its barrier");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_changed_demand_cancels_the_inflight_window_and_delivers_the_new_range_promptly() {
    let scratch = Scratch::new();
    let script = scratch.0.join("barrier-core");
    write_ready_executable(
        &script,
        &format!(
            "#!/bin/sh\nif [ \"$4\" = '1d' ]; then\n : > '{dir}/started'\n exec sleep 30\nfi\nprintf '%s' '{{\"sinceMs\":1,\"observedAtMs\":2,\"rows\":[]}}'\n",
            dir = scratch.0.display()
        ),
    );
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(
        &Core::at(script),
        Timing {
            every: Duration::from_secs(3600),
            ..quick()
        },
        move |d| sender.send(d).is_ok(),
    );
    let wants = reader.wants();
    wants.set(Some(super::Request::new(Since::Day, [])));
    reached(&scratch.0.join("started"));
    // Repeating the same demand must leave the blocked read running.
    wants.set(Some(super::Request::new(Since::Day, [])));
    assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
    let start = std::time::Instant::now();
    wants.set(Some(super::Request::new(Since::Week, [])));
    let delivery = received
        .recv_timeout(Duration::from_secs(5))
        .expect("new range does not wait behind the 10-second read deadline");
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(
        delivery.since,
        Since::Week,
        "the old result never reaches App"
    );
    assert!(delivery.result.is_ok());
    drop(reader);
    assert!(received.try_recv().is_err());
}

#[test]
fn dropping_the_reader_cancels_an_inflight_public_read_and_joins() {
    let scratch = Scratch::new();
    let script = scratch.0.join("blocked-core");
    write_ready_executable(
        &script,
        &format!(
            "#!/bin/sh\n: > '{dir}/started'\nexec sleep 30\n",
            dir = scratch.0.display()
        ),
    );
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(
        &Core::at(script),
        Timing {
            deadline: Duration::from_secs(40),
            ..quick()
        },
        move |d| sender.send(d).is_ok(),
    );
    reader
        .wants()
        .set(Some(super::Request::new(Since::Day, [])));
    reached(&scratch.0.join("started"));
    let start = std::time::Instant::now();
    drop(reader);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "drop cancels rather than waiting for the command deadline"
    );
    assert!(
        matches!(received.try_recv(), Err(mpsc::TryRecvError::Disconnected)),
        "joined reader publishes no cancelled observation"
    );
}
