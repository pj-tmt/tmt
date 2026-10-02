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
        };
        expected(v, &canonical::enrollment(&value).unwrap());
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
        ("origin", "http://127.0.0.1:0"),
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
fn door_origin_is_an_envelope_origin() {
    let fixtures = fixtures();
    let mut value = input(&fixtures["envelopes"][0]["input"], b"{}");
    value.origin = "http://127.0.0.1:7341";
    assert!(canonical::envelope(&value).is_ok());
}
#[test]
fn reject_bad_enrollment_names_kinds_and_origins() {
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
    assert!(
        canonical::enrollment(&Enrollment {
            name: &"é".repeat(32),
            ..value
        })
        .is_ok()
    );
    let addon = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let door = "http://127.0.0.1:7341";
    // Each kind accepts exactly its own origin form.
    for (kind, origin, valid) in [
        ("cli", "cli", true),
        ("addon", addon, true),
        ("browser", door, true),
        ("browser", "http://127.0.0.1:65535", true),
        ("browser", "http://127.0.0.1:1", true),
        ("cli", addon, false),
        ("cli", door, false),
        ("addon", "cli", false),
        ("addon", door, false),
        ("browser", "cli", false),
        ("browser", addon, false),
        ("browser", "http://127.0.0.1:0", false),
        ("browser", "http://127.0.0.1:07341", false),
        ("browser", "http://127.0.0.1:65536", false),
        ("browser", "http://127.0.0.1", false),
        ("browser", "http://127.0.0.1:7341/", false),
        ("browser", "http://localhost:7341", false),
        ("browser", "https://127.0.0.1:7341", false),
        ("browser", "null", false),
        ("device", "cli", false),
    ] {
        assert_eq!(
            canonical::enrollment(&Enrollment {
                kind,
                origin,
                name: "é🚀",
                ..value
            })
            .is_ok(),
            valid,
            "{kind} {origin}"
        );
    }
}
#[test]
fn independent_python_pairing_code_and_fingerprint_vectors() {
    let fixtures = fixtures();
    for v in fixtures["pairingCodes"].as_array().unwrap() {
        let code: [u8; 16] = bytes(text(v, "code")).try_into().unwrap();
        let shown = text(v, "text");
        assert_eq!(canonical::pairing_code_text(&code), shown);
        assert_eq!(canonical::pairing_code(shown).unwrap(), code);
        // Only ASCII spaces and hyphens are removed before decoding.
        let compact: String = shown.chars().filter(|c| *c != '-').collect();
        assert_eq!(canonical::pairing_code(&compact).unwrap(), code);
        assert_eq!(
            canonical::pairing_code(&shown.replace('-', " ")).unwrap(),
            code
        );
    }
    let valid = text(&fixtures["pairingCodes"][0], "text");
    let compact: String = valid.chars().filter(|c| *c != '-').collect();
    for invalid in [
        compact.to_lowercase(),
        compact.replace('A', "1"),
        compact.replace('A', "="),
        format!("{compact}A"),
        compact[..25].to_owned(),
        format!("{}\t{}", &compact[..4], &compact[4..]),
        format!("{}_{}", &compact[..4], &compact[4..]),
        // Nonzero unused bits: the last symbol may only encode two data bits.
        format!("{}5", &compact[..25]),
    ] {
        assert!(canonical::pairing_code(&invalid).is_err(), "{invalid}");
    }
    assert_eq!(
        fixtures["wordlist"]["sha256"],
        "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
    );
    for v in fixtures["fingerprints"].as_array().unwrap() {
        let key: [u8; 32] = bytes(text(v, "publicKey")).try_into().unwrap();
        let indexes: Vec<u16> = v["indexes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as u16)
            .collect();
        let words: Vec<&str> = v["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        assert_eq!(
            canonical::fingerprint_indexes(&key).unwrap().to_vec(),
            indexes
        );
        assert_eq!(canonical::fingerprint_words(&key).unwrap().to_vec(), words);
    }
}
#[test]
fn independent_python_base64url_vectors_and_strict_refusals() {
    let fixtures = fixtures();
    for v in fixtures["base64url"]["valid"].as_array().unwrap() {
        let raw = bytes(text(v, "hex"));
        assert_eq!(canonical::base64url(&raw), text(v, "text"));
        let length = v["length"].as_u64().unwrap() as usize;
        assert_eq!(
            canonical::base64url_bytes(text(v, "text"), length).unwrap(),
            raw
        );
    }
    for v in fixtures["base64url"]["invalid"].as_array().unwrap() {
        let length = v["length"].as_u64().unwrap() as usize;
        assert!(
            canonical::base64url_bytes(text(v, "text"), length).is_err(),
            "{}",
            v["reason"]
        );
    }
}
#[test]
fn independent_python_extension_certificate_vectors() {
    let fixtures = fixtures();
    for v in fixtures["extCerts"].as_array().unwrap() {
        let i = &v["input"];
        let public_key: [u8; 32] = bytes(text(i, "publicKey")).try_into().unwrap();
        let value = canonical::ExtCert {
            extension: text(i, "extension"),
            purpose: text(i, "purpose"),
            public_key: &public_key,
            issued_at_ms: i["issuedAtMs"].as_u64().unwrap(),
        };
        expected(v, &canonical::ext_cert(&value).unwrap());
    }
    // A wrong key length cannot be expressed: the builder takes exactly 32 bytes.
    for v in fixtures["invalidExtCerts"].as_array().unwrap() {
        if v["publicKeyLength"] != 32 {
            continue;
        }
        let value = canonical::ExtCert {
            extension: text(v, "extension"),
            purpose: text(v, "purpose"),
            public_key: &[0; 32],
            issued_at_ms: v["issuedAtMs"].as_u64().unwrap(),
        };
        assert!(canonical::ext_cert(&value).is_err(), "{}", v["reason"]);
    }
}
