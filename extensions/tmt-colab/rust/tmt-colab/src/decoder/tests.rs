use super::*;
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
