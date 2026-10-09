use super::*;

#[test]
fn shared_attachment_grammar_admits_document_and_message_projections() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let value: serde_json::Value =
            serde_json::from_str(case["input"].as_str().unwrap()).unwrap();
        let result = match case["operation"].as_str().unwrap() {
            "document" => validate_projection(Namespace::Content, &value).is_ok(),
            "comment" => crate::threads::validate_record(
                "messages",
                &format!(
                    "{}:{}",
                    value["messageId"].as_str().unwrap(),
                    value["revision"].as_str().unwrap()
                ),
                &value,
            )
            .is_ok(),
            _ => continue,
        };
        assert_eq!(result, case["admit"].as_bool().unwrap(), "{}", case["name"]);
    }
}

#[test]
fn invocation_phases_follow_existing_namespace_edit_merge_and_baseline_actions() {
    for (namespace, edit, merge, expected) in [
        (Namespace::Content, false, false, "content.decode"),
        (Namespace::Own, false, false, "own.decode"),
        (Namespace::Own, true, false, "own.edit"),
        (Namespace::Content, false, true, "content.merge"),
        (Namespace::Own, false, true, "own.merge"),
    ] {
        assert_eq!(
            ChildCommand::decode(namespace, edit, merge).phase(),
            expected
        );
    }
    assert_eq!(ChildCommand::BaselineProduce.phase(), "baseline.produce");
    assert_eq!(ChildCommand::BaselinePage.phase(), "baseline.page");
    assert_eq!(ChildCommand::BaselineVerify.phase(), "baseline.verify");
    assert_eq!(
        ChildCommand::PrepareContent.phase(),
        "content.prepare-content"
    );
}

#[test]
fn failure_record_is_finite_ascii_and_excludes_original_private_causes() {
    use tmt_invoke::{FailureKind, InvokeError, Phase, Stream};
    for command in [
        ChildCommand::ContentDecode,
        ChildCommand::OwnDecode,
        ChildCommand::ContentMerge,
        ChildCommand::OwnMerge,
        ChildCommand::OwnEdit,
        ChildCommand::BaselineProduce,
        ChildCommand::BaselinePage,
        ChildCommand::BaselineVerify,
        ChildCommand::PrepareContent,
    ] {
        for kind in [
            FailureKind::Spawn,
            FailureKind::Deadline,
            FailureKind::Interrupted,
            FailureKind::OutputLimit(Stream::Stdout),
            FailureKind::OutputLimit(Stream::Stderr),
            FailureKind::Io(Phase::OpenPipes),
            FailureKind::Io(Phase::Communicate),
            FailureKind::Io(Phase::Wait),
        ] {
            for cleanup in [
                Cleanup::NotStarted,
                Cleanup::Confirmed,
                Cleanup::CallerOwned,
                Cleanup::Unconfirmed(std::io::Error::other("private-cleanup-🌱")),
            ] {
                let error = InvokeError {
                    kind,
                    cause: Some(std::io::Error::other("/private/source-key-🌱")),
                    cleanup,
                };
                let mut record = Vec::new();
                write_invocation_failure(
                    &mut record,
                    &error,
                    InvocationObservation {
                        command,
                        input_bytes: usize::MAX,
                        remaining: Duration::MAX,
                        elapsed: Duration::MAX,
                        parent_timing: Some(ParentTiming {
                            wire: Duration::MAX,
                            json: Duration::MAX,
                            hash: Duration::MAX,
                        }),
                    },
                );
                assert!(record.is_ascii() && record.len() <= 512);
                let line = String::from_utf8(record).unwrap();
                assert_eq!(line.lines().count(), 1);
                assert!(line.contains(&format!("phase={} ", command.phase())));
                assert!(line.contains(&format!("input_bytes={} ", usize::MAX)));
                assert!(line.contains(&format!("remaining_ns={} ", Duration::MAX.as_nanos())));
                assert!(line.contains(&format!("invocation_ns={} ", Duration::MAX.as_nanos())));
                assert!(
                    !line.contains("private") && !line.contains("source") && !line.contains("key")
                );
                assert_eq!(
                    error.cause.as_ref().unwrap().to_string(),
                    "/private/source-key-🌱"
                );
            }
        }
    }
    let mut record = Vec::new();
    write_invocation_failure(
        &mut record,
        &tmt_invoke::InvokeError {
            kind: FailureKind::Deadline,
            cause: None,
            cleanup: Cleanup::Confirmed,
        },
        InvocationObservation {
            command: ChildCommand::OwnDecode,
            input_bytes: 7,
            remaining: Duration::ZERO,
            elapsed: Duration::from_secs(2),
            parent_timing: None,
        },
    );
    assert_eq!(record, b"colab decoder failure phase=own.decode input_bytes=7 remaining_ns=0 invocation_ns=2000000000 kind=deadline cleanup=confirmed\n");
}

#[test]
fn failed_record_write_preserves_original_error_and_cleanup_fence() {
    struct FailedWriter;
    impl std::io::Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("diagnostic unavailable"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            panic!("failure reporting must not flush");
        }
    }
    for cleanup in [
        Cleanup::NotStarted,
        Cleanup::Confirmed,
        Cleanup::CallerOwned,
        Cleanup::Unconfirmed(std::io::Error::other("original-cleanup")),
    ] {
        let expected_blocked = cleanup_blocks(&cleanup);
        let mut blocked = !expected_blocked;
        let error = tmt_invoke::InvokeError {
            kind: tmt_invoke::FailureKind::Deadline,
            cause: Some(std::io::Error::other("original-cause")),
            cleanup,
        };
        let result = invocation_failure(
            &mut blocked,
            error,
            InvocationObservation {
                command: ChildCommand::PrepareContent,
                input_bytes: 2 * 1024 * 1024,
                remaining: DEADLINE,
                elapsed: DEADLINE,
                parent_timing: Some(ParentTiming {
                    wire: Duration::MAX,
                    json: Duration::MAX,
                    hash: Duration::MAX,
                }),
            },
            || FailedWriter,
        );
        assert_eq!(blocked, expected_blocked);
        let DecodeFault::Invoke(original) = result else {
            panic!("original Invoke error replaced");
        };
        assert_eq!(original.kind, tmt_invoke::FailureKind::Deadline);
        assert_eq!(original.cause.unwrap().to_string(), "original-cause");
        assert_eq!(cleanup_blocks(&original.cleanup), expected_blocked);
        if let Cleanup::Unconfirmed(cause) = original.cleanup {
            assert_eq!(cause.to_string(), "original-cleanup");
        }
    }
}

#[test]
fn input_role_denial_and_missing_program_leave_no_child() {
    let mut decoder = Decoder::new("/definitely-missing-tmt-colab".into()).unwrap();
    let bytes = vec![0; STATE_BYTES + 1];
    let updates = [bytes.as_slice()];
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &updates
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Commenter,
            None
        ),
        Err(DecodeFault::Denied)
    ));
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &[]
            },
            Role::Viewer,
            None
        ),
        Err(DecodeFault::Denied)
    ));
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::Invoke(e)) if matches!(e.cleanup, Cleanup::NotStarted)
    ));
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::Invoke(e)) if matches!(e.cleanup, Cleanup::NotStarted)
    ));
}
#[test]
fn cleanup_blocks_only_when_a_child_may_survive() {
    assert!(!cleanup_blocks(&Cleanup::NotStarted));
    assert!(!cleanup_blocks(&Cleanup::Confirmed));
    assert!(cleanup_blocks(&Cleanup::Unconfirmed(
        std::io::Error::other("wait failed")
    )));
}
#[test]
fn child_projection_rejects_namespace_type_and_size_substitution() {
    for value in [
        serde_json::json!({"html":"ok","meta":{},"threads":{}}),
        serde_json::json!({"html":[],"meta":{}}),
        serde_json::json!({"html":"ok","meta":{"other":"title"}}),
    ] {
        assert!(validate_projection(Namespace::Content, &value).is_err());
    }
    let own = serde_json::json!({"threads":{},"messages":{"id":{"body":"x".repeat(16*1024+1)}},"intents":{},"replies":{}});
    assert!(validate_projection(Namespace::Own, &own).is_err());
    assert!(binary("AA==", 1).is_err());
}

#[test]
fn injected_deadline_preserves_production_defaults_and_cleanup_fence() {
    let config = Config::new("/missing-decoder".into());
    assert_eq!(config.deadline, Duration::from_secs(2));
    let mut decoder = Decoder::with_config(Config {
        deadline: Duration::from_secs(60),
        ..config.clone()
    })
    .unwrap();
    assert_eq!(decoder.config.deadline, Duration::from_secs(60));
    decoder.blocked = true;
    decoder.set_deadline(DEADLINE).unwrap();
    assert!(matches!(
        decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None,
        ),
        Err(DecodeFault::CleanupBlocked)
    ));
    for deadline in [Duration::ZERO, Duration::MAX] {
        assert!(matches!(
            Decoder::with_config(Config {
                deadline,
                ..config.clone()
            }),
            Err(DecodeFault::InvalidInput)
        ));
        assert!(matches!(
            decoder.set_deadline(deadline),
            Err(DecodeFault::InvalidInput)
        ));
        assert_eq!(decoder.config.deadline, DEADLINE);
    }
    assert!(matches!(
        Decoder::with_config(Config {
            program: "relative-decoder".into(),
            ..config
        }),
        Err(DecodeFault::InvalidInput)
    ));
}

#[test]
fn content_batch_parent_rejects_structure_bounds_correlation_and_projection_substitution() {
    let expected = serde_json::json!({"html":"new","meta":{"title":"T"}});
    let valid = serde_json::json!({
        "version":1,"input_hash":"binding","batch":{"kind":"updates","updates":[URL_SAFE_NO_PAD.encode([1])]},
        "projection":expected,"memory_limit":memory_limit(),"pid":1
    });
    let mut invalid = Vec::new();
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("input_hash", serde_json::json!("wrong")),
        ("pid", serde_json::json!(0)),
        ("extra", serde_json::json!(true)),
        (
            "memory_limit",
            serde_json::json!(if memory_limit() == MemoryLimit::Enforced {
                MemoryLimit::Unavailable
            } else {
                MemoryLimit::Enforced
            }),
        ),
        (
            "projection",
            serde_json::json!({"html":"wrong","meta":{"title":"T"}}),
        ),
        (
            "projection",
            serde_json::json!({"html":"new","meta":{"title":"wrong"}}),
        ),
        (
            "projection",
            serde_json::json!({"html":"new","meta":{"title":"T","publisherAgent":"wrong"}}),
        ),
    ] {
        let mut value2 = valid.clone();
        value2[key] = value;
        invalid.push(value2);
    }
    for updates in [
        Vec::new(),
        vec![String::new()],
        vec!["AA==".to_owned()],
        vec![URL_SAFE_NO_PAD.encode(vec![0; UPDATE_BYTES + 1])],
        vec!["AQ".to_owned(); WRITE_TAIL_UPDATES + 1],
        vec![URL_SAFE_NO_PAD.encode(vec![0; UPDATE_BYTES]); 17],
    ] {
        let mut value = valid.clone();
        value["batch"]["updates"] = serde_json::json!(updates);
        invalid.push(value);
    }
    let mut noop = valid.clone();
    noop["batch"] = serde_json::json!({"kind":"noop"});
    invalid.push(noop.clone());
    noop["batch"]["updates"] = serde_json::json!([]);
    invalid.push(noop);
    let mut extra = valid.clone();
    extra["batch"]["extra"] = serde_json::json!(true);
    invalid.push(extra);
    for value in invalid {
        assert!(
            serde_json::from_value::<WirePreparedContent>(value)
                .and_then(
                    |wire| admit_prepared_content(wire, "binding", &expected, false)
                        .map_err(serde::de::Error::custom)
                )
                .is_err()
        );
    }
    let result = admit_prepared_content(
        serde_json::from_value(valid).unwrap(),
        "binding",
        &expected,
        false,
    )
    .unwrap();
    assert_eq!(result.batch, ContentBatch::Updates(vec![vec![1]]));
}

#[test]
fn strict_binary_matches_independent_vectors_bounds_and_old_malformed_predicate() {
    for (wire, bytes) in [
        ("", Vec::new()),
        ("AA", vec![0]),
        ("_w", vec![255]),
        ("AAE", vec![0, 1]),
        ("AAEC", vec![0, 1, 2]),
        ("8J-QiA", "🐈".as_bytes().to_vec()),
    ] {
        assert_eq!(binary(wire, bytes.len()).unwrap(), bytes);
        if !bytes.is_empty() {
            assert!(matches!(
                binary(wire, bytes.len() - 1),
                Err(DecodeFault::InvalidInput)
            ));
        }
    }
    for wire in [
        "A", "AA=", "AA==", "AA\n", " A", "+w", "/w", "AB", "AAF", "====",
    ] {
        assert!(
            matches!(binary(wire, 8), Err(DecodeFault::InvalidInput)),
            "{wire:?}"
        );
    }
    // Enumerate malformed short encodings independently of production admission.
    let alphabet = b"A_B/+= \n";
    for len in 0..=4u32 {
        for mut index in 0..alphabet.len().pow(len) {
            let mut candidate = vec![0; len as usize];
            for byte in &mut candidate {
                *byte = alphabet[index % alphabet.len()];
                index /= alphabet.len();
            }
            let wire = std::str::from_utf8(&candidate).unwrap();
            for limit in [0usize, 1, 2, 3, 8] {
                let old = if wire.len() > limit.div_ceil(3) * 4 {
                    None
                } else {
                    URL_SAFE_NO_PAD.decode(wire).ok().filter(|bytes| {
                        bytes.len() <= limit && URL_SAFE_NO_PAD.encode(bytes) == wire
                    })
                };
                assert_eq!(binary(wire, limit).ok(), old, "wire={wire:?} limit={limit}");
            }
        }
    }
}

#[test]
fn borrowed_preparation_wire_preserves_exact_bytes_hash_and_strict_owned_parse() {
    let base = serde_json::json!({"html":"old\r\n", "meta":{"foreign":null,"title":"T 🐈"}});
    let source = "new 🐈\n\"\\";
    let expected = r#"{"version":1,"baseline":"AA","updates":["AAE"],"expected_base":{"html":"old\r\n","meta":{"foreign":null,"title":"T 🐈"}},"source":"new 🐈\n\"\\","publisher_agent":null}"#;
    for publisher in [None, Some("agent")] {
        let borrowed = WireContentPreparation {
            version: 1,
            baseline: String::from("AA"),
            updates: vec![String::from("AAE")],
            expected_base: &base,
            source,
            publisher_agent: publisher,
        };
        let (bytes, hash) = SerializedInput::serialize(&borrowed).unwrap().finish();
        let oracle = if publisher.is_some() {
            expected.replace("\"publisher_agent\":null", "\"publisher_agent\":\"agent\"")
        } else {
            expected.into()
        };
        assert_eq!(bytes, oracle.as_bytes());
        assert_eq!(
            hash,
            URL_SAFE_NO_PAD.encode(Sha256::digest(oracle.as_bytes()))
        );
        let specialized: WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>> =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&specialized).unwrap(), bytes);
        assert!(matches!(
            specialized.baseline.0,
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(specialized.source.0, std::borrow::Cow::Owned(_)));
        let owned: WireContentPreparation = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(owned.expected_base, base);
        assert_eq!(owned.source, source);
        assert_eq!(owned.publisher_agent.as_deref(), publisher);
        assert_eq!(serde_json::to_vec(&owned).unwrap(), bytes);
        assert_eq!(Sha256::digest(&bytes), Sha256::digest(oracle.as_bytes()));
        for malformed in [
            oracle.replacen("\"version\":1", "\"version\":1,\"extra\":true", 1),
            oracle.replacen("\"source\":", "\"source\":null,\"source\":", 1),
            oracle.replacen("\"source\":\"new", "\"source\":null,\"unused\":\"new", 1),
        ] {
            assert!(serde_json::from_str::<WireContentPreparation>(&malformed).is_err());
        }
    }
}

#[test]
fn edited_projection_preserves_full_metadata_publisher_set_clear_and_noop() {
    let base = serde_json::json!({"html":"old 🐈", "meta":{
        "title":"T", "publisherAgent":"old", "foreign":{"nested":[null,1,"x"]}
    }});
    for (source, publisher) in [
        ("new\n🐈", Some("agent")),
        ("old 🐈", Some("old")),
        ("old 🐈", None),
    ] {
        let result = edited_projection(
            &base,
            ContentEdit {
                source,
                publisher_agent: publisher,
            },
        );
        let mut oracle = base.clone();
        oracle["html"] = Value::String(source.into());
        let meta = oracle["meta"].as_object_mut().unwrap();
        if let Some(agent) = publisher {
            meta.insert("publisherAgent".into(), Value::String(agent.into()));
        } else {
            meta.remove("publisherAgent");
        }
        assert_eq!(result, oracle);
        assert_eq!(
            serde_json::to_vec(&result).unwrap(),
            serde_json::to_vec(&oracle).unwrap()
        );
    }
    assert_eq!(base["html"], "old 🐈");
    assert_eq!(base["meta"]["publisherAgent"], "old");
}

#[test]
fn borrowed_binary_wire_matches_fixed_own_content_read_and_merge_bytes() {
    let baseline = [0, 1, 2];
    let first = [255];
    for (namespace, merge, expected) in [
        (
            Namespace::Own,
            false,
            r#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""]}"#,
        ),
        (
            Namespace::Content,
            false,
            r#"{"version":1,"namespace":"content","baseline":"AAEC","updates":["_w",""]}"#,
        ),
        (
            Namespace::Content,
            true,
            r#"{"version":1,"namespace":"content","baseline":"AAEC","updates":["_w",""],"merge_only":true}"#,
        ),
        (
            Namespace::Own,
            true,
            r#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""],"merge_only":true}"#,
        ),
    ] {
        let wire = WireBatch {
            version: 1,
            namespace,
            baseline: EncodedBytes(&baseline),
            updates: vec![EncodedBytes(&first), EncodedBytes(&[])],
            records: None,
            merge_only: merge,
        };
        let (bytes, hash) = SerializedInput::serialize(&wire).unwrap().finish();
        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(
            hash,
            URL_SAFE_NO_PAD.encode(Sha256::digest(expected.as_bytes()))
        );
        assert_eq!(Sha256::digest(&bytes), Sha256::digest(expected.as_bytes()));
        if namespace == Namespace::Own && !merge {
            // Independent Python hashlib digest of the fixed literal above.
            assert_eq!(
                Sha256::digest(&bytes).as_slice(),
                &[
                    0xa3, 0x19, 0x5c, 0x00, 0x40, 0x4a, 0x75, 0x77, 0xc1, 0x01, 0x28, 0xc0, 0x5c,
                    0xb8, 0x9c, 0x89, 0xfa, 0x32, 0xf6, 0xea, 0xeb, 0x29, 0xff, 0xb3, 0xf7, 0xea,
                    0x51, 0x2f, 0x05, 0x63, 0xb1, 0x53,
                ]
            );
        }
        let specialized: WireBatch<BorrowedWireText<'_>> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&specialized).unwrap(), bytes);
        assert!(matches!(
            specialized.baseline.0,
            std::borrow::Cow::Borrowed(_)
        ));
        let owned: WireBatch = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(owned.baseline, "AAEC");
        assert_eq!(owned.updates, ["_w", ""]);
        assert_eq!(serde_json::to_vec(&owned).unwrap(), bytes);
        for malformed in [
            expected.replacen("\"version\":1", "\"version\":1,\"extra\":0", 1),
            expected.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
            expected.replacen("\"baseline\":\"AAEC\"", "\"baseline\":null", 1),
            expected.replacen("\"updates\":[\"_w\",\"\"]", "\"updates\":[null]", 1),
            expected.replacen("\"updates\":[\"_w\",\"\"]", "\"updates\":0", 1),
        ] {
            assert!(serde_json::from_str::<WireBatch>(&malformed).is_err());
        }
    }
    // The own-record optional field accepts explicit null and serializes by omission.
    let explicit_null =
        r#"{"version":1,"records":null,"namespace":"own","baseline":"","updates":[]}"#;
    let wire: WireBatch = serde_json::from_str(explicit_null).unwrap();
    assert_eq!(
        serde_json::to_string(&wire).unwrap(),
        r#"{"version":1,"namespace":"own","baseline":"","updates":[]}"#
    );
}

#[test]
fn encoded_bytes_match_canonical_vectors_chunk_edges_and_raw_containment() {
    for (raw, encoded) in [
        (&b""[..], ""),
        (&[0][..], "AA"),
        (&[0, 1][..], "AAE"),
        (&[0, 1, 2][..], "AAEC"),
        (&[255][..], "_w"),
        ("🐈".as_bytes(), "8J-QiA"),
    ] {
        assert_eq!(
            serde_json::to_string(&EncodedBytes(raw)).unwrap(),
            format!("\"{encoded}\"")
        );
    }
    for length in [767, 768, 769, 1535, 1536, 1537, STATE_BYTES] {
        let raw = (0..length).map(|i| (i % 256) as u8).collect::<Vec<_>>();
        let (encoded, hash) = SerializedInput::serialize(&EncodedBytes(&raw))
            .unwrap()
            .finish();
        let old = serde_json::to_vec(&URL_SAFE_NO_PAD.encode(&raw)).unwrap();
        assert_eq!(encoded, old, "length={length}");
        assert_eq!(hash, URL_SAFE_NO_PAD.encode(Sha256::digest(&old)));
        let text: String = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(binary(&text, length).unwrap(), raw);
        assert!(matches!(
            binary(&text, length - 1),
            Err(DecodeFault::InvalidInput)
        ));
    }
}

#[test]
fn parent_wall_intervals_are_ordered_optional_finite_and_private() {
    let start = Instant::now();
    let timing = ParentTiming::from_samples([
        start,
        start + Duration::from_nanos(7),
        start + Duration::from_nanos(18),
        start + Duration::from_nanos(31),
    ]);
    assert_eq!(
        (timing.wire, timing.json, timing.hash),
        (
            Duration::from_nanos(7),
            Duration::from_nanos(11),
            Duration::from_nanos(13)
        )
    );
    let error = tmt_invoke::InvokeError {
        kind: tmt_invoke::FailureKind::Deadline,
        cause: Some(std::io::Error::other("/private/content-and-key-🐈")),
        cleanup: Cleanup::Confirmed,
    };
    for timing in [
        timing,
        ParentTiming {
            wire: Duration::MAX,
            json: Duration::MAX,
            hash: Duration::MAX,
        },
    ] {
        let mut record = Vec::new();
        write_invocation_failure(
            &mut record,
            &error,
            InvocationObservation {
                command: ChildCommand::OwnDecode,
                input_bytes: usize::MAX,
                remaining: Duration::MAX,
                elapsed: Duration::MAX,
                parent_timing: Some(timing),
            },
        );
        assert!(record.is_ascii() && record.len() <= 512);
        let record = String::from_utf8(record).unwrap();
        assert_eq!(record.lines().count(), 1);
        assert!(record.ends_with(&format!(
            " wire_ns={} json_ns={} hash_ns={}\n",
            timing.wire.as_nanos(),
            timing.json.as_nanos(),
            timing.hash.as_nanos()
        )));
        assert!(!record.contains("private") && !record.contains("content-and-key"));
        assert_eq!(
            error.cause.as_ref().unwrap().to_string(),
            "/private/content-and-key-🐈"
        );
    }
}

#[test]
fn incremental_input_hash_matches_independent_digest_across_write_partitions() {
    use std::io::Write;
    let literal = br#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""]}"#;
    let digest = [
        0xa3, 0x19, 0x5c, 0x00, 0x40, 0x4a, 0x75, 0x77, 0xc1, 0x01, 0x28, 0xc0, 0x5c, 0xb8, 0x9c,
        0x89, 0xfa, 0x32, 0xf6, 0xea, 0xeb, 0x29, 0xff, 0xb3, 0xf7, 0xea, 0x51, 0x2f, 0x05, 0x63,
        0xb1, 0x53,
    ];
    for chunk in [1, 2, 3, 7, 16, 64, 128] {
        let mut input = SerializedInput {
            bytes: Vec::with_capacity(128),
            hash: Sha256::new(),
        };
        assert_eq!(input.bytes.capacity(), 128);
        assert_eq!(input.write(&[]).unwrap(), 0);
        for part in literal.chunks(chunk) {
            assert_eq!(input.write(part).unwrap(), part.len());
        }
        input.flush().unwrap();
        let (bytes, hash) = input.finish();
        assert_eq!(bytes, literal);
        assert_eq!(hash, URL_SAFE_NO_PAD.encode(digest));
    }
}

#[test]
fn serialization_failure_discards_partial_input_and_digest() {
    struct FailAfterElement;
    impl Serialize for FailAfterElement {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element("partial 🐈")?;
            Err(serde::ser::Error::custom(
                "deliberate serialization failure",
            ))
        }
    }
    // Partial JSON is actually emitted; neither completed bytes nor hash are returned.
    let mut old = Vec::new();
    assert!(serde_json::to_writer(&mut old, &FailAfterElement).is_err());
    assert_eq!(old, "[\"partial 🐈\"".as_bytes());
    assert!(matches!(
        SerializedInput::serialize(&FailAfterElement),
        Err(DecodeFault::InvalidInput)
    ));
    // Failure does not affect another request or its private initial capacity.
    let input = SerializedInput::serialize(&serde_json::json!({"ok":true})).unwrap();
    assert_eq!(input.bytes.capacity(), 128);
    let (bytes, hash) = input.finish();
    assert_eq!(bytes, br#"{"ok":true}"#);
    assert_eq!(
        hash,
        URL_SAFE_NO_PAD.encode(Sha256::digest(br#"{"ok":true}"#))
    );
}

#[test]
fn child_borrowed_parse_matches_owned_strict_grammar_and_escape_fallback() {
    let literal = r#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""]}"#;
    for input in [literal.to_owned(), literal.replace("AAEC", r"\u0041AEC")] {
        let old: WireBatch = serde_json::from_str(&input).unwrap();
        let new: WireBatch<BorrowedWireText<'_>> = serde_json::from_str(&input).unwrap();
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        assert_eq!(
            binary(&old.baseline, 3).unwrap(),
            binary(&new.baseline, 3).unwrap()
        );
        assert!(binary(&new.baseline, 2).is_err());
        assert_eq!(
            matches!(new.baseline.0, std::borrow::Cow::Owned(_)),
            input != literal
        );
        // Correlation is the exact original input, never normalized parsed JSON.
        if input != literal {
            assert_ne!(
                Sha256::digest(input.as_bytes()),
                Sha256::digest(literal.as_bytes())
            );
        }
    }
    let mut malformed: Vec<Vec<u8>> = [
        literal.replace("\"version\":1", "\"version\":1,\"version\":1"),
        literal.replace("\"version\":1", "\"version\":1,\"extra\":0"),
        literal.replace("\"version\":1", "\"version\":1,\"source\":null"),
        literal.replace("\"version\":1", "\"version\":1,\"publisher_agent\":null"),
        literal.replace("\"AAEC\"", "null"),
        literal.replace("\"AAEC\"", "1"),
        literal.replace("[\"_w\",\"\"]", "[null]"),
        literal.replace("[\"_w\",\"\"]", "{}"),
        format!("{literal} true"),
    ]
    .into_iter()
    .map(String::into_bytes)
    .collect();
    let mut utf8 = literal.as_bytes().to_vec();
    let pos = utf8.iter().position(|b| *b == b'A').unwrap();
    utf8[pos] = 255;
    malformed.push(utf8);
    for bytes in malformed {
        assert!(serde_json::from_slice::<WireBatch>(&bytes).is_err());
        assert!(serde_json::from_slice::<WireBatch<BorrowedWireText<'_>>>(&bytes).is_err());
    }
}

#[test]
fn preparation_borrowed_unicode_null_grammar_and_source_byte_bounds_match_owned() {
    let literal = r#"{"version":1,"baseline":"","updates":[],"expected_base":{"html":"old","meta":{"title":"T","publisherAgent":"agent"}},"source":"🐈","publisher_agent":null}"#;
    for input in [literal.to_owned(), literal.replace("🐈", r"\ud83d\udc08")] {
        let old: WireContentPreparation = serde_json::from_str(&input).unwrap();
        let new: WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>> =
            serde_json::from_str(&input).unwrap();
        assert_eq!(old.source, &*new.source);
        assert_eq!(old.expected_base, new.expected_base);
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        assert_eq!(
            matches!(new.source.0, std::borrow::Cow::Owned(_)),
            input != literal
        );
        assert!(new.publisher_agent.is_none());
    }
    for source in ["x".repeat(BASELINE_BYTES), "x".repeat(BASELINE_BYTES + 1)] {
        let bytes = literal.replace("🐈", &source);
        let old: WireContentPreparation = serde_json::from_str(&bytes).unwrap();
        let new: WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>> =
            serde_json::from_str(&bytes).unwrap();
        assert_eq!(
            old.source.len() > BASELINE_BYTES,
            new.source.len() > BASELINE_BYTES
        );
        assert!(matches!(new.source.0, std::borrow::Cow::Borrowed(_)));
    }
    for malformed in [
        literal.replace("\"source\":\"🐈\"", "\"source\":null"),
        literal.replace("\"source\":\"🐈\"", "\"source\":2"),
        literal.replace("\"source\":\"🐈\"", "\"source\":\"🐈\",\"source\":\"x\""),
        literal.replace("\"version\":1", "\"version\":1,\"unknown\":true"),
        literal.replace("\"publisher_agent\":null", "\"publisher_agent\":false"),
        format!("{literal} []"),
    ] {
        assert!(serde_json::from_str::<WireContentPreparation>(&malformed).is_err());
        assert!(
            serde_json::from_str::<
                WireContentPreparation<BorrowedWireText<'_>, Value, BorrowedWireText<'_>>,
            >(&malformed)
            .is_err()
        );
    }
}

// Explicit diagnostic helpers stay in the libtest executable, never in the shipped child.
struct Progress<W> {
    writer: W,
    rows: usize,
    valid: bool,
}
impl<W: std::io::Write> Progress<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            rows: 0,
            valid: true,
        }
    }
    fn record(
        &mut self,
        stage: child::Stage,
        edge: child::CheckpointBoundary,
        count: usize,
        nanos: u64,
    ) {
        if !self.valid {
            return;
        }
        if self.rows == 1024 {
            self.valid = false;
            return;
        }
        let edge = match edge {
            child::CheckpointBoundary::Enter => "enter",
            child::CheckpointBoundary::Leave => "leave",
        };
        let row = format!(
            "seq={} stage={} edge={} n={} ns={}\n",
            self.rows + 1,
            stage.label(),
            edge,
            count.min(u32::MAX as usize),
            nanos
        );
        if row.len() > 96 || !row.is_ascii() || self.writer.write_all(row.as_bytes()).is_err() {
            self.valid = false;
            return;
        }
        self.rows += 1;
    }
}

#[test]
fn observation_records_are_finite_private_and_do_not_replace_algorithm_errors() {
    let mut progress = Progress::new(Vec::new());
    for _ in 0..1024 {
        progress.record(
            child::Stage::ReplySerialize,
            child::CheckpointBoundary::Leave,
            usize::MAX,
            u64::MAX,
        );
    }
    assert!(progress.valid);
    assert!(progress.writer.len() <= 96 * 1024);
    assert!(progress.writer.is_ascii());
    assert!(
        progress
            .writer
            .split_inclusive(|v| *v == b'\n')
            .all(|row| row.len() <= 96)
    );
    progress.record(child::Stage::Input, child::CheckpointBoundary::Enter, 0, 0);
    assert!(!progress.valid);
    struct Failed;
    impl std::io::Write for Failed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("private-payload/path/identity"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut failed = Progress::new(Failed);
    let mut output = Vec::new();
    let result = child::request_for_test(
        b"not-json".as_slice(),
        None,
        &mut output,
        &mut |stage, edge, count| failed.record(stage, edge, count, 0),
    );
    assert!(!failed.valid);
    assert!(matches!(result, Err(DecodeFault::InvalidInput)));
    assert!(output.is_empty());
    let input = br#"{"version":1,"namespace":"own","baseline":"","updates":[]}"#;
    child::request_for_test(
        input.as_slice(),
        None,
        &mut output,
        &mut |stage, edge, count| failed.record(stage, edge, count, 0),
    )
    .unwrap();
    let reply: WireResult = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        reply.input_hash,
        URL_SAFE_NO_PAD.encode(Sha256::digest(input))
    );
    assert!(!failed.valid);
}

#[test]
fn observation_reader_reply_preserves_deterministic_read_merge_and_failure_order() {
    use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
    for namespace in [Namespace::Own, Namespace::Content] {
        let doc = Doc::with_client_id(181);
        let expected = match namespace {
            Namespace::Own => {
                for name in ["threads", "messages", "intents", "replies"] {
                    doc.get_or_insert_map(name);
                }
                doc.get_or_insert_map("threads")
                    .insert(&mut doc.transact_mut(), "legacy", "kept");
                serde_json::json!({"threads":{"legacy":"kept"},"messages":{},"intents":{},"replies":{}})
            }
            Namespace::Content => {
                doc.get_or_insert_text("html")
                    .insert(&mut doc.transact_mut(), 0, "old 🐈");
                doc.get_or_insert_map("meta")
                    .insert(&mut doc.transact_mut(), "title", "T");
                serde_json::json!({"html":"old 🐈","meta":{"title":"T"}})
            }
        };
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        for merge_only in [false, true] {
            let wire = WireBatch {
                version: 1,
                namespace,
                baseline: String::new(),
                updates: vec![URL_SAFE_NO_PAD.encode(&update)],
                records: None,
                merge_only,
            };
            let input = serde_json::to_vec(&wire).unwrap();
            let mut plain = Vec::new();
            child::request_for_test(input.as_slice(), None, &mut plain, &mut |_, _, _| {}).unwrap();
            let mut observed = Vec::new();
            let mut checkpoints = Vec::new();
            child::request_for_test(input.as_slice(), None, &mut observed, &mut |s, e, n| {
                checkpoints.push((s, e, n))
            })
            .unwrap();
            // Same process, deterministic read/merge: compare all bytes including PID/hash.
            assert_eq!(plain, observed);
            let reply: WireResult = serde_json::from_slice(&observed).unwrap();
            assert_eq!(
                reply.input_hash,
                URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
            );
            assert_eq!(
                reply.projection,
                if merge_only {
                    Value::Null
                } else {
                    expected.clone()
                }
            );
            assert_eq!(
                checkpoints[0],
                (child::Stage::Input, child::CheckpointBoundary::Enter, 0)
            );
            assert_eq!(
                checkpoints[1],
                (
                    child::Stage::Input,
                    child::CheckpointBoundary::Leave,
                    input.len()
                )
            );
            assert_eq!(
                checkpoints.last(),
                Some(&(
                    child::Stage::ReplyWrite,
                    child::CheckpointBoundary::Leave,
                    observed.len()
                ))
            );
            for stage in [
                child::Stage::Json,
                child::Stage::Binary,
                child::Stage::Decode,
                child::Stage::Apply,
                child::Stage::Merge,
            ] {
                assert!(
                    checkpoints
                        .iter()
                        .any(|v| v.0 == stage && v.1 == child::CheckpointBoundary::Enter)
                );
                assert!(
                    checkpoints
                        .iter()
                        .any(|v| v.0 == stage && v.1 == child::CheckpointBoundary::Leave)
                );
            }
        }
    }
    let mut checkpoints = Vec::new();
    assert!(
        child::request_for_test(b"{".as_slice(), None, &mut Vec::new(), &mut |s, e, n| {
            checkpoints.push((s, e, n))
        })
        .is_err()
    );
    assert_eq!(
        checkpoints.last(),
        Some(&(child::Stage::Json, child::CheckpointBoundary::Enter, 0))
    );
}

#[test]
fn observation_preparation_replays_independently_without_normalizing_fresh_bytes() {
    use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode};
    let doc = Doc::with_client_id(182);
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, "old");
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "T");
    let baseline = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for (source, publisher) in [("old", None), ("new 🐈", Some("agent"))] {
        let wire = WireContentPreparation {
            version: 1,
            baseline: URL_SAFE_NO_PAD.encode(&baseline),
            updates: Vec::<String>::new(),
            expected_base: serde_json::json!({"html":"old","meta":{"title":"T"}}),
            source,
            publisher_agent: publisher,
        };
        let input = serde_json::to_vec(&wire).unwrap();
        let mut output = Vec::new();
        let mut checkpoints = Vec::new();
        child::request_for_test(
            input.as_slice(),
            Some("prepare-content"),
            &mut output,
            &mut |s, e, n| checkpoints.push((s, e, n)),
        )
        .unwrap();
        let reply: WirePreparedContent = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            reply.input_hash,
            URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
        );
        let expected = if publisher.is_some() {
            serde_json::json!({"html":"new 🐈","meta":{"title":"T","publisherAgent":"agent"}})
        } else {
            serde_json::json!({"html":"old","meta":{"title":"T"}})
        };
        assert_eq!(reply.projection, expected);
        let reader = Doc::new();
        reader.get_or_insert_text("html");
        reader.get_or_insert_map("meta");
        reader
            .transact_mut()
            .apply_update(Update::decode_v1(&baseline).unwrap())
            .unwrap();
        match reply.batch {
            WireContentBatch::Noop => assert_eq!(source, "old"),
            WireContentBatch::Updates { updates } => {
                for delta in updates {
                    reader
                        .transact_mut()
                        .apply_update(
                            Update::decode_v1(&binary(&delta, UPDATE_BYTES).unwrap()).unwrap(),
                        )
                        .unwrap();
                }
            }
        }
        use yrs::GetString;
        assert_eq!(
            reader
                .get_or_insert_text("html")
                .get_string(&reader.transact()),
            source
        );
        assert!(
            checkpoints
                .iter()
                .any(|v| v.0 == child::Stage::Replay && v.1 == child::CheckpointBoundary::Leave)
        );
        assert!(
            checkpoints
                .iter()
                .any(|v| v.0 == child::Stage::Generation && v.1 == child::CheckpointBoundary::Leave)
        );
    }
}

#[test]
#[ignore = "explicit supplementary child entry, selected only by the isolated observer wrapper"]
fn observer_child_entry() {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    let Some(root) = std::env::var_os("TMT_COLAB_OBSERVER_CHILD_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("TMT_COLAB_OBSERVER_CHILD_MODE").unwrap();
    assert!(mode == "decode" || mode == "prepare-content");
    let file = |name: &str| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join(name))
            .unwrap()
    };
    let mut output = file("reply.json");
    let mut progress = Progress::new(file("progress.txt"));
    let started = Instant::now();
    let result = child::observed_request(
        std::io::stdin(),
        if mode == "decode" {
            None
        } else {
            Some("prepare-content")
        },
        &mut output,
        &mut |s, e, n| {
            progress.record(
                s,
                e,
                n,
                started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            )
        },
    );
    use std::io::Write;
    if let Ok(mut quality) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("quality.txt"))
    {
        let _ = quality.write_all(if progress.valid {
            b"terminal-valid\n"
        } else {
            b"invalid\n"
        });
    }
    if result.is_ok()
        && let Ok(mut complete) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("reply.complete"))
    {
        let _ = complete.write_all(b"complete\n");
    }
    // Do not emit harness assertions/private causes or expose a partial reply as success.
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

#[test]
#[ignore = "exports an isolated wrapper artifact; does not launch native fixtures"]
fn observer_wrapper_artifact() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let root = PathBuf::from(
        std::env::var_os("TMT_COLAB_OBSERVER_ARTIFACT_ROOT").expect("explicit artifact root"),
    );
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let executable = std::env::current_exe().unwrap();
    let wrapper = format!(
        r#"#!/usr/bin/python3
import os, pathlib, subprocess, sys
root=pathlib.Path({root})
case=root / ('invocation-' + str(os.getpid()))
case.mkdir(mode=0o700)
def store(name, data):
    fd=os.open(case / name, os.O_WRONLY|os.O_CREAT|os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as f: f.write(data)
mode='prepare-content' if sys.argv[1:]==['__decoder','prepare-content'] else 'decode' if sys.argv[1:]==['__decoder'] else None
if mode is None: sys.exit(2)
data=sys.stdin.buffer.read({limit}+1)
store('request.json', data)
if len(data)>{limit}: store('request.overflow', b'overflow\n'); sys.exit(2)
store('request.complete', b'complete\n')
store('command.txt', mode.encode('ascii'))
env={{'TMT_COLAB_OBSERVER_CHILD_ROOT':str(case), 'TMT_COLAB_OBSERVER_CHILD_MODE':mode}}
child=subprocess.run([{executable}, '--ignored', '--exact', 'decoder::tests::observer_child_entry', '--test-threads=1'], input=data, stdout=subprocess.DEVNULL, env=env)
if child.returncode: sys.exit(1)
with (case/'reply.json').open('rb') as f: reply=f.read({limit}+1)
if len(reply)>{limit} or not (case/'reply.complete').exists(): sys.exit(2)
sys.stdout.buffer.write(reply)
"#,
        root = serde_json::to_string(&root).unwrap(),
        executable = serde_json::to_string(&executable).unwrap(),
        limit = STREAM_BYTES
    );
    tmt_test_support::write_executable(&root.join("program"), wrapper.as_bytes(), 0o700).unwrap();
    // The artifact path is private; no request payload or data-derived identifier is printed.
}

#[test]
fn observation_preparation_delta_and_wrong_base_keep_independent_semantics() {
    use yrs::{
        Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode,
    };
    let doc = Doc::with_client_id(183);
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, "old");
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "T");
    let baseline = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let wire = WireContentPreparation {
        version: 1,
        baseline: URL_SAFE_NO_PAD.encode(&baseline),
        updates: Vec::<String>::new(),
        expected_base: serde_json::json!({"html":"old","meta":{"title":"T"}}),
        source: "new 🐈",
        publisher_agent: Some("agent"),
    };
    let input = serde_json::to_vec(&wire).unwrap();
    for observe in [false, true] {
        let mut output = Vec::new();
        let mut checkpoints = Vec::new();
        child::request_for_test(
            input.as_slice(),
            Some("prepare-content"),
            &mut output,
            &mut |s, e, n| {
                if observe {
                    checkpoints.push((s, e, n));
                }
            },
        )
        .unwrap();
        let reply: WirePreparedContent = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            reply.projection,
            serde_json::json!({"html":"new 🐈","meta":{"title":"T","publisherAgent":"agent"}})
        );
        assert_eq!(
            reply.input_hash,
            URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
        );
        let reader = Doc::new();
        reader.get_or_insert_text("html");
        reader.get_or_insert_map("meta");
        reader
            .transact_mut()
            .apply_update(Update::decode_v1(&baseline).unwrap())
            .unwrap();
        let WireContentBatch::Updates { updates } = &reply.batch else {
            panic!("changed source must produce causal updates");
        };
        for delta in updates {
            reader
                .transact_mut()
                .apply_update(Update::decode_v1(&binary(delta, UPDATE_BYTES).unwrap()).unwrap())
                .unwrap();
        }
        assert_eq!(
            reader
                .get_or_insert_text("html")
                .get_string(&reader.transact()),
            "new 🐈"
        );
        if observe {
            assert!(checkpoints.iter().any(
                |v| v.0 == child::Stage::Generation && v.1 == child::CheckpointBoundary::Leave
            ));
        }
    }
    let wrong = WireContentPreparation {
        version: 1,
        baseline: URL_SAFE_NO_PAD.encode(&baseline),
        updates: Vec::<String>::new(),
        expected_base: serde_json::json!({"html":"wrong","meta":{"title":"T"}}),
        source: "new",
        publisher_agent: None::<&str>,
    };
    let mut checkpoints = Vec::new();
    assert!(matches!(
        child::request_for_test(
            serde_json::to_vec(&wrong).unwrap().as_slice(),
            Some("prepare-content"),
            &mut Vec::new(),
            &mut |s, e, n| checkpoints.push((s, e, n))
        ),
        Err(DecodeFault::Rejected)
    ));
    assert!(!checkpoints.iter().any(|v| v.0 == child::Stage::Generation));
}

#[test]
fn observer_prefix_is_incomplete_after_torn_or_capped_records_and_cleanup_is_original() {
    let mut progress = Progress::new(Vec::new());
    progress.record(child::Stage::Input, child::CheckpointBoundary::Enter, 0, 0);
    let row = progress.writer.clone();
    for end in 1..row.len() {
        assert_ne!(row[..end].last(), Some(&b'\n'));
    }
    for cleanup in [
        Cleanup::Confirmed,
        Cleanup::CallerOwned,
        Cleanup::Unconfirmed(std::io::Error::other("private")),
    ] {
        let expected_blocked = cleanup_blocks(&cleanup);
        let mut blocked = false;
        let fault = invocation_failure(
            &mut blocked,
            tmt_invoke::InvokeError {
                kind: tmt_invoke::FailureKind::Deadline,
                cause: None,
                cleanup,
            },
            InvocationObservation {
                command: ChildCommand::OwnDecode,
                input_bytes: 9787831,
                remaining: DEADLINE,
                elapsed: DEADLINE,
                parent_timing: None,
            },
            Vec::new,
        );
        assert_eq!(blocked, expected_blocked);
        assert!(
            matches!(fault, DecodeFault::Invoke(error) if error.kind == tmt_invoke::FailureKind::Deadline)
        );
        assert_eq!(progress.writer, row);
        // Reached rows alone are not a valid stopping-phase classification: no terminal quality marker.
    }
}

// Fixed whole-wire literals and Python hashlib digests are independent of Rust serialization.
const REPLY_VECTOR_0: &str = r#"{"version":1,"namespace":"content","input_hash":"input","merged":"AAEC_w","projection":{"html":"🐈\n\"\\","meta":{"title":"T","publisherAgent":"agent"}},"memory_limit":"memory limit unavailable","pid":7}"#;
const REPLY_DIGEST_0: [u8; 32] = [
    209, 143, 141, 33, 209, 128, 34, 134, 29, 100, 100, 213, 37, 53, 232, 226, 191, 248, 46, 22,
    28, 68, 245, 84, 18, 76, 164, 149, 97, 49, 95, 37,
];
const REPLY_VECTOR_1: &str = r#"{"version":1,"input_hash":"input","batch":{"kind":"updates","updates":["AAEC_w","","AQ"]},"projection":{"html":"🐈\n","meta":{"title":"T","publisherAgent":"agent"}},"memory_limit":"memory limit unavailable","pid":7}"#;
const REPLY_DIGEST_1: [u8; 32] = [
    78, 125, 194, 80, 197, 139, 164, 21, 199, 228, 101, 135, 181, 132, 118, 85, 21, 194, 239, 188,
    71, 58, 180, 199, 69, 84, 51, 88, 78, 16, 110, 8,
];
const REPLY_VECTOR_2: &str = r#"{"version":1,"input_hash":"input","batch":{"kind":"noop"},"projection":{"html":"🐈\n","meta":{"title":"T"}},"memory_limit":"memory limit unavailable","pid":7}"#;
const REPLY_DIGEST_2: [u8; 32] = [
    189, 235, 163, 137, 134, 202, 188, 40, 220, 194, 181, 8, 55, 246, 40, 2, 10, 254, 80, 245, 8,
    85, 113, 78, 100, 121, 127, 157, 245, 24, 117, 62,
];

#[test]
fn borrowed_response_vectors_preserve_whole_wire_digest_and_string_default_parse() {
    let raw = [0, 1, 2, 255];
    let projection =
        serde_json::json!({"html":"🐈\n\"\\","meta":{"title":"T","publisherAgent":"agent"}});
    let owned = WireResult {
        version: 1,
        namespace: Namespace::Content,
        input_hash: "input".into(),
        merged: "AAEC_w".to_owned(),
        projection: projection.clone(),
        memory_limit: MemoryLimit::Unavailable,
        pid: 7,
    };
    let borrowed = WireResult {
        version: 1,
        namespace: Namespace::Content,
        input_hash: "input".into(),
        merged: EncodedBytes(&raw),
        projection,
        memory_limit: MemoryLimit::Unavailable,
        pid: 7,
    };
    let actual = serde_json::to_vec(&borrowed).unwrap();
    assert_eq!(actual, REPLY_VECTOR_0.as_bytes());
    assert_eq!(actual, serde_json::to_vec(&owned).unwrap());
    assert_eq!(Sha256::digest(&actual).as_slice(), &REPLY_DIGEST_0);
    let parsed: WireResult = serde_json::from_slice(&actual).unwrap();
    assert_eq!(binary(&parsed.merged, raw.len()).unwrap(), raw);
    assert!(binary(&parsed.merged, raw.len() - 1).is_err());
    assert_eq!(parsed.projection, owned.projection);
    assert_eq!(parsed.memory_limit, owned.memory_limit);
    assert_eq!(parsed.pid, owned.pid);

    for (noop, literal, digest) in [
        (false, REPLY_VECTOR_1, REPLY_DIGEST_1),
        (true, REPLY_VECTOR_2, REPLY_DIGEST_2),
    ] {
        let projection = if noop {
            serde_json::json!({"html":"🐈\n","meta":{"title":"T"}})
        } else {
            serde_json::json!({"html":"🐈\n","meta":{"title":"T","publisherAgent":"agent"}})
        };
        let raw_updates: [&[u8]; 3] = [&raw, &[], &[1]];
        let owned = WirePreparedContent {
            version: 1,
            input_hash: "input".into(),
            batch: if noop {
                WireContentBatch::Noop
            } else {
                WireContentBatch::Updates {
                    updates: vec!["AAEC_w".to_owned(), String::new(), "AQ".to_owned()],
                }
            },
            projection: projection.clone(),
            memory_limit: MemoryLimit::Unavailable,
            pid: 7,
        };
        let borrowed = WirePreparedContent {
            version: 1,
            input_hash: "input".into(),
            batch: if noop {
                WireContentBatch::Noop
            } else {
                WireContentBatch::Updates {
                    updates: raw_updates.iter().map(|v| EncodedBytes(v)).collect(),
                }
            },
            projection,
            memory_limit: MemoryLimit::Unavailable,
            pid: 7,
        };
        let actual = serde_json::to_vec(&borrowed).unwrap();
        assert_eq!(actual, literal.as_bytes());
        assert_eq!(actual, serde_json::to_vec(&owned).unwrap());
        assert_eq!(Sha256::digest(&actual).as_slice(), &digest);
        let parsed: WirePreparedContent = serde_json::from_slice(&actual).unwrap();
        assert_eq!(serde_json::to_vec(&parsed).unwrap(), actual);
        // An empty update is syntactically valid JSON, but remains inadmissible.
        if !noop {
            assert!(admit_prepared_content(parsed, "input", &owned.projection, false).is_err());
        }
    }
}

#[test]
fn borrowed_response_chunk_edges_and_default_parser_equivalence_keep_raw_bounds() {
    for length in [
        0,
        1,
        2,
        3,
        767,
        768,
        769,
        1535,
        1536,
        1537,
        UPDATE_BYTES,
        STATE_BYTES,
    ] {
        let raw: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        let owned = WireResult {
            version: 1,
            namespace: Namespace::Own,
            input_hash: "input".into(),
            merged: URL_SAFE_NO_PAD.encode(&raw),
            projection: Value::Null,
            memory_limit: MemoryLimit::Unavailable,
            pid: 7,
        };
        let borrowed = WireResult {
            version: 1,
            namespace: Namespace::Own,
            input_hash: "input".into(),
            merged: EncodedBytes(&raw),
            projection: Value::Null,
            memory_limit: MemoryLimit::Unavailable,
            pid: 7,
        };
        let actual = serde_json::to_vec(&borrowed).unwrap();
        assert_eq!(actual, serde_json::to_vec(&owned).unwrap());
        assert_eq!(
            Sha256::digest(&actual),
            Sha256::digest(serde_json::to_vec(&owned).unwrap())
        );
        let parsed: WireResult = serde_json::from_slice(&actual).unwrap();
        assert_eq!(binary(&parsed.merged, length).unwrap(), raw);
        if length > 0 {
            assert!(binary(&parsed.merged, length - 1).is_err());
        }
    }
    for malformed in ["AA==", "AB", "A", "+/8", "_w=", "AA\n", "AA "] {
        assert!(binary(malformed, STATE_BYTES).is_err(), "{malformed:?}");
    }
    for (case, invalid) in [
        (
            "result-null-binary",
            REPLY_VECTOR_0.replace("\"merged\":\"AAEC_w\"", "\"merged\":null"),
        ),
        (
            "result-array-binary",
            REPLY_VECTOR_0.replace("\"merged\":\"AAEC_w\"", "\"merged\":[]"),
        ),
        (
            "result-duplicate-version",
            REPLY_VECTOR_0.replace("\"version\":1", "\"version\":1,\"version\":1"),
        ),
        (
            "result-unknown-field",
            REPLY_VECTOR_0.replace("\"version\":1", "\"version\":1,\"unknown\":0"),
        ),
    ] {
        assert!(
            serde_json::from_str::<WireResult>(&invalid).is_err(),
            "{case}"
        );
    }
    for (case, invalid) in [
        (
            "updates-null",
            REPLY_VECTOR_1.replace("\"updates\":[\"AAEC_w\",\"\",\"AQ\"]", "\"updates\":null"),
        ),
        (
            "unknown-kind",
            REPLY_VECTOR_1.replace("\"kind\":\"updates\"", "\"kind\":\"unknown\""),
        ),
        (
            "duplicate-kind",
            REPLY_VECTOR_1.replace(
                "\"kind\":\"updates\"",
                "\"kind\":\"updates\",\"kind\":\"updates\"",
            ),
        ),
        (
            "updates-unknown-field",
            REPLY_VECTOR_1.replace("\"kind\":\"updates\"", "\"kind\":\"updates\",\"unknown\":0"),
        ),
    ] {
        assert!(
            serde_json::from_str::<WirePreparedContent>(&invalid).is_err(),
            "{case}"
        );
    }

    // Independent former non-generic DTO: same original String fields and Serde attributes.
    // A unit Noop consumes extra map fields; no new rejection policy is introduced here.
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
    enum FormerContentBatch {
        Noop,
        Updates { updates: Vec<String> },
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FormerPreparedContent {
        version: u8,
        input_hash: String,
        batch: FormerContentBatch,
        projection: Value,
        memory_limit: MemoryLimit,
        pid: u32,
    }
    let extra_noop = REPLY_VECTOR_2
        .replace("\"kind\":\"noop\"", "\"kind\":\"noop\",\"updates\":[]")
        .replace(
            "\"memory limit unavailable\"",
            &serde_json::to_string(&memory_limit()).unwrap(),
        );
    let former: FormerPreparedContent = serde_json::from_str(&extra_noop).unwrap();
    let current: WirePreparedContent = serde_json::from_str(&extra_noop).unwrap();
    assert!(matches!(&former.batch, FormerContentBatch::Noop));
    assert!(matches!(&current.batch, WireContentBatch::Noop));
    assert_eq!(current.version, former.version);
    assert_eq!(current.input_hash, former.input_hash);
    assert_eq!(current.projection, former.projection);
    assert_eq!(current.memory_limit, former.memory_limit);
    assert_eq!(current.pid, former.pid);
    assert_eq!(
        serde_json::to_vec(&current).unwrap(),
        serde_json::to_vec(&former).unwrap()
    );
    let expected = serde_json::json!({"html":"🐈\n","meta":{"title":"T"}});
    assert_eq!(current.projection, expected);
    assert_eq!(current.memory_limit, memory_limit());
    let admitted = admit_prepared_content(current, "input", &expected, true).unwrap();
    assert!(matches!(admitted.batch, ContentBatch::Noop));
    assert_eq!(admitted.projection, expected);
    let current: WirePreparedContent = serde_json::from_str(&extra_noop).unwrap();
    assert!(matches!(
        admit_prepared_content(current, "input", &expected, false),
        Err(DecodeFault::InvalidOutput)
    ));
}

#[test]
fn one_generated_reply_preserves_owned_wire_projection_and_independent_replay() {
    use yrs::{
        Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode,
    };
    for namespace in [Namespace::Content, Namespace::Own] {
        let doc = Doc::with_client_id(1934);
        let expected = match namespace {
            Namespace::Content => {
                doc.get_or_insert_text("html")
                    .insert(&mut doc.transact_mut(), 0, "old 🐈");
                let meta = doc.get_or_insert_map("meta");
                meta.insert(&mut doc.transact_mut(), "title", "T");
                meta.insert(&mut doc.transact_mut(), "publisherAgent", "agent");
                serde_json::json!({"html":"old 🐈","meta":{"title":"T","publisherAgent":"agent"}})
            }
            Namespace::Own => {
                for root in ["threads", "messages", "intents", "replies"] {
                    doc.get_or_insert_map(root);
                }
                doc.get_or_insert_map("threads")
                    .insert(&mut doc.transact_mut(), "legacy", "kept");
                serde_json::json!({"threads":{"legacy":"kept"},"messages":{},"intents":{},"replies":{}})
            }
        };
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        for merge_only in [false, true] {
            let input = serde_json::to_vec(&WireBatch {
                version: 1,
                namespace,
                baseline: String::new(),
                updates: vec![URL_SAFE_NO_PAD.encode(&update)],
                records: None,
                merge_only,
            })
            .unwrap();
            let mut output = Vec::new();
            child::request_for_test(input.as_slice(), None, &mut output, &mut |_, _, _| {})
                .unwrap();
            let reply: WireResult = serde_json::from_slice(&output).unwrap();
            let merged = binary(&reply.merged, STATE_BYTES).unwrap();
            let borrowed = WireResult {
                version: reply.version,
                namespace: reply.namespace,
                input_hash: reply.input_hash.clone(),
                merged: EncodedBytes(&merged),
                projection: reply.projection.clone(),
                memory_limit: reply.memory_limit,
                pid: reply.pid,
            };
            // One algorithm result: no regenerated Yrs bytes or PID normalization.
            assert_eq!(serde_json::to_vec(&reply).unwrap(), output);
            assert_eq!(serde_json::to_vec(&borrowed).unwrap(), output);
            assert_eq!(
                reply.input_hash,
                URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
            );
            let reader = Doc::new();
            match namespace {
                Namespace::Content => {
                    reader.get_or_insert_text("html");
                    reader.get_or_insert_map("meta");
                }
                Namespace::Own => {
                    for root in ["threads", "messages", "intents", "replies"] {
                        reader.get_or_insert_map(root);
                    }
                }
            }
            reader
                .transact_mut()
                .apply_update(Update::decode_v1(&merged).unwrap())
                .unwrap();
            if merge_only {
                assert_eq!(reply.projection, Value::Null);
            } else {
                assert_eq!(reply.projection, expected);
                if namespace == Namespace::Content {
                    assert_eq!(
                        reader
                            .get_or_insert_text("html")
                            .get_string(&reader.transact()),
                        "old 🐈"
                    );
                } else {
                    assert_eq!(
                        reader
                            .get_or_insert_map("threads")
                            .get(&reader.transact(), "legacy")
                            .unwrap()
                            .to_string(&reader.transact()),
                        "kept"
                    );
                }
            }
        }
        if namespace != Namespace::Content {
            continue;
        }
        for (source, publisher) in [
            ("old 🐈", Some("agent")),
            ("old 🐈", None),
            ("new 🐈", Some("next")),
        ] {
            let input = serde_json::to_vec(&WireContentPreparation {
                version: 1,
                baseline: URL_SAFE_NO_PAD.encode(&update),
                updates: Vec::<String>::new(),
                expected_base: expected.clone(),
                source,
                publisher_agent: publisher,
            })
            .unwrap();
            let mut output = Vec::new();
            child::request_for_test(
                input.as_slice(),
                Some("prepare-content"),
                &mut output,
                &mut |_, _, _| {},
            )
            .unwrap();
            let reply: WirePreparedContent = serde_json::from_slice(&output).unwrap();
            assert_eq!(serde_json::to_vec(&reply).unwrap(), output);
            assert_eq!(
                reply.input_hash,
                URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
            );
            let raw = match &reply.batch {
                WireContentBatch::Noop => Vec::new(),
                WireContentBatch::Updates { updates } => updates
                    .iter()
                    .map(|v| binary(v, UPDATE_BYTES).unwrap())
                    .collect::<Vec<_>>(),
            };
            let borrowed = WirePreparedContent {
                version: reply.version,
                input_hash: reply.input_hash.clone(),
                batch: if matches!(&reply.batch, WireContentBatch::Noop) {
                    WireContentBatch::Noop
                } else {
                    WireContentBatch::Updates {
                        updates: raw.iter().map(|v| EncodedBytes(v)).collect(),
                    }
                },
                projection: reply.projection.clone(),
                memory_limit: reply.memory_limit,
                pid: reply.pid,
            };
            assert_eq!(serde_json::to_vec(&borrowed).unwrap(), output);
            let reader = Doc::new();
            reader.get_or_insert_text("html");
            let meta = reader.get_or_insert_map("meta");
            reader
                .transact_mut()
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
            for delta in &raw {
                reader
                    .transact_mut()
                    .apply_update(Update::decode_v1(delta).unwrap())
                    .unwrap();
            }
            assert_eq!(
                reader
                    .get_or_insert_text("html")
                    .get_string(&reader.transact()),
                source
            );
            assert_eq!(
                meta.get(&reader.transact(), "title")
                    .unwrap()
                    .to_string(&reader.transact()),
                "T"
            );
            assert_eq!(
                meta.get(&reader.transact(), "publisherAgent")
                    .map(|v| v.to_string(&reader.transact())),
                publisher.map(str::to_owned)
            );
            let expected = edited_projection(
                &expected,
                ContentEdit {
                    source,
                    publisher_agent: publisher,
                },
            );
            assert_eq!(reply.projection, expected);
            let noop = source == "old 🐈" && publisher == Some("agent");
            assert!(
                admit_prepared_content(
                    reply,
                    &URL_SAFE_NO_PAD.encode(Sha256::digest(&input)),
                    &expected,
                    noop
                )
                .is_ok()
            );
        }
        assert_eq!(
            doc.transact()
                .encode_state_as_update_v1(&StateVector::default()),
            update
        );
    }
}
