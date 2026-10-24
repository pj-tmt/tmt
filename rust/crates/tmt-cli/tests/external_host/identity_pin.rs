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
