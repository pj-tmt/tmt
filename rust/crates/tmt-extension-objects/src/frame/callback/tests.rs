//! Callback and lifecycle frames: every accepted frame is pinned to its canonical
//! bytes, the method x checkpoint x disclosure matrix is enumerated cell by cell
//! against a table written independently of the code, and every refusal is a
//! one-token mutation of an accepted twin.
use super::super::tests::{
    DIGEST, GENERATION, ONES, ORIGIN, POLICY, STATES, TRANSFER, TWOS, digest, key, mutate,
    namespace, policy, uuid, with_prefix,
};
use super::*;

const DEVICE: &str = "5a1f0c3e-7b2d-4e9a-9c4b-3d8e6f1a2b07";
const LOCAL: &str = r#"{"kind":"local-extension"}"#;
const BACKSLASH: char = '\\';

fn owner_json(revision: u64) -> String {
    format!(
        r#"{{"kind":"owner-session","originId":"{ORIGIN}","deviceId":"{DEVICE}","grantRevision":{revision}}}"#
    )
}
fn mounted_json() -> String {
    format!(r#"{{"kind":"mounted","originId":"{ORIGIN}"}}"#)
}
fn retained_json() -> String {
    format!(
        r#"{{"namespace":"{ONES}","opaqueKey":"{TWOS}","policyInput":"{POLICY}","payloadSha256":"{DIGEST}","payloadBytes":3}}"#
    )
}
fn retained() -> Retained {
    Retained {
        namespace: namespace(),
        opaque_key: key(),
        policy: policy(),
        payload_sha256: digest(),
        payload_bytes: 3,
    }
}

/// One input per method with the exact JSON it must write.
fn inputs() -> Vec<(Method, AdmitInput, String)> {
    let transfer_id = uuid(TRANSFER);
    let policy_input = format!(r#""namespace":"{ONES}","policyInput":"{POLICY}""#);
    let begin = BeginInput {
        transfer_id,
        namespace: namespace(),
        opaque_key: key(),
        policy: policy(),
        payload_sha256: digest(),
        payload_bytes: 3,
    };
    let status = StatusInput {
        transfer_id,
        namespace: namespace(),
        policy: policy(),
    };
    let read = ReadInput {
        namespace: namespace(),
        opaque_key: key(),
        policy: policy(),
        payload_sha256: digest(),
        payload_bytes: 3,
        offset: 0,
        count: 3,
    };
    let transfer = TransferAdmit {
        transfer_id,
        retained: retained(),
    };
    let held = format!(
        r#"{{"transferId":"{TRANSFER}","retained":{}}}"#,
        retained_json()
    );
    vec![
        (
            Method::Config,
            AdmitInput::Config(ConfigInput {
                namespace: namespace(),
                policy: policy(),
            }),
            format!("{{{policy_input}}}"),
        ),
        (
            Method::Begin,
            AdmitInput::Begin(begin),
            format!(
                r#"{{"transferId":"{TRANSFER}","namespace":"{ONES}","opaqueKey":"{TWOS}","policyInput":"{POLICY}","payloadSha256":"{DIGEST}","payloadBytes":3}}"#
            ),
        ),
        (
            Method::Part,
            AdmitInput::Part(PartAdmit {
                transfer_id,
                index: 0,
                length: 3,
                retained: retained(),
            }),
            format!(
                r#"{{"transferId":"{TRANSFER}","index":0,"length":3,"retained":{}}}"#,
                retained_json()
            ),
        ),
        (
            Method::Commit,
            AdmitInput::Commit(transfer.clone()),
            held.clone(),
        ),
        (
            Method::Status,
            AdmitInput::Status(status),
            format!(r#"{{"transferId":"{TRANSFER}",{policy_input}}}"#),
        ),
        (
            Method::Read,
            AdmitInput::Read(read),
            format!(
                r#"{{"namespace":"{ONES}","opaqueKey":"{TWOS}","policyInput":"{POLICY}","payloadSha256":"{DIGEST}","payloadBytes":3,"offset":0,"count":3}}"#
            ),
        ),
        (Method::Discard, AdmitInput::Discard(transfer), held),
    ]
}
fn input(method: Method) -> (AdmitInput, String) {
    let (_, input, json) = inputs().into_iter().find(|row| row.0 == method).unwrap();
    (input, json)
}
fn word(method: Method) -> &'static str {
    match method {
        Method::Config => "config",
        Method::Begin => "begin",
        Method::Part => "part",
        Method::Commit => "commit",
        Method::Status => "status",
        Method::Read => "read",
        Method::Discard => "discard",
    }
}

/// Every disclosure class with the JSON it must write. The first column names the
/// class for the independent matrix below.
fn disclosures() -> Vec<(String, Disclosure, String)> {
    let mut all = vec![
        (
            "config".to_owned(),
            Disclosure::Config {
                projection: Projection::Browser,
            },
            r#"{"class":"config","projection":"browser"}"#.to_owned(),
        ),
        (
            "status".to_owned(),
            Disclosure::Status {
                next_index: 0,
                received: 0,
                expires_at_ms: None,
            },
            r#"{"class":"status","nextIndex":0,"received":0}"#.to_owned(),
        ),
        (
            "status-expiry".to_owned(),
            Disclosure::Status {
                next_index: 2,
                received: 65_536,
                expires_at_ms: Some(MAX_SAFE_INTEGER),
            },
            r#"{"class":"status","nextIndex":2,"received":65536,"expiresAtMs":9007199254740991}"#
                .to_owned(),
        ),
        (
            "progress".to_owned(),
            Disclosure::Progress {
                next_index: 1,
                received: 3,
            },
            r#"{"class":"progress","nextIndex":1,"received":3}"#.to_owned(),
        ),
        (
            "receipt".to_owned(),
            Disclosure::Receipt {
                opaque_key: key(),
                payload_sha256: digest(),
                payload_bytes: 3,
            },
            format!(
                r#"{{"class":"receipt","opaqueKey":"{TWOS}","payloadSha256":"{DIGEST}","payloadBytes":3}}"#
            ),
        ),
        (
            "bytes".to_owned(),
            Disclosure::Bytes {
                offset: 0,
                length: 3,
            },
            r#"{"class":"bytes","offset":0,"length":3}"#.to_owned(),
        ),
        (
            "bytes-empty".to_owned(),
            Disclosure::Bytes {
                offset: 0,
                length: 0,
            },
            r#"{"class":"bytes","offset":0,"length":0}"#.to_owned(),
        ),
    ];
    for (name, state) in STATES {
        all.push((
            format!("terminal:{name}"),
            Disclosure::Terminal { state },
            format!(r#"{{"class":"terminal","state":"{name}"}}"#),
        ));
    }
    all
}
fn disclosure(class: &str) -> (Disclosure, String) {
    let (_, value, json) = disclosures()
        .into_iter()
        .find(|row| row.0 == class)
        .unwrap();
    (value, json)
}

fn admit(
    checkpoint: Checkpoint,
    context: Context,
    input: AdmitInput,
    disclosure: Option<Disclosure>,
) -> Frame {
    Frame::Admit(Admit {
        generation: uuid(GENERATION),
        callback_id: Counter::new(3).unwrap(),
        request_id: Counter::new(7).unwrap(),
        boundary: checkpoint,
        context,
        operation: Operation { input, disclosure },
    })
}
fn admit_json(
    boundary: &str,
    context: &str,
    method: Method,
    input: &str,
    disclosure: Option<&str>,
) -> String {
    let disclosure = disclosure.map_or_else(String::new, |json| format!(r#","disclosure":{json}"#));
    format!(
        r#"{{"version":1,"kind":"admit","generation":"{GENERATION}","callbackId":"3","requestId":"7","boundary":"{boundary}","context":{context},"operation":{{"method":"objects.{}","input":{input}{disclosure}}}}}"#,
        word(method)
    )
}
fn owner(revision: u64) -> Context {
    Context::OwnerSession {
        origin_id: uuid(ORIGIN),
        device_id: uuid(DEVICE),
        grant_revision: revision,
    }
}
fn admission_json(decision: &str) -> String {
    format!(
        r#"{{"version":1,"kind":"admission","generation":"{GENERATION}","callbackId":"3","requestId":"7","decision":"{decision}"}}"#
    )
}
fn origin_state_json(state: &str) -> String {
    format!(
        r#"{{"version":1,"kind":"origin-state","generation":"{GENERATION}","originId":"{ORIGIN}","state":"{state}"}}"#
    )
}

/// Every accepted shape with the exact compact bytes it must encode to.
fn vectors() -> Vec<(String, Frame, String)> {
    let mut all = Vec::new();
    let mut push = |name: String, frame: Frame, json: String| all.push((name, frame, json));
    for (method, value, json) in inputs() {
        push(
            format!("admit {} acquire", word(method)),
            admit(
                Checkpoint::Acquire,
                Context::LocalExtension,
                value.clone(),
                None,
            ),
            admit_json("acquire", LOCAL, method, &json, None),
        );
        if method.mutates() {
            push(
                format!("admit {} effect", word(method)),
                admit(Checkpoint::Effect, owner(4), value, None),
                admit_json("effect", &owner_json(4), method, &json, None),
            );
        }
    }
    let disclosed = [
        (Method::Config, "config"),
        (Method::Begin, "status"),
        (Method::Begin, "receipt"),
        (Method::Begin, "terminal:expired"),
        (Method::Part, "progress"),
        (Method::Commit, "receipt"),
        (Method::Status, "status-expiry"),
        (Method::Status, "receipt"),
        (Method::Status, "terminal:unknown"),
        (Method::Read, "bytes"),
        (Method::Read, "bytes-empty"),
        (Method::Discard, "terminal:discarded"),
    ];
    for (method, class) in disclosed {
        let (value, json) = input(method);
        let (shown, shown_json) = disclosure(class);
        push(
            format!("admit {} disclose {class}", word(method)),
            admit(
                Checkpoint::Disclose,
                Context::Mounted {
                    origin_id: uuid(ORIGIN),
                },
                value,
                Some(shown),
            ),
            admit_json(
                "disclose",
                &mounted_json(),
                method,
                &json,
                Some(&shown_json),
            ),
        );
    }
    push(
        "admit owner at the largest revision".to_owned(),
        admit(
            Checkpoint::Acquire,
            owner(MAX_SAFE_INTEGER),
            input(Method::Commit).0,
            None,
        ),
        admit_json(
            "acquire",
            &owner_json(MAX_SAFE_INTEGER),
            Method::Commit,
            &input(Method::Commit).1,
            None,
        ),
    );
    for (word, decision) in [
        ("allow", Decision::Allow),
        ("deny", Decision::Deny),
        ("unavailable", Decision::Unavailable),
    ] {
        push(
            format!("admission {word}"),
            Frame::Admission(Admission {
                generation: uuid(GENERATION),
                callback_id: Counter::new(3).unwrap(),
                request_id: Counter::new(7).unwrap(),
                decision,
            }),
            admission_json(word),
        );
    }
    for (word, phase) in [
        ("established", OriginPhase::Established),
        ("closed", OriginPhase::Closed),
    ] {
        push(
            format!("origin-state {word}"),
            Frame::OriginState(OriginState {
                generation: uuid(GENERATION),
                origin_id: uuid(ORIGIN),
                phase,
            }),
            origin_state_json(word),
        );
    }
    all
}
fn sample(name: &str) -> String {
    vectors()
        .into_iter()
        .find(|row| row.0 == name)
        .unwrap_or_else(|| panic!("{name}"))
        .2
}

#[test]
fn every_accepted_callback_frame_has_one_canonical_spelling() {
    for (name, frame, json) in vectors() {
        assert_eq!(
            encode(&frame).unwrap(),
            with_prefix(&json),
            "{name} encodes"
        );
        assert_eq!(decode(json.as_bytes()).unwrap(), frame, "{name} decodes");
    }
}

/// What may be asked at which checkpoint and what may be disclosed, written out
/// without reference to the code's own association.
fn expected(method: Method, checkpoint: Checkpoint, class: Option<&str>) -> bool {
    let disclosable: &[&str] = match method {
        Method::Config => &["config"],
        Method::Begin => &[
            "status",
            "receipt",
            "terminal:expired",
            "terminal:discarded",
            "terminal:unavailable",
            "terminal:unknown",
            "terminal:notObserved",
        ],
        Method::Part => &["progress"],
        Method::Commit => &["receipt"],
        Method::Status => &[
            "status",
            "status-expiry",
            "receipt",
            "terminal:expired",
            "terminal:discarded",
            "terminal:unavailable",
            "terminal:unknown",
            "terminal:notObserved",
        ],
        Method::Read => &["bytes", "bytes-empty"],
        Method::Discard => &["terminal:discarded"],
    };
    let mutator = matches!(
        method,
        Method::Begin | Method::Part | Method::Commit | Method::Discard
    );
    match (checkpoint, class) {
        (Checkpoint::Acquire, None) => true,
        (Checkpoint::Effect, None) => mutator,
        (Checkpoint::Disclose, Some(class)) => disclosable.contains(&class),
        _ => false,
    }
}

#[test]
fn every_method_checkpoint_and_disclosure_cell_is_enumerated() {
    let classes = disclosures();
    let mut accepted = 0;
    for (method, value, _) in inputs() {
        for checkpoint in [
            Checkpoint::Acquire,
            Checkpoint::Effect,
            Checkpoint::Disclose,
        ] {
            let choices = std::iter::once((None, None)).chain(
                classes
                    .iter()
                    .map(|row| (Some(row.0.as_str()), Some(row.1))),
            );
            for (class, shown) in choices {
                let frame = admit(checkpoint, Context::LocalExtension, value.clone(), shown);
                let allowed = expected(method, checkpoint, class);
                assert_eq!(
                    encode(&frame).is_ok(),
                    allowed,
                    "{method:?} {checkpoint:?} {class:?}"
                );
                if let Ok(bytes) = encode(&frame) {
                    accepted += 1;
                    assert_eq!(decode(&bytes[PREFIX_BYTES..]).unwrap(), frame);
                }
            }
        }
    }
    // 7 acquire + 4 effect + the disclosable classes of each method.
    assert_eq!(accepted, 7 + 4 + (1 + 7 + 1 + 1 + 8 + 2 + 1));
}

#[test]
fn a_disclosure_class_names_the_same_answer_as_the_result_it_announces() {
    let key = key();
    let digest = digest();
    let results = [
        (
            "status",
            Success::Pending {
                next_index: 0,
                received: 0,
                expires_at_ms: None,
            },
        ),
        (
            "progress",
            Success::Progress {
                next_index: 1,
                received: 3,
            },
        ),
        (
            "receipt",
            Success::Committed {
                opaque_key: key,
                payload_sha256: digest,
                payload_bytes: 3,
            },
        ),
        (
            "bytes",
            Success::Read {
                offset: 0,
                total_bytes: 3,
                bytes: Chunk::new(b"abc".to_vec()).unwrap(),
            },
        ),
        ("terminal:expired", Success::State(State::Expired)),
        ("terminal:discarded", Success::State(State::Discarded)),
    ];
    for (class, success) in results {
        assert_eq!(disclosure(class).0.answer(), success.answer(), "{class}");
    }
}

/// Each row: the accepted vector, the token changed, its replacement, the class.
/// Structure (unknown or missing member, wrong JSON type, unknown discriminator) is
/// `Shape`; a well-formed value outside its grammar, range or association is `Value`.
#[test]
fn every_refusal_is_one_token_from_an_accepted_twin() {
    use ErrorClass::{Shape, Value};
    let local_commit = "admit commit acquire";
    let part_effect = "admit part effect";
    let begin_status = "admit begin disclose status";
    let read_bytes = "admit read disclose bytes";
    let admission = "admission allow";
    let origin = "origin-state established";
    #[rustfmt::skip]
    let mut cases: Vec<(&str, &str, String, ErrorClass)> = vec![
        // Kinds and identifiers.
        (local_commit, r#""kind":"admit""#, r#""kind":"admitted""#.into(), Shape),
        (local_commit, r#""callbackId":"3""#, r#""callbackId":3"#.into(), Shape),
        (local_commit, r#""callbackId":"3""#, r#""callbackId":"0""#.into(), Value),
        (local_commit, r#""callbackId":"3""#, r#""callbackId":"03""#.into(), Value),
        (local_commit, r#""callbackId":"3""#, r#""callbackId":"18446744073709551616""#.into(), Value),
        (local_commit, r#""callbackId":"3""#, r#""callbackid":"3""#.into(), Shape),
        (local_commit, r#""requestId":"7""#, r#""requestId":7"#.into(), Shape),
        // Checkpoints.
        (part_effect, r#""boundary":"effect""#, r#""boundary":"release""#.into(), Value),
        (part_effect, r#""boundary":"effect""#, r#""boundary":"Effect""#.into(), Value),
        ("admit read acquire", r#""boundary":"acquire""#, r#""boundary":"effect""#.into(), Value),
        ("admit config acquire", r#""boundary":"acquire""#, r#""boundary":"effect""#.into(), Value),
        ("admit status acquire", r#""boundary":"acquire""#, r#""boundary":"effect""#.into(), Value),
        ("admit begin acquire", r#""boundary":"acquire""#, r#""boundary":"disclose""#.into(), Value),
        ("admit begin acquire", r#""method":"objects.begin""#, r#""disclosure":{"class":"status","nextIndex":0,"received":0},"method":"objects.begin""#.into(), Value),
        (part_effect, r#""method":"objects.part""#, r#""disclosure":{"class":"progress","nextIndex":1,"received":3},"method":"objects.part""#.into(), Value),
        // No byte payload and no authority inside a callback.
        (part_effect, r#""length":3"#, r#""length":3,"bytes":"YWJj""#.into(), Shape),
        (part_effect, r#""length":3"#, r#""length":0"#.into(), Value),
        (part_effect, r#""length":3"#, r#""length":32769"#.into(), Value),
        (part_effect, r#""length":3"#, r#""length":"3""#.into(), Shape),
        (part_effect, r#""index":0"#, r#""index":4294967296"#.into(), Value),
        (read_bytes, r#""offset":0,"length":3"#, r#""offset":0,"length":3,"bytes":"YWJj""#.into(), Shape),
        (read_bytes, r#""length":3"#, r#""length":32769"#.into(), Value),
        (read_bytes, r#""class":"bytes","offset":0"#, r#""class":"bytes","offset":12582913"#.into(), Value),
        ("admit begin acquire", r#""payloadBytes":3"#, r#""payloadBytes":3,"principal":"x""#.into(), Shape),
        (local_commit, r#""retained":{"#, r#""role":"owner","retained":{"#.into(), Shape),
        // Contexts.
        (part_effect, r#""grantRevision":4"#, r#""grantRevision":0"#.into(), Value),
        (part_effect, r#""grantRevision":4"#, r#""grantRevision":9007199254740992"#.into(), Value),
        (part_effect, r#""grantRevision":4"#, r#""grantRevision":"4""#.into(), Shape),
        (part_effect, r#""kind":"owner-session""#, r#""kind":"browser""#.into(), Shape),
        (part_effect, r#""kind":"owner-session""#, r#""kind":"mounted""#.into(), Shape),
        (part_effect, r#""deviceId""#, r#""device""#.into(), Shape),
        (part_effect, DEVICE, DEVICE.to_uppercase(), Value),
        ("admit begin acquire", r#"{"kind":"local-extension"}"#, format!(r#"{{"kind":"local-extension","originId":"{ORIGIN}"}}"#), Shape),
        ("admit begin disclose receipt", r#""kind":"mounted""#, format!(r#""kind":"mounted","deviceId":"{DEVICE}""#), Shape),
        // Operations and their inputs.
        (local_commit, r#""method":"objects.commit""#, r#""method":"objects.read""#.into(), Shape),
        (local_commit, r#""method":"objects.commit""#, r#""method":"objects.part""#.into(), Shape),
        (local_commit, r#""method":"objects.commit""#, r#""method":"objects.copy""#.into(), Shape),
        (local_commit, r#""payloadBytes":3"#, r#""payloadBytes":12582913"#.into(), Value),
        (local_commit, r#""payloadBytes":3"#, r#""payload":3"#.into(), Shape),
        ("admit read acquire", r#""count":3"#, r#""count":0"#.into(), Value),
        ("admit read acquire", r#""offset":0"#, r#""offset":3"#.into(), Value),
        // Disclosures: unknown class, wrong association, content out of range.
        (begin_status, r#""class":"status""#, r#""class":"gone""#.into(), Shape),
        (begin_status, r#""class":"status""#, r#""class":"progress""#.into(), Value),
        (begin_status, r#""class":"status""#, r#""class":"bytes""#.into(), Shape),
        (begin_status, r#""received":0"#, r#""received":0,"expiresAtMs":1"#.into(), Value),
        (begin_status, r#""nextIndex":0"#, r#""nextIndex":4294967296"#.into(), Value),
        (begin_status, r#""received":0"#, r#""received":12582913"#.into(), Value),
        ("admit status disclose status-expiry", "9007199254740991", "9007199254740992".into(), Value),
        ("admit status disclose terminal:unknown", r#""state":"unknown""#, r#""state":"gone""#.into(), Value),
        ("admit discard disclose terminal:discarded", r#""state":"discarded""#, r#""state":"expired""#.into(), Value),
        ("admit config disclose config", r#""projection":"browser""#, r#""projection":"remote""#.into(), Value),
        ("admit commit disclose receipt", r#""class":"receipt""#, r#""class":"status""#.into(), Shape),
        // Admission replies are a closed decision.
        (admission, r#""decision":"allow""#, r#""decision":"grant""#.into(), Value),
        (admission, r#""decision":"allow""#, r#""decision":"Allow""#.into(), Value),
        (admission, r#""decision":"allow""#, r#""decision":true"#.into(), Shape),
        (admission, r#""callbackId""#, r#""callback""#.into(), Shape),
        (admission, r#""requestId":"7""#, r#""requestId":7"#.into(), Shape),
        (admission, r#""kind":"admission""#, r#""kind":"Admission""#.into(), Shape),
        // Lifecycle.
        (origin, r#""state":"established""#, r#""state":"open""#.into(), Value),
        (origin, r#""state":"established""#, r#""state":"established","reason":"x""#.into(), Shape),
        (origin, r#""originId""#, r#""origin""#.into(), Shape),
        (origin, ORIGIN, ORIGIN.to_uppercase(), Value),
        (origin, r#""kind":"origin-state""#, r#""kind":"origin-closed""#.into(), Shape),
    ];
    for member in [
        "permit",
        "role",
        "principal",
        "scope",
        "target",
        "deviceId",
        "originId",
        "owner",
    ] {
        cases.push((
            admission,
            r#""decision":"allow""#,
            format!(r#""decision":"allow","{member}":"x""#),
            Shape,
        ));
        cases.push((
            admission,
            r#""decision":"allow""#,
            format!(r#""decision":"allow","{member}":true"#),
            Shape,
        ));
    }
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
fn escaped_duplicates_and_malformed_bytes_are_refused_in_callback_frames() {
    let escape = |text: &str| text.replace("%", &BACKSLASH.to_string());
    let allow = sample("admission allow");
    let repeat_id = mutate(
        &allow,
        r#""callbackId":"3""#,
        &escape(r#""callbackId":"3","callb%u0061ckId":"3""#),
    );
    let local = sample("admit begin acquire");
    let repeat_in_context = mutate(
        &local,
        LOCAL,
        &escape(r#"{"kind":"local-extension","k%u0069nd":"local-extension"}"#),
    );
    assert!(repeat_id.contains(&escape("%u0061")) && repeat_in_context.contains(&escape("%u0069")));
    let cases: Vec<(&str, Vec<u8>, ErrorClass)> = vec![
        (
            "repeated member via an escape",
            repeat_id.into_bytes(),
            ErrorClass::Duplicate,
        ),
        (
            "repeated nested member via an escape",
            repeat_in_context.into_bytes(),
            ErrorClass::Duplicate,
        ),
        (
            "trailing bytes",
            format!("{allow}x").into_bytes(),
            ErrorClass::Trailing,
        ),
        (
            "not UTF-8",
            [allow.as_bytes(), &[0xff]].concat(),
            ErrorClass::Utf8,
        ),
        (
            "truncated",
            allow.as_bytes()[..allow.len() - 1].to_vec(),
            ErrorClass::Syntax,
        ),
        (
            "two objects",
            format!("{allow}{allow}").into_bytes(),
            ErrorClass::Trailing,
        ),
    ];
    for (name, body, class) in cases {
        assert_eq!(decode(&body), Err(class), "{name}");
    }
}

#[test]
fn maxima_are_accepted_and_one_over_is_refused() {
    let frame = Frame::Admission(Admission {
        generation: uuid(GENERATION),
        callback_id: Counter::new(u64::MAX).unwrap(),
        request_id: Counter::new(u64::MAX).unwrap(),
        decision: Decision::Deny,
    });
    let bytes = encode(&frame).unwrap();
    assert_eq!(decode(&bytes[PREFIX_BYTES..]).unwrap(), frame);
    let maxed = String::from_utf8(bytes[PREFIX_BYTES..].to_vec()).unwrap();
    let over = mutate(
        &maxed,
        r#""callbackId":"18446744073709551615""#,
        r#""callbackId":"18446744073709551616""#,
    );
    assert_eq!(decode(over.as_bytes()), Err(ErrorClass::Value));
    let part = |index: u32, length: u32| {
        admit(
            Checkpoint::Effect,
            Context::LocalExtension,
            AdmitInput::Part(PartAdmit {
                transfer_id: uuid(TRANSFER),
                index,
                length,
                retained: retained(),
            }),
            None,
        )
    };
    for (index, length) in [(0, 1), (384, 32_768), (3071, 32_768), (u32::MAX, 32_768)] {
        let frame = part(index, length);
        let bytes = encode(&frame).unwrap();
        assert_eq!(decode(&bytes[PREFIX_BYTES..]).unwrap(), frame);
    }
    assert!(encode(&part(0, 0)).is_err() && encode(&part(0, 32_769)).is_err());
    let largest = admit(
        Checkpoint::Acquire,
        Context::Mounted {
            origin_id: uuid(ORIGIN),
        },
        AdmitInput::Begin(BeginInput {
            transfer_id: uuid(TRANSFER),
            namespace: namespace(),
            opaque_key: key(),
            policy: Policy::new(vec![0xff; crate::limits::POLICY_BYTES]).unwrap(),
            payload_sha256: digest(),
            payload_bytes: crate::limits::PAYLOAD_BYTES,
        }),
        None,
    );
    let bytes = encode(&largest).unwrap();
    assert!(bytes.len() <= PREFIX_BYTES + FRAME_BYTES);
    assert_eq!(decode(&bytes[PREFIX_BYTES..]).unwrap(), largest);
}
