use super::*;
use crate::squads::{DeliveryPhase, Firestore, Interface, Record, SummaryReader};
struct FailedSource;
impl SummaryReader for FailedSource {
    fn read(&self) -> Result<Vec<Record>, RemoteError> {
        Err(RemoteError::new(
            "REMOTE_SUMMARY_UNAVAILABLE",
            "Fixture source failure",
        ))
    }
}
struct Empty;
impl SummaryReader for Empty {
    fn read(&self) -> Result<Vec<Record>, RemoteError> {
        Ok(vec![])
    }
}
#[test]
fn directory_adapter_reads_original_delivery_and_final_without_dispatch_replay() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::squad_interface_tests::directory_adapter_reads_original_delivery_and_final_without_dispatch_replay",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    std::fs::write(core.root.join("multi"), "").unwrap();
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
    let (owner, _owner_lease) = service(&core.root.join("owner"), &core, make_ports());
    let (sender, _sender_lease) = service(&core.root.join("sender"), &core, make_ports());
    let events = pair_with_reciprocal_choice(&owner, &sender, false);
    let remote = events
        .iter()
        .find(|event| event["event"] == "connected")
        .unwrap()["remote"]
        .as_str()
        .unwrap()
        .to_owned();
    wire.route(&owner);
    wire.route(&sender);
    assert!(
        Firestore::new(&sender, &FailedSource).list().is_err(),
        "failed source is not an empty list"
    );
    let interface = Firestore::new(&sender, &Empty);
    let before = wire.reads.lock().unwrap().len();
    assert_eq!(
        interface.delivery(&remote, TARGET).unwrap().state,
        DeliveryPhase::Unknown
    );
    assert_eq!(
        wire.reads.lock().unwrap().len(),
        before,
        "unknown original does no provider work"
    );
    let raw = prepared(&remote);
    let operation = crate::cloud_command::Request::read(&raw)
        .unwrap()
        .request_id;
    assert_eq!(
        interface.send(&raw, Some(&remote)).unwrap().state,
        crate::cloud_command::State::Sent
    );
    assert_eq!(core.sends(), 1);
    // Exact provider evidence can show the original carrier queued without another dispatch.
    let response_name = wire
        .docs
        .lock()
        .unwrap()
        .keys()
        .find(|name| name.ends_with(&format!("/responses/{operation}")))
        .unwrap()
        .clone();
    let response = wire.docs.lock().unwrap().remove(&response_name).unwrap();
    assert_eq!(
        interface.delivery(&remote, &operation).unwrap().state,
        DeliveryPhase::Queued
    );
    assert_eq!(core.sends(), 1);
    wire.docs.lock().unwrap().insert(response_name, response);
    let carrier_name = wire
        .docs
        .lock()
        .unwrap()
        .keys()
        .find(|name| name.ends_with(&format!("/inbox/{operation}")))
        .unwrap()
        .clone();
    let carrier = wire.docs.lock().unwrap().remove(&carrier_name).unwrap();
    assert_eq!(
        interface.delivery(&remote, &operation).unwrap_err().code,
        "REMOTE_CLOUD_UNCONFIRMED"
    );
    assert_eq!(
        core.sends(),
        1,
        "missing carrier is never recreated by observation"
    );
    wire.docs.lock().unwrap().insert(carrier_name, carrier);
    for (phase, wanted) in [
        ("sent", DeliveryPhase::Queued),
        ("batched", DeliveryPhase::Held),
        ("delivered", DeliveryPhase::Delivered),
        ("working", DeliveryPhase::Delivered),
        ("unconfirmed", DeliveryPhase::Unconfirmed),
        ("not_sent", DeliveryPhase::Failed),
        ("future_state", DeliveryPhase::Unknown),
    ] {
        core.message_state(&json!({"state":phase,"reason":{"code":"fixture","text":"PRIVATE"},"latestMs":null,"remainingMs":null,"canSendNow":false}));
        let observed = interface.delivery(&remote, &operation).unwrap();
        assert_eq!(observed.state, wanted, "{phase}");
        assert!(
            !serde_json::to_string(&observed)
                .unwrap()
                .contains("PRIVATE")
        );
        assert_eq!(core.sends(), 1, "observation never repeats dispatch.create");
    }
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(core.root.join(format!("receipt-{operation}"))).unwrap(),
    )
    .unwrap();
    let request = receipt["items"][0]["requestId"].as_str().unwrap();
    std::fs::write(
        core.root.join(format!("final-{request}")),
        json!({"status":"retained","response":"PRIVATE-FINAL"}).to_string(),
    )
    .unwrap();
    core.message_state(&json!({"state":"answered","reason":{"code":"answered","text":"PRIVATE"},"latestMs":null,"remainingMs":null,"canSendNow":false}));
    let observed = interface.delivery(&remote, &operation).unwrap();
    assert_eq!(observed.state, DeliveryPhase::Answered);
    assert!(
        !serde_json::to_string(&observed)
            .unwrap()
            .contains("PRIVATE")
    );
    assert_eq!(core.sends(), 1);
    std::fs::write(
        core.root.join(format!("final-{request}")),
        json!({"status":"expired"}).to_string(),
    )
    .unwrap();
    assert_eq!(
        interface.delivery(&remote, &operation).unwrap().state,
        DeliveryPhase::Answered,
        "answer identity survives final-body expiry"
    );
    assert_eq!(core.sends(), 1);
    let query = sender
        .store()
        .unwrap()
        .connection
        .query_row(
            "SELECT id FROM cloud_transport WHERE kind='outbox' AND binding IS NULL LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(
        interface.delivery(&remote, &query).unwrap().state,
        DeliveryPhase::Unknown,
        "a read-only control is not a send"
    );
    owner.store().unwrap().revoke(&sender.machine).unwrap();
    owner.flush_routes().unwrap();
    assert!(
        interface.delivery(&remote, &operation).is_err(),
        "revocation refuses fresh observation"
    );
    assert_eq!(core.sends(), 1, "revocation cannot replay the original");
    sender.stop.store(true, Ordering::Release);
    owner.stop.store(true, Ordering::Release);
}
