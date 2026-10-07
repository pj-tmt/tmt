use super::*;

#[test]
fn invocation_phases_follow_existing_namespace_edit_merge_and_baseline_actions() {
    for (namespace, edit, merge, expected) in [
        (Namespace::Content, false, false, "content.decode"),
        (Namespace::Own, false, false, "own.decode"),
        (Namespace::Content, true, false, "content.edit"),
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
        ChildCommand::ContentEdit,
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
fn borrowed_binary_wire_matches_fixed_own_content_edit_and_merge_bytes() {
    let baseline = [0, 1, 2];
    let first = [255];
    for (namespace, source, publisher, merge, expected) in [
        (
            Namespace::Own,
            None,
            None,
            false,
            r#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""]}"#,
        ),
        (
            Namespace::Content,
            Some("x🐈\n\"\\"),
            Some("agent"),
            false,
            r#"{"version":1,"source":"x🐈\n\"\\","publisher_agent":"agent","namespace":"content","baseline":"AAEC","updates":["_w",""]}"#,
        ),
        (
            Namespace::Content,
            Some(""),
            None,
            false,
            r#"{"version":1,"source":"","namespace":"content","baseline":"AAEC","updates":["_w",""]}"#,
        ),
        (
            Namespace::Own,
            None,
            None,
            true,
            r#"{"version":1,"namespace":"own","baseline":"AAEC","updates":["_w",""],"merge_only":true}"#,
        ),
    ] {
        let wire = WireBatch {
            version: 1,
            source: source.map(str::to_owned),
            publisher_agent: publisher.map(str::to_owned),
            namespace,
            baseline: EncodedBytes(&baseline),
            updates: vec![EncodedBytes(&first), EncodedBytes(&[])],
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
    // The original optional fields accept explicit null and serialize by omission.
    let explicit_null = r#"{"version":1,"source":null,"publisher_agent":null,"namespace":"own","baseline":"","updates":[]}"#;
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
