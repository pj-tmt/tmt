use super::*;
use crate::cron::ScheduleInput;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::mpsc,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "squad-cron-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> Store {
        Store::new(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn job(squad: &str) -> Job {
    Job::new(
        squad.into(),
        "room-uuid".into(),
        "owner-uuid".into(),
        "literal {time}\n!\0 message".into(),
        Schedule::parse(
            ScheduleInput::Every {
                duration: "30m",
                from: None,
            },
            "UTC",
            123,
        )
        .unwrap(),
    )
}

#[test]
fn reads_create_nothing_and_commits_preserve_exact_content_and_non_reused_ids() {
    let f = Fixture::new();
    let store = f.store();
    assert!(store.read().unwrap().jobs().is_empty());
    assert!(!f.0.join("squad").exists());
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c1"
    );
    let read = store.read().unwrap();
    assert_eq!(read.jobs()[0].message, "literal {time}\n!\0 message");
    assert_eq!(read.jobs()[0].schedule.document()["anchorMs"], 123);
    store
        .update(|jobs| {
            jobs.remove("product", "c1").unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c2"
    );
    assert_eq!(
        store.update(|jobs| jobs.insert(job("infra"))).unwrap(),
        "c1"
    );
    for path in [store.directory.clone(), f.0.join("squad")] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for name in ["jobs.json", "jobs.lock"] {
        assert_eq!(
            fs::metadata(store.directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn rejected_edits_and_corruption_preserve_the_published_bytes() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let path = store.directory.join("jobs.json");
    let before = fs::read(&path).unwrap();
    let fail = store.update(|jobs| {
        jobs.remove("product", "c1");
        Err::<(), _>(corrupt("rejected"))
    });
    assert!(fail.is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        store
            .update(|jobs| {
                jobs.find_mut("product", "c1").unwrap().revision = 0;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    for bytes in [
        b"broken".as_slice(),
        br#"{"version":2,"counters":{},"jobs":[]}"#,
    ] {
        fs::write(&path, bytes).unwrap();
        assert_eq!(store.read().unwrap_err().code, "SQUAD_CRON_STORE_INVALID");
        assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn a_live_lock_refuses_competing_edits_and_an_abandoned_temp_is_replaced() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let (ready, waiting) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let root = f.0.clone();
    let writer = std::thread::spawn(move || {
        Store::new(&root).unwrap().update(|jobs| {
            ready.send(()).unwrap();
            hold.recv().unwrap();
            jobs.insert(job("product"))
        })
    });
    waiting.recv().unwrap();
    assert_eq!(
        store
            .update(|jobs| jobs.insert(job("product")))
            .unwrap_err()
            .code,
        "SQUAD_CRON_STORE_BUSY"
    );
    release.send(()).unwrap();
    assert_eq!(writer.join().unwrap().unwrap(), "c2");
    fs::write(
        store.directory.join("jobs.tmp"),
        "partial interrupted publication",
    )
    .unwrap();
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c3"
    );
    assert!(!store.directory.join("jobs.tmp").exists());
    assert_eq!(store.read().unwrap().jobs().len(), 3);
}

#[test]
fn ownership_states_and_invalid_counters_are_checked_on_reload() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            let id = jobs.insert(job("product"))?;
            let j = jobs.find_mut("product", &id).unwrap();
            j.pause = Some(Pause {
                by: "lead-uuid".into(),
                at_ms: 1000,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(store.read().unwrap().jobs()[0].state(), "paused");
    store
        .update(|jobs| {
            jobs.find_mut("product", "c1").unwrap().owner_id = None;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.read().unwrap().jobs()[0].state(), "no owner");
    let mut document = store.read().unwrap().document();
    document["counters"]["product"] = json!(0);
    fs::write(store.directory.join("jobs.json"), document.to_string()).unwrap();
    assert!(store.read().is_err());
}

#[test]
fn unsafe_file_types_and_failed_publication_do_not_replace_existing_state() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let path = store.directory.join("jobs.json");
    let before = fs::read(&path).unwrap();
    fs::create_dir(store.directory.join("jobs.tmp")).unwrap();
    assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir(store.directory.join("jobs.tmp")).unwrap();
    fs::remove_file(&path).unwrap();
    let foreign = f.0.join("foreign");
    fs::write(&foreign, &before).unwrap();
    symlink(&foreign, &path).unwrap();
    assert!(store.read().is_err());
    assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), before);
}
