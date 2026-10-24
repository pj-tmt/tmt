use super::*;
const ID: &str = "11111111-1111-4111-8111-111111111111";

fn vector() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../../contracts/recent-responses-v1.json"
    ))
    .unwrap()
}

#[test]
fn shared_capture_vector_preserves_duplicate_text_rollback_and_exact_bytes() {
    let items = parse(ID, &vector()["resource"]).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], items[1]);
    assert_eq!(items[2].text, "last\r\nline\n");
    assert_eq!(
        items.iter().map(|item| item.ended_ms).collect::<Vec<_>>(),
        [1000, 1000, 999]
    );
    // The rollback remains behind the equal-time captures; an exchange wins ties.
    assert_eq!(
        merge(&[998, 1000, 1001], &items),
        [
            Source::Exchange(0),
            Source::Exchange(1),
            Source::Pane(0),
            Source::Pane(1),
            Source::Pane(2),
            Source::Exchange(2),
        ]
    );
}

#[test]
fn capability_is_first_and_missing_or_failed_is_unavailable_not_empty() {
    let mut calls = Vec::new();
    let result = read_with(ID, |operation, _| {
        calls.push(operation.to_owned());
        Some(json!({"operations":[]}))
    });
    assert_eq!(calls, ["capabilities"]);
    assert_eq!(result, Observation::Unavailable);
    for resource in [None, Some(json!({"error":{"code":"API_INPUT_INVALID"}}))] {
        let mut calls = Vec::new();
        let result = read_with(ID, |operation, input| {
            calls.push(operation.to_owned());
            if operation == "capabilities" {
                Some(json!({"operations":["responses.recent"]}))
            } else {
                assert_eq!(input, json!({"identityId":ID,"limit":50}));
                resource.clone()
            }
        });
        assert_eq!(calls, ["capabilities", "responses.recent"]);
        assert_eq!(result, Observation::Unavailable);
    }
    assert_eq!(
        read_with(ID, |operation, _| Some(if operation == "capabilities" {
            json!({"operations":["responses.recent"]})
        } else {
            json!({"identityId":ID,"items":[],"coverage":{"olderDropped":false}})
        })),
        Observation::observed(vec![])
    );
}

#[test]
fn response_bounds_identity_and_required_metadata_fail_closed_additive_fields_work() {
    let item = json!({"text":"🦀".repeat(4096),"endedMs":3,"provider":"codex","truncated":true,"originalBytes":20000,"future":1});
    let resource =
        json!({"identityId":ID,"items":[item],"coverage":{"olderDropped":false},"future":true});
    let parsed = parse(ID, &resource).unwrap();
    assert!(parsed[0].truncated);
    assert_eq!(parsed[0].text.len(), 16384);
    assert_eq!(
        parse("22222222-2222-4222-8222-222222222222", &resource),
        None
    );
    for (field, bad) in [
        ("text", json!("x".repeat(16385))),
        ("provider", Value::Null),
        ("endedMs", json!(-1)),
        ("truncated", Value::Null),
        ("sessionId", Value::Null),
        ("originalBytes", json!(16383)),
    ] {
        let mut changed = resource.clone();
        changed["items"][0][field] = bad;
        assert_eq!(parse(ID, &changed), None, "{field}");
    }
    assert_eq!(
        parse(
            ID,
            &json!({"identityId":ID,"items":vec![resource["items"][0].clone();51],"coverage":{"olderDropped":false}})
        ),
        None
    );
    assert_eq!(
        read_with("name", |_, _| panic!("names never reach Core")),
        Observation::Unavailable
    );
}

#[test]
fn public_page_keeps_fifty_answers_and_every_coverage_capture_and_cursor_field() {
    let mut resource = vector()["resource"].clone();
    resource["items"] = json!(vec![resource["items"][0].clone(); 50]);
    resource["coverage"] = json!({"olderDropped":true,"droppedThroughMs":42});
    resource["capture"] = json!({"enabled":false});
    resource["nextBefore"] = json!(17);
    let page = parse(ID, &resource).unwrap();
    assert_eq!(page.len(), 50);
    assert_eq!(
        page.coverage,
        Coverage {
            older_dropped: true,
            dropped_through_ms: Some(42)
        }
    );
    assert_eq!(page.capture, Some(false));
    assert_eq!(page.next_before, Some(17));
    for field in ["capture", "nextBefore"] {
        let mut invalid = resource.clone();
        invalid[field] = Value::Null;
        assert_eq!(parse(ID, &invalid), None, "{field}");
    }
    let mut calls = Vec::new();
    assert!(
        page_with(ID, Some(17), &mut |_, input| {
            calls.push(input);
            Some(resource.clone())
        })
        .is_none(),
        "same cursor cannot spin"
    );
    assert_eq!(calls, [json!({"identityId":ID,"before":17,"limit":50})]);
}
