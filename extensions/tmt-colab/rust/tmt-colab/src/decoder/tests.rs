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
