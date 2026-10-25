//! Actual Service callers over the shared injected HTTP fixture and real Store.
//! This tests lost answers before spending a separate emulator qualification.
use super::*;

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
