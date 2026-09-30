use serde_json::Value;
use sha2::{Digest, Sha256};
use tmt_remote::canonical::{self, Enrollment, Envelope};

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../typescript/remote-client/test/vectors.json"
    ))
    .unwrap()
}
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap()
}
fn input<'a>(v: &'a Value, payload: &'a [u8]) -> Envelope<'a> {
    Envelope {
        kind: text(v, "kind"),
        id: text(v, "id"),
        correlation_id: v["correlationId"].as_str(),
        machine_id: text(v, "machineId"),
        window_id: text(v, "windowId"),
        client_id: text(v, "clientId"),
        session_id: text(v, "sessionId"),
        sequence: text(v, "sequence"),
        timestamp_ms: v["timestampMs"].as_u64().unwrap(),
        origin: text(v, "origin"),
        operation: text(v, "operation"),
        payload,
    }
}
fn expected(v: &Value, actual: &[u8]) {
    assert_eq!(actual, bytes(text(v, "hex")), "{}", v["name"]);
    assert_eq!(Sha256::digest(actual).as_slice(), bytes(text(v, "sha256")));
}
#[test]
fn independent_python_envelope_vectors() {
    for v in fixtures()["envelopes"].as_array().unwrap() {
        let payload = bytes(text(&v["input"], "payload"));
        expected(
            v,
            &canonical::envelope(&input(&v["input"], &payload)).unwrap(),
        );
    }
}
#[test]
fn independent_python_enrollment_and_possession_vectors() {
    let fixtures = fixtures();
    for v in fixtures["enrollments"].as_array().unwrap() {
        let i = &v["input"];
        let agents: Vec<_> = i["agentIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let scopes: Vec<_> = i["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let value = Enrollment {
            machine_id: text(i, "machineId"),
            window_id: text(i, "windowId"),
            offer_id: text(i, "offerId"),
            server_challenge: &bytes(text(i, "serverChallenge")).try_into().unwrap(),
            client_nonce: &bytes(text(i, "clientNonce")).try_into().unwrap(),
            kind: text(i, "kind"),
            origin: text(i, "origin"),
            name: text(i, "name"),
            public_key: &bytes(text(i, "publicKey")).try_into().unwrap(),
            agent_ids: &agents,
            scopes: &scopes,
        };
        expected(v, &canonical::enrollment(&value).unwrap());
        if v["name"] == "addon" {
            // Independent Python oracle hashes coordinated with #691; no product encoder oracle.
            for (id, digest) in [
                (
                    "00000000-0000-1000-8000-000000000008",
                    "3b04e045f03e8eb236b8f8518105449c3f04b586dbbbffc0b7563627e0849382",
                ),
                (
                    "00000000-0000-5000-8000-000000000008",
                    "79bb1944f9db766ae1880ed97b064f477a8430290c2df60d82b41e6d1618db40",
                ),
                (
                    "00000000-0000-0000-0000-000000000001",
                    "150a2d2f244ad3225c33905567329e9ed5500c227e31d337747243c48f36cbfe",
                ),
            ] {
                let ids = [id];
                let value = Enrollment {
                    agent_ids: &ids,
                    ..value
                };
                assert_eq!(
                    Sha256::digest(canonical::enrollment(&value).unwrap()).as_slice(),
                    bytes(digest)
                );
            }
        }
    }
    let v = &fixtures["possession"];
    expected(
        v,
        &canonical::possession(
            &bytes(text(&v["input"], "enrollment")),
            &bytes(text(&v["input"], "mac")).try_into().unwrap(),
        )
        .unwrap(),
    );
}
#[test]
fn reject_invalid_envelope_values_without_normalizing() {
    let fixtures = fixtures();
    let original = &fixtures["envelopes"][0]["input"];
    for (field, invalid) in [
        ("kind", "unknown"),
        ("id", "00000000-0000-1000-8000-000000000008"),
        ("id", "00000000-0000-4000-8000-00000000000A"),
        ("sequence", "01"),
        ("sequence", "+1"),
        ("sequence", "18446744073709551616"),
        ("sequence", "0"),
        ("sessionId", "new"),
        ("operation", ""),
        ("origin", "http://localhost"),
        ("origin", "chrome-extension://short"),
    ] {
        let mut changed = original.clone();
        changed[field] = invalid.into();
        assert!(
            canonical::envelope(&input(&changed, b"{}")).is_err(),
            "{field}: {invalid}"
        );
    }
    let mut value = input(original, b"{}");
    value.timestamp_ms = 9_007_199_254_740_992;
    assert!(canonical::envelope(&value).is_err());
    value.timestamp_ms = 0;
    value.correlation_id = Some(value.id);
    assert!(canonical::envelope(&value).is_err());
    value.kind = "response";
    assert!(canonical::envelope(&value).is_ok());
    value.correlation_id = None;
    assert!(canonical::envelope(&value).is_err());
    value.kind = "control";
    value.operation = "unknown";
    assert!(canonical::envelope(&value).is_err());
    value.operation = "session.open";
    assert!(canonical::envelope(&value).is_err());
    value.session_id = "new";
    value.sequence = "0";
    assert!(canonical::envelope(&value).is_ok());
}
#[test]
fn exact_payload_and_unicode_are_not_reserialized_or_normalized() {
    let fixtures = fixtures();
    let v = &fixtures["envelopes"][0]["input"];
    assert_ne!(
        canonical::envelope(&input(v, b"{}")),
        canonical::envelope(&input(v, b"{ }"))
    );
    let mut a = input(v, b"{}");
    a.operation = "probe.é";
    let mut b = input(v, b"{}");
    b.operation = "probe.e\u{301}";
    assert_ne!(canonical::envelope(&a), canonical::envelope(&b));
    let invalid_utf8 = vec![0xed, 0xa0, 0x80];
    assert!(String::from_utf8(invalid_utf8).is_err());
}
#[test]
fn reject_bad_enrollment_names_lists_and_origin() {
    let id = "00000000-0000-4000-8000-000000000001";
    let value = Enrollment {
        machine_id: id,
        window_id: id,
        offer_id: id,
        server_challenge: &[0; 16],
        client_nonce: &[1; 16],
        kind: "cli",
        origin: "cli",
        name: "CLI",
        public_key: &[2; 32],
        agent_ids: &[],
        scopes: &[],
    };
    assert!(canonical::enrollment(&value).is_ok());
    for name in [
        "",
        " ",
        "\u{feff}",
        "bad\nname",
        "bad\u{85}name",
        &"é".repeat(33),
    ] {
        assert!(
            canonical::enrollment(&Enrollment { name, ..value }).is_err(),
            "{name:?}"
        );
    }
    for scopes in [
        &["talk.hold", "agents.read"][..],
        &["status.read", "status.read"],
        &["admin"],
    ] {
        assert!(canonical::enrollment(&Enrollment { scopes, ..value }).is_err());
    }
    assert!(
        canonical::enrollment(&Enrollment {
            agent_ids: &[id, id],
            ..value
        })
        .is_err()
    );
    let distinct_ids: Vec<_> = (1..=257)
        .map(|n| format!("00000000-0000-4000-8000-{n:012x}"))
        .collect();
    let agent_ids: Vec<_> = distinct_ids.iter().map(String::as_str).collect();
    assert!(
        canonical::enrollment(&Enrollment {
            agent_ids: &agent_ids[..256],
            ..value
        })
        .is_ok()
    );
    assert!(
        canonical::enrollment(&Enrollment {
            agent_ids: &agent_ids,
            ..value
        })
        .is_err()
    );
    for good in [
        "00000000-0000-1000-8000-000000000008",
        "00000000-0000-5000-8000-000000000008",
        "00000000-0000-0000-0000-000000000001",
    ] {
        assert!(
            canonical::enrollment(&Enrollment {
                agent_ids: &[good],
                ..value
            })
            .is_ok()
        );
    }
    for bad in [
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-1000-8000-00000000000A",
        "00000000000010008000000000000001",
        "00000000-0000-1000-8000-000000000001\n",
    ] {
        assert!(
            canonical::enrollment(&Enrollment {
                agent_ids: &[bad],
                ..value
            })
            .is_err()
        );
    }
    assert!(
        canonical::enrollment(&Enrollment {
            kind: "addon",
            ..value
        })
        .is_err()
    );
    assert!(
        canonical::enrollment(&Enrollment {
            kind: "addon",
            origin: "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            name: "é🚀",
            ..value
        })
        .is_ok()
    );
}
