//! Approval-blocked notices use real durable claims and a controlled driver.
use super::*;
use crate::test_support::TestDirectory;
use std::{cell::RefCell, rc::Rc};
use tmt_core::{
    endpoint::ProcessIncarnation,
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, PrepareRequest, RequestEndpoint, RequestKind, RequestRoute, ResponseProof,
        Settlement, SubmitResponse,
        notification::{NotificationPolicy, batch::SendClaim},
    },
};

struct Approvals {
    calls: Rc<RefCell<Vec<String>>>,
    accepted: Rc<RefCell<Vec<String>>>,
    blocked: String,
}

impl Driver for Approvals {
    type Target = BindingEntry;
    type Error = RuntimeError;
    type Launch = crate::runtime::RuntimeCommand;

    fn send(
        &mut self,
        _: &BindingEntry,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<RuntimeError>> {
        self.calls.borrow_mut().push(message.into());
        if message == self.blocked {
            ActionResult::Failed(SendFailure::AwaitingApproval(RuntimeError::InvalidSession))
        } else {
            self.accepted.borrow_mut().push(message.into());
            ActionResult::Completed(DeliveryAcceptance::Submitted)
        }
    }
}

#[test]
fn blocked_notice_is_final_without_fallback_and_later_members_still_deliver() {
    for blocked in ["one", "three"] {
        let directory = TestDirectory::new();
        let database = directory.path.join("notices.db");
        let mut storage = Storage::open(&database).unwrap();
        let identity = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        let entry = BindingEntry {
            identity,
            binding: None,
        };
        let endpoint = RequestEndpoint {
            server: tmt_core::endpoint::ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
                server_id: "notice-server".into(),
                socket_path: "/tmp/notice-test.sock".into(),
                server_pid: 41,
                server_start_time: "server-start".into(),
            },
            pane_id: "%1".into(),
            pane_pid: 42,
        };
        let mut queued = None;
        for id in ["one", "two", "three"] {
            let attempt = format!("{id}-attempt");
            let mut service = RequestService::new(&mut storage, || 1_700_000_000_000);
            service
                .prepare(
                    PrepareRequest {
                        room_id: None,
                        kind: RequestKind::Request,
                        request_id: id.into(),
                        message: format!("question {id}"),
                        route: RequestRoute::Pane(endpoint.clone()),
                        wait: false,
                        expires_at_ms: 1_700_000_060_000,
                        originator: Originator::Explicit(entry.identity.id.clone()),
                        recipient_identity_id: Some(entry.identity.id.clone()),
                        preamble: None,
                    },
                    attempt.clone(),
                    1,
                )
                .unwrap();
            service
                .enable_notifications(
                    id,
                    NotificationPolicy {
                        deadline_ms: 1_700_000_001_000,
                        timeout_ms: 1000,
                        waiter: None,
                    },
                )
                .unwrap();
            service.begin_send(&attempt).unwrap();
            service.settle(&attempt, Settlement::Sent).unwrap();
            let hint = service
                .submit_response_with_hint(
                    SubmitResponse {
                        request_id: id.into(),
                        proof: ResponseProof::Recorded {
                            attempt_id: attempt,
                            endpoint: endpoint.clone(),
                        },
                        body: format!("accepted answer {id}"),
                    },
                    None,
                )
                .unwrap()
                .1
                .unwrap();
            queued = Some(
                storage
                    .queue_reply_notice(&hint, "binding", id, 5000, 2000, 1_700_000_000_000)
                    .unwrap(),
            );
        }
        let batch = queued.unwrap();
        let worker = ProcessIncarnation::new(123, "worker-start").unwrap();
        assert!(storage.claim_reply_notice_worker(&batch, &worker).unwrap());
        let SendClaim::Ready(notices) =
            storage.claim_reply_notice_send(&batch.id, &worker).unwrap()
        else {
            panic!("worker must own the sealed batch");
        };
        let calls = Rc::new(RefCell::new(Vec::new()));
        let accepted = Rc::new(RefCell::new(Vec::new()));
        let harness = HarnessId::new("notice-test").unwrap();
        let mut registry = RuntimeRegistry::default();
        registry
            .register(
                harness.clone(),
                "notice-test",
                0,
                Approvals {
                    calls: Rc::clone(&calls),
                    accepted: Rc::clone(&accepted),
                    blocked: blocked.into(),
                },
            )
            .unwrap();
        let progress = NoticeAttempt {
            batch: &batch,
            worker: &worker,
            notices: &notices,
            progress: std::cell::Cell::new(NoticeProgress::Unstarted),
            storage_error: std::cell::Cell::new(None),
        };
        let storage = RefCell::new(&mut storage);
        let outcome = send_preferred(
            || Messages::Notices(&progress).registered(&mut registry, &harness, &entry, &storage),
            || panic!("approval-required delivery must never run host input fallback"),
        );
        assert!(matches!(
            outcome,
            ActionResult::Failed(SendFailure::AwaitingApproval(Delivery::AwaitingApproval))
        ));
        assert_eq!(*calls.borrow(), ["one", "three", "two"]);
        assert_eq!(
            *accepted.borrow(),
            ["one", "three", "two"]
                .into_iter()
                .filter(|id| *id != blocked)
                .collect::<Vec<_>>()
        );
        assert!(progress.storage_error.take().is_none());
        // Durable SQL, independent of driver output: each settled hint is
        // definitely unavailable or sent, and accepted bodies are unchanged.
        let oracle = rusqlite::Connection::open(&database).unwrap();
        for id in ["one", "two", "three"] {
            let state: String = oracle
                .query_row(
                    "SELECT reply_state FROM request_notifications WHERE request_id=?",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, if id == blocked { "unavailable" } else { "sent" });
            let body: String = oracle
                .query_row(
                    "SELECT body FROM request_responses WHERE request_id=?",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(body, format!("accepted answer {id}"));
        }
        let remaining: i64 = oracle
            .query_row("SELECT count(*) FROM reply_notices", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            remaining, 0,
            "blocked notices cannot remain eligible for replay"
        );
    }
}
