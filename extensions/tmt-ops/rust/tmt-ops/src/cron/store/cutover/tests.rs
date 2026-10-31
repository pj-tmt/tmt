use super::*;
use crate::cron::{Clock, ClockStatus, ScheduleInput};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
const SOURCE: &str = "11111111-1111-4111-8111-111111111111";
const ROOM: &str = "22222222-2222-4222-8222-222222222222";
const OWNER: &str = "33333333-3333-4333-8333-333333333333";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "cron-cutover-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn root(&self) -> PathBuf {
        self.0.join("ops")
    }
    fn store(&self) -> Store {
        Store::new(&self.root()).unwrap()
    }
    fn clock(&self) -> Clock {
        Clock::new(&self.root()).unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root().join("ops.db")).unwrap()
    }
    fn seed(&self) {
        self.store()
            .update(|jobs| {
                let job = Job::new(
                    ROOM.into(),
                    OWNER.into(),
                    "exact\0雪\r\nmessage".into(),
                    Schedule::parse(
                        ScheduleInput::Every {
                            duration: "10s",
                            from: None,
                        },
                        "UTC",
                        0,
                    )
                    .unwrap(),
                );
                jobs.insert(job.clone())?;
                jobs.insert(job)?;
                jobs.remove(ROOM, "c1");
                Ok(())
            })
            .unwrap();
        let mut bytes =
            serde_json::to_vec_pretty(&self.store().read().unwrap().document()).unwrap();
        bytes.extend_from_slice(b"\n  ");
        self.db()
            .execute(
                "UPDATE documents SET body=? WHERE kind='cron' AND key=''",
                [bytes],
            )
            .unwrap();
    }
    fn raw(&self) -> Vec<u8> {
        self.db()
            .query_row(
                "SELECT body FROM documents WHERE kind='cron' AND key=''",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn retained_count(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE kind='cron-cutover-source'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn freeze_retains_literal_source_and_old_writers_refuse_after_reopen() {
    let f = Fixture::new();
    f.seed();
    let old = f.store();
    let original = f.raw();
    let (hash, bytes) = f.store().cutover_source().unwrap();
    assert_eq!(bytes, original);
    let guard = f.clock().quiescent_for_cutover().unwrap();
    let frozen = f.store().freeze_for_core(&guard, SOURCE, &hash).unwrap();
    assert_eq!(frozen.bytes, original);
    assert_eq!(frozen.source_digest, hash);
    assert_eq!(old.read().unwrap_err().code, "SQUAD_CRON_STORE_NEWER");
    assert_eq!(
        old.update(|_| Ok(())).unwrap_err().code,
        "SQUAD_CRON_STORE_NEWER"
    );
    assert_eq!(f.store().frozen_for_core().unwrap(), Some(frozen.clone()));
    assert_eq!(f.retained_count(), 1);
    let value: Value = serde_json::from_slice(&frozen.bytes).unwrap();
    assert_eq!(value["counters"][ROOM], 2);
    assert_eq!(value["jobs"][0]["id"], "c2");
    assert_eq!(value["jobs"][0]["message"], "exact\0雪\r\nmessage");
    assert_eq!(
        f.clock()
            .acquire(100, 123, None, "1.0.0")
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_CLOCK_BUSY"
    );
    drop(guard);
    let lease = f.clock().acquire(100, 123, None, "1.0.0").unwrap().unwrap();
    // An old clock may acquire after a crashed adopter; the source fence still
    // prevents it from obtaining mutable jobs or resending its old document.
    assert_eq!(f.store().read().unwrap_err().code, "SQUAD_CRON_STORE_NEWER");
    lease.release().unwrap();
    assert_eq!(f.store().frozen_for_core().unwrap(), Some(frozen));
}
#[test]
fn changed_source_fails_hash_cas_without_fencing_or_exporting_it() {
    let f = Fixture::new();
    f.seed();
    let (hash, _) = f.store().cutover_source().unwrap();
    f.store()
        .update(|jobs| {
            jobs.find_mut(ROOM, "c2").unwrap().message = "New definition".into();
            Ok(())
        })
        .unwrap();
    let changed = f.raw();
    let guard = f.clock().quiescent_for_cutover().unwrap();
    assert_eq!(
        f.store()
            .freeze_for_core(&guard, SOURCE, &hash)
            .unwrap_err()
            .code,
        "SQUAD_CRON_CUTOVER_CONFLICT"
    );
    assert_eq!(f.raw(), changed);
    assert_eq!(f.retained_count(), 0);
    assert!(f.store().frozen_for_core().unwrap().is_none());
    assert_eq!(
        f.store().read().unwrap().jobs()[0].message,
        "New definition"
    );
}
#[test]
fn retained_source_failure_rolls_back_the_old_writer_fence() {
    let f = Fixture::new();
    f.seed();
    let original = f.raw();
    let (hash, _) = f.store().cutover_source().unwrap();
    f.db().execute_batch("CREATE TRIGGER fail_source BEFORE INSERT ON documents WHEN NEW.kind='cron-cutover-source' BEGIN SELECT RAISE(ABORT,'injected retained-source failure'); END;").unwrap();
    let guard = f.clock().quiescent_for_cutover().unwrap();
    assert!(f.store().freeze_for_core(&guard, SOURCE, &hash).is_err());
    assert_eq!(f.raw(), original);
    assert_eq!(f.retained_count(), 0);
    assert_eq!(f.store().read().unwrap().jobs().len(), 1);
    assert!(f.store().frozen_for_core().unwrap().is_none());
}
#[test]
fn expired_holder_is_not_quiescence_and_another_root_guard_cannot_freeze() {
    let f = Fixture::new();
    f.seed();
    let lease = f.clock().acquire(100, 123, None, "1.0.0").unwrap().unwrap();
    assert_eq!(f.clock().status(100_000), ClockStatus::NoClock);
    assert_eq!(
        f.clock().quiescent_for_cutover().err().unwrap().code,
        "SQUAD_CRON_CUTOVER_CLOCK_PRESENT"
    );
    lease.release().unwrap();
    let other = Fixture::new();
    let guard = other.clock().quiescent_for_cutover().unwrap();
    let (hash, _) = f.store().cutover_source().unwrap();
    assert_eq!(
        f.store()
            .freeze_for_core(&guard, SOURCE, &hash)
            .unwrap_err()
            .code,
        "SQUAD_CRON_CUTOVER_CONFLICT"
    );
    assert_eq!(f.retained_count(), 0);
}
