//! Flush admission is pinned to an active UUID. Its one knob skips only the live-idle evidence.
use super::*;

#[test]
fn flush_admits_only_a_canonical_uuid_without_originator_or_delivery_options() {
    let id = "10000000-0000-4000-8000-000000000001";
    let valid = json!({"version":1,"operation":"digest.checklist.flush","input":{"identityId":id}});
    let Request::Digest(operation) = super::super::decode(&valid.to_string()).unwrap() else {
        panic!("expected Digest operation");
    };
    assert!(matches!(*operation,Operation::Flush{identity,when_busy:false} if identity==id));
    for input in [
        json!({}),
        json!({"identityId":"worker"}),
        json!({"identityId":"10000000-0000-4000-8000-00000000000A"}),
        json!({"identityId":id,"opportunity":"turn_boundary"}),
        json!({"identityId":id,"force":true}),
        json!({"identityId":id,"whenBusy":"true"}),
        json!({"identityId":id,"whenBusy":1}),
        json!({"identityId":id,"whenBusy":null}),
        json!({"identityId":id,"when_busy":true}),
    ] {
        let mut request = valid.clone();
        request["input"] = input;
        assert!(super::super::decode(&request.to_string()).is_err());
    }
    for field in ["identity", "originator"] {
        let mut request = valid.clone();
        request[field] = json!("anonymous");
        assert!(super::super::decode(&request.to_string()).is_err());
    }
}

#[test]
fn flush_when_busy_is_a_boolean_that_only_flush_admits() {
    let id = "10000000-0000-4000-8000-000000000001";
    for (when_busy, expected) in [(true, true), (false, false)] {
        let request = json!({"version":1,"operation":"digest.checklist.flush","input":{"identityId":id,"whenBusy":when_busy}});
        let Request::Digest(operation) = super::super::decode(&request.to_string()).unwrap() else {
            panic!("expected Digest operation");
        };
        assert!(matches!(*operation, Operation::Flush { when_busy, .. } if when_busy == expected));
    }
    let due = json!({"version":1,"operation":"digest.checklist.dueNow","input":{"identityId":id,"whenBusy":true}});
    assert!(super::super::decode(&due.to_string()).is_err());
}

#[test]
fn history_rejects_unknown_inputs_and_admits_only_bounded_uuid_scopes() {
    let id = "10000000-0000-4000-8000-000000000001";
    let valid =
        json!({"version":1,"operation":"digest.checklist.history","input":{"identityId":id}});
    assert!(
        matches!(super::super::decode(&valid.to_string()).unwrap(), Request::Digest(op) if matches!(*op, Operation::History {after:0,limit:32,..}))
    );
    for input in [
        json!({"identityId":"worker"}),
        json!({"identityId":id,"limit":0}),
        json!({"identityId":id,"limit":129}),
        json!({"identityId":id,"after":-1}),
        json!({"identityId":id,"after":9007199254740992_u64}),
        json!({"identityId":id,"checklistId":"wrong"}),
        json!({"identityId":id,"opportunity":"idle"}),
        json!({"identityId":id,"attemptToken":id}),
    ] {
        let mut request = valid.clone();
        request["input"] = input;
        assert!(super::super::decode(&request.to_string()).is_err());
    }
    for field in ["identity", "originator"] {
        let mut request = valid.clone();
        request[field] = json!("anonymous");
        assert!(super::super::decode(&request.to_string()).is_err());
    }
}

#[test]
fn history_vectors_pin_content_free_projection_and_fit_the_consumer_byte_budget() {
    use tmt_core::request::digest::{
        DigestChecklist, DigestHistoryChecklist, DigestHistoryRead, DigestHistoryRecord,
        DigestItem, DigestKind, DigestSource,
    };
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/digest-checklist-history-v1.json"
    ))
    .unwrap();
    for vector in vectors["cases"].as_array().unwrap() {
        let given = &vector["given"];
        let checklists = given["checklists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let identity = given["identityId"].as_str().unwrap();
                DigestHistoryRecord {
                    evidence: DigestHistoryChecklist {
                        checklist: DigestChecklist {
                            id: row["checklistId"].as_str().unwrap().into(),
                            identity_id: identity.into(),
                            attempt_token: "must-never-be-published".into(),
                            through_sequence: 999,
                            state: DigestState::parse(row["outcome"].as_str().unwrap()).unwrap(),
                            created_at_ms: row["claimedAtMs"].as_u64().unwrap(),
                        },
                        settlement_sequence: row["settlementSequence"].as_u64().unwrap(),
                        settled_at_ms: row["settledAtMs"].as_u64(),
                        sealed_count: row["sealedCount"].as_u64(),
                        opportunity: row["opportunity"].as_str().map(|value| match value {
                            "idle" => DigestOpportunity::Idle,
                            "turn_boundary" => DigestOpportunity::TurnBoundary,
                            _ => panic!("invalid vector"),
                        }),
                    },
                    retained_count: row["retainedCount"].as_u64().unwrap(),
                    next_item_after: row["nextItemAfter"].as_u64(),
                    items: row["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| DigestItem {
                            sequence: item["sequence"].as_u64().unwrap(),
                            identity_id: identity.into(),
                            request_id: "content-not-read".into(),
                            kind: DigestKind::Fyi,
                            source: DigestSource::Incoming,
                            created_at_ms: item["createdAtMs"].as_u64().unwrap(),
                            checklist_id: None,
                            context_tokens_at_arrival: item["contextTokensAtArrival"].as_u64(),
                            context_observed_at_ms: item["contextObservedAtMs"].as_u64(),
                        })
                        .collect(),
                }
            })
            .collect();
        let page = DigestHistoryRead {
            observed_at_ms: given["observedAtMs"].as_u64().unwrap(),
            through_sequence: given["throughSequence"].as_u64().unwrap(),
            retained_from_sequence: given["retainedFromSequence"].as_u64(),
            legacy_settled_count: given["legacySettledCount"].as_u64().unwrap(),
            delivered_digests: given["deliveredDigests"].as_u64().unwrap(),
            next_after: given["nextAfter"].as_u64(),
            checklists,
        };
        let actual = history_value(given["identityId"].as_str().unwrap(), &page);
        assert_eq!(actual, vector["expected"], "{}", vector["name"]);
        let mut largest = page.clone();
        if let Some(row) = largest.checklists.first().cloned() {
            let mut header = row.clone();
            header.evidence.settlement_sequence = MAX_JS_SAFE_INTEGER;
            header.evidence.settled_at_ms = Some(MAX_JS_SAFE_INTEGER);
            header.evidence.checklist.created_at_ms = MAX_JS_SAFE_INTEGER;
            header.evidence.sealed_count = Some(MAX_JS_SAFE_INTEGER);
            header.retained_count = MAX_JS_SAFE_INTEGER;
            header.next_item_after = Some(MAX_JS_SAFE_INTEGER);
            header.items.clear();
            largest.checklists = vec![header; 128];
            if let Some(item) = row.items.first() {
                let mut item = item.clone();
                item.sequence = MAX_JS_SAFE_INTEGER;
                item.created_at_ms = MAX_JS_SAFE_INTEGER;
                item.context_tokens_at_arrival = Some(MAX_JS_SAFE_INTEGER);
                item.context_observed_at_ms = Some(MAX_JS_SAFE_INTEGER);
                largest.checklists[0].items = vec![item; 128];
            }
        }
        assert!(
            serde_json::to_vec(&history_value(
                given["identityId"].as_str().unwrap(),
                &largest
            ))
            .unwrap()
            .len()
                < 128 * 1024
        );
    }
}
