//! Actual Service callers over the shared injected HTTP fixture and real Store.
//! This tests lost answers before spending a separate emulator qualification.
use super::*;

mod preparation {
    use super::*;

    struct PausedHttp {
        wire: Arc<Wire>,
        operation: Operation,
        fail: bool,
        armed: AtomicBool,
        reached: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        calls: Arc<AtomicUsize>,
    }
    impl Http for PausedHttp {
        fn send(
            &self,
            request: &Request,
            deadline: u64,
            cancel: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            if request.operation() == self.operation && self.armed.swap(false, Ordering::AcqRel) {
                self.reached.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                if self.fail {
                    return Ok(Response {
                        status: 403,
                        retry_after_ms: None,
                        body: br#"{"error":{"status":"PERMISSION_DENIED"}}"#.to_vec(),
                    });
                }
            }
            self.wire.send(request, deadline, cancel)
        }
    }
    pub(super) struct Pause {
        pub reached: mpsc::Receiver<()>,
        pub calls: Arc<AtomicUsize>,
        release: Option<mpsc::SyncSender<()>>,
    }
    impl Pause {
        pub fn release(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
        }
    }
    impl Drop for Pause {
        fn drop(&mut self) {
            self.release();
        }
    }
    pub(super) fn block(
        wire: Arc<Wire>,
        operation: Operation,
        fail: bool,
    ) -> (ProjectPorts, Pause) {
        let (reached_tx, reached) = mpsc::sync_channel(1);
        let (release, release_rx) = mpsc::sync_channel(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let ports = Arc::new(Ports {
            wire: wire.clone(),
            clock: SystemClock::default(),
        });
        let http = Arc::new(PausedHttp {
            wire,
            operation,
            fail,
            armed: AtomicBool::new(true),
            reached: reached_tx,
            release: Mutex::new(release_rx),
            calls: calls.clone(),
        });
        (
            ProjectPorts {
                http,
                clock: ports.clone(),
                jitter: ports,
            },
            Pause {
                reached,
                calls,
                release: Some(release),
            },
        )
    }
}

#[test]
fn preparing_an_invite_keeps_status_idle_and_competing_admission_responsive() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::invitation_cutover_tests::preparing_an_invite_keeps_status_idle_and_competing_admission_responsive",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    for (index, (operation, fail)) in [
        (Operation::SignUp, false),
        (Operation::Read, false),
        (Operation::Read, true),
    ]
    .into_iter()
    .enumerate()
    {
        let wire = Arc::new(Wire::new());
        let (ports, pause) = preparation::block(wire, operation, fail);
        let (owner, lease) = service(&core.root.join(format!("owner-{index}")), &core, ports);
        let (observed, before, after, result) = std::thread::scope(|scope| {
            let mut pause = pause;
            let inviter = scope.spawn(|| {
                owner.invite_selected(PROJECT, config(), |event| {
                    if event["event"] == "invite" {
                        return Err(RemoteError::new(
                            "FIXTURE_STOP",
                            "Stop after code presentation.",
                        ));
                    }
                    Ok(None)
                })
            });
            pause.reached.recv_timeout(Duration::from_secs(10)).unwrap();
            let before = pause.calls.load(Ordering::Acquire);
            let (tx, rx) = mpsc::sync_channel(1);
            let owner_ref = &owner;
            let reader = scope.spawn(move || {
                let no_offer = owner_ref.invitation_status().unwrap().is_none();
                let idle = owner_ref.idle();
                let busy = owner_ref
                    .invite_selected(PROJECT, config(), |_| {
                        Err(RemoteError::new(
                            "FIXTURE_STOP",
                            "A competing invite must not reach presentation.",
                        ))
                    })
                    .unwrap_err()
                    .code;
                tx.send((no_offer, idle, busy)).unwrap();
            });
            let observed = rx.recv_timeout(Duration::from_secs(2));
            let after = pause.calls.load(Ordering::Acquire);
            pause.release();
            let result = inviter.join().unwrap();
            reader.join().unwrap();
            (observed, before, after, result)
        });
        // Release and join first, including against the original blocking code.
        // A timeout is a regression, never a reason to leave a fixture running.
        assert_eq!(
            observed
                .expect("status, idle and busy admission must finish while provider I/O is paused"),
            (true, false, "REMOTE_PAIRING_BUSY".into())
        );
        assert_eq!(
            after, before,
            "observations and competing admission make no provider call"
        );
        assert_eq!(
            result.unwrap_err().code,
            if fail {
                "REMOTE_CLOUD_ACCESS_DENIED"
            } else {
                "FIXTURE_STOP"
            }
        );
        assert!(owner.idle());
        let status = owner.invitation_status().unwrap();
        if fail {
            assert!(
                status.is_none(),
                "failed preparation releases the unpublished slot"
            );
        } else {
            let status = status.unwrap();
            assert_eq!(status.state, crate::invitations::service::State::Cancelled);
            assert_eq!(
                status.cleanup,
                crate::invitations::service::ProviderCleanup::Acknowledged
            );
            assert!(!status.local_confirmation_known);
        }
        assert_eq!(
            owner
                .invite_selected(PROJECT, config(), |_| {
                    Err(RemoteError::new(
                        "FIXTURE_STOP",
                        "Reservation was released.",
                    ))
                })
                .unwrap_err()
                .code,
            "FIXTURE_STOP"
        );
        assert!(owner.idle());
        assert!(owner.store().unwrap().grants().unwrap().is_empty());
        assert_eq!(core.sends(), 0);
        drop(owner);
        drop(lease);
    }
}

#[test]
fn v2_lost_receipt_answer_preserves_confirmation_and_the_worker_observes_one_original() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::invitation_cutover_tests::v2_lost_receipt_answer_preserves_confirmation_and_the_worker_observes_one_original",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let make_ports = || ProjectPorts {
        http: ports.clone(),
        clock: ports.clone(),
        jitter: ports.clone(),
    };
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, make_ports());
    let (peer, peer_lease) = service(&core.root.join("peer"), &core, make_ports());
    wire.receipt_answers
        .lose_next
        .store(true, Ordering::Release);
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        let _cancel = Cancel(vec![owner.stop.clone(), peer.stop.clone()]);
        let inviter = scope.spawn(|| {
            owner.invite_selected(PROJECT, config(), |event| {
                if event["event"] == "waiting" {
                    owner.flush_routes()?;
                }
                if event["event"] == "invite" {
                    tx.send(event["code"].as_str().unwrap().to_owned()).unwrap();
                }
                Ok((event["event"] == "candidate").then_some((true, false)))
            })
        });
        let code = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        peer.join(&code, |_| Ok(Some(false))).unwrap();
        assert_eq!(
            inviter.join().unwrap().unwrap_err().code,
            "REMOTE_CLOUD_PROVIDER_UNAVAILABLE"
        );
    });
    let before = wire.receipt_answers.commits.lock().unwrap().clone();
    assert_eq!(before.values().sum::<usize>(), 1);
    let status = owner.invitation_status().unwrap().unwrap();
    assert!(status.local_confirmation_known);
    assert_eq!(
        status.state,
        crate::invitations::service::State::ProviderOutcomeUnconfirmed
    );
    owner.reconcile().unwrap();
    assert_eq!(
        owner.invitation_status().unwrap().unwrap().state,
        crate::invitations::service::State::Confirmed
    );
    assert_eq!(*wire.receipt_answers.commits.lock().unwrap(), before);
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    assert_eq!(peer.store().unwrap().grants().unwrap().len(), 1);
    assert_eq!(core.sends(), 0);
    drop(peer);
    drop(owner);
    drop(peer_lease);
    drop(owner_lease);
}
