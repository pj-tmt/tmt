//! Remote-authored frozen vectors, not a provider or Colab admission fixture.
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};
use tmt_remote::{objects::IoBudget, sealed_sync::*};
struct Auth {
    project: String,
}
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: self.project.clone(),
            uid: "fixture-user".into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        panic!("pure binding must not acquire a token")
    }
}
#[test]
fn frozen_objects_are_complete_or_absent_in_both_chunk_profiles() {
    let vectors: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/sealed_sync/v1.json")).unwrap();
    for v in vectors {
        let manifest: ObjectManifest = serde_json::from_value(v["manifest"].clone()).unwrap();
        let data: Vec<u8> = (0..manifest.bytes).map(|i| (i % 251) as u8).collect();
        let chunks: Vec<Vec<u8>> = data
            .chunks(manifest.chunk_bytes as usize)
            .map(<[u8]>::to_vec)
            .collect();
        verify_object("fixture-original", &manifest, &chunks, 12_582_912).unwrap();
        assert_eq!(manifest.count, v["count"].as_u64().unwrap());
        assert_eq!(
            chunks.last().map_or(0, Vec::len) as u64,
            v["lastBytes"].as_u64().unwrap()
        );
        let mut canonical = serde_json::to_value(&manifest).unwrap();
        canonical.sort_all_objects();
        assert_eq!(
            serde_json::to_string(&canonical).unwrap(),
            v["manifestBytes"].as_str().unwrap()
        );
        let ids: Vec<_> = (0..manifest.count)
            .map(|i| format!("fixture-original+{i}"))
            .collect();
        assert_eq!(serde_json::to_value(ids).unwrap(), v["ids"]);
        assert_eq!(
            verify_object("another-original", &manifest, &chunks, 12_582_912),
            Err(SyncError::Invalid)
        );
        let mut changed = manifest.clone();
        changed.digest = "0".repeat(64);
        assert_eq!(
            verify_object("fixture-original", &changed, &chunks, 12_582_912),
            Err(SyncError::Invalid)
        );
        if !chunks.is_empty() {
            assert_eq!(
                verify_object(
                    "fixture-original",
                    &manifest,
                    &chunks[..chunks.len() - 1],
                    12_582_912
                ),
                Err(SyncError::Invalid)
            );
            let mut bad = chunks.clone();
            bad[0][0] ^= 1;
            assert_eq!(
                verify_object("fixture-original", &manifest, &bad, 12_582_912),
                Err(SyncError::Invalid)
            );
        }
    }
}
#[test]
fn bound_targets_refuse_context_mismatch_without_auth_io() {
    let bytes = include_bytes!("fixtures/declarations/colab-firestore.json");
    let config = serde_json::json!({"apiKey":"public", "appId":"1:123:web:fixture", "projectId":"demo-remote-1", "authDomain":"demo-remote-1.firebaseapp.com"});
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let auth: Arc<dyn AuthPort> = Arc::new(Auth {
        project: "demo-remote-1".into(),
    });
    let context = SyncContext::bind(config.clone(), bytes, &digest, "fixture-space", auth).unwrap();
    let resource = &context.resources()[0];
    let target = Target {
        resource: resource.name.clone(),
        id: "fixture-user+1".into(),
    };
    assert_eq!(
        context.document_path(&target).unwrap(),
        format!("x/colab/{}/fixture-user+1", resource.path)
    );
    for id in ["", "../escape", "x/append", "a b"] {
        assert_eq!(
            context.document_path(&Target {
                resource: resource.name.clone(),
                id: id.into()
            }),
            Err(SyncError::Invalid)
        );
    }
    let wrong: Arc<dyn AuthPort> = Arc::new(Auth {
        project: "another-project".into(),
    });
    assert!(matches!(
        SyncContext::bind(config, bytes, &digest, "fixture-space", wrong),
        Err(SyncError::Denied)
    ));
}

#[test]
fn codec_mapping_and_exact_scope_are_shared_frozen_data() {
    use std::collections::BTreeMap;
    let v: Value =
        serde_json::from_str(include_str!("fixtures/sealed_sync/codec-v1.json")).unwrap();
    for bytes in v["valid"].as_array().unwrap() {
        validate_fields(bytes.as_str().unwrap().as_bytes()).unwrap();
    }
    for bytes in v["invalid"].as_array().unwrap() {
        assert!(
            validate_fields(bytes.as_str().unwrap().as_bytes()).is_err(),
            "{}",
            bytes
        );
    }
    let config = serde_json::json!({"apiKey":"public", "appId":"1:123:web:fixture", "projectId":"demo-remote-1", "authDomain":"demo-remote-1.firebaseapp.com"});
    let context = SyncContext::bind(
        config,
        v["declaration"].as_str().unwrap().as_bytes(),
        v["declarationDigest"].as_str().unwrap(),
        "fixture-space",
        Arc::new(Auth {
            project: "demo-remote-1".into(),
        }),
    )
    .unwrap();
    let document = Document {
        target: Target {
            resource: "rows".into(),
            id: "writer-stream+1".into(),
        },
        bytes: v["document"].as_str().unwrap().as_bytes().to_vec(),
        digest: v["documentDigest"].as_str().unwrap().into(),
    };
    validate_document(&context, &document, DocumentAccess::Publish).unwrap();
    let mut query = Query {
        resource: "rows".into(),
        scope: BTreeMap::from([
            (
                "tenant".into(),
                serde_json::json!({"stringValue":"fixture-space"}),
            ),
            (
                "streamKey".into(),
                serde_json::json!({"stringValue":"writer-stream"}),
            ),
        ]),
        stream: "writer-stream".into(),
        cursor: Some(SyncCursor {
            stream: "writer-stream".into(),
            seq: 1,
        }),
    };
    validate_query(&context, &query).unwrap();
    query.scope.insert(
        "tenant".into(),
        serde_json::json!({"stringValue":"another-space"}),
    );
    assert_eq!(validate_query(&context, &query), Err(SyncError::Denied));
    let mut changed: Value = serde_json::from_str(v["declaration"].as_str().unwrap()).unwrap();
    changed["resources"][0]["sync"]["query"]["seqField"] = serde_json::json!("undeclared");
    assert!(tmt_remote::declaration::parse(&serde_json::to_vec(&changed).unwrap()).is_err());
}

#[test]
fn budget_refusal_keeps_existing_code_and_reset_fields() {
    let refusal = tmt_remote::firestore_budget::Refusal {
        reset_at_ms: 123,
        retry_after_ms: 45,
    };
    let error = SyncError::BudgetExhausted(refusal);
    assert_eq!(error.code(), "REMOTE_BUDGET_EXHAUSTED");
    assert_eq!(error, SyncError::BudgetExhausted(refusal));
}

#[test]
fn finite_query_gap_has_a_distinct_state_without_a_new_refusal_code() {
    assert_eq!(
        SyncError::CatchUpRequired.code(),
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(
        SyncError::CatchUpRequired.state(),
        Some("catch-up-required")
    );
    assert_eq!(SyncError::Unavailable.state(), None);
}
