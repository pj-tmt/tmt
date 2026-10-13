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
        let cap = v["maxObjectBytes"].as_u64().unwrap_or(12_582_912);
        verify_object("fixture-original", &manifest, &chunks, cap).unwrap();
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
    validate_mutation(&context, &Mutation::Create(document.clone())).unwrap();
    assert_eq!(
        validate_mutation(&context, &Mutation::Put(document.clone())),
        Err(SyncError::Unsupported)
    );
    assert_eq!(
        validate_mutation(&context, &Mutation::Delete(document.target.clone())),
        Err(SyncError::Unsupported)
    );
    validate_discovery(
        &context,
        &DiscoveryQuery {
            shape: "arrivals".into(),
            continuation: None,
            resource: "rows".into(),
            scope: BTreeMap::from([(
                "tenant".into(),
                serde_json::json!({"stringValue":"fixture-space"}),
            )]),
        },
    )
    .unwrap();
    assert_eq!(
        validate_discovery(
            &context,
            &DiscoveryQuery {
                shape: "arrivals".into(),
                continuation: None,
                resource: "rows".into(),
                scope: BTreeMap::from([
                    (
                        "tenant".into(),
                        serde_json::json!({"stringValue":"fixture-space"})
                    ),
                    (
                        "streamKey".into(),
                        serde_json::json!({"stringValue":"writer-stream"})
                    )
                ])
            }
        ),
        Err(SyncError::Invalid)
    );
    for case in v["discoveryCases"].as_array().unwrap() {
        let query = DiscoveryQuery {
            resource: "rows".into(),
            shape: case["shape"].as_str().unwrap().into(),
            continuation: case["continuation"]
                .as_str()
                .map(|s| DiscoveryContinuation(s.into())),
            scope: BTreeMap::from([(
                "tenant".into(),
                serde_json::json!({"stringValue":"fixture-space"}),
            )]),
        };
        assert_eq!(
            validate_discovery(&context, &query).is_ok(),
            case["valid"].as_bool().unwrap()
        );
    }
    for patch in v["invalidMappings"].as_array().unwrap() {
        let mut declaration: Value =
            serde_json::from_str(v["declaration"].as_str().unwrap()).unwrap();
        for (key, value) in patch.as_object().unwrap() {
            if key == "discovery" {
                declaration["resources"][0]["sync"]["query"][key] = value.clone();
            } else {
                declaration["resources"][0]["sync"][key] = value.clone();
            }
        }
        assert!(
            tmt_remote::declaration::parse(&serde_json::to_vec(&declaration).unwrap()).is_err()
        );
    }
    let mut mutable: Value = serde_json::from_str(v["declaration"].as_str().unwrap()).unwrap();
    mutable["resources"][0]["kind"] = serde_json::json!("checkpoint");
    mutable["resources"][0]["sync"]["writeModes"] = serde_json::json!(["create", "put", "delete"]);
    let bytes = serde_json::to_vec(&mutable).unwrap();
    use sha2::Digest;
    let digest = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mutable_context = SyncContext::bind(
        context.config().clone(),
        &bytes,
        &digest,
        "fixture-space",
        Arc::new(Auth {
            project: "demo-remote-1".into(),
        }),
    )
    .unwrap();
    validate_mutation(&mutable_context, &Mutation::Put(document.clone())).unwrap();
    validate_mutation(&mutable_context, &Mutation::Delete(document.target.clone())).unwrap();
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
fn budget_refusal_variant_preserves_the_existing_code_and_values() {
    let refusal = tmt_remote::firestore_budget::Refusal {
        reset_at_ms: 123,
        retry_after_ms: 45,
    };
    let error = SyncError::BudgetExhausted(refusal);
    assert_eq!(error.code(), "REMOTE_BUDGET_EXHAUSTED");
    let SyncError::BudgetExhausted(carried) = error else {
        panic!("lost typed budget refusal")
    };
    assert_eq!(carried.reset_at_ms, 123);
    assert_eq!(carried.retry_after_ms, 45);
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

#[test]
fn shared_operation_corpus_pins_exact_refusal_code_and_state() {
    use sha2::Digest;
    use std::collections::BTreeMap;
    let source: Value =
        serde_json::from_str(include_str!("fixtures/sealed_sync/codec-v1.json")).unwrap();
    for case in source["operations"].as_array().unwrap() {
        let mut declaration: Value =
            serde_json::from_str(source["declaration"].as_str().unwrap()).unwrap();
        match case["context"].as_str().unwrap() {
            "nosync" => {
                declaration["resources"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("sync");
            }
            "no-query" => declaration["resources"][0]["sync"]["query"] = Value::Null,
            "no-discovery" => {
                declaration["resources"][0]["sync"]["query"]
                    .as_object_mut()
                    .unwrap()
                    .remove("discovery");
            }
            "small" => {
                declaration["resources"][0]["limits"]["maxObjectBytes"] = serde_json::json!(16)
            }
            "normal" => {}
            _ => panic!("unknown corpus context"),
        }
        let bytes = serde_json::to_vec(&declaration).unwrap();
        let digest = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let context = SyncContext::bind(serde_json::json!({"apiKey":"public","appId":"1:123:web:fixture","projectId":"demo-remote-1","authDomain":"demo-remote-1.firebaseapp.com"}),&bytes,&digest,"fixture-space",Arc::new(Auth{project:"demo-remote-1".into()})).unwrap();
        let resource = case["resource"].as_str().unwrap().to_owned();
        let scope =
            || serde_json::from_value::<BTreeMap<String, Value>>(case["scope"].clone()).unwrap();
        let result = match case["op"].as_str().unwrap() {
            "read" | "publish" => validate_document(
                &context,
                &Document {
                    target: Target {
                        resource,
                        id: case["id"].as_str().unwrap().into(),
                    },
                    bytes: case["bytes"].as_str().unwrap().as_bytes().to_vec(),
                    digest: case["digest"].as_str().unwrap().into(),
                },
                if case["op"] == "read" {
                    DocumentAccess::Read
                } else {
                    DocumentAccess::Publish
                },
            ),
            "discovery" => validate_discovery(
                &context,
                &DiscoveryQuery {
                    resource,
                    shape: case["shape"].as_str().unwrap().into(),
                    continuation: None,
                    scope: scope(),
                },
            ),
            "query" => validate_query(
                &context,
                &Query {
                    resource,
                    scope: scope(),
                    stream: "writer-stream".into(),
                    cursor: None,
                },
            ),
            "mutation" => validate_mutation(
                &context,
                &Mutation::Delete(Target {
                    resource,
                    id: case["id"].as_str().unwrap().into(),
                }),
            ),
            "interest" => validate_interest(
                &context,
                &SyncInterest {
                    resource,
                    scope: scope(),
                },
            ),
            _ => panic!("unknown corpus operation"),
        };
        let actual = match result {
            Ok(()) => serde_json::json!({"code":null,"state":null}),
            Err(e) => serde_json::json!({"code":e.code(),"state":e.state()}),
        };
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}
