//! Every accepted frame is pinned to its canonical bytes; every refusal is a
//! one-token mutation of an accepted twin, so an unrelated failure cannot satisfy it.
use super::*;
use crate::limits::POLICY_BYTES;

const GENERATION: &str = "7f3c1a52-9d4e-4b86-8a21-5c0e9b7d3f14";
const ORIGIN: &str = "c1d2e3f4-a5b6-4c7d-9e8f-0a1b2c3d4e5f";
const TRANSFER: &str = "0b5e6d1c-2a47-4f93-b8e0-61c4d7a92f35";
/// 32 bytes of 0x01 and 0x02, spelled by hand from the base64url alphabet.
const ONES: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const TWOS: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";
/// SHA-256 of the three bytes `abc`.
const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
/// The ASCII bytes `policy` and `abc`.
const POLICY: &str = "cG9saWN5";
const ABC: &str = "YWJj";
const LOCAL: &str = r#"{"kind":"local-extension"}"#;
const BROWSER_LIMITS: &str = r#"{"payloadBytes":12582912,"chunkBytes":32768}"#;
const LOCAL_LIMITS: &str = r#"{"payloadBytes":12582912,"chunkBytes":32768,"namespaceBytes":67108864,"extensionBytes":536870912,"installationBytes":1073741824}"#;
const STATES: [(&str, State); 5] = [
    ("expired", State::Expired),
    ("discarded", State::Discarded),
    ("unavailable", State::Unavailable),
    ("unknown", State::Unknown),
    ("notObserved", State::NotObserved),
];
const LIMITS: [(&str, Limit); 10] = [
    ("namespace-bytes", Limit::NamespaceBytes),
    ("extension-bytes", Limit::ExtensionBytes),
    ("installation-bytes", Limit::InstallationBytes),
    ("namespace-entries", Limit::NamespaceEntries),
    ("extension-entries", Limit::ExtensionEntries),
    ("installation-entries", Limit::InstallationEntries),
    ("active-intents", Limit::ActiveIntents),
    ("retained-extension", Limit::RetainedExtension),
    ("retained-installation", Limit::RetainedInstallation),
    ("requests", Limit::Requests),
];

fn uuid(text: &str) -> Uuid4 {
    Uuid4::parse(text).unwrap()
}
fn namespace() -> Bytes32 {
    Bytes32::parse(ONES).unwrap()
}
fn key() -> Bytes32 {
    Bytes32::parse(TWOS).unwrap()
}
fn digest() -> Sha256Hex {
    Sha256Hex::parse(DIGEST).unwrap()
}
fn policy() -> Policy {
    Policy::new(b"policy".to_vec()).unwrap()
}
fn abc() -> Chunk {
    Chunk::new(b"abc".to_vec()).unwrap()
}
fn bounds() -> TransferBounds {
    TransferBounds {
        payload_bytes: PAYLOAD_BYTES,
        chunk_bytes: 32_768,
    }
}
fn config() -> Config {
    Config {
        backend_id: "local-fs".to_owned(),
        immutable_create: true,
        chunked_read: true,
        recover_by_original_id: true,
        limits: Limits::Browser(bounds()),
    }
}
fn committed_value() -> Success {
    Success::Committed {
        opaque_key: key(),
        payload_sha256: digest(),
        payload_bytes: 3,
    }
}

fn request(origin: Origin, call: Call) -> Frame {
    Frame::Request(Request {
        generation: uuid(GENERATION),
        request_id: Counter::new(7).unwrap(),
        origin,
        call,
    })
}
fn result(method: Method, outcome: Outcome) -> Frame {
    Frame::Result(ResultFrame {
        generation: uuid(GENERATION),
        request_id: Counter::new(7).unwrap(),
        method,
        transfer_id: method.carries_transfer().then(|| uuid(TRANSFER)),
        outcome,
    })
}
fn success(method: Method, value: Success) -> Frame {
    result(method, Outcome::Success(value))
}
fn failure(method: Method, code: ErrorCode) -> Frame {
    result(method, Outcome::Failure(code))
}

fn request_json(origin: &str, method: &str, input: &str) -> String {
    format!(
        r#"{{"version":1,"kind":"request","generation":"{GENERATION}","requestId":"7","origin":{origin},"method":"objects.{method}","input":{input}}}"#
    )
}
fn result_json(method: &str, transfer: bool, body: &str) -> String {
    let transfer = if transfer {
        format!(r#","transferId":"{TRANSFER}""#)
    } else {
        String::new()
    };
    format!(
        r#"{{"version":1,"kind":"result","generation":"{GENERATION}","requestId":"7","method":"objects.{method}"{transfer},{body}}}"#
    )
}
fn config_ok(projection: &str, limits: &str) -> String {
    format!(
        r#""ok":{{"result":"config","projection":"{projection}","backend":{{"id":"local-fs","source":"default","editable":false}},"capabilities":{{"immutableCreate":true,"chunkedRead":true,"recoverByOriginalId":true}},"limits":{limits}}}"#
    )
}
fn with_prefix(body: &str) -> Vec<u8> {
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Every accepted shape with the exact compact bytes it must encode to.
fn vectors() -> Vec<(String, Frame, String)> {
    let transfer = format!(r#"{{"transferId":"{TRANSFER}"}}"#);
    let committed = format!(
        r#""ok":{{"result":"committed","opaqueKey":"{TWOS}","payloadSha256":"{DIGEST}","payloadBytes":3}}"#
    );
    let state = |name: &str| format!(r#""ok":{{"result":"state","state":"{name}"}}"#);
    let mut local_config = config();
    local_config.limits = Limits::Local {
        bounds: bounds(),
        namespace_bytes: 64 << 20,
        extension_bytes: 512 << 20,
        installation_bytes: 1 << 30,
    };
    let mut all = vec![
        (
            "request config",
            request(
                Origin::LocalExtension,
                Call::Config(ConfigInput {
                    namespace: namespace(),
                    policy: policy(),
                }),
            ),
            request_json(
                LOCAL,
                "config",
                &format!(r#"{{"namespace":"{ONES}","policyInput":"{POLICY}"}}"#),
            ),
        ),
        (
            "request begin",
            request(
                Origin::Mounted(uuid(ORIGIN)),
                Call::Begin(BeginInput {
                    transfer_id: uuid(TRANSFER),
                    namespace: namespace(),
                    opaque_key: key(),
                    policy: policy(),
                    payload_sha256: digest(),
                    payload_bytes: 3,
                }),
            ),
            request_json(
                &format!(r#"{{"kind":"mounted","originId":"{ORIGIN}"}}"#),
                "begin",
                &format!(
                    r#"{{"transferId":"{TRANSFER}","namespace":"{ONES}","opaqueKey":"{TWOS}","policyInput":"{POLICY}","payloadSha256":"{DIGEST}","payloadBytes":3}}"#
                ),
            ),
        ),
        (
            "request part",
            request(
                Origin::LocalExtension,
                Call::Part(PartInput {
                    transfer_id: uuid(TRANSFER),
                    index: 0,
                    bytes: abc(),
                }),
            ),
            request_json(
                LOCAL,
                "part",
                &format!(r#"{{"transferId":"{TRANSFER}","index":0,"bytes":"{ABC}"}}"#),
            ),
        ),
        (
            "request commit",
            request(
                Origin::LocalExtension,
                Call::Commit(TransferInput {
                    transfer_id: uuid(TRANSFER),
                }),
            ),
            request_json(LOCAL, "commit", &transfer),
        ),
        (
            "request discard",
            request(
                Origin::LocalExtension,
                Call::Discard(TransferInput {
                    transfer_id: uuid(TRANSFER),
                }),
            ),
            request_json(LOCAL, "discard", &transfer),
        ),
        (
            "request status",
            request(
                Origin::LocalExtension,
                Call::Status(StatusInput {
                    transfer_id: uuid(TRANSFER),
                    namespace: namespace(),
                    policy: policy(),
                }),
            ),
            request_json(
                LOCAL,
                "status",
                &format!(
                    r#"{{"transferId":"{TRANSFER}","namespace":"{ONES}","policyInput":"{POLICY}"}}"#
                ),
            ),
        ),
        (
            "request read",
            request(
                Origin::LocalExtension,
                Call::Read(ReadInput {
                    namespace: namespace(),
                    opaque_key: key(),
                    policy: policy(),
                    payload_sha256: digest(),
                    payload_bytes: 3,
                    offset: 0,
                    count: 3,
                }),
            ),
            request_json(
                LOCAL,
                "read",
                &format!(
                    r#"{{"namespace":"{ONES}","opaqueKey":"{TWOS}","policyInput":"{POLICY}","payloadSha256":"{DIGEST}","payloadBytes":3,"offset":0,"count":3}}"#
                ),
            ),
        ),
        (
            "config browser",
            success(Method::Config, Success::Config(config())),
            result_json("config", false, &config_ok("browser", BROWSER_LIMITS)),
        ),
        (
            "config local",
            success(Method::Config, Success::Config(local_config)),
            result_json("config", false, &config_ok("local", LOCAL_LIMITS)),
        ),
        (
            "begin pending",
            success(
                Method::Begin,
                Success::Pending {
                    next_index: 0,
                    received: 0,
                    expires_at_ms: None,
                },
            ),
            result_json(
                "begin",
                true,
                r#""ok":{"result":"pending","nextIndex":0,"received":0}"#,
            ),
        ),
        (
            "status pending",
            success(
                Method::Status,
                Success::Pending {
                    next_index: 2,
                    received: 65_536,
                    expires_at_ms: Some(MAX_SAFE_INTEGER),
                },
            ),
            result_json(
                "status",
                true,
                r#""ok":{"result":"pending","nextIndex":2,"received":65536,"expiresAtMs":9007199254740991}"#,
            ),
        ),
        (
            "part progress",
            success(
                Method::Part,
                Success::Progress {
                    next_index: 1,
                    received: 3,
                },
            ),
            result_json(
                "part",
                true,
                r#""ok":{"result":"progress","nextIndex":1,"received":3}"#,
            ),
        ),
        (
            "commit committed",
            success(Method::Commit, committed_value()),
            result_json("commit", true, &committed),
        ),
        (
            "discard discarded",
            success(Method::Discard, Success::State(State::Discarded)),
            result_json("discard", true, &state("discarded")),
        ),
        (
            "read part",
            success(
                Method::Read,
                Success::Read {
                    offset: 0,
                    total_bytes: 3,
                    bytes: abc(),
                },
            ),
            result_json(
                "read",
                false,
                &format!(
                    r#""ok":{{"result":"read","offset":0,"totalBytes":3,"bytes":"{ABC}"}}"#
                ),
            ),
        ),
        (
            "read empty",
            success(
                Method::Read,
                Success::Read {
                    offset: 0,
                    total_bytes: 0,
                    bytes: Chunk::new(Vec::new()).unwrap(),
                },
            ),
            result_json(
                "read",
                false,
                r#""ok":{"result":"read","offset":0,"totalBytes":0,"bytes":""}"#,
            ),
        ),
        (
            "begin unknown",
            failure(Method::Begin, ErrorCode::Unknown),
            result_json("begin", true, r#""error":{"code":"unknown"}"#),
        ),
        (
            "read not-found",
            failure(Method::Read, ErrorCode::NotFound),
            result_json("read", false, r#""error":{"code":"not-found"}"#),
        ),
        (
            "config denied",
            failure(Method::Config, ErrorCode::Denied),
            result_json("config", false, r#""error":{"code":"denied"}"#),
        ),
    ]
    .into_iter()
    .map(|(name, frame, json)| (name.to_owned(), frame, json))
    .collect::<Vec<_>>();
    for (method, word) in [(Method::Begin, "begin"), (Method::Status, "status")] {
        all.push((
            format!("{word} committed"),
            success(method, committed_value()),
            result_json(word, true, &committed),
        ));
        for (name, value) in STATES {
            all.push((
                format!("{word} {name}"),
                success(method, Success::State(value)),
                result_json(word, true, &state(name)),
            ));
        }
    }
    for (name, limit) in LIMITS {
        all.push((
            format!("capacity {name}"),
            failure(Method::Part, ErrorCode::Capacity(limit)),
            result_json(
                "part",
                true,
                &format!(r#""error":{{"code":"capacity","limit":"{name}"}}"#),
            ),
        ));
    }
    all
}
fn sample(name: &str) -> String {
    vectors()
        .into_iter()
        .find(|(vector, _, _)| vector == name)
        .unwrap()
        .2
}
/// `json` with its one occurrence of `from` replaced by `to`.
fn mutate(json: &str, from: &str, to: &str) -> String {
    assert_eq!(json.matches(from).count(), 1, "{from} must occur once");
    json.replacen(from, to, 1)
}

#[test]
fn every_accepted_frame_has_one_canonical_spelling() {
    for (name, frame, json) in vectors() {
        assert_eq!(
            encode(&frame).unwrap(),
            with_prefix(&json),
            "{name} encodes"
        );
        assert_eq!(decode(json.as_bytes()).unwrap(), frame, "{name} decodes");
    }
}

#[test]
fn field_order_and_whitespace_do_not_change_the_frame() {
    let canonical = sample("request commit");
    let reordered = format!(
        r#" {{ "input" : {{"transferId":"{TRANSFER}"}} , "method":"objects.commit", "origin":{LOCAL}, "requestId":"7", "generation":"{GENERATION}", "kind":"request", "version":1 }} "#
    );
    let frame = decode(canonical.as_bytes()).unwrap();
    assert_eq!(decode(reordered.as_bytes()).unwrap(), frame);
    assert_eq!(encode(&frame).unwrap(), with_prefix(&canonical));
}

/// Each row: the accepted vector, the token changed, its replacement, the class.
/// Structure (unknown or missing field, wrong JSON type, unknown discriminator) is
/// `Shape`; a well-formed value outside its grammar, range or association is `Value`.
#[test]
fn every_refusal_is_one_token_from_an_accepted_twin() {
    use ErrorClass::{Shape, Value};
    let begin = "request begin";
    let transfer_field = format!(r#""transferId":"{TRANSFER}","#);
    let big = format!(r#""id":"a{}""#, "b".repeat(32));
    #[rustfmt::skip]
    let cases: Vec<(&str, &str, String, ErrorClass)> = vec![
        // Kinds, versions, discriminators and field structure.
        (begin, r#""kind":"request""#, r#""kind":"admit""#.into(), Shape),
        (begin, r#""kind":"request""#, r#""kind":"admission""#.into(), Shape),
        (begin, r#""kind":"request""#, r#""kind":"origin-state""#.into(), Shape),
        (begin, r#""kind":"request""#, r#""kind":"Request""#.into(), Shape),
        (begin, r#""version":1"#, r#""version":2"#.into(), Value),
        (begin, r#""version":1"#, r#""version":"1""#.into(), Shape),
        (begin, r#""objects.begin""#, r#""objects.copy""#.into(), Shape),
        (begin, r#""kind":"mounted""#, r#""kind":"remote""#.into(), Shape),
        (begin, r#""generation""#, r#""generated""#.into(), Shape),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":3,"extra":1"#.into(), Shape),
        ("request commit", r#"{"transferId""#, r#"{"index":0,"transferId""#.into(), Shape),
        ("request part", r#""index":0"#, r#""offset":0"#.into(), Shape),
        // Counters are strings; numbers are unsigned integers within the safe range.
        (begin, r#""requestId":"7""#, r#""requestId":7"#.into(), Shape),
        (begin, r#""requestId":"7""#, r#""requestId":"07""#.into(), Value),
        (begin, r#""requestId":"7""#, r#""requestId":"0""#.into(), Value),
        (begin, r#""requestId":"7""#, r#""requestId":"18446744073709551616""#.into(), Value),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":"3""#.into(), Shape),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":3.0"#.into(), Shape),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":-3"#.into(), Shape),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":9007199254740992"#.into(), Value),
        (begin, r#""payloadBytes":3"#, r#""payloadBytes":12582913"#.into(), Value),
        // Canonical spellings.
        (begin, ORIGIN, ORIGIN.to_uppercase(), Value),
        (begin, GENERATION, "7f3c1a52-9d4e-1b86-8a21-5c0e9b7d3f14".into(), Value),
        (begin, ONES, format!("{ONES}="), Value),
        (begin, ONES, ONES.replace('Q', "+"), Value),
        (begin, ONES, format!("{}F", &ONES[..42]), Value),
        (begin, DIGEST, DIGEST.to_uppercase(), Value),
        (begin, DIGEST, DIGEST[..63].into(), Value),
        (begin, POLICY, format!("{POLICY}="), Value),
        ("request part", ABC, "YWJ+".into(), Value),
        ("request part", ABC, "QR".into(), Value),
        ("request part", r#""bytes":"YWJj""#, r#""bytes":"""#.into(), Value),
        // Read windows.
        ("request read", r#""count":3"#, r#""count":0"#.into(), Value),
        ("request read", r#""count":3"#, r#""count":32769"#.into(), Value),
        ("request read", r#""offset":0"#, r#""offset":3"#.into(), Value),
        ("request read", r#""offset":0"#, r#""offset":4"#.into(), Value),
        // Result and error associations.
        ("part progress", r#""ok":"#, r#""error":{"code":"denied"},"ok":"#.into(), Shape),
        ("part progress", &transfer_field, String::new(), Value),
        ("part progress", r#""result":"progress""#, r#""result":"pending""#.into(), Value),
        ("part progress", r#""result":"progress""#, r#""result":"advanced""#.into(), Shape),
        ("part progress", r#""method":"objects.part""#, r#""method":"objects.read""#.into(), Value),
        ("part progress", r#""received":3"#, r#""received":12582913"#.into(), Value),
        ("part progress", r#""nextIndex":1"#, r#""nextIndex":4294967296"#.into(), Value),
        ("status pending", r#""method":"objects.status""#, r#""method":"objects.begin""#.into(), Value),
        ("status pending", "9007199254740991", "9007199254740992".into(), Value),
        ("begin pending", r#""received":0"#, r#""received":0,"expiresAtMs":1"#.into(), Value),
        ("begin pending", r#""method":"objects.begin""#, r#""method":"objects.part""#.into(), Value),
        ("commit committed", r#""method":"objects.commit""#, r#""method":"objects.discard""#.into(), Value),
        ("discard discarded", r#""state":"discarded""#, r#""state":"expired""#.into(), Value),
        ("discard discarded", r#""state":"discarded""#, r#""state":"gone""#.into(), Value),
        ("read part", r#""totalBytes":3"#, r#""totalBytes":2"#.into(), Value),
        ("read part", r#""offset":0"#, r#""offset":1"#.into(), Value),
        ("begin unknown", r#""code":"unknown""#, r#""code":"capacity""#.into(), Shape),
        ("begin unknown", r#""method":"objects.begin""#, r#""method":"objects.status""#.into(), Value),
        ("read not-found", r#""method":"objects.read""#, r#""method":"objects.begin""#.into(), Value),
        ("capacity requests", r#""code":"capacity","limit":"requests""#, r#""code":"capacity""#.into(), Shape),
        ("capacity requests", r#""code":"capacity","limit":"requests""#, r#""code":"denied","limit":"requests""#.into(), Shape),
        ("capacity requests", r#""limit":"requests""#, r#""limit":"bytes""#.into(), Value),
        // Configuration.
        ("config browser", r#""chunkBytes":32768"#, r#""chunkBytes":32769"#.into(), Value),
        ("config browser", r#""chunkBytes":32768"#, r#""chunkBytes":0"#.into(), Value),
        ("config browser", r#""payloadBytes":12582912"#, r#""payloadBytes":12582913"#.into(), Value),
        ("config browser", r#""projection":"browser""#, r#""projection":"remote""#.into(), Value),
        ("config browser", r#""projection":"browser""#, r#""projection":"local""#.into(), Shape),
        ("config local", r#""projection":"local""#, r#""projection":"browser""#.into(), Shape),
        ("config local", r#""installationBytes":1073741824"#, r#""installationBytes":0"#.into(), Value),
        ("config browser", r#""source":"default""#, r#""source":"user""#.into(), Value),
        ("config browser", r#""editable":false"#, r#""editable":true"#.into(), Value),
        ("config browser", r#""id":"local-fs""#, r#""id":"Local""#.into(), Value),
        ("config browser", r#""id":"local-fs""#, r#""id":"-local""#.into(), Value),
        ("config browser", r#""id":"local-fs""#, big, Value),
        ("config browser", r#""chunkedRead":true"#, r#""chunkedRead":1"#.into(), Shape),
    ];
    for (twin, from, to, class) in cases {
        let accepted = sample(twin);
        assert!(decode(accepted.as_bytes()).is_ok(), "{twin} twin");
        let refused = mutate(&accepted, from, &to);
        assert_eq!(
            decode(refused.as_bytes()),
            Err(class),
            "{twin}: {from} -> {to}"
        );
    }
}

#[test]
fn bytes_that_are_not_one_frame_are_refused_by_admission() {
    let accepted = sample("request commit");
    let doubled = mutate(
        &accepted,
        r#""kind":"request""#,
        r#""kind":"request","kind":"request""#,
    );
    // `i` spells `i`: the member name differs in bytes but repeats after decoding.
    let escaped = mutate(
        &accepted,
        r#""kind":"request""#,
        r#""kind":"request","kind":"request""#,
    );
    assert!(escaped.contains(r"kind") && escaped.matches("\"kind\"").count() == 1);
    let cases: Vec<(&str, Vec<u8>, ErrorClass)> = vec![
        (
            "repeated member",
            doubled.into_bytes(),
            ErrorClass::Duplicate,
        ),
        (
            "repeated member via an escape",
            escaped.into_bytes(),
            ErrorClass::Duplicate,
        ),
        (
            "trailing bytes",
            format!("{accepted}x").into_bytes(),
            ErrorClass::Trailing,
        ),
        (
            "not UTF-8",
            [accepted.as_bytes(), &[0xff]].concat(),
            ErrorClass::Utf8,
        ),
        ("bare array", b"[]".to_vec(), ErrorClass::Shape),
        (
            "truncated",
            accepted.as_bytes()[..accepted.len() - 1].to_vec(),
            ErrorClass::Syntax,
        ),
    ];
    for (name, body, class) in cases {
        assert_eq!(decode(&body), Err(class), "{name}");
    }
    assert!(decode(accepted.as_bytes()).is_ok());
}

/// Methods and the results each may answer, written independently of the code.
const ASSOCIATIONS: [(Method, &[&str]); 7] = [
    (Method::Config, &["config"]),
    (
        Method::Begin,
        &[
            "pending",
            "committed",
            "expired",
            "discarded",
            "unavailable",
            "unknown",
            "notObserved",
        ],
    ),
    (Method::Part, &["progress"]),
    (Method::Commit, &["committed"]),
    (
        Method::Status,
        &[
            "pending",
            "committed",
            "expired",
            "discarded",
            "unavailable",
            "unknown",
            "notObserved",
        ],
    ),
    (Method::Read, &["read"]),
    (Method::Discard, &["discarded"]),
];
const RESULTS: [&str; 10] = [
    "config",
    "pending",
    "progress",
    "committed",
    "read",
    "expired",
    "discarded",
    "unavailable",
    "unknown",
    "notObserved",
];

fn success_named(name: &str) -> Success {
    match name {
        "config" => Success::Config(config()),
        "pending" => Success::Pending {
            next_index: 0,
            received: 0,
            expires_at_ms: None,
        },
        "progress" => Success::Progress {
            next_index: 0,
            received: 0,
        },
        "committed" => committed_value(),
        "read" => Success::Read {
            offset: 0,
            total_bytes: 3,
            bytes: abc(),
        },
        state => Success::State(STATES.iter().find(|(word, _)| *word == state).unwrap().1),
    }
}

#[test]
fn each_method_answers_only_its_own_results_and_errors() {
    for (method, allowed) in ASSOCIATIONS {
        for name in RESULTS {
            let frame = success(method, success_named(name));
            assert_eq!(
                encode(&frame).is_ok(),
                allowed.contains(&name),
                "{method:?} answers {name}"
            );
            // What encodes must come back through the decoder's own checks.
            if let Ok(bytes) = encode(&frame) {
                assert_eq!(decode(&bytes[PREFIX_BYTES..]).unwrap(), frame);
            }
        }
        let mutator = matches!(
            method,
            Method::Begin | Method::Part | Method::Commit | Method::Discard
        );
        let codes = [
            (ErrorCode::Denied, true),
            (ErrorCode::Unavailable, true),
            (ErrorCode::Invalid, true),
            (ErrorCode::Conflict, true),
            (ErrorCode::Capacity(Limit::Requests), true),
            (ErrorCode::NotFound, method == Method::Read),
            (ErrorCode::Unknown, mutator),
        ];
        for (code, allowed) in codes {
            let frame = failure(method, code);
            assert_eq!(encode(&frame).is_ok(), allowed, "{method:?} fails {code:?}");
        }
    }
}

#[test]
fn the_original_transfer_id_is_repeated_exactly_where_a_transfer_is_named() {
    for (method, _) in ASSOCIATIONS {
        let named = !matches!(method, Method::Config | Method::Read);
        for transfer_id in [None, Some(uuid(TRANSFER))] {
            let frame = Frame::Result(ResultFrame {
                generation: uuid(GENERATION),
                request_id: Counter::new(7).unwrap(),
                method,
                transfer_id,
                outcome: Outcome::Failure(ErrorCode::Denied),
            });
            assert_eq!(
                encode(&frame).is_ok(),
                transfer_id.is_some() == named,
                "{method:?}"
            );
        }
    }
}

#[test]
fn part_index_is_a_u32_independent_of_the_transport_chunk_ceiling() {
    let part = |index: &str| {
        mutate(
            &sample("request part"),
            r#""index":0"#,
            &format!(r#""index":{index}"#),
        )
    };
    for index in [0, 1, 383, 384, 3071, 3072, u32::MAX] {
        let json = part(&index.to_string());
        let frame = decode(json.as_bytes()).unwrap();
        let Frame::Request(Request {
            call: Call::Part(input),
            ..
        }) = &frame
        else {
            panic!("part request");
        };
        assert_eq!(input.index, index);
        assert_eq!(encode(&frame).unwrap(), with_prefix(&json));
    }
    assert_eq!(
        decode(part("4294967296").as_bytes()),
        Err(ErrorClass::Value)
    );
}

#[test]
fn byte_strings_stop_at_their_bounds_and_the_largest_frames_fit() {
    let part = |bytes: usize| {
        request(
            Origin::LocalExtension,
            Call::Part(PartInput {
                transfer_id: uuid(TRANSFER),
                index: 0,
                bytes: Chunk::new(vec![0xa5; bytes]).unwrap(),
            }),
        )
    };
    assert!(encode(&part(1)).is_ok());
    assert!(encode(&part(0)).is_err(), "an empty part is refused");
    let full = encode(&part(CHUNK_BYTES)).unwrap();
    let announced = crate::decode_length(full[..PREFIX_BYTES].try_into().unwrap());
    assert_eq!(announced, Ok(full.len() - PREFIX_BYTES));
    assert!(decode(&full[PREFIX_BYTES..]).is_ok());
    assert!(Chunk::new(vec![0; CHUNK_BYTES + 1]).is_err());
    let text = Chunk::new(vec![0; CHUNK_BYTES]).unwrap().to_string();
    assert_eq!(Chunk::parse(&text).unwrap().as_bytes().len(), CHUNK_BYTES);
    assert!(Chunk::parse(&format!("{text}AAAA")).is_err());
    let policy = Policy::new(vec![1; POLICY_BYTES]).unwrap().to_string();
    assert_eq!(
        Policy::parse(&policy).unwrap().as_bytes().len(),
        POLICY_BYTES
    );
    assert!(Policy::parse(&format!("{policy}AAAA")).is_err());
    assert!(Policy::new(vec![1; POLICY_BYTES + 1]).is_err());
    let read = success(
        Method::Read,
        Success::Read {
            offset: 0,
            total_bytes: PAYLOAD_BYTES,
            bytes: Chunk::new(vec![0xff; CHUNK_BYTES]).unwrap(),
        },
    );
    assert!(encode(&read).unwrap().len() <= PREFIX_BYTES + FRAME_BYTES);
}

#[test]
fn read_requests_may_start_only_where_bytes_exist() {
    let read = |payload_bytes: u64, offset: u64, count: u32| {
        request(
            Origin::LocalExtension,
            Call::Read(ReadInput {
                namespace: namespace(),
                opaque_key: key(),
                policy: policy(),
                payload_sha256: digest(),
                payload_bytes,
                offset,
                count,
            }),
        )
    };
    let accepted = [
        (3, 2, 1),
        (3, 0, 32_768),
        (PAYLOAD_BYTES, PAYLOAD_BYTES - 1, 1),
        (0, 0, 1),
    ];
    for (payload, offset, count) in accepted {
        assert!(
            encode(&read(payload, offset, count)).is_ok(),
            "{payload} {offset} {count}"
        );
    }
    let refused = [
        (3, 3, 1),
        (3, 4, 1),
        (3, 0, 0),
        (3, 0, 32_769),
        (PAYLOAD_BYTES + 1, 0, 1),
        (0, 1, 1),
    ];
    for (payload, offset, count) in refused {
        assert!(
            encode(&read(payload, offset, count)).is_err(),
            "{payload} {offset} {count}"
        );
    }
}
