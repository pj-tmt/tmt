use super::*;
use std::fs;
use tmt_adapters::reply_receipt::decode_reply_receipt;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        ResponseLookup, ResponseRejection, SubmitResponse,
        history::{AnswersQuery, HistoryQuery, HistoryScope},
    },
};

struct Setup {
    root: std::path::PathBuf,
    storage: Storage,
    ben: String,
}

impl Setup {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("tmt-talk-remote-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut storage = Storage::open(root.join("tmt.db")).unwrap();
        let ben = create_or_resolve(&mut storage, "Ben", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        Self { root, storage, ben }
    }

    /// A script standing in for `tmt-remote`: it records its stdin, then runs `body`.
    fn remote(&self, body: &str) -> std::path::PathBuf {
        let path = self.root.join("tmt-remote");
        let _ = fs::remove_file(self.root.join("stdin"));
        tmt_test_support::write_executable(
            &path,
            format!(
                "#!/bin/sh\ncat > \"{}/stdin\"\n{body}\n",
                self.root.display()
            )
            .as_bytes(),
            0o755,
        )
        .unwrap();
        path
    }

    fn stdin(&self) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(self.root.join("stdin")).unwrap()).unwrap()
    }

    fn send(&mut self, input: &Input, program: &Path) -> Result<Report, Failure> {
        send(
            &mut self.storage,
            input,
            &Settings::default(),
            program.as_os_str(),
            &UnixCommandRunner,
            Duration::from_secs(10),
        )
    }

    fn service(&mut self) -> RequestService<'_, Storage, fn() -> u64> {
        RequestService::new(&mut self.storage, wall_time_ms)
    }

    fn delivery(&mut self, request_id: &str) -> AttemptStatus {
        self.service()
            .request_detail(request_id)
            .unwrap()
            .item
            .delivery
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn input(target: &str, expect_name: Option<&str>) -> Input {
    Input {
        remote: "bob".into(),
        target: target.into(),
        expect_name: expect_name.map(str::to_owned),
        message: "hello across machines".into(),
        originator: Some("Ben".into()),
    }
}

fn answer(state: &str) -> String {
    format!(
        "printf '%s' '{{\"version\":1,\"state\":\"{state}\",\"remote\":\"bob\",\"target\":{{\"name\":\"reviewer\"}}}}'"
    )
}

#[test]
fn a_sent_request_hands_remote_a_receipt_that_later_takes_the_peers_final() {
    let mut setup = Setup::new("sent");
    let program = setup.remote(&answer("sent"));
    let report = setup.send(&input("reviewer", None), &program).unwrap();
    assert_eq!(report.status, AttemptStatus::Sent);
    assert_eq!(setup.delivery(&report.request_id), AttemptStatus::Sent);

    let stdin = setup.stdin();
    let request_id = report.request_id.clone();
    assert_eq!(stdin["version"], 1);
    assert_eq!(stdin["remote"], "bob");
    assert_eq!(stdin["requestId"], request_id.strip_prefix("req_").unwrap());
    assert_eq!(stdin["from"]["name"], "Ben");
    assert_eq!(stdin["from"]["id"], setup.ben.as_str());
    assert_eq!(stdin["target"], serde_json::json!({"name": "reviewer"}));
    assert_eq!(stdin["message"], "hello across machines");
    assert_eq!(stdin["reply"]["requestId"], request_id.as_str());
    let created = setup
        .service()
        .request_detail(&request_id)
        .unwrap()
        .item
        .prepared_at_ms;
    assert_eq!(stdin["createdAtMs"], created);

    // The receipt Remote holds is the request's own: the peer's final reaches every reader.
    let receipt = stdin["reply"]["receipt"].as_str().unwrap();
    let proof = decode_reply_receipt(receipt, &request_id).unwrap();
    let submit = |body: &str, proof| SubmitResponse {
        request_id: request_id.clone(),
        proof,
        body: body.into(),
    };
    setup
        .service()
        .submit_response(submit("peer final", proof.clone()))
        .unwrap();
    let ResponseLookup::Available(stored) = setup.service().get_response(&request_id).unwrap()
    else {
        panic!("the final is retained");
    };
    assert_eq!(stored.body, "peer final");
    let ben = setup.ben.clone();
    let answers = setup
        .service()
        .request_answers(AnswersQuery {
            originator_id: ben.clone(),
            before: None,
            limit: 20,
        })
        .unwrap();
    assert_eq!(answers.items.len(), 1);
    let results = setup
        .service()
        .request_history(HistoryQuery {
            scope: HistoryScope::OriginatorResults {
                originator_id: ben,
                recipient_id: None,
            },
            before: None,
            limit: 20,
        })
        .unwrap();
    assert_eq!(results.items.len(), 1);
    // An identical retry is safe; a different body conflicts.
    assert!(
        setup
            .service()
            .submit_response(submit("peer final", proof.clone()))
            .is_ok()
    );
    assert!(matches!(
        setup.service().submit_response(submit("another", proof)),
        Err(RequestError::Response(ResponseRejection::Conflict))
    ));
}

#[test]
fn a_held_request_is_queued_and_can_still_be_answered() {
    let mut setup = Setup::new("held");
    let program = setup.remote(&answer("held"));
    let report = setup.send(&input("reviewer", None), &program).unwrap();
    assert_eq!(report.status, AttemptStatus::Queued);
    assert_eq!(setup.delivery(&report.request_id), AttemptStatus::Queued);
}

#[test]
fn a_target_named_by_uuid_is_pinned_with_its_expected_name() {
    let mut setup = Setup::new("pinned");
    let program = setup.remote(&answer("sent"));
    let id = "7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f";
    let report = setup.send(&input(id, Some("reviewer")), &program).unwrap();
    assert_eq!(report.target_id.as_deref(), Some(id));
    assert_eq!(
        setup.stdin()["target"],
        serde_json::json!({"id": id, "name": "reviewer"})
    );
}

#[test]
fn what_remote_proves_undelivered_fails_definitely_and_takes_no_final() {
    for (tag, body, code) in [
        ("refused", "printf '%s' '{\"version\":1,\"state\":\"refused\",\"remote\":\"bob\",\"target\":{\"name\":\"reviewer\"},\"reason\":\"REMOTE_AGENT_NOT_FOUND\"}'".to_owned(), "REMOTE_REFUSED"),
        ("unavailable", answer("unavailable"), "REMOTE_UNAVAILABLE"),
        (
            "envelope",
            "printf '%s' '{\"error\":{\"code\":\"REMOTE_CLOUD_MESSAGE_TOO_LARGE\",\"message\":\"x\"}}'; exit 1".to_owned(),
            "REMOTE_CLOUD_MESSAGE_TOO_LARGE",
        ),
    ] {
        let mut setup = Setup::new(tag);
        let program = setup.remote(&body);
        let failure = setup.send(&input("reviewer", None), &program).unwrap_err();
        assert_eq!(failure.code, code, "{tag}");
        let document = failure.document();
        let request_id = document["requestId"].as_str().unwrap().to_owned();
        assert!(document.get("deliveryState").is_none(), "{tag}");
        assert_eq!(setup.delivery(&request_id), AttemptStatus::DefinitelyFailed, "{tag}");
        let proof = decode_reply_receipt(
            setup.stdin()["reply"]["receipt"].as_str().unwrap(),
            &request_id,
        )
        .unwrap();
        assert!(
            setup
                .service()
                .submit_response(SubmitResponse {
                    request_id,
                    proof,
                    body: "late".into()
                })
                .is_err(),
            "{tag}"
        );
    }
}

#[test]
fn anything_that_may_have_sent_is_uncertain_never_retried_and_still_answerable() {
    for (tag, body) in [
        ("unconfirmed", answer("unconfirmed")),
        ("silent", "exit 0".to_owned()),
        ("crashed", "exit 9".to_owned()),
    ] {
        let mut setup = Setup::new(tag);
        let program = setup.remote(&body);
        let failure = setup.send(&input("reviewer", None), &program).unwrap_err();
        assert_eq!(failure.code, "REMOTE_UNCONFIRMED", "{tag}");
        let document = failure.document();
        assert_eq!(document["deliveryState"], "uncertain", "{tag}");
        let request_id = document["requestId"].as_str().unwrap().to_owned();
        assert_eq!(
            setup.delivery(&request_id),
            AttemptStatus::Uncertain,
            "{tag}"
        );
        let proof = decode_reply_receipt(
            setup.stdin()["reply"]["receipt"].as_str().unwrap(),
            &request_id,
        )
        .unwrap();
        assert!(
            setup
                .service()
                .submit_response(SubmitResponse {
                    request_id,
                    proof,
                    body: "it arrived".into()
                })
                .is_ok(),
            "{tag}"
        );
    }
}

#[test]
fn an_unknown_sender_stops_before_anything_is_stored_or_sent() {
    let mut setup = Setup::new("nosender");
    let program = setup.remote(&answer("sent"));
    let mut unknown = input("reviewer", None);
    unknown.originator = Some("Nobody".into());
    let failure = setup.send(&unknown, &program).unwrap_err();
    assert_eq!(failure.code, "NAME_NOT_FOUND");
    assert!(!setup.root.join("stdin").exists(), "Remote was never run");
}

#[test]
fn names_and_the_peer_are_bounded_before_anything_is_stored() {
    let long = "n".repeat(129);
    for (remote, target, expect) in [
        ("", "reviewer", None),
        (&"r".repeat(65)[..], "reviewer", None),
        ("bob", "", None),
        ("bob", &long[..], None),
        ("bob", "with\ncontrol", None),
        ("bob", "7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f", Some("")),
    ] {
        let mut candidate = input(target, expect);
        candidate.remote = remote.into();
        assert_eq!(
            validate(&candidate).unwrap_err().code,
            "REMOTE_TARGET_INVALID",
            "{remote:?} {target:?}"
        );
    }
    assert!(validate(&input("reviewer", None)).is_ok());
}
