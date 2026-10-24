//! Every verb that names a member accepts its saved identity UUID as well as its name; an
//! unknown or retired UUID is not found, whatever the verb.
use super::*;

fn identity(fixture: &Fixture, name: &str) -> tmt_core::identity::Identity {
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let saved = tmt_core::identity::create_or_resolve(
        &mut storage,
        name,
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    storage.close().unwrap();
    saved
}

fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn mv_and_rm_take_the_identity_uuid_and_a_retired_uuid_is_not_found() {
    register();
    let fixture = Fixture::new("uuid-mv-rm");
    let _ = fixture.approve();
    let renamed = identity(&fixture, "before");
    let removed = identity(&fixture, "doomed");

    let moved = fixture.tmt(&["mv", &renamed.id, "after", "--json"]);
    assert!(moved.status.success(), "{moved:?}");
    let listed = fixture.json(&["ls", "--json"]);
    let names = listed.to_string();
    assert!(
        names.contains("\"after\"") && !names.contains("\"before\""),
        "{names}"
    );

    let gone = fixture.tmt(&["rm", &removed.id, "--force", "--json"]);
    assert!(gone.status.success(), "{gone:?}");
    assert!(
        !fixture
            .json(&["ls", "--json"])
            .to_string()
            .contains("doomed")
    );

    // The retired UUID, and one that never existed, are not found by any verb that names a member.
    let unknown = "7f0c2f2e-0000-4000-8000-000000000000";
    for selector in [removed.id.as_str(), unknown] {
        for args in [
            vec!["mv", selector, "elsewhere", "--json"],
            vec!["rm", selector, "--force", "--json"],
            vec!["stop", selector, "--json"],
        ] {
            let output = fixture.tmt(&args);
            assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
            assert_eq!(json(&output)["error"]["code"], "NAME_NOT_FOUND", "{args:?}");
        }
        for args in [vec!["resume", selector], vec!["resume", "--move", selector]] {
            let output = fixture.tmt_in(Some("f1"), &args);
            assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(selector), "{args:?}: {stderr}");
        }
    }
}
