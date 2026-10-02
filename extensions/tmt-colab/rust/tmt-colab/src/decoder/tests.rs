use super::*;
#[test]
fn input_role_and_failed_launch_cannot_start_another_child() {
    let mut decoder = Decoder::new("/definitely-missing-tmt-colab".into()).unwrap();
    let bytes = vec![0; UPDATE_BYTES + 1];
    let updates = [bytes.as_slice()];
    assert!(matches!(
        decoder.decode(
            Batch {
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
            Batch {
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
            Batch {
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
            Batch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::Invoke(_))
    ));
    assert!(matches!(
        decoder.decode(
            Batch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::CleanupBlocked)
    ));
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
