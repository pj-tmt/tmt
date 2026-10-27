//! A normal signed Session remains live at coordinator disclosure and writer entry.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::{
    admission::{BindingAction, MessagePermit},
    session::DoorSessions,
    store::Grant,
};
use serde_json::{Value, json};

fn signed(
    f: &DurableFixture,
    browser: &Grant,
    window: &str,
    session: &str,
    id: &str,
    operation: &str,
    payload: &Value,
) -> Vec<u8> {
    let payload = payload.to_string().into_bytes();
    let kind = if operation == "session.open" {
        "control"
    } else {
        "request"
    };
    let sequence = if operation == "session.open" {
        "0"
    } else {
        "1"
    };
    let now = crate::pairing::now_ms().unwrap();
    let envelope = canonical::Envelope {
        kind,
        id,
        correlation_id: None,
        machine_id: &f.context.owner,
        window_id: window,
        client_id: &browser.client_id,
        session_id: session,
        sequence,
        timestamp_ms: now,
        origin: &browser.origin,
        operation,
        payload: &payload,
    };
    let key = ed25519_dalek::SigningKey::from_bytes(&[24; 32]);
    let signature = key
        .sign(&canonical::envelope(&envelope).unwrap())
        .to_bytes();
    serde_json::to_vec(
        &json!({"version":1,"profile":"local-v1","kind":kind,"id":id,
        "correlationId":null,"machineId":f.context.owner,"windowId":window,
        "clientId":browser.client_id,"sessionId":session,"sequence":sequence,
        "timestampMs":now,"origin":browser.origin,"operation":operation,
        "payload":canonical::base64url(&payload),"signature":canonical::base64url(&signature)}),
    )
    .unwrap()
}
fn permit(
    f: &DurableFixture,
    browser: &Grant,
    snapshot: &Snapshot,
) -> (Arc<DoorSessions>, MessagePermit) {
    let window = crate::store::uuid_v4().unwrap();
    let sessions = Arc::new(DoorSessions::new(
        f.context.owner.clone(),
        window.clone(),
        browser.origin.clone(),
        "/r/fixture/x/".into(),
        f.machine_key(),
        f.store.clone(),
        crate::session::IDLE,
    ));
    let open = signed(
        f,
        browser,
        &window,
        "new",
        &crate::store::uuid_v4().unwrap(),
        "session.open",
        &json!({"clientNonce":crate::store::uuid_v4().unwrap().replace('-', "")}),
    );
    let opened = sessions.open(Some(&browser.origin), &open).unwrap();
    let reply: Value = serde_json::from_slice(&opened.response).unwrap();
    let id = crate::store::uuid_v4().unwrap();
    let input = json!({"operationId":id,"offerId":snapshot.context.offer,
        "snapshot":canonical::base64url(&snapshot.digest),"matched":true,"sending":false});
    let bytes = signed(
        f,
        browser,
        &window,
        reply["sessionId"].as_str().unwrap(),
        &id,
        "remote.invites.confirm",
        &input,
    );
    let permit = sessions
        .admit(BindingAction::Append, Some(&browser.origin), &bytes, 1024)
        .unwrap();
    (sessions, permit)
}
#[test]
fn ended_session_cannot_disclose_or_confirm_the_previously_admitted_candidate() {
    for ended in [false, true] {
        let f = DurableFixture::new();
        let browser = f.browser();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (coordinator, _, snapshot) =
            f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
        let (sessions, permit) = permit(&f, &browser, &snapshot);
        let id = permit.message.envelope().id.to_owned();
        let intent = crate::journal::intent(&permit.message).unwrap();
        f.store
            .lock()
            .unwrap()
            .adopt_invitation_confirmation(&browser, &id, &intent)
            .unwrap();
        let read_actor = OwnerActor::owner_browser(
            browser.client_id.clone(),
            browser.public_key,
            browser.revision,
        )
        .with_admission(&permit)
        .unwrap();
        let actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: false,
            },
        )
        .unwrap()
        .with_admission(&permit)
        .unwrap();
        if ended {
            sessions.end_device(&browser.client_id);
        }
        let observed =
            coordinator.owner_observation(&read_actor, crate::pairing::now_ms().unwrap());
        let confirmed = coordinator.confirm(
            &actor,
            &snapshot,
            true,
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        );
        if ended {
            assert!(observed.is_err(), "ended Session disclosed the candidate");
            assert!(confirmed.is_err(), "ended Session supplied confirmation");
            assert_eq!(f.counts(), (1, 1, 1, 1));
            assert_eq!(
                crate::management::receipt(
                    &f.store.lock().unwrap().connection,
                    &browser.client_id,
                    &id,
                    crate::pairing::now_ms().unwrap()
                )
                .unwrap()["state"],
                "unknown"
            );
        } else {
            assert!(observed.unwrap().unwrap().candidate.is_some());
            assert!(confirmed.unwrap().local_confirmation_known);
            let winner = coordinator
                .owner_observation(
                    &OwnerActor::local_interactive(),
                    crate::pairing::now_ms().unwrap(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                winner.decided_by,
                Some(crate::invitations::service::OwnerDecisionSource::Browser)
            );
            assert!(winner.committed.is_some());
            assert!(
                coordinator
                    .confirm(
                        &OwnerActor::local_interactive(),
                        &snapshot,
                        false,
                        &PairingPolicy::default(),
                        crate::pairing::now_ms().unwrap(),
                        &io
                    )
                    .is_err()
            );

            assert_eq!(f.counts(), (2, 2, 2, 2));
        }
    }
}
#[test]
fn browser_rejection_settles_the_original_without_creating_a_member() {
    for fail in [false, true] {
        let f = DurableFixture::new();
        let browser = f.browser();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (coordinator, _, snapshot) =
            f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
        let id = crate::store::uuid_v4().unwrap();
        let intent = [25; 32];
        f.store
            .lock()
            .unwrap()
            .adopt_invitation_confirmation(&browser, &id, &intent)
            .unwrap();
        if fail {
            f.store.lock().unwrap().connection.execute_batch(&format!(
            "CREATE TRIGGER fail_decline BEFORE UPDATE OF outcome ON management_receipts WHEN OLD.id='{id}' BEGIN SELECT RAISE(ABORT,'decline fault'); END;"
        )).unwrap();
        }
        let actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: false,
                enable_sending: false,
            },
        )
        .unwrap();
        let result = coordinator.confirm(
            &actor,
            &snapshot,
            false,
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        );
        assert_eq!(f.counts(), (1, 1, 1, 1));
        let original = crate::management::receipt(
            &f.store.lock().unwrap().connection,
            &browser.client_id,
            &id,
            crate::pairing::now_ms().unwrap(),
        )
        .unwrap();
        if fail {
            assert!(result.is_err());
            assert_eq!(original["state"], "unknown");
            assert_eq!(
                coordinator
                    .status(crate::pairing::now_ms().unwrap())
                    .unwrap()
                    .state,
                State::CandidateAwaitingOwner
            );
            f.store
                .lock()
                .unwrap()
                .connection
                .execute_batch("DROP TRIGGER fail_decline")
                .unwrap();
            assert!(
                coordinator
                    .confirm(
                        &OwnerActor::local_interactive(),
                        &snapshot,
                        true,
                        &PairingPolicy::default(),
                        crate::pairing::now_ms().unwrap(),
                        &io
                    )
                    .unwrap()
                    .local_confirmation_known
            );
        } else {
            assert_eq!(result.unwrap().state, State::Refused);
            assert_eq!(original["state"], "committed");
            assert_eq!(original["result"]["invitation"]["state"], "refused");
            assert!(!coordinator.retains_secret().unwrap());
            f.reopen();
            assert_eq!(
                crate::management::receipt(
                    &f.store.lock().unwrap().connection,
                    &browser.client_id,
                    &id,
                    crate::pairing::now_ms().unwrap()
                )
                .unwrap(),
                original
            );
        }
    }
}

#[test]
fn browser_decision_rechecks_designation_role_key_origin_expiry_and_revocation() {
    for change in [
        "designation",
        "role",
        "key",
        "origin",
        "expiry",
        "revocation",
    ] {
        let f = DurableFixture::new();
        let browser = f.browser();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (coordinator, _, snapshot) =
            f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
        let (_sessions, permit) = permit(&f, &browser, &snapshot);
        let id = permit.message.envelope().id.to_owned();
        let intent = crate::journal::intent(&permit.message).unwrap();
        f.store
            .lock()
            .unwrap()
            .adopt_invitation_confirmation(&browser, &id, &intent)
            .unwrap();
        let read_actor = OwnerActor::owner_browser(
            browser.client_id.clone(),
            browser.public_key,
            browser.revision,
        )
        .with_admission(&permit)
        .unwrap();
        let actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: false,
            },
        )
        .unwrap()
        .with_admission(&permit)
        .unwrap();
        {
            let store = f.store.lock().unwrap();
            match change {
                "designation" => {
                    store
                        .connection
                        .execute("DELETE FROM settings_designation", [])
                        .unwrap();
                }
                "role" => {
                    store
                        .connection
                        .execute("UPDATE members SET role='user'", [])
                        .unwrap();
                }
                "key" => {
                    let key = ed25519_dalek::SigningKey::from_bytes(&[25; 32])
                        .verifying_key()
                        .to_bytes();
                    store
                        .connection
                        .execute(
                            "UPDATE grants SET public_key=?1 WHERE client_id=?2",
                            rusqlite::params![key.as_slice(), browser.client_id],
                        )
                        .unwrap();
                }
                "origin" => {
                    store
                        .connection
                        .execute(
                            "UPDATE grants SET origin='http://127.0.0.1:49153' WHERE client_id=?1",
                            [&browser.client_id],
                        )
                        .unwrap();
                }
                "expiry" => {
                    store
                        .connection
                        .execute(
                            "UPDATE grants SET expires_at_ms=?1 WHERE client_id=?2",
                            rusqlite::params![
                                i64::try_from(crate::pairing::now_ms().unwrap() - 1).unwrap(),
                                browser.client_id
                            ],
                        )
                        .unwrap();
                }
                "revocation" => {
                    store
                        .connection
                        .execute(
                            "UPDATE grants SET disabled=1,revision=revision+1 WHERE client_id=?1",
                            [&browser.client_id],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
        }
        assert!(
            coordinator
                .owner_observation(&read_actor, crate::pairing::now_ms().unwrap())
                .is_err(),
            "{change} disclosed the candidate"
        );
        assert!(
            coordinator
                .confirm(
                    &actor,
                    &snapshot,
                    true,
                    &PairingPolicy::default(),
                    crate::pairing::now_ms().unwrap(),
                    &io
                )
                .is_err(),
            "{change} supplied authority"
        );
        assert_eq!(
            f.counts(),
            (1, 1, 1, 1),
            "{change} created partial authority"
        );
        assert_eq!(
            coordinator
                .status(crate::pairing::now_ms().unwrap())
                .unwrap()
                .state,
            State::CandidateAwaitingOwner
        );
        let saved: String = f
            .store
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT outcome FROM management_receipts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&saved).unwrap()["outcome"]["state"],
            "unknown"
        );
    }
}
