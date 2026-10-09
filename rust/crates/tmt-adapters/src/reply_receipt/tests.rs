use super::*;
use serde_json::json;

const LEGACY_RECEIPT: &str = "eyJ2ZXJzaW9uIjoxLCJyZXF1ZXN0SWQiOiJsZWdhY3kiLCJhdHRlbXB0SWQiOiJsZWdhY3ktYXR0ZW1wdCIsImVuZHBvaW50Ijp7InNlcnZlcklkIjoic2VydmVyLWZvci1yZXF1ZXN0LXRlc3RzIiwic29ja2V0UGF0aCI6Ii90bXAvdG10LXJlcXVlc3QtdGVzdHMuc29jayIsInNlcnZlclBpZCI6NDEsInNlcnZlclN0YXJ0VGltZSI6InJlcXVlc3QtdGVzdC1zZXJ2ZXItc3RhcnQiLCJwYW5lSWQiOiIlOTAiLCJwYW5lUGlkIjoxOTB9fQ";

fn endpoint() -> RequestEndpoint {
    RequestEndpoint {
        server: ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server-1".into(),
            socket_path: "/tmp/tmt-test.sock".into(),
            server_pid: 1234,
            server_start_time: "2026-09-06T00:00:00.000Z".into(),
        },
        pane_id: "%17".into(),
        pane_pid: 5678,
    }
}

fn envelope() -> Value {
    json!({"version":1,"requestId":"legacy","attemptId":"legacy-attempt",
        "endpoint":{"serverId":"server-for-request-tests","socketPath":"/tmp/tmt-request-tests.sock",
        "serverPid":41,"serverStartTime":"request-test-server-start","paneId":"%90","panePid":190}})
}

fn wire(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

fn invalid(encoded: &str) {
    assert_eq!(
        decode_reply_receipt(encoded, "legacy"),
        Err(ReplyReceiptError::Invalid)
    );
}

#[test]
fn compact_matches_independent_node_crypto_golden_and_binds_every_field() {
    // Independent Node crypto SHA-256 over the specified byte preimage, not
    // an expected value derived from the Rust production helper.
    let base = encode_short_receipt("request-α", "attempt-1", &endpoint());
    assert_eq!(base, "v2_UrBhy1oeIPSUGg7Uo9o2Tw");
    assert_eq!(base.len(), 25);
    assert_eq!(
        decode_reply_receipt(&base, "unrelated"),
        Ok(ResponseProof::Compact([
            0x52, 0xb0, 0x61, 0xcb, 0x5a, 0x1e, 0x20, 0xf4, 0x94, 0x1a, 0x0e, 0xd4, 0xa3, 0xda,
            0x36, 0x4f,
        ]))
    ); // Request matching is transactional, not a decoder lookup.
    assert_ne!(
        encode_short_receipt("request-β", "attempt-1", &endpoint()),
        base
    );
    assert_ne!(
        encode_short_receipt("request-α", "attempt-2", &endpoint()),
        base
    );
    for field in 0..6 {
        let mut target = endpoint();
        match field {
            0 => target.server.server_id.push('x'),
            1 => target.server.socket_path.push('x'),
            2 => target.server.server_pid += 1,
            3 => target.server.server_start_time.push('x'),
            4 => target.pane_id.push('1'),
            5 => target.pane_pid += 1,
            _ => unreachable!(),
        }
        assert_ne!(
            encode_short_receipt("request-α", "attempt-1", &target),
            base
        );
    }
    assert_ne!(
        encode_short_receipt("ab", "c", &endpoint()),
        encode_short_receipt("a", "bc", &endpoint())
    );
    let mut long = endpoint();
    long.server.socket_path = "s".repeat(100_000);
    let encoded = encode_short_receipt("request", "attempt", &long);
    assert_eq!(encoded.len(), COMPACT_REPLY_RECEIPT_LENGTH);
    assert!(encoded.is_ascii());
}

#[test]
fn compact_rejects_noncanonical_version_length_alphabet_and_trailing_bits() {
    for value in [
        "",
        "v2_",
        "v3_eO65v2RuWainfXCY5QqO-Q",
        "v20_eO65v2RuWainfXCY5QqO-Q",
        "v2_eO65v2RuWainfXCY5QqO",
        "v2_eO65v2RuWainfXCY5QqO-QQ",
        "v2_eO65v2RuWainfXCY5QqO-Q=",
        "v2_eO65v2RuWainfXCY5QqO+Q",
        "v2_eO65v2RuWainfXCY5QqO.Q",
        "v2_eO65v2RuWainfXCY5QqO-R",
        " v2_eO65v2RuWainfXCY5QqO-Q",
        "v2_eO65v2RuWainfXCY5QqO-Q\n",
    ] {
        invalid(value);
    }
}

#[test]
fn original_ts_literal_reordered_whitespace_and_duplicate_keys_share_one_proof() {
    let expected = ResponseProof::Recorded {
        attempt_id: "legacy-attempt".into(),
        endpoint: RequestEndpoint {
            server: ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
                server_id: "server-for-request-tests".into(),
                socket_path: "/tmp/tmt-request-tests.sock".into(),
                server_pid: 41,
                server_start_time: "request-test-server-start".into(),
            },
            pane_id: "%90".into(),
            pane_pid: 190,
        },
    };
    assert_eq!(
        decode_reply_receipt(LEGACY_RECEIPT, "legacy"),
        Ok(expected.clone())
    );
    let reordered = r#" {
      "endpoint": {"panePid":190,"paneId":"%90","serverStartTime":"request-test-server-start","serverPid":41,"socketPath":"/tmp/tmt-request-tests.sock","serverId":"server-for-request-tests"},
      "attemptId":"legacy-attempt","requestId":"legacy","version":1
    } "#;
    assert_eq!(
        decode_reply_receipt(&URL_SAFE_NO_PAD.encode(reordered), "legacy"),
        Ok(expected.clone())
    );
    let duplicate = reordered
        .replace(r#""version":1"#, r#""version":2,"version":1"#)
        .replace(
            r#""requestId":"legacy""#,
            r#""requestId":"wrong","requestId":"legacy""#,
        );
    assert_eq!(
        decode_reply_receipt(&URL_SAFE_NO_PAD.encode(duplicate), "legacy"),
        Ok(expected)
    );
    assert_eq!(
        decode_reply_receipt(LEGACY_RECEIPT, "other"),
        Err(ReplyReceiptError::Mismatch)
    );
}

#[test]
fn v1_wire_limit_and_utf8_checks_have_independently_valid_base64() {
    let mut unicode = envelope();
    unicode["endpoint"]["serverId"] = json!("🙂");
    let url_safe = (0..3)
        .map(|padding| URL_SAFE_NO_PAD.encode(format!("{}{}", " ".repeat(padding), unicode)))
        .find(|text| text.contains('-') || text.contains('_'))
        .expect("fixture exercises URL alphabet");
    assert!(decode_reply_receipt(&url_safe, "legacy").is_ok());
    invalid(&url_safe.replace('-', "+").replace('_', "/"));
    invalid(&format!("{LEGACY_RECEIPT}="));
    let mut trailing = LEGACY_RECEIPT.to_owned();
    assert!(trailing.ends_with('Q'));
    trailing.pop();
    trailing.push('R');
    invalid(&trailing);
    for bytes in [&[0xff, 0xfe][..], &[0xed, 0xa0, 0x80], &[0xf0, 0x9f]] {
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        assert_eq!(URL_SAFE_NO_PAD.decode(&encoded).unwrap(), bytes);
        invalid(&encoded);
    }
    let json = envelope().to_string();
    let padded = format!(
        "{}{json}",
        " ".repeat(MAX_REPLY_RECEIPT_LENGTH / 4 * 3 - json.len())
    );
    let encoded = URL_SAFE_NO_PAD.encode(&padded);
    assert_eq!(encoded.len(), MAX_REPLY_RECEIPT_LENGTH);
    assert!(decode_reply_receipt(&encoded, "legacy").is_ok());
    invalid(&URL_SAFE_NO_PAD.encode(format!("{padded} ")));
    invalid(&URL_SAFE_NO_PAD.encode(format!("\u{feff}{json}")));
}

#[test]
fn v1_rejects_missing_unknown_or_wrongly_typed_fields() {
    for key in ["version", "requestId", "attemptId", "endpoint"] {
        let mut value = envelope();
        value.as_object_mut().unwrap().remove(key);
        invalid(&wire(&value));
    }
    for key in [
        "serverId",
        "socketPath",
        "serverPid",
        "serverStartTime",
        "paneId",
        "panePid",
    ] {
        let mut value = envelope();
        value["endpoint"].as_object_mut().unwrap().remove(key);
        invalid(&wire(&value));
    }
    for value in [Value::Null, json!([]), json!([envelope()])] {
        invalid(&wire(&value));
    }
    let mut value = envelope();
    value["extra"] = json!(true);
    invalid(&wire(&value));
    value = envelope();
    value["endpoint"]["extra"] = json!(true);
    invalid(&wire(&value));
    for wrong in [json!(null), json!([]), json!("endpoint")] {
        value = envelope();
        value["endpoint"] = wrong;
        invalid(&wire(&value));
    }
    for wrong in [json!(0), json!(2), json!("1"), json!(null)] {
        value = envelope();
        value["version"] = wrong;
        invalid(&wire(&value));
    }
}

#[test]
fn v1_string_limits_are_utf8_bytes_not_characters() {
    // Unicode is intentional protocol fixture data, not translated instructions.
    for field in [
        "requestId",
        "attemptId",
        "serverId",
        "socketPath",
        "serverStartTime",
    ] {
        let limit = if field == "requestId" { 256 } else { 4096 };
        for (text, accepted) in [
            ("é".repeat(limit / 2), true),
            ("é".repeat(limit / 2 + 1), false),
            ("".into(), false),
        ] {
            let mut value = envelope();
            let slot = if field == "requestId" || field == "attemptId" {
                &mut value[field]
            } else {
                &mut value["endpoint"][field]
            };
            *slot = json!(text);
            let result = decode_reply_receipt(&wire(&value), value["requestId"].as_str().unwrap());
            assert_eq!(result.is_ok(), accepted, "{field}");
        }
    }
    for pane in ["%", "17", "%-1", "%١", "%1x"] {
        let mut value = envelope();
        value["endpoint"]["paneId"] = json!(pane);
        invalid(&wire(&value));
    }
    for (length, accepted) in [(4096, true), (4097, false)] {
        let mut value = envelope();
        value["endpoint"]["paneId"] = json!(format!("%{}", "1".repeat(length - 1)));
        assert_eq!(
            decode_reply_receipt(&wire(&value), "legacy").is_ok(),
            accepted
        );
    }
    let malformed = envelope()
        .to_string()
        .replace("server-for-request-tests", r#"\ud800"#);
    invalid(&URL_SAFE_NO_PAD.encode(malformed));
}

#[test]
fn v1_pid_numbers_preserve_json_parse_semantics_and_safe_integer_bounds() {
    for field in ["serverPid", "panePid"] {
        for number in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(9007199254740992u64),
            json!(null),
            json!("1"),
        ] {
            let mut value = envelope();
            value["endpoint"][field] = number;
            invalid(&wire(&value));
        }
        let mut value = envelope();
        value["endpoint"][field] = json!(9007199254740991u64);
        let ResponseProof::Recorded { endpoint, .. } =
            decode_reply_receipt(&wire(&value), "legacy").unwrap()
        else {
            panic!("recorded proof")
        };
        assert_eq!(
            if field == "serverPid" {
                endpoint.server.server_pid
            } else {
                endpoint.pane_pid
            },
            9007199254740991
        );
    }
    let json = envelope()
        .to_string()
        .replace(r#""serverPid":41"#, r#""serverPid":1e0"#)
        .replace(r#""panePid":190"#, r#""panePid":2.0"#);
    let ResponseProof::Recorded { endpoint, .. } =
        decode_reply_receipt(&URL_SAFE_NO_PAD.encode(json), "legacy").unwrap()
    else {
        panic!("recorded proof")
    };
    assert_eq!((endpoint.server.server_pid, endpoint.pane_pid), (1, 2));
}
