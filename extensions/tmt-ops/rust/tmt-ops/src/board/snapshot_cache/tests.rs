use super::*;
use crate::board::app::tests::snapshot;
use std::{
    os::unix::fs::symlink,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ops-snapshot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join("ops"))
            .unwrap();
        Self(root)
    }
    fn store(&self) -> Store {
        Store::new(&self.0).unwrap()
    }
    fn path(&self) -> PathBuf {
        self.0.join("ops/cache/board/squad-product.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn rooms() -> Rooms {
    [("product".into(), ROOM.into())].into()
}
fn display(name: &str) -> Display {
    Display::project(
        &snapshot(
            "product",
            json!([{"title": null, "rows": [{"name": name, "fields": {"task": "ship"}}]}]),
        ),
        &rooms(),
        123,
    )
    .unwrap()
}
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn legacy_root_writes_create_no_ops_state_until_migration_creates_it() {
    let fixture = Fixture::new();
    fs::remove_dir(fixture.0.join("ops")).unwrap();
    let legacy = fixture.0.join("squad");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("clock.json"), b"legacy holder").unwrap();
    let store = fixture.store();
    store.write(&display("alice"), || true).unwrap();
    assert!(!fixture.0.join("ops").exists());
    assert!(store.load("product", &rooms()).is_none());
    assert_eq!(
        fs::read(legacy.join("clock.json")).unwrap(),
        b"legacy holder"
    );
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);

    // Only the migration owner supplies this directory; its mode is preserved.
    fs::DirBuilder::new()
        .mode(0o755)
        .create(fixture.0.join("ops"))
        .unwrap();
    store.write(&display("alice"), || true).unwrap();
    assert!(store.load("product", &rooms()).is_some());
    assert_eq!(mode(&fixture.0.join("ops")), 0o755);
    fs::remove_dir_all(fixture.0.join("ops")).unwrap();
    store.write(&display("bob"), || true).unwrap();
    assert!(
        !fixture.0.join("ops").exists(),
        "a retained store cannot recreate Ops"
    );
}

#[test]
fn publication_does_not_recreate_a_parent_removed_after_admission() {
    let fixture = Fixture::new();
    fixture.store().write(&display("alice"), || true).unwrap();
    fs::remove_dir_all(fixture.0.join("ops")).unwrap();
    assert_eq!(
        cache::replace_if(&fixture.path(), b"candidate", || Ok(()))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!fixture.0.join("ops").exists());
}

#[test]
fn closed_projection_round_trips_without_action_or_detail_content() {
    let fixture = Fixture::new();
    let mut snapshot = snapshot(
        "product",
        json!([{"title": "Members", "rows": [{
            "id": "FORBIDDEN-id", "name": "alice", "squad": "product", "presence": "FORBIDDEN-presence", "state": "working",
            "pane": "FORBIDDEN-pane", "tty": "FORBIDDEN-tty", "pending": "FORBIDDEN-question",
            "fields": {"role": "FORBIDDEN-role", "task": "ship", "notes": "FORBIDDEN-note", "custom": "FORBIDDEN-custom", "pr_link": "FORBIDDEN-link"},
            "waitingOnYou": [{"requestId": "FORBIDDEN-request", "receipt": "FORBIDDEN-receipt", "body": "FORBIDDEN-body"}],
            "annotation": {"text": "FORBIDDEN-annotation"}, "checklist": "FORBIDDEN-checklist", "resume": {"id": "FORBIDDEN-session"},
        }]}]),
    );
    snapshot.tabs = vec!["product".into(), tabs::ALL.into()];
    snapshot.attention.insert(
        "product".into(),
        crate::attention::Attention {
            waiting: 1,
            blocked: 0,
        },
    );
    let view = snapshot.view.as_mut().unwrap();
    view.notes = super::super::app::Notes::Text("FORBIDDEN-notebook".into());
    view.replies = vec![json!({"body": "FORBIDDEN-reply"})];
    view.document["columns"] = json!([{"field": "custom", "from": "FORBIDDEN-source"}]);
    view.document["squad"]["lead"] = json!({"name": "lead", "id": "FORBIDDEN-lead-id"});
    let projected = Display::project(&snapshot, &rooms(), 123).unwrap();
    fixture.store().write(&projected, || true).unwrap();
    let bytes = fs::read(fixture.path()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("FORBIDDEN"));
    let loaded = fixture.store().load("product", &rooms()).unwrap();
    assert_eq!(loaded.0, projected.0);
    assert_eq!(
        loaded.0["view"]["sections"][0]["rows"][0],
        json!({
            "name": "alice", "squad": "product", "state": "working", "task": "ship", "waiting": true,
        })
    );
    assert_eq!(
        loaded.0["attention"]["product"],
        json!({"waiting": 1, "blocked": 0})
    );
}

#[test]
fn home_keeps_only_painted_counts_names_and_age_evidence() {
    let fixture = Fixture::new();
    let mut snapshot = snapshot(
        tabs::ALL,
        json!([{"rows": [{"name": "unpainted", "fields": {"task": "FORBIDDEN-home-task"}}]}]),
    );
    snapshot.view.as_mut().unwrap().home = Some(home::Home {
        summary: home::Counts {
            members: 2,
            waiting: 1,
            ..Default::default()
        },
        windows: crate::config::TokenWindow::DEFAULTS,
        sections: vec![home::MemberSection {
            key: "blocked".into(),
            rows: vec![home::MemberRow {
                squad: "product".into(),
                member: json!({"name": "alice", "id": "FORBIDDEN-id", "pending": "FORBIDDEN-pending"}),
                lead: Some("FORBIDDEN-lead-id".into()),
                age: Some(home::Age { since_ms: 100 }),
            }],
        }],
        squads: vec![home::SquadLine {
            squad: "product".into(),
            lead: Some(json!({"name": "lead", "id": "FORBIDDEN-id"})),
            counts: home::Counts {
                members: 2,
                waiting: 1,
                ..Default::default()
            },
            members: home::Counts {
                members: 1,
                waiting: 1,
                ..Default::default()
            },
        }],
        failures: vec![],
        incomplete: false,
    });
    let value = Display::project(&snapshot, &rooms(), 123).unwrap();
    fixture.store().write(&value, || true).unwrap();
    let loaded = fixture.store().load(tabs::ALL, &rooms()).unwrap();
    assert_eq!(loaded.0["view"]["sections"], json!([]));
    assert_eq!(
        loaded.0["view"]["home"]["sections"][0]["rows"][0],
        json!({"name":"alice", "squad":"product", "age":{"sinceMs":100}})
    );
    assert!(!loaded.0.to_string().contains("FORBIDDEN"));
}

#[test]
fn namespace_mapping_is_bounded_and_disjoint_including_home_named_all_and_user_tabs() {
    let keys = [
        tabs::ALL,
        tabs::LEADS,
        "all",
        "leads",
        "home",
        "product",
        "@tab:home",
        "@tab:product",
    ];
    let names = keys.map(|key| filename(key).unwrap());
    assert_eq!(
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        keys.len()
    );
    assert_eq!(names[0], "home.json");
    assert_eq!(names[2], "squad-all.json");
    assert_eq!(filename(&"a".repeat(24)).unwrap().len(), 35);
    for key in [
        "",
        "/product",
        "../product",
        "@tab:../home",
        "@tab:all",
        "@tab:leads",
        "@unknown",
        "PRODUCT",
        &"a".repeat(25),
    ] {
        assert!(filename(key).is_none(), "{key}");
    }
}

#[test]
fn absent_corrupt_truncated_wrong_tab_versions_room_changes_and_unknown_fields_are_misses() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.load("product", &rooms()).is_none());
    assert!(
        !fixture.0.join("ops/cache").exists(),
        "a load creates nothing"
    );
    store.write(&display("alice"), || true).unwrap();
    let original = fs::read(fixture.path()).unwrap();
    for bytes in [b"{broken".as_slice(), &original[..original.len() / 2]] {
        fs::write(fixture.path(), bytes).unwrap();
        assert!(store.load("product", &rooms()).is_none());
    }
    let value: Value = serde_json::from_slice(&original).unwrap();
    for (field, new) in [
        ("version", json!(0)),
        ("version", json!(2)),
        ("tabKey", json!("other")),
        ("writtenAtMs", json!("today")),
        ("notes", json!("unknown")),
    ] {
        let mut invalid = value.clone();
        invalid[field] = new;
        fs::write(fixture.path(), invalid.to_string()).unwrap();
        assert!(store.load("product", &rooms()).is_none(), "{field}");
    }
    let mut invalid = value.clone();
    invalid["view"]["sections"][0]["rows"][0]["receipt"] = json!("extra");
    fs::write(fixture.path(), invalid.to_string()).unwrap();
    assert!(store.load("product", &rooms()).is_none());
    fs::write(fixture.path(), original).unwrap();
    let changed = [(
        "product".into(),
        "22222222-2222-4222-8222-222222222222".into(),
    )]
    .into();
    assert!(store.load("product", &changed).is_none());
    assert!(store.load("product", &Rooms::new()).is_none());
    let mut added = rooms();
    added.insert("new".into(), ROOM.into());
    assert!(store.load("product", &added).is_none());
    // Old age does not grant freshness or erase the last usable display.
    assert_eq!(
        store.load("product", &rooms()).unwrap().0["writtenAtMs"],
        123
    );
}

#[test]
fn byte_limit_preserves_the_previous_file_and_oversized_reads_are_ignored() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.write(&display("old"), || true).unwrap();
    let original = fs::read(fixture.path()).unwrap();
    let mut huge = display("huge");
    huge.0["view"]["sections"][0]["rows"] = json!(vec![json!({"name": "a".repeat(1024)}); 1100]);
    assert!(store.write(&huge, || true).is_err());
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
    let mut bytes = Bytes(Vec::new());
    assert_eq!(bytes.write(&vec![b'x'; LIMIT]).unwrap(), LIMIT);
    assert!(bytes.write(b"x").is_err());
    assert_eq!(bytes.0.len(), LIMIT);
    fs::write(fixture.path(), vec![b' '; LIMIT + 1]).unwrap();
    assert!(store.load("product", &rooms()).is_none());
    assert_eq!(
        fs::read_dir(fixture.path().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn permissions_are_private_and_unsafe_existing_paths_are_skipped_without_repair() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let root_mode = mode(&fixture.0);
    store.write(&display("alice"), || true).unwrap();
    assert_eq!(mode(&fixture.0), root_mode);
    for path in [
        fixture.0.join("ops"),
        fixture.0.join("ops/cache"),
        fixture.0.join("ops/cache/board"),
    ] {
        assert_eq!(mode(&path), 0o700);
    }
    assert_eq!(mode(&fixture.path()), 0o600);
    fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load("product", &rooms()).is_none());
    assert!(store.write(&display("new"), || true).is_err());
    assert_eq!(mode(&fixture.path()), 0o644);
    fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(
        fixture.0.join("ops/cache"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(store.load("product", &rooms()).is_none());
    assert!(store.write(&display("new"), || true).is_err());
    fs::set_permissions(
        fixture.0.join("ops/cache"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::remove_file(fixture.path()).unwrap();
    let victim = fixture.0.join("victim");
    fs::write(&victim, b"preserve").unwrap();
    symlink(&victim, fixture.path()).unwrap();
    assert!(store.load("product", &rooms()).is_none());
    assert!(store.write(&display("new"), || true).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"preserve");
    fs::remove_file(fixture.path()).unwrap();
    fs::remove_dir(fixture.0.join("ops/cache/board")).unwrap();
    symlink(&fixture.0, fixture.0.join("ops/cache/board")).unwrap();
    assert!(store.write(&display("new"), || true).is_err());
    assert!(store.load("product", &rooms()).is_none());
}

#[test]
fn cancellation_before_and_during_staging_preserves_bytes_and_cleans_only_owned_temporary() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.write(&display("old"), || true).unwrap();
    let original = fs::read(fixture.path()).unwrap();
    store.write(&display("new"), || false).unwrap();
    let checks = AtomicU64::new(0);
    assert!(
        store
            .write(&display("new"), || checks.fetch_add(1, Ordering::Relaxed)
                == 0)
            .is_err()
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
    assert_eq!(
        fs::read_dir(fixture.path().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    let foreign = fixture.path().with_extension("foreign.tmp");
    fs::write(&foreign, b"preserve").unwrap();
    cache::replace_if(&fixture.path(), b"new", || {
        Err(io::ErrorKind::PermissionDenied.into())
    })
    .unwrap_err();
    assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
    assert_eq!(fs::read(fixture.path()).unwrap(), original);
    assert_eq!(
        fs::read_dir(fixture.path().parent().unwrap())
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn failed_and_partial_views_never_project_a_candidate() {
    let mut snapshot = snapshot("product", json!([]));
    snapshot.view.as_mut().unwrap().document["partial"] = json!(true);
    assert!(Display::project(&snapshot, &rooms(), 100).is_none());
    snapshot.view = Err("unavailable".into());
    assert!(Display::project(&snapshot, &rooms(), 100).is_none());
}

#[test]
fn two_in_process_writers_use_distinct_staging_and_last_rename_wins() {
    let fixture = Fixture::new();
    fixture.store().write(&display("initial"), || true).unwrap();
    let staged = Arc::new(Barrier::new(3));
    let finish = Arc::new(Barrier::new(2));
    let first_finish = Arc::new(Barrier::new(2));
    let path = fixture.path();
    std::thread::scope(|scope| {
        let staged_a = staged.clone();
        let path_a = path.clone();
        let first_finish_a = first_finish.clone();
        let a = scope.spawn(move || {
            cache::replace_if(&path_a, b"first", || {
                staged_a.wait();
                first_finish_a.wait();
                Ok(())
            })
            .unwrap()
        });
        let staged_b = staged.clone();
        let finish_b = finish.clone();
        let path_b = path.clone();
        let b = scope.spawn(move || {
            cache::replace_if(&path_b, b"last", || {
                staged_b.wait();
                finish_b.wait();
                Ok(())
            })
            .unwrap()
        });
        staged.wait();
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            3,
            "both staging files coexist"
        );
        first_finish.wait();
        a.join().unwrap();
        finish.wait();
        b.join().unwrap();
    });
    assert_eq!(fs::read(&path).unwrap(), b"last");
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn concurrent_complete_json_publications_never_expose_a_torn_file() {
    let fixture = Fixture::new();
    fixture.store().write(&display("initial"), || true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let root = &fixture.0;
        let reader = scope.spawn(|| {
            start.wait();
            let mut count = 0;
            while !done.load(Ordering::Acquire) {
                let loaded = fixture
                    .store()
                    .load("product", &rooms())
                    .expect("always complete valid JSON");
                assert!(matches!(
                    loaded.0["view"]["sections"][0]["rows"][0]["name"].as_str(),
                    Some("initial" | "alice" | "bob")
                ));
                count += 1;
            }
            count
        });
        let writer = scope.spawn(|| {
            start.wait();
            let store = Store::new(root).unwrap();
            for _ in 0..50 {
                store.write(&display("alice"), || true).unwrap();
            }
        });
        start.wait();
        for _ in 0..50 {
            fixture.store().write(&display("bob"), || true).unwrap();
        }
        writer.join().unwrap();
        done.store(true, Ordering::Release);
        assert!(reader.join().unwrap() > 0);
    });
    assert_eq!(
        fs::read_dir(fixture.path().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

// This test-only child exercises the same Store in a different process. Its
// stdin gates the final rename so completion order is independent of scheduling.
#[test]
#[ignore = "started only by the concurrent-process publication test"]
fn process_writer() {
    let root =
        PathBuf::from(std::env::var_os("TMT_SNAPSHOT_TEST_ROOT").expect("owned fixture root"));
    let name = std::env::var("TMT_SNAPSHOT_TEST_NAME").unwrap();
    let store = Store::new(&root).unwrap();
    for _ in 0..40 {
        store.write(&display(&name), || true).unwrap();
    }
    println!("snapshot-writer-ready");
    std::io::stdout().flush().unwrap();
    let mut command = String::new();
    assert!(std::io::stdin().read_line(&mut command).unwrap() > 0);
    store.write(&display(&name), || true).unwrap();
}

#[test]
fn separate_boards_publish_complete_files_and_the_last_writer_wins() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::Duration,
    };
    struct Writer(std::process::Child);
    impl Drop for Writer {
        fn drop(&mut self) {
            if self.0.try_wait().unwrap().is_none() {
                self.0.kill().unwrap();
            }
            self.0.wait().unwrap();
        }
    }
    let fixture = Fixture::new();
    fixture.store().write(&display("initial"), || true).unwrap();
    let spawn = |name: &str| {
        Writer(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "board::snapshot_cache::tests::process_writer",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("TMT_SNAPSHOT_TEST_ROOT", &fixture.0)
                .env("TMT_SNAPSHOT_TEST_NAME", name)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    };
    let a = spawn("alice");
    let b = spawn("bob");
    let done = AtomicBool::new(false);
    struct End<'a>(&'a AtomicBool);
    impl Drop for End<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    std::thread::scope(|scope| {
        // Unwind kills/reaps both children and releases the reader before the
        // scope joins; a failed readiness assertion cannot strand the test.
        let _end = End(&done);
        let mut a = a;
        let mut b = b;
        let reader = scope.spawn(|| {
            let mut count = 0;
            while !done.load(Ordering::Acquire) {
                let loaded = fixture
                    .store()
                    .load("product", &rooms())
                    .expect("old or new complete snapshot");
                assert!(matches!(
                    loaded.0["view"]["sections"][0]["rows"][0]["name"].as_str(),
                    Some("initial" | "alice" | "bob")
                ));
                count += 1;
            }
            count
        });
        let (ready, signals) = mpsc::channel();
        let handles = [&mut a, &mut b]
            .into_iter()
            .map(|writer| {
                let stdout = writer.0.stdout.take().unwrap();
                let ready = ready.clone();
                scope.spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        if line.unwrap().contains("snapshot-writer-ready") {
                            ready.send(()).unwrap();
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        signals
            .recv_timeout(Duration::from_secs(30))
            .expect("first writer staged");
        signals
            .recv_timeout(Duration::from_secs(30))
            .expect("second writer staged");
        a.0.stdin.take().unwrap().write_all(b"finish\n").unwrap();
        assert!(a.0.wait().unwrap().success());
        b.0.stdin.take().unwrap().write_all(b"finish\n").unwrap();
        assert!(b.0.wait().unwrap().success());
        for handle in handles {
            handle.join().unwrap();
        }
        done.store(true, Ordering::Release);
        assert!(reader.join().unwrap() > 0);
    });
    assert_eq!(
        fixture.store().load("product", &rooms()).unwrap().0["view"]["sections"][0]["rows"][0]["name"],
        "bob"
    );
    assert_eq!(
        fs::read_dir(fixture.path().parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn aggregate_tabs_round_trip_in_their_own_namespace_without_member_authority() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for tab in [tabs::LEADS, "@tab:needs-me"] {
        let snapshot = snapshot(
            tab,
            json!([{"title":"Needs me", "rows":[{"name":"alice", "squad":"product", "id":"FORBIDDEN-id", "state":"blocked"}]}]),
        );
        let projected = Display::project(&snapshot, &rooms(), 123).unwrap();
        store.write(&projected, || true).unwrap();
        assert_eq!(store.load(tab, &rooms()).unwrap().0, projected.0);
        assert!(!projected.0.to_string().contains("FORBIDDEN"));
    }
    assert!(store.load("product", &rooms()).is_none());
    assert_eq!(
        fs::read_dir(fixture.0.join("ops/cache/board"))
            .unwrap()
            .count(),
        2
    );
}
