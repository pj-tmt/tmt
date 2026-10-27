//! Two actual consumers of the same typed port: a key-addressed mailbox and an
//! append-only log with delayed visibility. Neither manufactures owner consent.
mod adoption;
mod backends;
mod browser_outcomes;
mod claims;
mod confirmation;
mod durable;
mod durable_fixture;
mod durable_refusals;
mod faults;
mod firestore;
mod incarnation;
mod lifecycle;
mod policies;
mod reinvite;
mod submissions;
mod transactions;
use super::{
    port::*,
    service::{Coordinator, State, gate},
    *,
};
use crate::{objects::IoBudget, pairing::PairingPolicy};
use backends::{AppendLog, Mailbox, ScenarioBackend};
use confirmation::{AdmissionRace, Sink};
use ed25519_dalek::Signer;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Key = (String, u8);
fn key(exchange: &InvitationExchange) -> Key {
    (exchange.context.offer.clone(), exchange.part as u8)
}
fn bounded(io: &IoBudget<'_>) -> Result<(), BackendFailure> {
    io.check().map_err(|_| BackendFailure::Cancelled)
}
fn scenarios<B: ScenarioBackend>(backend: B, mut context: Context, sink: Sink) {
    context.binding = backend.binding();
    let (_, candidate, signing) = super::tests::fixture();
    let cancelled = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &cancelled,
    };
    let owner = OwnerActor::local_interactive();
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    assert_eq!(
        coordinator.create(&owner, context, 1, &io).unwrap().state,
        State::Open
    );
    let delivery = coordinator.deliver(&owner, 2).unwrap();
    let peer = backend.peer(candidate.principal.clone());
    let mut attempt = delivery
        .with_code(|code| {
            super::submission::SubmissionAttempt::new(
                peer,
                delivery.context.clone(),
                candidate.clone(),
                code,
                &signing,
                2,
            )
        })
        .unwrap();
    assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
    assert!(attempt.observe_receipt(&io).unwrap().is_none());
    assert_eq!(
        coordinator.observe(2, &io).unwrap().state,
        State::CandidateAwaitingOwner
    );
    assert_eq!(
        sink.ledger.lock().unwrap().grants,
        0,
        "valid code and possession grant nothing"
    );
    let pending = coordinator.candidate(2).unwrap().unwrap();
    assert!(
        coordinator
            .confirm(
                &OwnerActor::automated(),
                &pending,
                true,
                &PairingPolicy::default(),
                3,
                &io
            )
            .is_err()
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    backend.lose_next_receipt();
    let status = coordinator
        .confirm(&owner, &pending, true, &PairingPolicy::default(), 3, &io)
        .unwrap();
    assert_eq!(status.state, State::ProviderOutcomeUnconfirmed);
    assert!(status.local_confirmation_known);
    assert!(!coordinator.retains_secret().unwrap());
    assert!(
        coordinator.deliver(&owner, 3).is_err(),
        "the confirmed offer retains no code"
    );
    assert!(
        coordinator
            .confirm(&owner, &pending, true, &PairingPolicy::default(), 3, &io)
            .is_err()
    );
    for now in [4, 5, 6] {
        coordinator.observe(now, &io).unwrap();
    }
    assert_eq!(coordinator.status(6).unwrap().state, State::Confirmed);
    assert_eq!(
        backend.writes(Part::Receipt),
        1,
        "lost writes are observed without another publication"
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);
    assert_eq!(sink.ledger.lock().unwrap().confirms, 1);
    let original = sink.original(&pending).unwrap().unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&original.receipt).unwrap();
    assert_eq!(receipt["grant"]["mode"], "direct");
    assert_eq!(receipt["grant"]["agents"], "all");
    assert_eq!(
        receipt["grant"]["scopes"],
        serde_json::json!(crate::store::DEFAULT_SCOPES)
    );
    delivery.with_code(|code| {
        let key = crypto::response_key(code, &enrollment(&delivery.context, &candidate).unwrap())
            .unwrap();
        crypto::verify_server_proof(&key, &original.receipt, &original.proof).unwrap();
    });
    assert!(attempt.observe_receipt(&io).unwrap().as_ref() == Some(&original.receipt));
    drop(coordinator);
    let restarted = Coordinator::new(backend, sink.clone());
    assert!(restarted.status(7).unwrap().offer.is_none());
    assert!(sink.original(&pending).unwrap().as_ref() == Some(&original));
    assert_eq!(
        sink.ledger.lock().unwrap().grants,
        1,
        "the fake durable sink survives the memory owner"
    );
}

#[test]
fn keyed_mailbox_consumes_the_shared_coordinator_without_auto_confirmation() {
    let (context, _, _) = super::tests::fixture();
    scenarios(
        Mailbox::new(context.binding.clone()),
        context,
        Sink {
            routes: true,
            ..Sink::default()
        },
    );
}
#[test]
fn delayed_append_log_consumes_the_same_coordinator_without_a_route_write() {
    let (mut context, _, _) = super::tests::fixture();
    context.binding.backend = "append-log".into();
    scenarios(
        AppendLog::new(context.binding.clone()),
        context,
        Sink::default(),
    );
}

fn signed_publication(
    delivery: &super::service::Delivery,
    candidate: &Candidate,
    signing: &ed25519_dalek::SigningKey,
) -> Publication {
    let submitted = delivery.with_code(|code| {
        let transcript = enrollment(&delivery.context, candidate).unwrap();
        let mac = crypto::enrollment_mac(code, &transcript);
        Submission {
            candidate: candidate.clone(),
            gate: gate(&delivery.context, code).unwrap(),
            signature: signing
                .sign(&canonical::possession(&transcript, &mac).unwrap())
                .to_bytes(),
            mac,
        }
    });
    Publication {
        exchange: InvitationExchange {
            context: delivery.context.clone(),
            part: Part::Candidate,
        },
        payload: Payload::Candidate(submitted),
    }
}

fn pending<B: ScenarioBackend>(
    backend: B,
    sink: Sink,
    io: &IoBudget<'_>,
) -> (Coordinator<B, Sink>, super::service::Delivery, Snapshot) {
    let (mut context, candidate, signing) = super::tests::fixture();
    context.binding = backend.binding();
    let coordinator = Coordinator::new(backend.clone(), sink);
    coordinator
        .create(&OwnerActor::local_interactive(), context, 1, io)
        .unwrap();
    let delivery = coordinator
        .deliver(&OwnerActor::local_interactive(), 2)
        .unwrap();
    backend
        .peer(candidate.principal.clone())
        .publish(&signed_publication(&delivery, &candidate, &signing), io)
        .unwrap();
    assert_eq!(
        coordinator.observe(2, io).unwrap().state,
        State::CandidateAwaitingOwner
    );
    let snapshot = coordinator.candidate(2).unwrap().unwrap();
    (coordinator, delivery, snapshot)
}

fn negative_scenarios<B: ScenarioBackend>(make: impl Fn() -> B) {
    // Each case gets a separate backend and sink: no leftover receipt can make
    // an unsuccessful ceremony look like a successful one.
    for case in 0..7 {
        let backend = make();
        let sink = Sink::default();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let (coordinator, delivery, snapshot) = pending(backend.clone(), sink.clone(), &io);
        let owner = OwnerActor::local_interactive();
        assert!(coordinator.retains_secret().unwrap());
        match case {
            0 => assert_eq!(
                coordinator
                    .confirm(&owner, &snapshot, false, &PairingPolicy::default(), 3, &io)
                    .unwrap()
                    .state,
                State::Refused
            ),
            1 => assert_eq!(
                coordinator
                    .cancel(&owner, &delivery.context.offer, 3, &io)
                    .unwrap()
                    .state,
                State::Cancelled
            ),
            2 => {
                assert!(
                    coordinator
                        .confirm(
                            &owner,
                            &snapshot,
                            true,
                            &PairingPolicy::default(),
                            delivery.context.expires_at_ms,
                            &io
                        )
                        .is_err()
                );
                assert_eq!(
                    coordinator
                        .status(delivery.context.expires_at_ms)
                        .unwrap()
                        .state,
                    State::Expired
                );
            }
            3 => {
                stopped.store(true, Ordering::Release);
                assert!(
                    coordinator
                        .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
                        .is_err()
                );
                // Disconnect/stop belongs to the local owner; it clears the
                // offer even when best-effort provider withdrawal cannot run.
                assert_eq!(
                    coordinator
                        .cancel(&owner, &delivery.context.offer, 3, &io)
                        .unwrap()
                        .state,
                    State::Cancelled
                );
            }
            4 => {
                sink.ledger.lock().unwrap().revoked = true;
                assert!(
                    coordinator
                        .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
                        .is_err()
                );
                assert!(!coordinator.status(3).unwrap().local_confirmation_known);
                coordinator
                    .disconnect(&delivery.context.offer, 3, &io)
                    .unwrap();
            }
            5 => {
                let browser = OwnerActor::owner_browser(
                    snapshot.candidate.machine.clone(),
                    snapshot.candidate.public_key,
                    1,
                );
                assert!(
                    coordinator
                        .confirm(&browser, &snapshot, true, &PairingPolicy::default(), 3, &io)
                        .is_err()
                );
                coordinator
                    .cancel(&owner, &delivery.context.offer, 3, &io)
                    .unwrap();
            }
            _ => {
                let mut stale = snapshot.clone();
                stale.digest[0] ^= 1;
                assert!(
                    coordinator
                        .confirm(&owner, &stale, true, &PairingPolicy::default(), 3, &io)
                        .is_err()
                );
                coordinator
                    .cancel(&owner, &delivery.context.offer, 3, &io)
                    .unwrap();
            }
        }
        assert!(
            !coordinator.retains_secret().unwrap(),
            "case {case} retained secret bytes"
        );
        assert!(
            coordinator.deliver(&owner, 3).is_err(),
            "case {case} retained a deliverable code"
        );
        assert_eq!(
            sink.ledger.lock().unwrap().grants,
            0,
            "case {case} created authority"
        );
        assert_eq!(
            sink.ledger.lock().unwrap().confirms,
            0,
            "case {case} reached the writer"
        );
        assert_eq!(
            backend.writes(Part::Receipt),
            0,
            "case {case} published a receipt"
        );
    }

    let backend = make();
    let sink = Sink::default();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let (coordinator, delivery, frozen) = pending(backend.clone(), sink.clone(), &io);
    let (_, mut second, signing) = super::tests::fixture();
    second.machine = "44444444-4444-4444-8444-444444444444".into();
    second.principal = b"another authenticated principal".to_vec();
    // The mailbox refuses an occupied slot; the append log can retain both.
    // Neither backend may replace the candidate the owner is comparing.
    let _ = backend
        .peer(second.principal.clone())
        .publish(&signed_publication(&delivery, &second, &signing), &io);
    coordinator.observe(3, &io).unwrap();
    assert!(coordinator.candidate(3).unwrap().as_ref() == Some(&frozen));
    coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &frozen,
            true,
            &PairingPolicy::default(),
            4,
            &io,
        )
        .unwrap();
    let original = sink.original(&frozen).unwrap().unwrap();
    assert!(original.snapshot.candidate == frozen.candidate);
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);
    assert_eq!(backend.writes(Part::Receipt), 1);

    // Two real threads compete at the coordinator boundary. A duplicate
    // confirmation must lose before entering the confirmation sink.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, _, frozen) = pending(backend.clone(), sink.clone(), &io);
    sink.ledger.lock().unwrap().owner_browser = Some((
        "55555555-5555-4555-8555-555555555555".into(),
        frozen.context.owner_key,
        1,
    ));
    let coordinator = Arc::new(coordinator);
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for index in 0..2 {
            let coordinator = coordinator.clone();
            let barrier = barrier.clone();
            let frozen = frozen.clone();
            let io = &io;
            jobs.push(scope.spawn(move || {
                barrier.wait();
                let actor = if index == 0 {
                    OwnerActor::local_interactive()
                } else {
                    OwnerActor::owner_browser(
                        "55555555-5555-4555-8555-555555555555".into(),
                        frozen.context.owner_key,
                        1,
                    )
                };
                coordinator
                    .confirm(&actor, &frozen, true, &PairingPolicy::default(), 3, io)
                    .is_ok()
            }));
        }
        barrier.wait();
        assert_eq!(
            jobs.into_iter()
                .map(|job| usize::from(job.join().unwrap()))
                .sum::<usize>(),
            1
        );
    });
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);
    assert_eq!(sink.ledger.lock().unwrap().confirms, 1);
    assert_eq!(backend.writes(Part::Receipt), 1);
}

#[test]
fn mailbox_refusals_frozen_candidates_and_competing_confirmations() {
    let (context, _, _) = super::tests::fixture();
    negative_scenarios(|| Mailbox::new(context.binding.clone()));
}

#[test]
fn append_log_refusals_frozen_candidates_and_competing_confirmations() {
    let (mut context, _, _) = super::tests::fixture();
    context.binding.backend = "append-log".into();
    negative_scenarios(|| AppendLog::new(context.binding.clone()));
}
