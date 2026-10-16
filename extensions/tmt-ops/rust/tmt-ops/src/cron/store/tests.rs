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
        Store::new(&self.0.join("ops")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Spend `wait` instead of the real writer budget on this test thread.
fn short_write_wait(wait: Duration) {
    WRITE_WAIT.with(|budget| budget.set(wait));
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
    assert!(!f.0.join("ops").exists());
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
    for path in [store.directory.clone(), f.0.join("ops")] {
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
fn shared_readers_overlap_without_admitting_a_writer() {
    short_write_wait(Duration::from_millis(100));
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    let reader = store.lock(false).unwrap().unwrap();
    let root = f.0.clone();
    let concurrent =
        std::thread::spawn(move || Store::new(&root.join("ops")).unwrap().read().unwrap());
    assert_eq!(store.read_locked().unwrap().jobs().len(), 1);
    assert_eq!(concurrent.join().unwrap().jobs().len(), 1);
    assert_eq!(
        store
            .update(|jobs| jobs.insert(job("product")))
            .unwrap_err()
            .code,
        "SQUAD_CRON_STORE_BUSY"
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
    drop(reader);
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c2"
    );
}

#[test]
fn a_read_waits_for_a_writer_then_observes_its_publication() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let (ready, waiting) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let root = f.0.clone();
    let writer = std::thread::spawn(move || {
        Store::new(&root.join("ops")).unwrap().update(|jobs| {
            ready.send(()).unwrap();
            hold.recv().unwrap();
            jobs.insert(job("product"))
        })
    });
    waiting.recv().unwrap();
    let (done, result) = mpsc::channel();
    let root = f.0.clone();
    let reader = std::thread::spawn(move || {
        done.send(Store::new(&root.join("ops")).unwrap().read())
            .unwrap();
    });
    // An immediate BUSY would complete this channel, rather than remain pending.
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release.send(()).unwrap();
    assert_eq!(writer.join().unwrap().unwrap(), "c2");
    assert_eq!(
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap()
            .jobs()
            .len(),
        2
    );
    reader.join().unwrap();
}

#[test]
fn a_read_returns_busy_after_its_budget_without_changing_state() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    let writer = store.lock(true).unwrap().unwrap();
    let root = f.0.clone();
    let (done, result) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let start = Instant::now();
        let read = Store::new(&root.join("ops")).unwrap().read();
        done.send((read, start.elapsed())).unwrap();
    });
    let (read, elapsed) = result.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(read.unwrap_err().code, "SQUAD_CRON_STORE_BUSY");
    assert!(
        elapsed >= READ_LOCK_WAIT,
        "read failed before its retry budget: {elapsed:?}"
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
    reader.join().unwrap();
    drop(writer);
    assert_eq!(store.read().unwrap().jobs().len(), 1);
}

#[test]
fn a_competing_edit_queues_behind_a_live_writer_and_an_abandoned_temp_is_replaced() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let (ready, waiting) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let root = f.0.clone();
    let writer = std::thread::spawn(move || {
        Store::new(&root.join("ops")).unwrap().update(|jobs| {
            ready.send(()).unwrap();
            hold.recv().unwrap();
            jobs.insert(job("product"))
        })
    });
    waiting.recv().unwrap();
    let (done, result) = mpsc::channel();
    let root = f.0.clone();
    let queued = std::thread::spawn(move || {
        done.send(
            Store::new(&root.join("ops"))
                .unwrap()
                .update(|jobs| jobs.insert(job("product"))),
        )
        .unwrap();
    });
    // An immediate BUSY would complete this channel, rather than remain pending.
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release.send(()).unwrap();
    assert_eq!(writer.join().unwrap().unwrap(), "c2");
    assert_eq!(
        result
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap(),
        "c3"
    );
    queued.join().unwrap();
    fs::write(
        store.directory.join("jobs.tmp"),
        "partial interrupted publication",
    )
    .unwrap();
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c4"
    );
    assert!(!store.directory.join("jobs.tmp").exists());
    assert_eq!(store.read().unwrap().jobs().len(), 4);
}

#[test]
fn a_write_returns_busy_after_its_budget_naming_the_cause_without_changing_state() {
    let wait = Duration::from_millis(150);
    short_write_wait(wait);
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    let holder = store.lock(true).unwrap().unwrap();
    let start = Instant::now();
    let error = store
        .update(|jobs| jobs.insert(job("product")))
        .unwrap_err();
    assert_eq!(error.code, STORE_BUSY);
    assert!(
        start.elapsed() >= wait,
        "write failed before its budget: {:?}",
        start.elapsed()
    );
    assert!(
        error.message.contains("still holds the cron store lock")
            && error.message.contains("nothing was saved"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
    drop(holder);
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c2"
    );
}

type Work<T> = Box<dyn FnOnce(Store) -> T + Send>;

/// Starts every closure together on its own `Store` over the real path and returns their results.
fn together<T: Send + 'static>(f: &Fixture, work: Vec<Work<T>>) -> Vec<T> {
    let start = std::sync::Arc::new(std::sync::Barrier::new(work.len()));
    let threads: Vec<_> = work
        .into_iter()
        .map(|work| {
            let (root, start) = (f.0.clone(), start.clone());
            std::thread::spawn(move || {
                let store = Store::new(&root.join("ops")).unwrap();
                start.wait();
                work(store)
            })
        })
        .collect();
    threads.into_iter().map(|t| t.join().unwrap()).collect()
}

#[test]
fn eight_parallel_adds_all_land_once_in_a_valid_document() {
    let f = Fixture::new();
    let adds: Vec<Work<Result<String, Error>>> = (0..8)
        .map(|_| {
            Box::new(|store: Store| store.update(|jobs| jobs.insert(job("product")))) as Work<_>
        })
        .collect();
    let mut ids: Vec<String> = together(&f, adds)
        .into_iter()
        .map(|id| id.expect("every parallel add is saved"))
        .collect();
    ids.sort();
    assert_eq!(ids, ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"]);
    let stored = f.store().read().unwrap();
    assert_eq!(stored.jobs().len(), 8);
    let mut on_disk: Vec<_> = stored.jobs().iter().map(Job::id).collect();
    on_disk.sort();
    assert_eq!(on_disk, ids);
    assert!(!f.0.join("ops/cron/jobs.tmp").exists());
}

#[test]
fn parallel_add_remove_enable_disable_and_edit_all_land() {
    let f = Fixture::new();
    let store = f.store();
    for _ in 0..5 {
        store.update(|jobs| jobs.insert(job("product"))).unwrap();
    }
    store
        .update(|jobs| {
            jobs.find_mut("product", "c4").unwrap().pause = Some(Pause {
                by: "owner-uuid".into(),
                at_ms: 7,
            });
            Ok(())
        })
        .unwrap();
    let work: Vec<Work<Result<(), Error>>> = vec![
        Box::new(|s| {
            s.update(|jobs| {
                jobs.remove("product", "c1")
                    .map(drop)
                    .ok_or_else(|| corrupt("c1"))
            })
        }),
        Box::new(|s| {
            s.update(|jobs| {
                jobs.remove("product", "c2")
                    .map(drop)
                    .ok_or_else(|| corrupt("c2"))
            })
        }),
        Box::new(|s| {
            s.update(|jobs| {
                jobs.find_mut("product", "c3").unwrap().pause = Some(Pause {
                    by: "owner-uuid".into(),
                    at_ms: 9,
                });
                Ok(())
            })
        }),
        Box::new(|s| {
            s.update(|jobs| {
                jobs.find_mut("product", "c4").unwrap().pause = None;
                Ok(())
            })
        }),
        Box::new(|s| {
            s.update(|jobs| {
                jobs.find_mut("product", "c5").unwrap().message = "edited".into();
                Ok(())
            })
        }),
        Box::new(|s| s.update(|jobs| jobs.insert(job("product"))).map(drop)),
        Box::new(|s| s.update(|jobs| jobs.insert(job("product"))).map(drop)),
        Box::new(|s| s.update(|jobs| jobs.insert(job("product"))).map(drop)),
    ];
    for result in together(&f, work) {
        result.expect("every parallel change is saved");
    }
    let stored = store.read().unwrap();
    let mut ids: Vec<_> = stored.jobs().iter().map(Job::id).collect();
    ids.sort();
    assert_eq!(ids, ["c3", "c4", "c5", "c6", "c7", "c8"]);
    let by_id = |id: &str| stored.jobs().iter().find(|j| j.id() == id).unwrap();
    assert_eq!(by_id("c3").state(), "paused");
    assert_ne!(by_id("c4").state(), "paused");
    assert_eq!(by_id("c5").message, "edited");
}

#[test]
fn a_reader_only_sees_whole_documents_while_eight_writers_publish() {
    let f = Fixture::new();
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut work: Vec<Work<Vec<usize>>> = (0..8)
        .map(|_| {
            let finished = finished.clone();
            Box::new(move |store: Store| {
                store.update(|jobs| jobs.insert(job("product"))).unwrap();
                finished.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Vec::new()
            }) as Work<Vec<usize>>
        })
        .collect();
    work.push(Box::new(move |store: Store| {
        let mut seen = Vec::new();
        while finished.load(std::sync::atomic::Ordering::SeqCst) < 8 {
            match store.read() {
                Ok(jobs) => seen.push(jobs.jobs().len()),
                // A reader may spend its budget behind a queue of writers, never read a torn file.
                Err(error) => assert_eq!(error.code, STORE_BUSY),
            }
        }
        seen
    }));
    let results = together(&f, work);
    let seen = results.last().unwrap();
    assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
    assert!(seen.iter().all(|len| *len <= 8), "{seen:?}");
    assert_eq!(f.store().read().unwrap().jobs().len(), 8);
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

#[test]
fn departure_pauses_only_the_original_room_and_never_replaces_a_pause() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            jobs.insert(job("product"))?;
            let mut other = job("other");
            other.room_id = "other-room".into();
            jobs.insert(other)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store
            .update(|jobs| jobs.pause_owner("room-uuid", "owner-uuid", 500))
            .unwrap(),
        ["c1"]
    );
    let paused = store.read().unwrap().jobs()[0].clone();
    assert_eq!(paused.revision, 2);
    assert_eq!(
        paused.pause,
        Some(Pause {
            by: "owner-uuid".into(),
            at_ms: 500
        })
    );
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    assert_eq!(
        store
            .update(|jobs| jobs.pause_owner("room-uuid", "owner-uuid", 900))
            .unwrap(),
        ["c1"]
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
    assert_eq!(store.read().unwrap().jobs()[1].pause, None);
}

#[test]
fn squad_rename_preserves_job_identity_content_pause_and_next_counter() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            jobs.insert(job("product"))?;
            jobs.pause_owner("room-uuid", "owner-uuid", 500)?;
            Ok(())
        })
        .unwrap();
    let before = store.read().unwrap().jobs()[0].clone();
    assert!(
        store
            .update(|jobs| jobs.rename_squad("product", "renamed", "room-uuid"))
            .unwrap()
    );
    let mut expected = before;
    expected.squad = "renamed".into();
    assert_eq!(store.read().unwrap().jobs(), &[expected]);
    assert_eq!(
        store.update(|jobs| jobs.insert(job("renamed"))).unwrap(),
        "c2"
    );
    assert!(
        !store
            .update(|jobs| jobs.rename_squad("product", "renamed", "room-uuid"))
            .unwrap()
    );
}

#[test]
fn rename_moves_the_counter_after_the_last_job_was_deleted() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            jobs.insert(job("product"))?;
            jobs.remove("product", "c1").unwrap();
            Ok(())
        })
        .unwrap();
    assert!(store.read().unwrap().jobs().is_empty());
    assert!(
        store
            .update(|jobs| jobs.rename_squad("product", "renamed", "room-uuid"))
            .unwrap()
    );
    assert_eq!(
        store.update(|jobs| jobs.insert(job("renamed"))).unwrap(),
        "c2"
    );
}

#[test]
fn conflicting_namespace_and_room_refuse_without_publishing_partial_edits() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            jobs.insert(job("product"))?;
            jobs.insert(job("occupied"))?;
            Ok(())
        })
        .unwrap();
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    assert_eq!(
        store
            .update(|jobs| jobs.rename_squad("product", "occupied", "room-uuid"))
            .unwrap_err()
            .code,
        "SQUAD_CRON_NAMESPACE_CONFLICT"
    );
    assert_eq!(
        store
            .update(|jobs| jobs.rename_squad("product", "new", "wrong-room"))
            .unwrap_err()
            .code,
        "SQUAD_CRON_ROOM_CONFLICT"
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
}

#[test]
fn interrupted_rename_publication_keeps_namespace_and_explicit_repair_continues_it() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = fs::read(store.directory.join("jobs.json")).unwrap();
    // A real failed temporary publication, after the caller's room CAS, must
    // retain the old namespace. Explicit repair repeats only this store step.
    fs::create_dir(store.directory.join("jobs.tmp")).unwrap();
    assert!(
        store
            .update(|jobs| jobs.rename_squad("product", "renamed", "room-uuid"))
            .is_err()
    );
    assert_eq!(fs::read(store.directory.join("jobs.json")).unwrap(), before);
    fs::remove_dir(store.directory.join("jobs.tmp")).unwrap();
    assert!(
        store
            .update(|jobs| jobs.rename_squad("product", "renamed", "room-uuid"))
            .unwrap()
    );
    assert_eq!(
        store.update(|jobs| jobs.insert(job("renamed"))).unwrap(),
        "c2"
    );
}

#[test]
fn committed_pause_is_visible_before_effect_and_locked_until_effect_returns() {
    short_write_wait(Duration::from_millis(100));
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let (paused, effect) = store
        .update_then(
            |jobs| jobs.pause_owner("room-uuid", "owner-uuid", 500),
            |paused| {
                assert_eq!(paused, &["c1"]);
                let bytes: Value =
                    serde_json::from_slice(&fs::read(store.directory.join("jobs.json")).unwrap())
                        .unwrap();
                assert_eq!(bytes["jobs"][0]["state"], "paused");
                // An independent writer cannot create a new owner job in the gap.
                assert_eq!(
                    store
                        .update(|jobs| jobs.insert(job("product")))
                        .unwrap_err()
                        .code,
                    "SQUAD_CRON_STORE_BUSY"
                );
                Err::<(), _>("leave failed")
            },
        )
        .unwrap();
    assert_eq!(paused, ["c1"]);
    assert_eq!(effect, Err("leave failed"));
    assert_eq!(store.read().unwrap().jobs()[0].state(), "paused");
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c2"
    );
}
