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
        Clock::new(&self.0).unwrap()
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
    assert!(!f.0.join("squad").exists());
    let mut lease = clock
        .acquire(100, 123, Some("%41".into()))
        .unwrap()
        .unwrap();
    assert_eq!(
        clock.status(100),
        ClockStatus::Running(lease.holder().clone())
    );
    assert!(
        clock
            .acquire(100, 123, Some("%41".into()))
            .unwrap()
            .is_none()
    );
    assert!(clock.acquire(30_099, 456, None).unwrap().is_none());
    assert!(lease.renew(20_000).unwrap());
    assert_eq!(lease.holder().since_ms, 100);
    assert_eq!(lease.holder().expires_ms, 50_000);
    assert!(clock.acquire(30_100, 456, None).unwrap().is_none());
    for path in [clock.directory.clone(), f.0.join("squad")] {
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
        let mut old = clock.acquire(100, 123, None).unwrap().unwrap();
        assert_eq!(clock.status(takeover), ClockStatus::NoClock);
        let new = clock.acquire(takeover, 456, None).unwrap().unwrap();
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
    let lease = clock.acquire(100, 123, None).unwrap().unwrap();
    let path = clock.directory.join("clock.json");
    let before = fs::read(&path).unwrap();
    assert_eq!(clock.status(30_100), ClockStatus::NoClock);
    assert_eq!(fs::read(&path).unwrap(), before);
    lease.release().unwrap();
    for bytes in [b"broken".as_slice(), br#"{"version":2}"#, &[b' '; 4097]] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(clock.status(100), ClockStatus::Unknown);
        assert!(clock.acquire(100, 123, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn unsafe_file_types_and_publication_failure_preserve_existing_state() {
    let f = Fixture::new();
    let clock = f.clock();
    let mut lease = clock.acquire(100, 123, None).unwrap().unwrap();
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
    assert!(clock.acquire(100, 123, None).is_err());
    assert_eq!(fs::read(foreign).unwrap(), before);
}

#[test]
fn a_publication_lock_is_nonblocking_and_never_authorizes_a_second_clock() {
    let f = Fixture::new();
    let clock = f.clock();
    let _lease = clock.acquire(100, 123, None).unwrap().unwrap();
    let _lock = clock.lock(false).unwrap().unwrap();
    assert_eq!(clock.status(100), ClockStatus::Unknown);
    assert_eq!(
        clock.acquire(100, 456, None).err().unwrap().code,
        "SQUAD_CRON_CLOCK_BUSY"
    );
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
                clock.acquire(100, pid, None)
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
