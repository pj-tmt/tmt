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
    let control = message_control(&sender, &_sender_lease);
    let before = wire.reads.lock().unwrap().len();
    assert_eq!(
        crate::control::squad_list(&sender.layout.directory).unwrap(),
        serde_json::to_value(Firestore::new(&sender, sender.as_ref()).list().unwrap()).unwrap()
    );
    assert_eq!(
        crate::control::squad_delivery(&sender.layout.directory, &remote, TARGET).unwrap()["state"],
        "unknown"
    );
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
    control.stop();
    assert!(
        !sender
            .layout
            .directory
            .join(crate::control::SOCKET)
            .exists()
    );
    sender.stop.store(true, Ordering::Release);
    owner.stop.store(true, Ordering::Release);
}

#[test]
fn recovered_original_without_core_message_state_is_unknown_not_delivered() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::squad_interface_tests::recovered_original_without_core_message_state_is_unknown_not_delivered",
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
    owner.store().unwrap().talk(&sender.machine, true).unwrap();
    let raw = prepared(&remote);
    let operation = crate::cloud_command::Request::read(&raw)
        .unwrap()
        .request_id;
    let interface = Firestore::new(&sender, &Empty);
    std::fs::write(core.root.join("lost"), "").unwrap();
    assert_eq!(
        interface.send(&raw, Some(&remote)).unwrap().state,
        crate::cloud_command::State::Unconfirmed
    );
    std::fs::remove_file(core.root.join("lost")).unwrap();
    owner.reconcile().unwrap();
    assert_eq!(core.sends(), 1);

    // Recovery proves the original request exists. The fixture deliberately omits
    // messageState, which cannot prove hand-over or even the Core queued state.
    assert_eq!(
        interface.delivery(&remote, &operation).unwrap().state,
        DeliveryPhase::Unknown
    );
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(core.root.join(format!("receipt-{operation}"))).unwrap(),
    )
    .unwrap();
    let request = receipt["items"][0]["requestId"].as_str().unwrap();
    assert!(
        core.calls()
            .iter()
            .any(|call| call["operation"] == "requests.show"
                && call["input"]["requestId"] == request)
    );
    assert_eq!(core.sends(), 1, "unknown evidence never repeats dispatch");
    for (state, expected) in [
        ("sent", DeliveryPhase::Queued),
        ("delivered", DeliveryPhase::Delivered),
    ] {
        core.message_state(&json!({"state":state,"reason":{"code":"fixture","text":"PRIVATE-REASON"},"latestMs":null,"remainingMs":null,"canSendNow":false}));
        let observed = interface.delivery(&remote, &operation).unwrap();
        assert_eq!(observed.operation_id, operation);
        assert_eq!(observed.state, expected);
        assert!(
            !serde_json::to_string(&observed)
                .unwrap()
                .contains("PRIVATE")
        );
        assert_eq!(core.sends(), 1, "authoritative observation is read only");
    }
    sender.stop.store(true, Ordering::Release);
    owner.stop.store(true, Ordering::Release);
}

#[test]
fn native_read_only_capability_gate_and_summary_cleanup_share_the_existing_channel() {
    crate::private_test_case::private_case(
        "cloud_pairing::service::tests::squad_interface_tests::native_read_only_capability_gate_and_summary_cleanup_share_the_existing_channel",
        |root| {
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
            let (owner, _owner_lease) = service(&root.join("owner"), &core, make_ports());
            let (sender, _sender_lease) =
                service_with_summary_source(&root.join("sender"), &core, make_ports());
            assert!(owner.layout.directory.starts_with(&root));
            assert!(sender.layout.directory.starts_with(&root));
            pair_with_reciprocal_choice(&owner, &sender, false);
            wire.route(&owner);
            wire.route(&sender);
            let source_grant = sender
                .store()
                .unwrap()
                .grant(&owner.machine)
                .unwrap()
                .unwrap();
            assert!(!source_grant.permits_scope("talk"));
            let summaries = || {
                wire.docs
                    .lock()
                    .unwrap()
                    .iter()
                    .filter_map(|(name, doc)| {
                        if !name.contains("/inbox/") {
                            return None;
                        }
                        let bytes = STANDARD
                            .decode(doc["fields"]["carrier"]["bytesValue"].as_str()?)
                            .ok()?;
                        let value: Value = serde_json::from_slice(&bytes).ok()?;
                        (value["header"]["operation"] == "squads.summary").then(|| name.clone())
                    })
                    .collect::<Vec<_>>()
            };
            assert!(summaries().is_empty());
            sender.publish_summaries().unwrap();
            assert_eq!(
                sender
                    .summary_support
                    .lock()
                    .unwrap()
                    .get(&owner.machine)
                    .unwrap()
                    .cached(&source_grant, Instant::now()),
                Some(Ok(true))
            );
            assert!(summaries().is_empty(), "capability lookup sends no summary");
            let reads = wire.reads.lock().unwrap().len();
            sender.publish_summaries().unwrap();
            assert_eq!(
                wire.reads.lock().unwrap().len(),
                reads,
                "positive capability is cached and content coalesces"
            );
            sender.summary_schedules.lock().unwrap().insert(
                owner.machine.clone(),
                crate::squads::publisher::Schedule::restore(
                    None,
                    Some(crate::limits::SQUAD_SUMMARY_COALESCE_MS),
                    0,
                    0,
                ),
            );
            sender.publish_summaries().unwrap();
            let names = summaries();
            assert_eq!(names.len(), 1);
            let id = names[0].rsplit('/').next().unwrap();
            assert!(
                !wire
                    .docs
                    .lock()
                    .unwrap()
                    .keys()
                    .any(|name| name.ends_with(&format!("/responses/{id}"))),
                "summary has no provider response write"
            );
            let rows = crate::squads::SummaryReader::read(owner.as_ref()).unwrap();
            assert_eq!(rows.len(), 1);
            assert!(
                crate::squads::Summary::read(&rows[0].summary)
                    .unwrap()
                    .squads()
                    .is_none(),
                "failed public Ops read becomes unavailable, never fabricated rows"
            );
            let before = wire.commits.lock().unwrap().len();
            owner.reconcile().unwrap();
            assert!(!wire.docs.lock().unwrap().contains_key(&names[0]));
            let deletes = wire.commits.lock().unwrap()[before..]
                .iter()
                .filter(|request| {
                    request["writes"].as_array().is_some_and(|writes| {
                        writes.iter().any(|write| write["delete"] == names[0])
                    })
                })
                .count();
            assert_eq!(
                deletes, 1,
                "summary cleanup is exactly one guarded provider delete"
            );
            assert_eq!(
                crate::squads::SummaryReader::read(owner.as_ref())
                    .unwrap()
                    .len(),
                1,
                "verified cache survives provider cleanup"
            );
            let outbox:i64=sender.store().unwrap().connection.query_row("SELECT COUNT(*) FROM cloud_transport WHERE kind='outbox' AND summary_metadata=0",[],|r|r.get(0)).unwrap();
            assert_eq!(
                outbox, 0,
                "completed capability observations do not form a second history"
            );
            owner.store().unwrap().connection.execute("UPDATE grants SET scopes='results.read',revision=revision+1 WHERE client_id=?1",[&sender.machine]).unwrap();
            sender
                .summary_support
                .lock()
                .unwrap()
                .remove(&owner.machine);
            sender.publish_summaries().unwrap();
            assert_eq!(
                sender
                    .summary_support
                    .lock()
                    .unwrap()
                    .get(&owner.machine)
                    .unwrap()
                    .cached(&source_grant, Instant::now()),
                Some(Ok(false))
            );
            let before = wire.commits.lock().unwrap().len();
            sender.publish_summaries().unwrap();
            assert_eq!(
                wire.commits.lock().unwrap().len(),
                before,
                "unsupported native peer gets no periodic metadata"
            );
            assert_eq!(
                core.sends(),
                0,
                "capability and summary work never dispatch agent work"
            );
            let strong = Arc::strong_count(&sender);
            let worker = sender.worker().unwrap();
            assert!(worker.publisher_join.is_some());
            drop(worker);
            assert!(sender.stop.load(Ordering::Acquire));
            assert_eq!(
                Arc::strong_count(&sender),
                strong,
                "both joined workers release the Service before lease cleanup"
            );
            owner.stop.store(true, Ordering::Release);
        },
    );
}
