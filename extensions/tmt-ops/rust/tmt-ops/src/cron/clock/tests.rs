use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "squad-clock-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn clock(&self) -> Clock {
        Clock::new(&self.0.join("ops")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn absent_status_creates_nothing_and_live_lease_excludes_every_competitor() {
    let f = Fixture::new();
    let clock = f.clock();
    assert_eq!(clock.status(100), ClockStatus::NoClock);
    assert!(!f.0.join("ops").exists());
    let mut lease = clock
        .acquire(100, 123, Some("%41".into()), "0.1.0")
        .unwrap()
        .unwrap();
    assert_eq!(
        clock.status(100),
        ClockStatus::Running(lease.holder().clone())
    );
    assert!(
        clock
            .acquire(100, 123, Some("%41".into()), "0.1.0")
            .unwrap()
            .is_none()
    );
    assert!(clock.acquire(30_099, 456, None, "0.1.0").unwrap().is_none());
    assert!(lease.renew(20_000).unwrap());
    assert_eq!(lease.holder().since_ms, 100);
    assert_eq!(lease.holder().expires_ms, 50_000);
    assert!(clock.acquire(30_100, 456, None, "0.1.0").unwrap().is_none());
    for path in [clock.directory.clone(), f.0.join("ops")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for name in ["clock.lock", "clock.json"] {
        assert_eq!(
            fs::metadata(clock.directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    lease.release().unwrap();
    assert_eq!(clock.status(20_000), ClockStatus::NoClock);
    assert!(clock.directory.join("clock.lock").exists());
}

#[test]
fn expired_or_rolled_back_lease_can_be_taken_over_without_old_owner_deleting_it() {
    for takeover in [30_100, 99] {
        let f = Fixture::new();
        let clock = f.clock();
        let mut old = clock.acquire(100, 123, None, "0.1.0").unwrap().unwrap();
        assert_eq!(clock.status(takeover), ClockStatus::NoClock);
        let new = clock
            .acquire(takeover, 456, None, "0.1.0")
            .unwrap()
            .unwrap();
        assert!(!old.renew(takeover).unwrap());
        old.release().unwrap();
        assert_eq!(
            clock.status(takeover),
            ClockStatus::Running(new.holder().clone())
        );
        new.release().unwrap();
    }
}

#[test]
fn status_is_read_only_even_for_stale_or_malformed_state() {
    let f = Fixture::new();
    let clock = f.clock();
    let lease = clock.acquire(100, 123, None, "0.1.0").unwrap().unwrap();
    let path = clock.directory.join("clock.json");
    let before = fs::read(&path).unwrap();
    assert_eq!(clock.status(30_100), ClockStatus::NoClock);
    assert_eq!(fs::read(&path).unwrap(), before);
    lease.release().unwrap();
    for bytes in [b"broken".as_slice(), br#"{"version":2}"#, &[b' '; 4097]] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(clock.status(100), ClockStatus::Unknown);
        assert!(clock.acquire(100, 123, None, "0.1.0").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn unsafe_file_types_and_publication_failure_preserve_existing_state() {
    let f = Fixture::new();
    let clock = f.clock();
    let mut lease = clock.acquire(100, 123, None, "0.1.0").unwrap().unwrap();
    let path = clock.directory.join("clock.json");
    let before = fs::read(&path).unwrap();
    fs::create_dir(clock.directory.join("clock.tmp")).unwrap();
    assert!(lease.renew(1000).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir(clock.directory.join("clock.tmp")).unwrap();
    fs::write(clock.directory.join("clock.tmp"), "abandoned publication").unwrap();
    assert!(lease.renew(1000).unwrap());
    assert!(!clock.directory.join("clock.tmp").exists());
    lease.release().unwrap();
    let foreign = f.0.join("foreign");
    fs::write(&foreign, &before).unwrap();
    symlink(&foreign, &path).unwrap();
    assert_eq!(clock.status(100), ClockStatus::Unknown);
    assert!(clock.acquire(100, 123, None, "0.1.0").is_err());
    assert_eq!(fs::read(foreign).unwrap(), before);
}

#[test]
fn a_publication_lock_reports_contention_without_changing_the_lease() {
    let f = Fixture::new();
    let clock = f.clock();
    let mut lease = clock.acquire(100, 123, None, "0.1.0").unwrap().unwrap();
    let path = clock.directory.join("clock.json");
    let before = fs::read(&path).unwrap();
    let lock = clock.lock(false).unwrap().unwrap();
    assert_eq!(clock.status(100), ClockStatus::Unknown);
    let errors = [
        clock.acquire(100, 456, None, "0.1.0").err().unwrap(),
        lease.renew(1000).err().unwrap(),
        lease.release().err().unwrap(),
    ];
    for error in errors {
        assert_eq!(error.code, "SQUAD_CRON_CLOCK_BUSY");
        assert_eq!(
            error.message,
            format!(
                "Clock is busy: could not acquire the lock at {} (nonblocking flock).",
                clock.directory.join("clock.lock").display()
            )
        );
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(lock);
    assert!(clock.acquire(100, 456, None, "0.1.0").unwrap().is_none());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn release_waits_out_a_brief_lock_holder() {
    let f = Fixture::new();
    let clock = f.clock();
    let lease = clock.acquire(100, 123, None, "0.1.0").unwrap().unwrap();
    let lock = clock.lock(false).unwrap().unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(lock);
    });
    lease.release().unwrap();
    holder.join().unwrap();
    assert_eq!(clock.status(100), ClockStatus::NoClock);
}

#[test]
fn simultaneous_clock_starts_publish_only_one_holder() {
    let f = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads: Vec<_> = [123, 456]
        .into_iter()
        .map(|pid| {
            let clock = f.clock();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                clock.acquire(100, pid, None, "0.1.0")
            })
        })
        .collect();
    barrier.wait();
    let mut leases = Vec::new();
    for thread in threads {
        match thread.join().unwrap() {
            Ok(Some(lease)) => leases.push(lease),
            Ok(None) => {}
            Err(error) => assert_eq!(error.code, "SQUAD_CRON_CLOCK_BUSY"),
        }
    }
    assert_eq!(leases.len(), 1);
    assert_eq!(
        f.clock().status(100),
        ClockStatus::Running(leases[0].holder().clone())
    );
    leases.pop().unwrap().release().unwrap();
    assert_eq!(f.clock().status(100), ClockStatus::NoClock);
}

/// A lease left by an Ops that predates the version field, written as such.
fn legacy_lease(f: &Fixture, clock: &Clock) {
    let _ = f;
    fs::create_dir_all(&clock.directory).unwrap();
    fs::write(clock.directory.join("clock.lock"), "").unwrap();
    fs::write(
        clock.directory.join("clock.json"),
        json!({"version":1,"pane":"%9","pid":9,"sinceMs":100,"expiresMs":30_100}).to_string(),
    )
    .unwrap();
}

#[test]
fn a_live_lease_of_an_ops_without_a_version_yields_to_a_current_one() {
    let f = Fixture::new();
    let clock = f.clock();
    legacy_lease(&f, &clock);
    let ClockStatus::Running(legacy) = clock.status(1_000) else {
        panic!("a legacy lease still reads as running");
    };
    assert_eq!((legacy.pid, legacy.build.as_deref()), (9, None));
    // Its owner is alive and its lease is live, yet a current Ops takes the clock.
    // A one-shot pass never displaces a running clock; a clock that keeps running does.
    assert!(
        clock
            .acquire(1_000, 456, None, "0.1.0-alpha.154")
            .unwrap()
            .is_none()
    );
    let current = clock
        .acquire_current(1_000, 456, Some("%1".into()), "0.1.0-alpha.154")
        .unwrap()
        .expect("a current Ops preempts a holder that predates versions");
    assert_eq!(current.holder().build.as_deref(), Some("0.1.0-alpha.154"));
    // The old owner finds the lease changed at its next renewal and stops.
    let mut old = Lease {
        clock: clock.clone(),
        holder: legacy,
    };
    assert!(!old.renew(2_000).unwrap());
    assert!(
        clock
            .acquire_current(2_000, 9, None, "0.1.0-alpha.154")
            .unwrap()
            .is_none(),
        "the current holder keeps the lease"
    );
}

#[test]
fn only_an_older_semantic_version_yields_and_the_version_is_published() {
    let held = |build: &str| {
        let f = Fixture::new();
        let clock = f.clock();
        let lease = clock
            .acquire(100, 123, None, build)
            .unwrap()
            .expect("an empty clock is acquired");
        let document: Value =
            serde_json::from_slice(&fs::read(clock.directory.join("clock.json")).unwrap()).unwrap();
        assert_eq!(document["opsVersion"], build);
        assert_eq!(lease.holder().build.as_deref(), Some(build));
        (f, clock)
    };
    for (holder, contender, preempts) in [
        // Numeric prerelease identifiers compare as numbers, not text.
        ("0.1.0-alpha.9", "0.1.0-alpha.10", true),
        ("0.1.0-alpha.153", "0.1.0-alpha.154", true),
        ("0.1.0-alpha.154", "0.1.0-alpha.154", false),
        ("0.1.0-alpha.154", "0.1.0-alpha.153", false),
        ("0.2.0", "0.1.0", false),
        ("0.1.0", "0.2.0", true),
        // Anything that is not a semantic version never preempts or yields.
        ("not-a-version", "0.2.0", false),
        ("0.1.0", "not-a-version", false),
    ] {
        let (_f, clock) = held(holder);
        let took = clock
            .acquire_current(200, 456, None, contender)
            .unwrap()
            .is_some();
        assert_eq!(took, preempts, "{holder} held, {contender} contends");
    }
    let (_f, clock) = held("0.1.0");
    let unversioned = clock.acquire_current(200, 456, None, "not-a-version");
    assert!(unversioned.unwrap().is_none());
    // A one-shot pass never preempts, whatever the versions.
    assert!(clock.acquire(200, 456, None, "9.9.9").unwrap().is_none());
}

#[test]
fn a_malformed_version_field_makes_the_lease_invalid() {
    for bad in [json!(7), json!(""), json!("x".repeat(65)), json!("0.1.0\n")] {
        let f = Fixture::new();
        let clock = f.clock();
        fs::create_dir_all(&clock.directory).unwrap();
        fs::write(
            clock.directory.join("clock.json"),
            json!({"version":1,"pane":null,"pid":9,"sinceMs":100,"expiresMs":30_100,"opsVersion":bad}).to_string(),
        )
        .unwrap();
        assert!(clock.read().is_err(), "{bad}");
    }
}
