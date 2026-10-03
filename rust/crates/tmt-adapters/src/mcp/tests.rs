use super::*;

fn message(session: &mut Session, value: Value) -> Option<Value> {
    session.receive(&serde_json::to_vec(&value).unwrap(), &mut |_| {
        panic!("unexpected command")
    })
}
fn initialized(version: &str) -> Session {
    let mut session = Session::default();
    let response = message(&mut session, json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"untrusted","version":"1"}}})).unwrap();
    let expected = if PROTOCOLS.contains(&version) {
        version
    } else {
        PROTOCOLS[0]
    };
    assert_eq!(response["result"]["protocolVersion"], expected);
    assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
    assert!(
        message(
            &mut session,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none()
    );
    session
}

#[test]
fn negotiates_the_qualified_revisions_and_requires_initialization() {
    let mut session = Session::default();
    let response = message(
        &mut session,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .unwrap();
    assert_eq!(response["error"]["code"], -32600);
    for version in PROTOCOLS.iter().copied().chain(["unknown"]) {
        let mut session = initialized(version);
        let list = message(
            &mut session,
            json!({"jsonrpc":"2.0","id":"tools","method":"tools/list"}),
        )
        .unwrap();
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "tmt_list",
                "tmt_operation",
                "tmt_inbox",
                "tmt_request",
                "tmt_result"
            ]
        );
        assert_eq!(
            message(
                &mut session,
                json!({"jsonrpc":"2.0","id":2,"method":"ping"})
            )
            .unwrap()["result"],
            json!({})
        );
    }
}

#[test]
fn rejects_invalid_calls_and_notification_calls_before_command_effects() {
    let mut session = initialized(PROTOCOLS[0]);
    for params in [
        json!({"name":"tmt_send","arguments":{}}),
        json!({"name":"tmt_list","arguments":null}),
        json!({"name":"tmt_list","arguments":{"identity":"Other"}}),
        json!({"name":"tmt_inbox","arguments":{"limit":0}}),
        json!({"name":"tmt_inbox","arguments":{"limit":201}}),
        json!({"name":"tmt_inbox","arguments":{"limit":null}}),
        json!({"name":"tmt_operation","arguments":{"operationId":"not-a-UUID"}}),
        json!({"name":"tmt_request","arguments":{"requestId":""}}),
        json!({"name":"tmt_result","arguments":{"requestId":"req_1","file":"/private"}}),
    ] {
        let response = message(
            &mut session,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":params}),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }
    assert!(message(&mut session, json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"tmt_list","arguments":{}}})).is_none());
    assert_eq!(session.receive(br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tmt_inbox","arguments":{"limit":1,"limit":2}}}"#, &mut |_| panic!("duplicate input reached command")).unwrap()["error"]["code"], -32602);
    assert_eq!(
        session
            .receive(b"{", &mut |_| panic!("parse error reached command"))
            .unwrap()["error"]["code"],
        -32700
    );
    for value in [
        json!([]),
        json!({"jsonrpc":"2.0","id":null,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":1.5,"method":"ping"}),
    ] {
        assert_eq!(
            message(&mut session, value).unwrap()["error"]["code"],
            -32600
        );
    }
}

#[test]
fn structured_and_text_results_keep_exact_resources_and_operation_errors() {
    let mut session = initialized(PROTOCOLS[0]);
    let resource =
        json!({"response":"\u{feff}\r\n\0日本語 😀\n","requestId":"req_test","submittedAtMs":1});
    for failed in [false, true] {
        let mut calls = 0;
        let result = session.receive(br#"{"jsonrpc":"2.0","id":"r","method":"tools/call","params":{"name":"tmt_result","arguments":{"requestId":"req_test"}}}"#, &mut |tool| {
            assert!(matches!(tool, ToolCall::Result(ref id) if id == "req_test"));
            calls += 1;
            if failed { Err(resource.clone()) } else { Ok(resource.clone()) }
        }).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result["id"], "r");
        assert_eq!(result["result"]["structuredContent"], resource);
        assert_eq!(
            serde_json::from_str::<Value>(result["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap(),
            resource
        );
        assert_eq!(result["result"]["isError"], failed);
    }
}

#[test]
fn bounded_encoding_never_returns_a_truncated_document() {
    let value = json!({"response":"\0".repeat(100)});
    assert!(stdio::encode(&value, 100).is_err());
    let bytes = stdio::encode(&value, 1000).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), value);
}

#[test]
fn stdio_closes_a_stalled_output_pipe_without_a_worker_or_core_cancellation() {
    use nix::unistd::pipe;
    use std::{fs::File, io::Write, sync::mpsc, thread, time::Duration};
    let (input, client_input) = pipe().unwrap();
    let (client_output, output) = pipe().unwrap();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let resource =
            json!({"response":"\0".repeat(tmt_core::exact_text::MAX_EXCHANGE_TEXT_BYTES)});
        tx.send(stdio::serve(&input, &output, |_| Ok(resource.clone())))
            .unwrap();
    });
    let mut client = File::from(client_input);
    client.write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tmt_result","arguments":{"requestId":"req_test"}}}
"#).unwrap();
    drop(client);
    // Keep the reader alive but do not drain it: output must hit its own deadline.
    let error = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("bounded output deadline")
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    server.join().unwrap();
    drop(client_output);
}

#[test]
fn protocol_metadata_is_descriptive_and_null_parameters_are_invalid() {
    let mut session = initialized(PROTOCOLS[0]);
    let value = message(&mut session, json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"identity":"Other","progressToken":"read"}}})).unwrap();
    assert!(value["result"]["tools"].is_array());
    assert_eq!(
        message(
            &mut session,
            json!({"jsonrpc":"2.0","id":2,"method":"ping","params":null})
        )
        .unwrap()["error"]["code"],
        -32602
    );
    assert_eq!(
        session
            .receive(
                br#"{"jsonrpc":"2.0","id":1,"id":2,"method":"ping"}"#,
                &mut |_| panic!("duplicate RPC ID reached core")
            )
            .unwrap()["error"]["code"],
        -32600
    );
    assert_eq!(
        message(
            &mut session,
            json!({"jsonrpc":"2.0","id":9007199254740992u64,"method":"ping"})
        )
        .unwrap()["error"]["code"],
        -32600
    );
}
