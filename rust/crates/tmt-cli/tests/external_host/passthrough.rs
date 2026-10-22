//! A passthrough is a person's own text typed into the agent's pane as written (#4240): the
//! agent gets no request to answer, so nothing of it shows in its inbox or attention.
use super::*;

/// A delivering driver with a saved `worker` in pane f1 and a person `ben`.
fn world(name: &str) -> Fixture {
    register();
    let fixture = Fixture::delivering(name);
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    let created = fixture.tmt(&["identity", "create", "ben", "--json"]);
    assert!(created.status.success(), "{created:?}");
    fixture
}

fn mark_person(fixture: &Fixture) {
    let marked = fixture.tmt(&[
        "identity",
        "meta",
        "set",
        "user",
        "true",
        "--identity",
        "ben",
    ]);
    assert!(marked.status.success(), "{marked:?}");
}

fn talk(fixture: &Fixture, agent: &str, args: &[&str]) -> Output {
    fs::write(fixture.root.join("driver/agent"), agent).unwrap();
    let _ = fs::remove_file(fixture.root.join("driver/typed"));
    let _ = fs::remove_file(fixture.root.join("driver/prompted"));
    let mut argv = vec!["talk", "worker"];
    argv.extend_from_slice(args);
    argv.extend_from_slice(&["--identity", "ben", "--detach", "--json"]);
    fixture.tmt(&argv)
}

#[test]
fn a_person_types_a_message_as_written_and_the_agent_gets_no_request() {
    let fixture = world("passthrough-types");
    mark_person(&fixture);
    // `!` is the one protected character; the rest reaches the agent exactly as written,
    // with no reply command, receipt or preamble around it.
    let sent = talk(
        &fixture,
        "ok",
        &["Please run the tests!\nthen report", "--passthrough"],
    );
    assert!(sent.status.success(), "{sent:?}");
    let report: Value = serde_json::from_slice(&sent.stdout).unwrap();
    assert_eq!(report["mode"], "passthrough", "{report:#}");
    assert_eq!(report["status"], "sent", "{report:#}");
    let prompted = fixture.requests("prompted");
    assert_eq!(prompted.len(), 1, "{prompted:?}");
    assert_eq!(
        prompted[0]["text"], "Please run the tests\u{ff01}\nthen report",
        "typed exactly, `!` protected, nothing appended"
    );

    // The agent has nothing to answer: no open request, no incoming attention.
    let inbox = fixture.tmt(&["inbox", "--identity", "worker", "--json"]);
    assert!(inbox.status.success(), "{inbox:?}");
    let listed: Value = serde_json::from_slice(&inbox.stdout).unwrap();
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(0),
        "{listed:#}"
    );
    let context = fixture.tmt_in(Some("f1"), &["whoami", "--context"]);
    assert!(
        !String::from_utf8_lossy(&context.stdout).contains("Incoming X items: 1"),
        "{context:?}"
    );
    // The sender still sees it, as its own exchange, marked as a passthrough.
    let request = report["requestId"].as_str().unwrap();
    let mine: Value =
        serde_json::from_slice(&fixture.tmt(&["x", "--identity", "ben", "--json"]).stdout).unwrap();
    let item = mine["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["requestId"] == request))
        .unwrap_or_else(|| panic!("the exchange is listed: {mine:#}"));
    assert_eq!(item["mode"], "passthrough", "{item:#}");
    let shown: Value = serde_json::from_slice(
        &fixture
            .tmt(&["x", "show", request, "--identity", "ben", "--json"])
            .stdout,
    )
    .unwrap();
    assert_eq!(shown["exchange"]["mode"], "passthrough", "{shown:#}");
    assert_eq!(shown["exchange"]["final"]["status"], "not_required");
}

#[test]
fn only_a_person_may_pass_text_through_and_only_to_a_live_pane() {
    let fixture = world("passthrough-refused");
    // Not marked as a person: nothing is typed, nothing is stored.
    let refused = talk(&fixture, "ok", &["as if I typed this", "--passthrough"]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let failure: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"], "PASSTHROUGH_NOT_ALLOWED",
        "{failure:#}"
    );
    assert!(fixture.requests("prompted").is_empty());
    assert!(fixture.requests("typed").is_empty());

    mark_person(&fixture);
    // A recipient with no live pane cannot be typed into.
    let created = fixture.tmt(&["identity", "create", "ghost", "--json"]);
    assert!(created.status.success(), "{created:?}");
    let gone = fixture.tmt(&[
        "talk",
        "ghost",
        "hello",
        "--passthrough",
        "--identity",
        "ben",
        "--detach",
        "--json",
    ]);
    assert_eq!(gone.status.code(), Some(1), "{gone:?}");
    let failure: Value = serde_json::from_slice(&gone.stdout).unwrap();
    assert_eq!(
        failure["error"]["code"], "PASSTHROUGH_UNAVAILABLE",
        "{failure:#}"
    );

    // The flag takes none of the options that wait for or frame a reply.
    for extra in [["--timeout", "5"], ["--no-preamble", ""]] {
        let mut argv = vec![
            "talk",
            "worker",
            "x",
            "--passthrough",
            "--identity",
            "ben",
            "--json",
        ];
        argv.extend(extra.iter().filter(|word| !word.is_empty()));
        let bad = fixture.tmt(&argv);
        assert_eq!(bad.status.code(), Some(1), "{extra:?}: {bad:?}");
        assert!(
            String::from_utf8_lossy(&bad.stdout).contains("USAGE_ERROR"),
            "{bad:?}"
        );
    }
}
