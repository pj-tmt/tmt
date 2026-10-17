use super::*;
use crate::cron::ScheduleInput;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    sync::mpsc,
    time::{Duration, Instant},
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
    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.0.join("ops/ops.db")).unwrap()
    }
    /// The stored jobs document, read behind the store's back.
    fn stored(&self) -> Vec<u8> {
        self.database()
            .query_row(
                "SELECT body FROM documents WHERE kind = 'cron' AND key = ''",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn overwrite(&self, bytes: &[u8]) {
        let changed = self
            .database()
            .execute(
                "UPDATE documents SET body = ?1 WHERE kind = 'cron'",
                [bytes],
            )
            .unwrap();
        assert_eq!(changed, 1);
    }
    fn legacy(&self) -> PathBuf {
        self.0.join("ops/cron/jobs.json")
    }
    /// Ops creates its state directories private; so does the fixture for a legacy tree.
    fn legacy_directory(&self) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.0.join("ops/cron"))
            .unwrap();
    }
    /// Archived copies of the legacy file, sorted by name.
    fn archived(&self) -> Vec<PathBuf> {
        let mut found: Vec<_> = fs::read_dir(self.0.join("ops/cron"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("jobs.json.migrated-")
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
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
    assert_eq!(
        fs::metadata(f.0.join("ops")).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(f.0.join("ops/ops.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn rejected_edits_and_corruption_preserve_the_stored_bytes() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = f.stored();
    let fail = store.update(|jobs| {
        jobs.remove("product", "c1");
        Err::<(), _>(corrupt("rejected"))
    });
    assert!(fail.is_err());
    assert_eq!(f.stored(), before);
    assert!(
        store
            .update(|jobs| {
                jobs.find_mut("product", "c1").unwrap().revision = 0;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(f.stored(), before);
    for bytes in [
        b"broken".as_slice(),
        br#"{"version":2,"counters":{},"jobs":[]}"#,
    ] {
        f.overwrite(bytes);
        assert_eq!(store.read().unwrap_err().code, "SQUAD_CRON_STORE_INVALID");
        assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
        assert_eq!(f.stored(), bytes);
    }
}

#[test]
fn a_read_waits_for_a_writer_then_observes_its_commit() {
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
fn a_competing_edit_queues_behind_a_live_writer() {
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
    assert_eq!(store.read().unwrap().jobs().len(), 3);
}

#[test]
fn a_read_and_a_write_return_busy_after_the_budget_without_changing_state() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = f.stored();
    // Another process holding the database's write transaction for longer than the budget.
    let holder = f.database();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = Instant::now();
    let read = std::thread::spawn({
        let root = f.0.clone();
        move || Store::new(&root.join("ops")).unwrap().read()
    });
    let write = std::thread::spawn({
        let root = f.0.clone();
        move || {
            Store::new(&root.join("ops"))
                .unwrap()
                .update(|jobs| jobs.insert(job("product")))
        }
    });
    let (read, write) = (read.join().unwrap(), write.join().unwrap());
    assert!(
        start.elapsed() >= Duration::from_secs(4),
        "failed before the budget: {:?}",
        start.elapsed()
    );
    assert_eq!(read.unwrap_err().code, STORE_BUSY);
    let error = write.unwrap_err();
    assert_eq!(error.code, STORE_BUSY);
    assert!(
        error.message.contains("still holds") && error.message.contains("nothing was saved"),
        "{}",
        error.message
    );
    assert_eq!(f.stored(), before);
    holder.execute_batch("ROLLBACK").unwrap();
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
fn a_reader_only_sees_whole_documents_while_eight_writers_commit() {
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
                // A reader may spend its budget behind a queue of writers, never read a torn document.
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
    f.overwrite(document.to_string().as_bytes());
    assert!(store.read().is_err());
}

#[test]
fn a_newer_database_and_an_unsafe_database_path_refuse_without_changing_anything() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = f.stored();
    // A newer Ops appended a migration this binary does not know.
    f.database()
        .execute(
            "INSERT INTO _tmt_state_migrations(version, name, sql) VALUES(2, 'later', 'SELECT 1')",
            [],
        )
        .unwrap();
    for error in [
        store.read().unwrap_err(),
        store
            .update(|jobs| jobs.insert(job("product")))
            .unwrap_err(),
    ] {
        assert_eq!(error.code, "SQUAD_CRON_STORE_NEWER");
        assert!(error.message.contains("Upgrade Ops"), "{}", error.message);
    }
    assert_eq!(f.stored(), before);
    let versions: i64 = f
        .database()
        .query_row("SELECT COUNT(*) FROM _tmt_state_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(versions, 2);

    // A link in place of the database is never followed.
    let foreign = f.0.join("foreign");
    fs::copy(f.0.join("ops/ops.db"), &foreign).unwrap();
    let copy = fs::read(&foreign).unwrap();
    fs::remove_file(f.0.join("ops/ops.db")).unwrap();
    symlink(&foreign, f.0.join("ops/ops.db")).unwrap();
    assert!(store.read().is_err());
    assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), copy);
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
    let before = f.stored();
    assert_eq!(
        store
            .update(|jobs| jobs.pause_owner("room-uuid", "owner-uuid", 900))
            .unwrap(),
        ["c1"]
    );
    assert_eq!(f.stored(), before);
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
fn conflicting_namespace_and_room_refuse_without_saving_partial_edits() {
    let f = Fixture::new();
    let store = f.store();
    store
        .update(|jobs| {
            jobs.insert(job("product"))?;
            jobs.insert(job("occupied"))?;
            Ok(())
        })
        .unwrap();
    let before = f.stored();
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
    assert_eq!(f.stored(), before);
}

#[test]
fn a_rename_that_fails_after_the_edit_saves_nothing_and_the_explicit_repair_continues_it() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let before = f.stored();
    // The caller's own step after the edit fails: the whole change is discarded, so the
    // old namespace stays. Explicit repair repeats only this store step.
    assert!(
        store
            .update(|jobs| {
                jobs.rename_squad("product", "renamed", "room-uuid")?;
                Err::<(), _>(corrupt("the room step failed"))
            })
            .is_err()
    );
    assert_eq!(f.stored(), before);
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
fn the_pause_is_committed_before_the_departure_and_a_job_added_meanwhile_is_paused_after() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let (paused, effect) = store
        .pause_owner_then("room-uuid", "owner-uuid", 500, |paused| {
            assert_eq!(paused, ["c1"]);
            // Committed already: an independent handle (another process) sees it.
            assert_eq!(f.store().read().unwrap().jobs()[0].state(), "paused");
            // Nothing is held across the departure, which runs a core command.
            assert_eq!(
                f.store()
                    .update(|jobs| jobs.insert(job("product")))
                    .unwrap(),
                "c2"
            );
            Err::<(), _>("leave failed")
        })
        .unwrap();
    assert_eq!(paused, ["c1"]);
    // The departure failed, yet the pause it ran beside is kept, and the job the owner
    // added in the meantime is paused by the second pass.
    assert_eq!(effect, Err("leave failed"));
    let read = store.read().unwrap();
    assert_eq!(read.jobs().len(), 2);
    assert!(read.jobs().iter().all(|job| job.state() == "paused"));
}

#[test]
fn a_departure_that_panics_leaves_the_committed_pause_in_place() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store
            .pause_owner_then("room-uuid", "owner-uuid", 500, |_| panic!("leave exploded"))
            .map(drop)
    }));
    assert!(caught.is_err());
    assert_eq!(store.read().unwrap().jobs()[0].state(), "paused");
}

#[test]
fn a_pause_that_cannot_be_saved_runs_no_departure() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    f.overwrite(b"broken");
    let mut ran = false;
    let error = store
        .pause_owner_then("room-uuid", "owner-uuid", 500, |_| ran = true)
        .unwrap_err();
    assert_eq!(error.code, "SQUAD_CRON_STORE_INVALID");
    assert!(!ran);
}

#[test]
fn the_first_use_after_an_upgrade_imports_jobs_json_and_keeps_the_old_file_renamed() {
    let f = Fixture::new();
    let seeded = {
        let store = f.store();
        store.update(|jobs| jobs.insert(job("product"))).unwrap();
        f.stored()
    };
    // Rewind to the previous release: the document lives in jobs.json and no database exists.
    for name in ["ops.db", "ops.db-wal", "ops.db-shm"] {
        let _ = fs::remove_file(f.0.join("ops").join(name));
    }
    f.legacy_directory();
    fs::write(f.legacy(), &seeded).unwrap();
    let store = f.store();
    let read = store.read().unwrap();
    assert_eq!(read.jobs().len(), 1);
    assert_eq!(read.jobs()[0].id(), "c1");
    assert_eq!(f.stored(), seeded);
    assert!(!f.legacy().exists());
    let archived = f.archived();
    assert_eq!(archived.len(), 1, "{archived:?}");
    assert_eq!(fs::read(&archived[0]).unwrap(), seeded);
    // The counters travelled with the document: the next id is not reused.
    assert_eq!(
        store.update(|jobs| jobs.insert(job("product"))).unwrap(),
        "c2"
    );
    assert_eq!(f.archived(), archived);
}

#[test]
fn a_corrupt_legacy_file_refuses_and_leaves_everything_in_place() {
    let f = Fixture::new();
    f.legacy_directory();
    fs::write(f.legacy(), b"not json").unwrap();
    let store = f.store();
    assert_eq!(store.read().unwrap_err().code, "SQUAD_CRON_STORE_INVALID");
    assert!(store.update(|jobs| jobs.insert(job("product"))).is_err());
    assert_eq!(fs::read(f.legacy()).unwrap(), b"not json");
    assert!(f.archived().is_empty());
    let rows: i64 = f
        .database()
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_crash_between_the_commit_and_the_rename_finishes_the_rename_and_trusts_the_store() {
    let f = Fixture::new();
    let store = f.store();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let committed = f.stored();
    // The process died after committing the import: the row exists and the old file does too,
    // even holding different bytes (an older Ops that kept running).
    f.legacy_directory();
    fs::write(f.legacy(), b"stale old document").unwrap();
    assert_eq!(store.read().unwrap().jobs().len(), 1);
    assert_eq!(f.stored(), committed);
    assert!(!f.legacy().exists());
    let archived = f.archived();
    assert_eq!(archived.len(), 1);
    assert_eq!(fs::read(&archived[0]).unwrap(), b"stale old document");
    // A second leftover never overwrites the first archive.
    fs::write(f.legacy(), b"another leftover").unwrap();
    store.update(|jobs| jobs.insert(job("product"))).unwrap();
    let archived = f.archived();
    assert_eq!(archived.len(), 2, "{archived:?}");
    let mut contents: Vec<_> = archived.iter().map(|p| fs::read(p).unwrap()).collect();
    contents.sort();
    assert_eq!(
        contents,
        [b"another leftover".to_vec(), b"stale old document".to_vec()]
    );
}

#[test]
fn eight_processes_importing_at_once_leave_one_document_and_one_archive() {
    let f = Fixture::new();
    let document = Jobs::default().document();
    f.legacy_directory();
    fs::write(f.legacy(), document.to_string()).unwrap();
    let work: Vec<Work<Result<String, Error>>> = (0..8)
        .map(|_| {
            Box::new(|store: Store| store.update(|jobs| jobs.insert(job("product")))) as Work<_>
        })
        .collect();
    let mut ids: Vec<String> = together(&f, work)
        .into_iter()
        .map(|id| id.expect("every add is saved"))
        .collect();
    ids.sort();
    assert_eq!(ids, ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"]);
    assert!(!f.legacy().exists());
    let archived = f.archived();
    assert_eq!(archived.len(), 1, "{archived:?}");
    assert_eq!(
        fs::read(&archived[0]).unwrap(),
        document.to_string().as_bytes()
    );
}

#[test]
fn a_commit_that_timed_out_is_reported_as_uncertain_not_as_a_plain_busy() {
    let error = refused(Refusal {
        cause: Cause::Busy,
        message:
            "Another Ops process still holds the state database, so the change may not be saved."
                .into(),
        may_not_be_saved: true,
    });
    assert_eq!(error.code, "SQUAD_CRON_STORE_UNCERTAIN");
    assert!(error.message.contains("List the jobs before repeating"));
    let busy = refused(Refusal {
        cause: Cause::Busy,
        message: "Another Ops process still holds the state database, so nothing was saved.".into(),
        may_not_be_saved: false,
    });
    assert_eq!(busy.code, STORE_BUSY);
}
