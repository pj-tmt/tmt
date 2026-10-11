use super::*;
use crate::test_support::TestDirectory;
use rusqlite::Connection;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::WakeState,
};

const NOW: u64 = 1_700_000_000_000;
const OPERATION: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn only_first_dispatch_creation_can_schedule_a_wake_even_after_attempt_retention() {
    let mut fixture = Fixture::new();
    let input = fixture.input();
    let (receipt, created) = fixture
        .storage
        .dispatch_request_with_creation(input.clone(), 7, || NOW)
        .unwrap();
    assert!(created);
    let (replayed, created) = fixture
        .storage
        .dispatch_request_with_creation(input.clone(), 7, || panic!("replay clock"))
        .unwrap();
    assert_eq!(replayed, receipt);
    assert!(!created);
    fixture
        .oracle()
        .execute("DELETE FROM request_attempts", [])
        .unwrap();
    let (replayed, created) = fixture
        .storage
        .dispatch_request_with_creation(input, 7, || panic!("retained replay clock"))
        .unwrap();
    assert_eq!(replayed, receipt);
    assert!(!created);
    assert_eq!(
        talk_flow(&fixture.oracle()),
        (2, 2 * fixture.input().message.len() as i64)
    );
}

#[test]
fn concurrent_wake_claims_are_at_most_once_and_leave_inbox_acceptance_unchanged() {
    use std::sync::{Arc, Barrier};
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.recipient_ids.truncate(1);
    let receipt = fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    let request_id = receipt.items[0].request_id.clone();
    let database = fixture.storage.path.clone();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let database = database.clone();
            let request_id = request_id.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let mut storage = Storage::open(database).unwrap();
                barrier.wait();
                RequestService::new(&mut storage, || NOW)
                    .claim_wake(&request_id)
                    .unwrap()
            })
        })
        .collect();
    let claims: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(claims.iter().filter(|claim| claim.claimed).count(), 1);
    assert!(claims.iter().all(|claim| claim.state == WakeState::Claimed));
    assert_eq!(
        fixture.storage.dispatch_receipt(OPERATION).unwrap(),
        Some(receipt)
    );
    let row: (String, String) = fixture
        .oracle()
        .query_row(
            "SELECT route_kind,status FROM request_attempts WHERE request_id=?",
            [&request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("inbox".into(), "queued".into()));
    let mut service = RequestService::new(&mut fixture.storage, || NOW);
    service
        .settle_wake(&request_id, WakeState::Uncertain)
        .unwrap();
    let replay = service.claim_wake(&request_id).unwrap();
    assert!(!replay.claimed);
    assert_eq!(replay.state, WakeState::Uncertain);
}

#[test]
fn unclosed_claim_remains_unknown_after_reopen_and_never_replays_pane_input() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.recipient_ids.truncate(1);
    let receipt = fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    let request_id = &receipt.items[0].request_id;
    assert!(
        RequestService::new(&mut fixture.storage, || NOW)
            .claim_wake(request_id)
            .unwrap()
            .claimed
    );
    let mut reopened = Storage::open(&fixture.storage.path).unwrap();
    let claim = RequestService::new(&mut reopened, || NOW)
        .claim_wake(request_id)
        .unwrap();
    assert!(!claim.claimed);
    assert_eq!(claim.state, WakeState::Claimed);
    assert_eq!(reopened.dispatch_receipt(OPERATION).unwrap(), Some(receipt));
}

#[test]
fn direct_room_wake_fence_rechecks_membership_after_claim() {
    use tmt_core::room::{MembershipChange, RoomRepository, RoomWrite};
    let mut fixture = Fixture::new();
    let recipient = fixture.recipients[0].clone();
    let room = "33333333-3333-4333-8333-333333333333";
    fixture
        .storage
        .save_meeting_room(
            room,
            RoomWrite {
                expected_revision: 0,
                name: "Design".into(),
                member_ids: vec![recipient.clone()],
            },
        )
        .unwrap();
    let mut input = fixture.input();
    input.recipient_ids = vec![recipient.clone()];
    input.room = Some(DispatchRoom::Direct {
        room_id: room.into(),
    });
    let receipt = fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    let request_id = &receipt.items[0].request_id;
    let mut service = RequestService::new(&mut fixture.storage, || NOW);
    assert!(service.claim_wake(request_id).unwrap().claimed);
    assert!(
        service
            .wake_recipient_is_eligible(request_id, &recipient)
            .unwrap()
    );
    fixture
        .storage
        .change_meeting_membership(room, &recipient, MembershipChange::Leave)
        .unwrap();
    let mut service = RequestService::new(&mut fixture.storage, || NOW);
    assert!(
        !service
            .wake_recipient_is_eligible(request_id, &recipient)
            .unwrap()
    );
    assert_eq!(
        fixture.storage.dispatch_receipt(OPERATION).unwrap(),
        Some(receipt)
    );
}

#[test]
fn dispatch_lookup_recovers_acceptance_without_replaying_or_requiring_live_requests() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.storage.dispatch_receipt(OPERATION).unwrap(), None);
    assert!(matches!(
        fixture.storage.dispatch_receipt("invalid"),
        Err(DispatchError::Invalid)
    ));
    let receipt = fixture
        .storage
        .dispatch_request(fixture.input(), 1, || NOW)
        .unwrap();
    assert_eq!(
        fixture.storage.dispatch_receipt(OPERATION).unwrap(),
        Some(receipt.clone())
    );
    let oracle = fixture.oracle();
    // Receipt history is immutable even after normal request retention has removed bodies/attempts.
    oracle.execute("DELETE FROM request_attempts", []).unwrap();
    assert_eq!(
        fixture.storage.dispatch_receipt(OPERATION).unwrap(),
        Some(receipt)
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn dispatch_lookup_rejects_corrupted_or_wrong_operation_receipts() {
    let mut fixture = Fixture::new();
    let receipt = fixture
        .storage
        .dispatch_request(fixture.input(), 1, || NOW)
        .unwrap();
    let mut wrong = receipt;
    wrong.operation_id = "22222222-2222-4222-8222-222222222222".into();
    let oracle = fixture.oracle();
    for value in [
        "{}".to_owned(),
        String::from_utf8(encode_receipt(&wrong)).unwrap(),
    ] {
        oracle
            .execute(
                "UPDATE office_dispatch_operations SET receipt=? WHERE operation_id=?",
                params![value, OPERATION],
            )
            .unwrap();
        assert!(matches!(
            fixture.storage.dispatch_receipt(OPERATION),
            Err(DispatchError::Storage(StorageError {
                code: StorageErrorCode::Corrupt,
                ..
            }))
        ));
    }
}

#[test]
fn known_sender_survives_enqueue_and_cannot_be_changed_by_replay() {
    use tmt_core::request::Originator;
    let mut fixture = Fixture::new();
    let sender = create_or_resolve(&mut fixture.storage, "Sender", Lifetime::Saved)
        .unwrap()
        .identity;
    let mut input = fixture.input();
    input.originator = Originator::Explicit(sender.id.clone());
    let receipt = fixture
        .storage
        .dispatch_request(input.clone(), 7, || NOW)
        .unwrap();
    let oracle = fixture.oracle();
    let rows = oracle
        .prepare("SELECT originator_kind,originator_identity_id FROM request_attempts")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows, vec![("explicit".into(), sender.id); 2]);
    assert_eq!(
        fixture
            .storage
            .dispatch_request(input.clone(), 1, || panic!("replay clock"))
            .unwrap(),
        receipt
    );
    input.originator = Originator::Unknown;
    assert!(matches!(
        fixture.storage.dispatch_request(input, 7, || NOW),
        Err(DispatchError::IdempotencyConflict)
    ));
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn announcements_share_atomic_dispatch_and_cannot_be_replayed_as_requests() {
    use tmt_core::request::{
        RequestKind,
        attention::{FinalState, IncomingKind},
    };
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.kind = RequestKind::Announcement;
    let receipt = fixture
        .storage
        .dispatch_request(input.clone(), 7, || NOW)
        .unwrap();
    assert_eq!(receipt.items.len(), 2);
    let mut requests = RequestService::new(&mut fixture.storage, || NOW);
    for item in &receipt.items {
        let incoming = requests
            .list_incoming(&item.recipient_id, None, None, None)
            .unwrap();
        assert_eq!(incoming.items.len(), 1);
        assert_eq!(incoming.items[0].exchange.request_id, item.request_id);
        assert_eq!(incoming.items[0].kind, IncomingKind::Announcement);
        assert_eq!(
            incoming.items[0].exchange.final_state,
            FinalState::NotRequired
        );
    }
    assert_eq!(
        fixture
            .storage
            .dispatch_request(input.clone(), 1, || panic!("replay clock"))
            .unwrap(),
        receipt
    );
    input.kind = RequestKind::Request;
    assert!(matches!(
        fixture.storage.dispatch_request(input, 7, || NOW),
        Err(DispatchError::IdempotencyConflict)
    ));
    assert_eq!(
        fixture
            .oracle()
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        fixture
            .oracle()
            .query_row("SELECT count(*) FROM request_responses", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

struct Fixture {
    _directory: TestDirectory,
    storage: Storage,
    recipients: Vec<String>,
}
impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
        let recipients = ["Alice", "Bob"]
            .into_iter()
            .map(|name| {
                create_or_resolve(&mut storage, name, Lifetime::Saved)
                    .unwrap()
                    .identity
                    .id
            })
            .collect();
        Self {
            _directory: directory,
            storage,
            recipients,
        }
    }
    fn input(&self) -> DispatchInput {
        DispatchInput {
            originator: tmt_core::request::Originator::Unknown,
            kind: tmt_core::request::RequestKind::Request,
            operation_id: OPERATION.into(),
            recipient_ids: self.recipients.clone(),
            message: "Review this together.\nKeep exact text!".into(),
            room: None,
        }
    }
    fn oracle(&self) -> Connection {
        Connection::open(&self.storage.path).unwrap()
    }
}

#[test]
fn recipients_are_deduplicated_and_replay_survives_reopen_without_dispatching_again() {
    let mut fixture = Fixture::new();
    let mut input = fixture.input();
    input.recipient_ids.push(input.recipient_ids[0].clone());
    let receipt = fixture.storage.dispatch_request(input, 90, || NOW).unwrap();
    assert_eq!(receipt.items.len(), 2);
    assert!(
        receipt
            .items
            .iter()
            .all(|item| item.acceptance == Acceptance::Queued)
    );
    let oracle = fixture.oracle();
    let rows: Vec<(String, String, String, String, i64)> = oracle.prepare(
        "SELECT request_id,recipient_identity_id,status,message_text,recipient_attention_revision FROM request_attempts ORDER BY recipient_identity_id"
    ).unwrap().query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows.len(), 2);
    for (row, item) in rows.iter().zip(&receipt.items) {
        assert_eq!(
            (&row.0, &row.1, row.2.as_str(), row.3.as_str(), row.4),
            (
                &item.request_id,
                &item.recipient_id,
                "queued",
                fixture.input().message.as_str(),
                1
            )
        );
    }
    let path = fixture.storage.path.clone();
    fixture.storage.close().unwrap();
    fixture.storage = Storage::open(path).unwrap();
    let mut replay = fixture.input();
    replay.recipient_ids.reverse();
    let repeated = fixture
        .storage
        .dispatch_request(replay, 7, || panic!("replay must not use a new clock"))
        .unwrap();
    assert_eq!(receipt, repeated);
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT sum(latest_revision) FROM request_attention_identities",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    // Missing request rows do not let a retained operation create work again.
    oracle
        .execute_batch("DELETE FROM request_attempts;")
        .unwrap();
    let replay = fixture.input();
    assert_eq!(
        fixture.storage.dispatch_request(replay, 7, || NOW).unwrap(),
        receipt
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn replay_rejects_message_and_audience_changes_before_any_new_request() {
    let mut fixture = Fixture::new();
    let input = fixture.input();
    fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    let mut changed = fixture.input();
    changed.message.push(' ');
    assert!(matches!(
        fixture.storage.dispatch_request(changed, 7, || NOW),
        Err(DispatchError::IdempotencyConflict)
    ));
    let mut changed = fixture.input();
    changed.recipient_ids.pop();
    assert!(matches!(
        fixture.storage.dispatch_request(changed, 7, || NOW),
        Err(DispatchError::IdempotencyConflict)
    ));
    assert_eq!(
        fixture
            .oracle()
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn concurrent_replays_share_one_receipt_and_one_request_per_recipient() {
    let fixture = Fixture::new();
    let input = fixture.input();
    let operations: [crate::storage::test_support::Operation<_>; 2] = [
        Box::new({
            let input = input.clone();
            move |storage| storage.dispatch_request(input, 7, || NOW)
        }),
        Box::new(move |storage| storage.dispatch_request(input, 7, || NOW)),
    ];
    let [first, second] =
        crate::storage::test_support::concurrent_pair(&fixture.storage.path, operations)
            .map(Result::unwrap);
    assert_eq!(first, second);
    let oracle = fixture.oracle();
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM request_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM office_dispatch_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        talk_flow(&oracle),
        (2, 2 * fixture.input().message.len() as i64)
    );
}

#[test]
fn receipt_failure_rolls_back_all_recipients_and_retry_publishes_once() {
    let mut fixture = Fixture::new();
    let oracle = fixture.oracle();
    oracle.execute_batch("CREATE TRIGGER reject_dispatch BEFORE INSERT ON office_dispatch_operations BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;").unwrap();
    let input = fixture.input();
    assert!(matches!(
        fixture.storage.dispatch_request(input, 7, || NOW),
        Err(DispatchError::Storage(_))
    ));
    for table in [
        "request_attempts",
        "exchange_flow",
        "request_attention_identities",
        "request_recipient_attention_identities",
        "office_dispatch_operations",
    ] {
        assert_eq!(
            oracle
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    oracle
        .execute_batch("DROP TRIGGER reject_dispatch;")
        .unwrap();
    let input = fixture.input();
    let receipt = fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    assert_eq!(receipt.items.len(), 2);
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM request_attempts WHERE status='queued'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM office_dispatch_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn retired_recipient_is_explicitly_rejected_without_retargeting_a_reused_name() {
    let mut fixture = Fixture::new();
    let retired = fixture.recipients[0].clone();
    let oracle = fixture.oracle();
    oracle
        .execute(
            "UPDATE identities SET retired_at_ms=? WHERE id=?",
            params![NOW as i64, retired],
        )
        .unwrap();
    let replacement = create_or_resolve(&mut fixture.storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let input = fixture.input();
    let receipt = fixture.storage.dispatch_request(input, 7, || NOW).unwrap();
    assert_eq!(
        receipt
            .items
            .iter()
            .find(|item| item.recipient_id == retired)
            .unwrap()
            .acceptance,
        Acceptance::RecipientUnavailable
    );
    assert_eq!(
        receipt
            .items
            .iter()
            .filter(|item| item.acceptance == Acceptance::Queued)
            .count(),
        1
    );
    assert!(
        receipt
            .items
            .iter()
            .all(|item| item.recipient_id != replacement.id)
    );
    let rejected: (String,i64,i64) = oracle.query_row("SELECT status,wait_active,recipient_attention_revision FROM request_attempts WHERE recipient_identity_id=?", [retired], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(rejected, ("definitely_failed".into(), 0, 0));
}

fn talk_flow(oracle: &Connection) -> (i64, i64) {
    oracle.query_row(
        "SELECT COALESCE(SUM(count),0),COALESCE(SUM(bytes),0) FROM exchange_flow WHERE kind='talk'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap()
}
