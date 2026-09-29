use super::{evidence, metadata};
use serde_json::{Value, json};
use tmt_core::{
    endpoint::{BindingMarker, EndpointSnapshot, ServerEvidence},
    limits::MAX_JS_SAFE_INTEGER,
};

const SERVER_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
const SOCKET: &str = "/tmp/tmux.sock";
const START_TIME: &str = "1700000000";
const SERVER_PID: &str = "321";
const PANE_PID: &str = "654";

fn server_row(server_id: &str, socket: &str, pid: &str, start_time: &str) -> String {
    [server_id, socket, pid, start_time].join(evidence::SEPARATOR)
}

fn endpoint_row(
    server_id: &str,
    server_pid: &str,
    pane_id: &str,
    target: &str,
    pane_pid: &str,
    attached: &str,
    metadata: &str,
) -> String {
    [
        server_id, SOCKET, server_pid, START_TIME, pane_id, target, "/repo", "codex", pane_pid,
        attached, metadata,
    ]
    .join(evidence::SEPARATOR)
}

fn valid_endpoint_row() -> String {
    endpoint_row(
        SERVER_ID,
        SERVER_PID,
        "%9",
        "main:1.0",
        PANE_PID,
        "0",
        r#"{"version":1}"#,
    )
}

fn assert_evidence_error<T: std::fmt::Debug>(result: Result<T, super::TmuxError>, label: &str) {
    let error = result.expect_err(label);
    assert_eq!(error.kind, super::TmuxFailure::Evidence, "{label}");
}

fn marker() -> BindingMarker {
    BindingMarker {
        name: "Alice".into(),
        canonical_name: "alice".into(),
        identity_id: "identity-1".into(),
        binding_id: "binding-1".into(),
        server_id: SERVER_ID.into(),
        pane_pid: 654,
    }
}

#[test]
fn read_only_snapshot_never_initializes_server_metadata() {
    use super::{OperationOptions, Tmux, test_support::ScriptedRunner};
    for (row, valid) in [(valid_endpoint_row(), true), (String::new(), false)] {
        let runner = ScriptedRunner::default();
        runner.push_output(row.into_bytes(), Vec::new());
        let tmux = Tmux::new(runner);
        assert_eq!(
            tmux.observe_snapshot(OperationOptions::default()).is_ok(),
            valid
        );
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].args.iter().any(|arg| arg == "list-panes"));
        assert!(!calls[0].args.iter().any(|arg| arg == "set-option"));
    }
}

#[test]
fn accepts_strict_v4_server_evidence_and_expected_identity() {
    let output = format!(
        "{}\n\n",
        server_row(SERVER_ID, SOCKET, SERVER_PID, START_TIME)
    );
    let expected = ServerEvidence {
        server_id: SERVER_ID.into(),
        socket_path: SOCKET.into(),
        server_pid: 321,
        server_start_time: START_TIME.into(),
    };

    assert_eq!(
        evidence::parse_server(&output, Some(SERVER_ID)).unwrap(),
        expected
    );
    assert_eq!(evidence::parse_server(&output, None).unwrap(), expected);
}

#[test]
fn rejects_empty_truncated_mismatched_and_non_v4_server_evidence() {
    let cases = [
        ("empty", "".to_owned(), None),
        (
            "truncated",
            [SERVER_ID, SOCKET, SERVER_PID].join(evidence::SEPARATOR),
            None,
        ),
        (
            "invalid UUID",
            server_row("not-a-server-id", SOCKET, SERVER_PID, START_TIME),
            None,
        ),
        (
            "version 1 UUID",
            server_row(
                "123e4567-e89b-12d3-a456-426614174000",
                SOCKET,
                SERVER_PID,
                START_TIME,
            ),
            None,
        ),
        (
            "zero PID",
            server_row(SERVER_ID, SOCKET, "0", START_TIME),
            None,
        ),
        (
            "fractional PID",
            server_row(SERVER_ID, SOCKET, "321.5", START_TIME),
            None,
        ),
        (
            "unsafe PID",
            server_row(SERVER_ID, SOCKET, "9007199254740992", START_TIME),
            None,
        ),
        (
            "empty socket",
            server_row(SERVER_ID, "", SERVER_PID, START_TIME),
            None,
        ),
        (
            "empty start time",
            server_row(SERVER_ID, SOCKET, SERVER_PID, ""),
            None,
        ),
        (
            "unexpected server ID",
            server_row(SERVER_ID, SOCKET, SERVER_PID, START_TIME),
            Some("123e4567-e89b-42d3-a456-426614174001"),
        ),
    ];

    for (label, output, expected) in cases {
        assert_evidence_error(evidence::parse_server(&output, expected), label);
    }

    let mixed = format!(
        "{}\n{}\n",
        server_row(SERVER_ID, SOCKET, SERVER_PID, START_TIME),
        server_row(SERVER_ID, SOCKET, "999", START_TIME)
    );
    assert_evidence_error(evidence::parse_server(&mixed, None), "mixed server rows");
}

#[test]
fn rejects_empty_truncated_and_incomplete_snapshot_rows() {
    assert_evidence_error(evidence::parse_snapshot("", None), "empty snapshot");

    let truncated = valid_endpoint_row()
        .split(evidence::SEPARATOR)
        .take(10)
        .collect::<Vec<_>>()
        .join(evidence::SEPARATOR);
    assert_evidence_error(
        evidence::parse_snapshot(&truncated, None),
        "truncated endpoint row",
    );

    for (label, row) in [
        (
            "missing pane ID",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "",
                "main:1.0",
                PANE_PID,
                "0",
                r#"{"version":1}"#,
            ),
        ),
        (
            "zero pane PID",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "%9",
                "main:1.0",
                "0",
                "0",
                r#"{"version":1}"#,
            ),
        ),
        (
            "unsafe pane PID",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "%9",
                "main:1.0",
                "9007199254740992",
                "0",
                r#"{"version":1}"#,
            ),
        ),
    ] {
        assert_evidence_error(evidence::parse_snapshot(&row, None), label);
    }
}

#[test]
fn deduplicates_scoped_ids_and_builds_nested_tmux_filter() {
    let ids = vec!["%9".to_owned(), "%9".to_owned(), "%10".to_owned()];
    assert_eq!(
        evidence::scoped_ids(Some(&ids)).unwrap(),
        Some(vec!["%9", "%10"])
    );
    assert_eq!(evidence::scoped_ids(None).unwrap(), None);
    assert_eq!(
        evidence::pane_filter(&["%9", "%10"]),
        "#{||:#{==:#{pane_id},%9},#{==:#{pane_id},%10}}"
    );

    let invalid = vec!["%9".to_owned(), "not-a-pane".to_owned()];
    assert_evidence_error(
        evidence::scoped_ids(Some(&invalid)),
        "invalid pane scope precheck",
    );
}

#[test]
fn scope_bounds_cover_argv_bytes_and_expression_depth_before_io() {
    let too_many = (0..1025)
        .map(|index| format!("%{index}"))
        .collect::<Vec<_>>();
    assert!(evidence::scoped_ids(Some(&too_many)).is_err());
    let oversized = vec![format!("%{}", "1".repeat(32 * 1024))];
    assert!(evidence::scoped_ids(Some(&oversized)).is_err());
    let aggregate = (0..1024)
        .map(|index| format!("%00000000000000000000{index}"))
        .collect::<Vec<_>>();
    assert!(evidence::scoped_ids(Some(&aggregate)).is_err());
    let ids = (0..1024)
        .map(|index| format!("%{index}"))
        .collect::<Vec<_>>();
    let scoped = evidence::scoped_ids(Some(&ids)).unwrap().unwrap();
    let filter = evidence::pane_filter(&scoped);
    assert!(filter.len() < 32 * 1024);
    assert_eq!(filter.matches("#{==:").count(), ids.len());
    let mut depth = 0;
    let mut maximum = 0;
    for character in filter.chars() {
        if character == '{' {
            depth += 1;
            maximum = maximum.max(depth);
        }
        if character == '}' {
            depth -= 1;
        }
    }
    assert_eq!(depth, 0);
    assert_eq!(maximum, 12); // Ten disjunction levels, equality and pane_id.
}

#[test]
fn prefers_attached_presentation_for_grouped_pane_rows() {
    let detached = endpoint_row(
        SERVER_ID,
        SERVER_PID,
        "%9",
        "detached:1.0",
        PANE_PID,
        "0",
        r#"{"version":1}"#,
    );
    let attached = endpoint_row(
        SERVER_ID,
        SERVER_PID,
        "%9",
        "attached:2.0",
        PANE_PID,
        "2",
        r#"{"version":1}"#,
    );

    let snapshot = evidence::parse_snapshot(&format!("{detached}\n{attached}\n"), None).unwrap();
    assert_eq!(snapshot.panes.len(), 1);
    assert_eq!(snapshot.panes[0].id, "%9");
    assert_eq!(snapshot.panes[0].target.as_deref(), Some("attached:2.0"));
    assert_eq!(snapshot.panes[0].suggested_name.as_deref(), Some("codex"));
}

#[test]
fn validates_conflicting_grouped_rows_before_deduplication() {
    let valid = endpoint_row(
        SERVER_ID,
        SERVER_PID,
        "%9",
        "attached:1.0",
        PANE_PID,
        "2",
        r#"{"version":1}"#,
    );
    let cases = [
        (
            "different pane PID",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "%9",
                "linked:1.0",
                "999",
                "0",
                r#"{"version":1}"#,
            ),
        ),
        (
            "different metadata",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "%9",
                "linked:1.0",
                PANE_PID,
                "0",
                r#"{"version":1,"opaque":true}"#,
            ),
        ),
        (
            "different server PID",
            endpoint_row(
                SERVER_ID,
                "999",
                "%9",
                "linked:1.0",
                PANE_PID,
                "0",
                r#"{"version":1}"#,
            ),
        ),
        (
            "invalid pane ID",
            endpoint_row(
                SERVER_ID,
                SERVER_PID,
                "not-a-pane",
                "linked:1.0",
                PANE_PID,
                "0",
                r#"{"version":1}"#,
            ),
        ),
        (
            "truncated row",
            valid
                .split(evidence::SEPARATOR)
                .take(10)
                .collect::<Vec<_>>()
                .join(evidence::SEPARATOR),
        ),
    ];

    for (label, conflicting) in cases {
        for (first, second) in [(&valid, &conflicting), (&conflicting, &valid)] {
            let output = format!("{first}\n{second}\n");
            assert_evidence_error(evidence::parse_snapshot(&output, None), label);
        }
    }
}

#[test]
fn preserves_metadata_containing_the_field_separator() {
    let metadata = json!({
        "version": 1,
        "opaque": evidence::SEPARATOR,
        "globalIdentity": {
            "name": "Alice",
            "canonicalName": "alice",
            "identityId": "identity-1",
            "bindingId": "binding-1",
            "serverId": SERVER_ID,
            "panePid": 654,
        },
    })
    .to_string();
    let first = endpoint_row(
        SERVER_ID, SERVER_PID, "%9", "main:1.0", PANE_PID, "0", &metadata,
    );
    let second = endpoint_row(
        SERVER_ID,
        SERVER_PID,
        "%9",
        "linked:7.0",
        PANE_PID,
        "0",
        &metadata,
    );

    let snapshot = evidence::parse_snapshot(&format!("{first}\n{second}\n"), None).unwrap();
    assert_eq!(snapshot.panes[0].marker, Some(marker()));
}

#[test]
fn decodes_only_version_one_object_metadata_and_defaults_malformed_envelopes() {
    for (label, text) in [
        ("invalid JSON", "not-json"),
        ("null", "null"),
        ("array", "[]"),
        ("scalar", "true"),
        ("missing version", "{}"),
        ("wrong version", r#"{"version":2,"opaque":true}"#),
    ] {
        assert_eq!(metadata::decode(text), json!({"version": 1}), "{label}");
    }

    assert_eq!(
        metadata::decode(r#"{"version":1.0,"opaque":true}"#)["opaque"],
        true
    );
}

#[test]
fn normalizes_js_numbers_before_extracting_marker_pid() {
    let base = |pane_pid: &str| {
        format!(
            r#"{{"version":1,"globalIdentity":{{"name":"Alice","canonicalName":"alice","identityId":"identity-1","bindingId":"binding-1","serverId":"{SERVER_ID}","panePid":{pane_pid}}}}}"#
        )
    };
    let cases = [
        ("integer", "654", Some(654)),
        ("integer-valued decimal", "654.0", Some(654)),
        (
            "safe integer maximum",
            "9007199254740991",
            Some(MAX_JS_SAFE_INTEGER),
        ),
        ("unsafe integer", "9007199254740992", None),
        ("fractional", "654.5", None),
        ("zero", "0", None),
        ("negative", "-1", None),
        ("string", "\"654\"", None),
    ];

    for (label, pane_pid, expected) in cases {
        let document = metadata::decode(&base(pane_pid));
        assert_eq!(
            metadata::marker(&document).map(|value| value.pane_pid),
            expected,
            "{label}"
        );
    }
}

#[test]
fn extracts_marker_only_when_all_identity_strings_are_nonempty() {
    let mut document = metadata::decode(
        r#"{"version":1,"globalIdentity":{"name":"Alice","canonicalName":"alice","identityId":"identity-1","bindingId":"binding-1","serverId":"123e4567-e89b-42d3-a456-426614174000","panePid":654}}"#,
    );
    assert_eq!(metadata::marker(&document), Some(marker()));

    document["globalIdentity"]["name"] = Value::String("  ".into());
    // Wire decoding retains nonempty strings verbatim; core binding evidence
    // owns name validity (including the distinct BOM and NEL trim semantics).
    assert_eq!(metadata::marker(&document).unwrap().name, "  ");
    document["globalIdentity"]["name"] = Value::String(String::new());
    assert_eq!(metadata::marker(&document), None);
}

#[test]
fn replaces_marker_without_dropping_opaque_metadata() {
    let mut document = json!({
        "version": 1,
        "workspaces": {"/repo": {"name": "legacy"}},
        "opaque": [true, 3],
        "globalIdentity": {"name": "Old"},
    });
    metadata::replace(&mut document, &marker());

    assert_eq!(document["version"], 1);
    assert_eq!(document["workspaces"]["/repo"]["name"], "legacy");
    assert_eq!(document["opaque"], json!([true, 3]));
    assert_eq!(metadata::marker(&document), Some(marker()));
}

#[test]
fn clears_matching_marker_and_preserves_opaque_siblings() {
    let mut document = json!({
        "version": 1,
        "future": {"flag": true},
        "globalIdentity": {
            "name": "Alice",
            "bindingId": "binding-1",
        },
    });
    assert!(metadata::clear(&mut document, Some("binding-1")));
    assert_eq!(document, json!({"version": 1, "future": {"flag": true}}));

    assert!(!metadata::clear(&mut document, Some("binding-1")));
    assert!(!metadata::clear(&mut document, None));
}

#[test]
fn clear_respects_false_like_malformed_marker_values() {
    for (label, value) in [
        ("null", Value::Null),
        ("false", Value::Bool(false)),
        ("zero", json!(0)),
        ("empty string", json!("")),
    ] {
        let mut document = json!({"version": 1, "globalIdentity": value});
        assert!(!metadata::clear(&mut document, None), "{label}");
        assert!(
            document.get("globalIdentity").is_some(),
            "{label} preserved"
        );
    }

    let mut object_marker = json!({"version": 1, "globalIdentity": {"opaque": true}});
    assert!(metadata::clear(&mut object_marker, None));
    assert_eq!(object_marker, json!({"version": 1}));
}

#[test]
fn snapshot_retains_server_and_pane_observations_as_typed_values() {
    let snapshot: EndpointSnapshot = evidence::parse_snapshot(&valid_endpoint_row(), None).unwrap();
    assert_eq!(snapshot.server.server_id, SERVER_ID);
    assert_eq!(snapshot.server.server_pid, 321);
    assert_eq!(snapshot.panes[0].pane_pid, 654);
    assert_eq!(snapshot.panes[0].target.as_deref(), Some("main:1.0"));
    assert_eq!(snapshot.panes[0].cwd.as_deref(), Some("/repo"));
}

#[test]
fn suggested_names_are_registered_drivers_matched_as_whole_words() {
    let first = tmt_core_driver_name();
    for command in [
        first.to_owned(),
        format!("/usr/local/bin/{first}"),
        format!("{first}-cli --flag"),
        first.to_uppercase(),
    ] {
        assert_eq!(
            evidence::suggested_name(&command).as_deref(),
            Some(first),
            "{command}"
        );
    }
    for command in [
        format!("{first}x"),
        format!("x{first}"),
        "zsh".into(),
        "".into(),
    ] {
        assert_eq!(evidence::suggested_name(&command), None, "{command}");
    }
}

fn tmt_core_driver_name() -> &'static str {
    crate::drivers::Registry::builtin().names()[0]
}
