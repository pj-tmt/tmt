use super::*;

fn request() -> QueueRequest {
    QueueRequest::with_ids(
        ProviderSessionId::new("owned-thread").unwrap(),
        "one message",
        "rpc-1".into(),
        "caller-1".into(),
    )
    .unwrap()
}

fn accepted() -> Value {
    json!({"id":"rpc-1", "result":{"queuedSubmission":{
        "id":"submission-1", "clientUserMessageId":"caller-1",
        "input":[{"type":"text", "text":"one message", "text_elements":[]}]
    }}})
}

fn receipt(value: &Value) -> QueueOutcome {
    request().receipt(&serde_json::to_vec(value).unwrap())
}

#[test]
fn one_frame_targets_the_exact_thread_without_starting_or_forking() {
    let frame = request().frame();
    assert_eq!(frame["method"], "thread/queue/add");
    assert_eq!(frame["params"]["threadId"], "owned-thread");
    assert_eq!(frame["params"]["clientUserMessageId"], "caller-1");
    assert_eq!(frame["params"]["input"][0]["text"], "one message");
}

#[test]
fn valid_receipt_records_submission_without_inventing_processing() {
    assert_eq!(
        receipt(&accepted()),
        QueueOutcome::Accepted {
            submission_id: "submission-1".into()
        }
    );
}

#[test]
fn exact_archived_and_deleted_thread_errors_are_pre_enqueue_refusals() {
    for (code, message) in [
        (
            -32600,
            "session owned-thread is archived. Run `codex unarchive owned-thread` to unarchive it first.",
        ),
        (
            -32603,
            "failed to read thread: invalid thread-store request: no rollout found for thread id owned-thread",
        ),
    ] {
        let mut response = json!({"id":"rpc-1", "error":{"code":code, "message":message}});
        assert_eq!(receipt(&response), QueueOutcome::Refused { code });
        response["id"] = json!("some-other-request");
        assert_eq!(receipt(&response), QueueOutcome::Uncertain);
        response["id"] = json!("rpc-1");
        response["error"]["message"] = json!(message.replace("owned-thread", "other-thread"));
        assert_eq!(receipt(&response), QueueOutcome::Uncertain);
        response["error"]["message"] = json!(message);
        response["error"]["code"] = json!(code.to_string());
        assert_eq!(receipt(&response), QueueOutcome::Uncertain);
    }
}

#[test]
fn correlated_internal_errors_do_not_prove_absence_of_enqueue_side_effects() {
    for (code, message) in [
        (-32603, "internal error"),
        (-32603, "failed to serialize queued submission"),
        (-32600, "owned thread is archived"),
        (-32600, "unknown request failure"),
    ] {
        assert_eq!(
            receipt(&json!({"id":"rpc-1", "error":{
                "code":code, "message":message
            }})),
            QueueOutcome::Uncertain
        );
    }
}

#[test]
fn changed_correlation_or_echo_never_turns_into_acceptance() {
    for pointer in [
        "/id",
        "/result/queuedSubmission/clientUserMessageId",
        "/result/queuedSubmission/input/0/text",
        "/result/queuedSubmission/input/0/type",
    ] {
        let mut response = accepted();
        *response.pointer_mut(pointer).unwrap() = json!("different");
        assert_eq!(receipt(&response), QueueOutcome::Uncertain, "{pointer}");
    }
    let mut response = accepted();
    response["result"]["queuedSubmission"]["input"] = json!([]);
    assert_eq!(receipt(&response), QueueOutcome::Uncertain);
}

#[test]
fn missing_malformed_ambiguous_and_oversized_responses_are_uncertain() {
    for response in [b"".as_slice(), b"{", b"null", b"{}"] {
        assert_eq!(request().receipt(response), QueueOutcome::Uncertain);
    }
    let mut response = accepted();
    response["error"] = json!({"code":-1,"message":"also an error"});
    assert_eq!(receipt(&response), QueueOutcome::Uncertain);
    assert_eq!(
        request().receipt(&vec![b' '; RESPONSE_LIMIT + 1]),
        QueueOutcome::Uncertain
    );
}

#[test]
fn repeated_caller_id_does_not_imply_provider_deduplication() {
    let first = accepted();
    let mut second = first.clone();
    second["result"]["queuedSubmission"]["id"] = json!("submission-2");
    assert_eq!(
        receipt(&first),
        QueueOutcome::Accepted {
            submission_id: "submission-1".into()
        }
    );
    assert_eq!(
        receipt(&second),
        QueueOutcome::Accepted {
            submission_id: "submission-2".into()
        }
    );
}

#[test]
fn oversized_input_is_rejected_before_transport_exists() {
    assert!(matches!(
        QueueRequest::new(
            ProviderSessionId::new("thread").unwrap(),
            &"x".repeat(MESSAGE_LIMIT + 1)
        ),
        Err(InvalidRequest::MessageTooLarge)
    ));
}
