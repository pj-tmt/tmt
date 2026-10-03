use super::*;
#[test]
fn input_role_denial_and_missing_program_leave_no_child() {
    let mut decoder = Decoder::new("/definitely-missing-tmt-colab".into()).unwrap();
    let bytes = vec![0; UPDATE_BYTES + 1];
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
