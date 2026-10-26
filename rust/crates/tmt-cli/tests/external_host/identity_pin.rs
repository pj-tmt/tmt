//! `identity show <uuid> --expect-name`: the delivery-time identity check (#4536).
use super::*;

fn saved(fixture: &Fixture, name: &str) -> tmt_core::identity::Identity {
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let identity = tmt_core::identity::create_or_resolve(
        &mut storage,
        name,
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    storage.close().unwrap();
    identity
}

fn pinned(fixture: &Fixture, id: &str, expected: &str) -> std::process::Output {
    fixture.tmt(&["identity", "show", id, "--expect-name", expected, "--json"])
}

fn refusal(output: &std::process::Output) -> (Option<i32>, String, String) {
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    (
        output.status.code(),
        error["error"]["code"].as_str().unwrap().to_owned(),
        error["error"]["message"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn capture_only_uuid_selects_the_recorded_binding_without_a_name_lookup() {
    register();
    let fixture = Fixture::delivering("capture-uuid");
    fixture.approve();
    let bound = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(bound.status.success(), "{bound:?}");
    let shown = fixture.json(&["identity", "show", "worker", "--json"]);
    let id = shown["identity"]["id"].as_str().unwrap();

    for lines in ["0", "43"] {
        let output = fixture.tmt(&["check", id, "--capture-only", "--lines", lines, "--json"]);
        assert!(output.status.success(), "{output:?}");
        let captured: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(captured["target"], id);
        assert_eq!(captured["pane"], "fake-1");
        assert_eq!(captured["identity"]["name"], "worker");
        assert_eq!(captured["output"], "captured line");
        assert_eq!(captured["lines"], lines.parse::<u64>().unwrap());
    }
    let requests = fixture.requests("captured");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["lines"], 0);
    assert_eq!(requests[1]["lines"], 43);
    assert!(fixture.requests("typed").is_empty());
    assert!(fixture.requests("prompted").is_empty());
}

#[test]
fn pinned_capture_refuses_rename_name_reuse_and_retirement_before_capture() {
    register();
    let fixture = Fixture::delivering("capture-name-pin");
    fixture.approve();
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    fs::write(
        fixture.root.join("driver/pid-2"),
        std::process::id().to_string(),
    )
    .unwrap();
    let bound = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(bound.status.success(), "{bound:?}");
    let shown = fixture.json(&["identity", "show", "worker", "--json"]);
    let id = shown["identity"]["id"].as_str().unwrap();
    let capture = |id: &str, name: &str| {
        let expected = format!("--expect-name={name}");
        fixture.tmt(&["check", "--capture-only", "--json", &expected, "--", id])
    };
    let matching = capture(id, "Worker");
    assert!(matching.status.success(), "{matching:?}");
    let document: Value = serde_json::from_slice(&matching.stdout).unwrap();
    assert_eq!(document["pane"], "fake-1");
    assert_eq!(document["identity"]["canonicalName"], "worker");
    assert_eq!(document.as_object().unwrap().len(), 5);

    fixture.json(&["mv", "worker", "reviewer", "--json"]);
    let successor = fixture.tmt_in(Some("f2"), &["name", "worker", "--save", "--json"]);
    assert!(successor.status.success(), "{successor:?}");
    // The old name is a real, capturable successor, so a fallback could leak it.
    let successor = fixture.json(&["check", "worker", "--capture-only", "--json"]);
    assert_eq!(successor["pane"], "fake-2");
    let before = fixture.requests("captured").len();
    let renamed = refusal(&capture(id, "worker"));
    assert_eq!(
        (renamed.0, renamed.1.as_str()),
        (Some(5), "IDENTITY_CHANGED")
    );
    assert!(renamed.2.contains("now named 'reviewer'"));
    assert_eq!(fixture.requests("captured").len(), before);
    let current = capture(id, "Reviewer");
    assert!(current.status.success(), "{current:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&current.stdout).unwrap()["pane"],
        "fake-1"
    );

    fixture.json(&["rm", "--force", "reviewer", "--json"]);
    let before = fixture.requests("captured").len();
    let retired = refusal(&capture(id, "reviewer"));
    assert_eq!(
        (retired.0, retired.1.as_str()),
        (Some(5), "IDENTITY_CHANGED")
    );
    assert!(retired.2.contains("retired"));
    let missing = refusal(&capture("7f0c2f2e-0000-4000-8000-000000000000", "worker"));
    assert_eq!((missing.0, missing.1.as_str()), (Some(3), "NAME_NOT_FOUND"));
    assert_eq!(fixture.requests("captured").len(), before);
    assert!(fixture.requests("typed").is_empty());
    assert!(fixture.requests("prompted").is_empty());
}

#[test]
fn pinned_capture_rechecks_a_name_changed_during_presence_acquisition() {
    use std::{fs::OpenOptions, time::Instant};
    register();
    let fixture = Fixture::delivering("capture-name-race");
    let driver = DRIVER.replace(
        "  snapshot)\n",
        "  snapshot)\n    if [ -f \"$d/capture-hold\" ]; then\n      printf ready > \"$d/capture-ready\"\n      IFS= read -r token < \"$d/capture-release\"\n    fi\n",
    );
    tmt_test_support::write_executable(&fixture.driver, driver.as_bytes(), 0o755).unwrap();
    fixture.approve();
    let bound = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(bound.status.success(), "{bound:?}");
    let shown = fixture.json(&["identity", "show", "worker", "--json"]);
    let id = shown["identity"]["id"].as_str().unwrap();
    let directory = fixture.root.join("driver");
    let fifo = directory.join("capture-release");
    nix::unistd::mkfifo(
        &fifo,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let mut release = OpenOptions::new()
        .read(true)
        .write(true)
        .open(fifo)
        .unwrap();
    fs::write(directory.join("capture-hold"), "").unwrap();
    std::thread::scope(|scope| {
        let capture = scope.spawn(|| {
            fixture.tmt(&[
                "check",
                id,
                "--capture-only",
                "--expect-name",
                "worker",
                "--json",
            ])
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while fs::read_to_string(directory.join("capture-ready"))
            .ok()
            .as_deref()
            != Some("ready")
        {
            if capture.is_finished() {
                panic!(
                    "capture exited before the presence barrier: {:?}",
                    capture.join().unwrap()
                );
            }
            assert!(
                Instant::now() < deadline,
                "presence barrier did not become ready"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // A committed rename can land while the host is observed outside the
        // writer lock. Its old cosmetic marker still has the exact ownership IDs.
        let db = rusqlite::Connection::open(fixture.state().join("tmt.db")).unwrap();
        db.busy_timeout(Duration::from_millis(500)).unwrap();
        assert_eq!(
            db.execute(
                "UPDATE identities SET name='reviewer', canonical_name='reviewer' WHERE id=?",
                [id],
            )
            .unwrap(),
            1
        );
        drop(db);
        fs::remove_file(directory.join("capture-hold")).unwrap();
        release.write_all(b"R\n").unwrap();
        let output = capture.join().unwrap();
        assert_eq!(
            output.status.code(),
            Some(5),
            "expected IDENTITY_CHANGED; {output:?}"
        );
        let rejected = refusal(&output);
        assert_eq!(
            (rejected.0, rejected.1.as_str()),
            (Some(5), "IDENTITY_CHANGED")
        );
        assert!(rejected.2.contains("now named 'reviewer'"));
    });
    assert!(fixture.requests("captured").is_empty());
    assert!(fixture.requests("typed").is_empty());
    assert!(fixture.requests("prompted").is_empty());
}

#[test]
fn capture_only_missing_or_retired_uuid_never_captures_a_uuid_shaped_name() {
    register();
    for retired in [false, true] {
        let fixture = Fixture::delivering(if retired {
            "capture-retired"
        } else {
            "capture-missing"
        });
        fixture.approve();
        let id = if retired {
            let bound = fixture.tmt_in(Some("f1"), &["name", "original", "--save", "--json"]);
            assert!(bound.status.success(), "{bound:?}");
            let shown = fixture.json(&["identity", "show", "original", "--json"]);
            let id = shown["identity"]["id"].as_str().unwrap().to_owned();
            fixture.json(&["rm", "--force", "original", "--json"]);
            id
        } else {
            "7f0c2f2e-0000-4000-8000-000000000000".to_owned()
        };
        let bound = fixture.tmt_in(Some("f1"), &["name", &id, "--save", "--json"]);
        assert!(bound.status.success(), "{bound:?}");
        // Positive control: the UUID-shaped name is bound and its pane is capturable.
        let actual = fixture.json(&["check", "f1", "--capture-only", "--json"]);
        assert_eq!(actual["pane"], "fake-1");
        let before = fixture.requests("captured").len();
        let output = fixture.tmt(&["check", &id, "--capture-only", "--json"]);
        let rejected = refusal(&output);
        assert_eq!(
            (rejected.0, rejected.1.as_str()),
            (Some(3), "NAME_NOT_FOUND")
        );
        assert_eq!(fixture.requests("captured").len(), before);
        assert!(fixture.requests("typed").is_empty());
        assert!(fixture.requests("prompted").is_empty());
    }
}

#[test]
fn a_pinned_show_reports_the_current_name_and_refuses_a_renamed_retired_or_unknown_identity() {
    register();
    let fixture = Fixture::new("identity-pin");
    let _ = fixture.approve();
    let reviewer = saved(&fixture, "reviewer");

    // The name it still has, in any case, is a match; presence is reported inside `identity`.
    for expected in ["reviewer", "Reviewer"] {
        let output = pinned(&fixture, &reviewer.id, expected);
        assert!(output.status.success(), "{expected}: {output:?}");
        let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(shown["identity"]["id"], reviewer.id.as_str());
        assert_eq!(shown["identity"]["name"], "reviewer");
        assert_eq!(shown["identity"]["lifetime"], "saved");
        let presence = shown["identity"]["presence"].as_str().unwrap();
        assert!(
            ["active", "offline", "unknown"].contains(&presence),
            "{presence}"
        );
    }
    // Plain show is unchanged: storage only, no presence.
    let plain = fixture.json(&["identity", "show", &reviewer.id, "--json"]);
    assert!(plain["identity"].get("presence").is_none(), "{plain}");

    // A name the identity does not have is a change, and so is a rename after the caller looked.
    let wrong = pinned(&fixture, &reviewer.id, "someone-else");
    let (code, error, message) = refusal(&wrong);
    assert_eq!((code, error.as_str()), (Some(5), "IDENTITY_CHANGED"));
    assert!(message.contains("now named 'reviewer'"), "{message}");
    assert!(fixture.tmt(&["mv", "reviewer", "renamed"]).status.success());
    let stale = refusal(&pinned(&fixture, &reviewer.id, "reviewer"));
    assert_eq!((stale.0, stale.1.as_str()), (Some(5), "IDENTITY_CHANGED"));
    assert!(stale.2.contains("now named 'renamed'"), "{}", stale.2);
    assert!(pinned(&fixture, &reviewer.id, "renamed").status.success());

    // Retired, even when a new identity took the old name: the UUID is not looked up as a name.
    assert!(fixture.tmt(&["rm", "--force", "renamed"]).status.success());
    let successor = saved(&fixture, "reviewer");
    assert_ne!(successor.id, reviewer.id);
    let retired = refusal(&pinned(&fixture, &reviewer.id, "reviewer"));
    assert_eq!(
        (retired.0, retired.1.as_str()),
        (Some(5), "IDENTITY_CHANGED")
    );
    assert!(retired.2.contains("retired"), "{}", retired.2);
    assert!(pinned(&fixture, &successor.id, "reviewer").status.success());

    // A UUID nobody ever had is simply not found.
    let unknown = refusal(&pinned(
        &fixture,
        "7f0c2f2e-0000-4000-8000-000000000000",
        "reviewer",
    ));
    assert_eq!((unknown.0, unknown.1.as_str()), (Some(3), "NAME_NOT_FOUND"));
}
