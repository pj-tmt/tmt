//! Signed application exchanges against a deterministic public-process fixture.
//! E owns the separately built real core/private-tmux/mock-agent acceptance.
use super::OwnerDoor;
use super::core_fixture::Core;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    sync::{Arc, atomic::AtomicBool},
};
use tmt_remote::{
    approval::Approval,
    control::{self, Control},
    devices::Devices,
    operations::Operations,
    pairing::{Pairing, Timing},
    store::uuid_v4,
    transport::{LoopbackTransport, Transport},
};
fn input(id: &str, recipient: &str, message: &str) -> Value {
    json!({"version":1,"operation":"dispatch.create","originator":"anonymous","input":{"operationId":id,"recipientIds":[recipient],"message":message,"kind":"request"}})
}
fn wire(
    owner: &OwnerDoor,
    session: &str,
    sequence: usize,
    id: &str,
    recipient: &str,
    message: &str,
) -> Value {
    let mut wire = owner.wire(
        session,
        &sequence.to_string(),
        "dispatch.create",
        input(id, recipient, message).to_string().as_bytes(),
    );
    wire["id"] = json!(id);
    owner.resign(&mut wire);
    wire
}
fn append(owner: &OwnerDoor, operations: Arc<Operations>, wire: &Value) -> Value {
    let transport =
        LoopbackTransport::new(Arc::clone(&owner.sessions), 65536).with_operations(operations);
    let bytes = transport
        .append(None, &serde_json::to_vec(wire).unwrap())
        .unwrap();
    owner.verify_reply(&serde_json::from_slice(&bytes).unwrap(), wire)
}
fn control(owner: &OwnerDoor, operations: Arc<Operations>) -> Control {
    let pairing = Arc::new(Pairing::new(
        owner.machine.clone(),
        owner.window.clone(),
        owner.public,
        "http://127.0.0.1:32100".into(),
        Arc::clone(&owner.store),
        Timing::CONTRACT,
    ));
    let devices = Arc::new(Devices::new(
        Arc::clone(&owner.store),
        Some(Arc::clone(&owner.sessions)),
    ));
    let approval = Arc::new(Approval::new(
        Arc::clone(&owner.store),
        Arc::clone(&owner.sessions),
        operations,
    ));
    Control::start(
        &owner._serving,
        pairing,
        devices,
        control::Door {
            origin: "http://127.0.0.1:32100".into(),
            prefix: "/r/test".into(),
        },
        Some(approval),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap()
}
fn line(reader: &mut BufReader<std::os::unix::net::UnixStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
#[test]
fn direct_exact_retry_keeps_one_core_acceptance_and_frozen_provenance() {
    let owner = OwnerDoor::new();
    let core = Core::new();
    let operations = core.operations();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let recipient = uuid_v4().unwrap();
    let sent = wire(
        &owner,
        &session,
        1,
        &id,
        &recipient,
        "exact ! bytes\nsecond line",
    );
    let accepted = append(&owner, Arc::clone(&operations), &sent);
    assert_eq!(
        accepted,
        json!({"state":"accepted","operationId":id,"requestId":"req_11111111-1111-4111-8111-111111111111"})
    );
    let retry = wire(
        &owner,
        &session,
        2,
        &id,
        &recipient,
        "exact ! bytes\nsecond line",
    );
    assert_eq!(append(&owner, Arc::clone(&operations), &retry), accepted);
    assert_eq!(core.sends(), 1);
    let calls = core.calls();
    let create = calls
        .iter()
        .find(|call| call["operation"] == "dispatch.create")
        .unwrap();
    assert_eq!(create["originator"], "anonymous");
    assert_eq!(
        create["input"]["message"],
        "[remote: Test device]\nexact ! bytes\nsecond line"
    );
    let owned = owner
        .store
        .lock()
        .unwrap()
        .owned(&owner.grant, &id, tmt_remote::pairing::now_ms().unwrap())
        .unwrap();
    assert_eq!(owned.phase, "accepted");
    assert!(owned.frozen.is_none());
    assert!(owned.references.contains(&recipient));
    assert!(
        owned
            .references
            .contains(&"11111111-1111-4111-8111-111111111111".into())
    );
    let changed = wire(&owner, &session, 3, &id, &recipient, "changed");
    assert_eq!(
        append(&owner, operations, &changed)["error"]["code"],
        "REMOTE_INTENT_CONFLICT"
    );
    assert_eq!(core.sends(), 1);
}
#[test]
fn held_operation_waits_for_exact_local_confirmation_and_cannot_be_remotely_approved() {
    let owner = OwnerDoor::with_policy(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|s| (*s).into())
            .collect(),
        "hold",
    );
    let core = Core::new();
    let operations = core.operations();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let recipient = uuid_v4().unwrap();
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(&owner, &session, 1, &id, &recipient, "frozen")
        ),
        json!({"state":"held","operationId":id})
    );
    assert!(core.calls().is_empty());
    let denied = owner.wire(
        &session,
        "2",
        "approve",
        json!({"operationId":id}).to_string().as_bytes(),
    );
    assert_eq!(
        append(&owner, Arc::clone(&operations), &denied)["error"]["code"],
        "REMOTE_INPUT_INVALID"
    );
    assert!(core.calls().is_empty());
    let control = control(&owner, operations);
    let mut stream = control::connect(owner._serving.layout().directory.as_path()).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    writeln!(stream, "{}", json!({"op":"approve","operationId":id})).unwrap();
    let mut events = BufReader::new(stream.try_clone().unwrap());
    let preview = line(&mut events);
    assert_eq!(preview["event"], "held");
    assert_eq!(preview["clientId"], owner.grant.client_id);
    assert_eq!(preview["recipientId"], recipient);
    assert_eq!(preview["message"], "[remote: Test device]\nfrozen");
    assert!(core.calls().is_empty());
    writeln!(stream, "{}", json!({"op":"confirm"})).unwrap();
    let ended = line(&mut events);
    assert_eq!(ended["event"], "ended");
    assert_eq!(ended["state"], "accepted");
    assert_eq!(ended["operationId"], id);
    assert_eq!(core.sends(), 1);
    let mut repeated = control::connect(owner._serving.layout().directory.as_path()).unwrap();
    writeln!(repeated, "{}", json!({"op":"approve","operationId":id})).unwrap();
    assert!(line(&mut BufReader::new(repeated)).get("error").is_some());
    assert_eq!(core.sends(), 1);
    control.stop();
}
#[test]
fn cancel_refuse_and_shutdown_never_dispatch_a_hold() {
    for action in ["cancel", "refuse", "duplicate-confirm", "stop"] {
        let owner = OwnerDoor::with_policy(
            tmt_remote::store::DEFAULT_SCOPES
                .iter()
                .map(|s| (*s).into())
                .collect(),
            "hold",
        );
        let core = Core::new();
        let operations = core.operations();
        let session = owner.open();
        let id = uuid_v4().unwrap();
        append(
            &owner,
            Arc::clone(&operations),
            &wire(&owner, &session, 1, &id, &uuid_v4().unwrap(), "not sent"),
        );
        let control = control(&owner, Arc::clone(&operations));
        let mut stream = control::connect(owner._serving.layout().directory.as_path()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        writeln!(
            stream,
            "{}",
            json!({"op":if action=="cancel" {"cancel"} else {"approve"},"operationId":id})
        )
        .unwrap();
        let mut events = BufReader::new(stream.try_clone().unwrap());
        if action != "cancel" {
            assert_eq!(line(&mut events)["event"], "held");
        }
        if action == "stop" {
            control.stop();
        } else {
            if action == "refuse" {
                writeln!(stream, "{}", json!({"op":"refuse"})).unwrap();
            } else if action == "duplicate-confirm" {
                writeln!(stream, "{{\"op\":\"refuse\",\"op\":\"confirm\"}}").unwrap();
            }
            assert_eq!(line(&mut events)["state"], "cancelled");
            control.stop();
        }
        if action == "stop" {
            assert_eq!(line(&mut events)["state"], "cancelled");
        }
        assert!(core.calls().is_empty());
        assert_eq!(
            owner
                .store
                .lock()
                .unwrap()
                .owned(&owner.grant, &id, tmt_remote::pairing::now_ms().unwrap())
                .unwrap()
                .phase,
            "cancelled"
        );
    }
}
#[test]
fn lost_acceptance_and_definitive_absence_recover_only_the_same_operation() {
    for accepted_before_loss in [true, false] {
        let owner = OwnerDoor::new();
        let core = Core::new();
        let operations = core.operations();
        let session = owner.open();
        let id = uuid_v4().unwrap();
        let recipient = uuid_v4().unwrap();
        fs::write(
            core.root.join(if accepted_before_loss {
                "lost"
            } else {
                "fault"
            }),
            b"",
        )
        .unwrap();
        assert_eq!(
            append(
                &owner,
                Arc::clone(&operations),
                &wire(&owner, &session, 1, &id, &recipient, "same intent")
            )["state"],
            "uncertain"
        );
        assert_eq!(core.sends(), 1);
        fs::remove_file(core.root.join(if accepted_before_loss {
            "lost"
        } else {
            "fault"
        }))
        .unwrap();
        let observation = owner.wire(
            &session,
            "2",
            "operation.show",
            json!({"operationId":id}).to_string().as_bytes(),
        );
        assert_eq!(
            append(&owner, Arc::clone(&operations), &observation)["state"],
            if accepted_before_loss {
                "accepted"
            } else {
                "uncertain"
            }
        );
        assert_eq!(core.sends(), 1, "read-only recovery must never send");
        let recovered = append(
            &owner,
            operations,
            &wire(&owner, &session, 3, &id, &recipient, "same intent"),
        );
        assert_eq!(recovered["state"], "accepted");
        assert_eq!(recovered["operationId"], id);
        assert_eq!(core.sends(), if accepted_before_loss { 1 } else { 2 });
        assert!(
            core.calls()
                .iter()
                .all(|call| call["input"]["operationId"] == id)
        );
    }
}
#[test]
fn allowlist_and_strict_intent_refuse_before_any_core_call() {
    let permitted = uuid_v4().unwrap();
    let owner = OwnerDoor::with_limits(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|s| (*s).into())
            .collect(),
        "direct",
        json!([permitted]).to_string(),
        None,
    );
    let core = Core::new();
    let operations = core.operations();
    let session = owner.open();
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(
                &owner,
                &session,
                1,
                &uuid_v4().unwrap(),
                &permitted,
                "allowed"
            )
        )["state"],
        "accepted"
    );
    let calls = core.calls().len();
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(
                &owner,
                &session,
                2,
                &uuid_v4().unwrap(),
                &uuid_v4().unwrap(),
                "blocked"
            )
        )["error"]["code"],
        "REMOTE_SCOPE_DENIED"
    );
    assert_eq!(core.calls().len(), calls);
    let owner = OwnerDoor::new();
    let session = owner.open();
    for (sequence, field) in (1..).zip(["identity", "kind", "fanout", "room", "operation", "extra"])
    {
        let id = uuid_v4().unwrap();
        let recipient = uuid_v4().unwrap();
        let mut payload = input(&id, &recipient, "valid base");
        match field {
            "identity" => payload["identity"] = json!("forged"),
            "kind" => payload["input"]["kind"] = json!("announcement"),
            "fanout" => payload["input"]["recipientIds"] = json!([recipient, uuid_v4().unwrap()]),
            "room" => payload["input"]["room"] = json!(uuid_v4().unwrap()),
            "operation" => payload["operation"] = json!("rooms.write"),
            "extra" => payload["input"]["extra"] = json!(true),
            _ => unreachable!(),
        }
        let mut request = owner.wire(
            &session,
            &sequence.to_string(),
            "dispatch.create",
            payload.to_string().as_bytes(),
        );
        request["id"] = json!(id);
        owner.resign(&mut request);
        assert_eq!(
            append(&owner, Arc::clone(&operations), &request)["error"]["code"],
            "REMOTE_INPUT_INVALID",
            "isolated {field}"
        );
        assert_eq!(
            core.calls().len(),
            calls,
            "isolated {field} must not invoke core"
        );
    }
}

#[test]
fn named_reads_project_authority_and_preserve_empty_final_without_dispatch() {
    let permitted = uuid_v4().unwrap();
    let other = uuid_v4().unwrap();
    let mut owner = OwnerDoor::with_limits(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|s| (*s).into())
            .collect(),
        "direct",
        json!([permitted]).to_string(),
        None,
    );
    let core = Core::new();
    fs::write(core.root.join("agents"), json!({"identities":[{"id":permitted,"name":"Allowed","presence":"active","pane":"%secret","cwd":"/secret","profile":{"secret":true},"delivery":{"state":"channel"}},{"id":other,"name":"Other","presence":"offline"}]}).to_string()).unwrap();
    fs::write(
        core.root.join("identities"),
        json!({"identities":[{"id":permitted,"name":"Allowed","canonicalName":"allowed","lifetime":"saved"},{"id":other,"name":"Other","canonicalName":"other","lifetime":"saved"}]}).to_string(),
    ).unwrap();
    let operations = core.operations();
    let session = owner.open();
    let mut sequence = 0;
    let mut read = |op: &str, payload: Value| {
        sequence += 1;
        append(
            &owner,
            Arc::clone(&operations),
            &owner.wire(
                &session,
                &sequence.to_string(),
                op,
                payload.to_string().as_bytes(),
            ),
        )
    };
    assert_eq!(
        read("agents.list", json!({})),
        json!({"identities":[{"id":permitted,"name":"Allowed","presence":"active","delivery":{"state":"channel"}}]})
    );
    assert_eq!(
        read(
            "identities.status",
            json!({"version":1,"operation":"identities.status","input":{"identityIds":[permitted]}})
        )["identities"][0]["id"],
        permitted
    );
    assert_eq!(
        read("check", json!({"agentId":permitted,"lines":5}))["output"],
        "bounded capture"
    );
    let observations = core.calls();
    assert_eq!(
        observations[observations.len() - 2]["argv"],
        json!(["identity", "list", "--json"])
    );
    assert_eq!(
        observations.last().unwrap()["argv"],
        json!(["check", "Allowed", "--json", "--lines", "5"])
    );
    let calls = observations.len();
    for (op, input) in [
        ("check", json!({"agentId":other})),
        (
            "identities.status",
            json!({"version":1,"operation":"identities.status","input":{"identityIds":[other]}}),
        ),
    ] {
        assert_eq!(read(op, input)["error"]["code"], "REMOTE_SCOPE_DENIED");
    }
    assert_eq!(core.calls().len(), calls);
    let request = format!("req_{}", uuid_v4().unwrap());
    for (final_value, state) in [
        (json!({"status":"not_submitted"}), "pending"),
        (json!({"status":"retained","response":""}), "replied"),
        (json!({"status":"expired"}), "unavailable"),
    ] {
        fs::write(core.root.join("final"), final_value.to_string()).unwrap();
        let result = read("result", json!({"requestId":request}));
        assert_eq!(result["state"], state);
        if state == "replied" {
            assert_eq!(result["message"], "");
        }
        assert_eq!(
            read(
                "requests.show",
                json!({"version":1,"operation":"requests.show","input":{"requestId":request}})
            )["final"],
            final_value
        );
    }
    let id = uuid_v4().unwrap();
    sequence += 1;
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(&owner, &session, sequence, &id, &permitted, "owned")
        )["state"],
        "accepted"
    );
    sequence += 1;
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &owner.wire(
                &session,
                &sequence.to_string(),
                "dispatch.show",
                json!({"version":1,"operation":"dispatch.show","input":{"operationId":id}})
                    .to_string()
                    .as_bytes()
            )
        )["operationId"],
        id
    );
    assert_eq!(core.sends(), 1);
    // A second device in the same database cannot observe the first one's operation.
    owner.key = ed25519_dalek::SigningKey::from_bytes(&[48; 32]);
    owner.grant.client_id = uuid_v4().unwrap();
    owner.grant.public_key = owner.key.verifying_key().to_bytes();
    owner
        .store
        .lock()
        .unwrap()
        .insert_grant(&owner.grant)
        .unwrap();
    let outside_session = owner.open();
    let before_owned = core.calls().len();
    for (sequence, op, payload) in [
        (1, "operation.show", json!({"operationId":id})),
        (
            2,
            "dispatch.show",
            json!({"version":1,"operation":"dispatch.show","input":{"operationId":id}}),
        ),
    ] {
        assert!(
            append(
                &owner,
                Arc::clone(&operations),
                &owner.wire(
                    &outside_session,
                    &sequence.to_string(),
                    op,
                    payload.to_string().as_bytes()
                )
            )
            .get("error")
            .is_some()
        );
    }
    assert_eq!(core.calls().len(), before_owned);
    let restricted = OwnerDoor::with_policy(vec!["talk".into()], "direct");
    let sid = restricted.open();
    let before = core.calls().len();
    assert_eq!(
        append(
            &restricted,
            operations,
            &restricted.wire(
                &sid,
                "1",
                "result",
                json!({"requestId":request}).to_string().as_bytes()
            )
        )["error"]["code"],
        "REMOTE_SCOPE_DENIED"
    );
    assert_eq!(core.calls().len(), before);
}

#[test]
fn another_store_revoke_waits_for_a_blocked_core_effect_beyond_the_old_timeout() {
    use std::time::{Duration, Instant};
    use tmt_remote::store::Store;
    let owner = OwnerDoor::new();
    let core = Core::new();
    let operations = core.operations();
    let session = owner.open();
    let id = uuid_v4().unwrap();
    let recipient = uuid_v4().unwrap();
    let sent = wire(
        &owner,
        &session,
        1,
        &id,
        &recipient,
        "ordered before revoke",
    );
    // Independent SQLite connection, as an owner authority writer in another process.
    let mut revoker = Store::open(&owner._serving).unwrap();
    nix::unistd::mkfifo(
        &core.root.join("gate"),
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let mut gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(core.root.join("gate"))
        .unwrap();
    std::thread::scope(|scope| {
        let sending = scope.spawn(|| append(&owner, Arc::clone(&operations), &sent));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !core.root.join("entered").exists() {
            assert!(
                Instant::now() < deadline,
                "core fixture did not reach the effect barrier"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!core.root.join("receipt").exists());
        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let client = owner.grant.client_id.clone();
        let revoking = scope.spawn(move || {
            attempted_tx.send(()).unwrap();
            done_tx.send(revoker.revoke(&client)).unwrap();
        });
        attempted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        // Wait on the revocation outcome, not a fixed sleep: the old five-second
        // timeout returns an error here while the effect barrier remains held.
        let early = done_rx.recv_timeout(Duration::from_millis(5200));
        let waited_for_effect = matches!(&early, Err(std::sync::mpsc::RecvTimeoutError::Timeout));
        gate.write_all(b"release\n").unwrap();
        let revoked = match early {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                done_rx.recv_timeout(Duration::from_secs(5)).unwrap()
            }
            Err(error) => panic!("revoker disconnected: {error}"),
        }
        .unwrap()
        .unwrap();
        revoking.join().unwrap();
        let observed = sending.join().unwrap();
        assert!(
            waited_for_effect,
            "revocation must wait past five seconds until the earlier effect releases its fence"
        );
        assert!(
            revoked.disabled,
            "revoke must commit after the earlier effect"
        );
        assert!(matches!(
            observed["state"].as_str(),
            Some("accepted" | "uncertain")
        ));
        assert_eq!(core.sends(), 1);
        let receipt: Value =
            serde_json::from_slice(&fs::read(core.root.join("receipt")).unwrap()).unwrap();
        assert_eq!(receipt["operationId"], id);
        assert!(
            owner
                .store
                .lock()
                .unwrap()
                .grant(&owner.grant.client_id)
                .unwrap()
                .unwrap()
                .disabled
        );
        let refused = owner.wire(
            &session,
            "2",
            "dispatch.create",
            input(&uuid_v4().unwrap(), &recipient, "after revoke")
                .to_string()
                .as_bytes(),
        );
        let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 65536)
            .with_operations(Arc::clone(&operations));
        assert!(
            transport
                .append(None, &serde_json::to_vec(&refused).unwrap())
                .is_err()
        );
        assert_eq!(core.sends(), 1);
    });
}

#[test]
fn session_eviction_preserves_grant_owned_holds_and_uncertainty() {
    let owner = OwnerDoor::with_policy(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|s| (*s).into())
            .collect(),
        "hold",
    );
    tmt_remote::settings::set_sessions_per_device(&owner._root.0, Some(2)).unwrap();
    let core = Core::new();
    let operations = core.operations();
    let first = owner.open();
    let second = owner.open();
    let first_id = uuid_v4().unwrap();
    let second_id = uuid_v4().unwrap();
    let uncertain_id = uuid_v4().unwrap();
    for (session, sequence, id) in [
        (&first, 1, &first_id),
        (&first, 2, &uncertain_id),
        (&second, 1, &second_id),
    ] {
        append(
            &owner,
            Arc::clone(&operations),
            &wire(
                &owner,
                session,
                sequence,
                id,
                &uuid_v4().unwrap(),
                "Unconfirmed",
            ),
        );
    }
    // Independent preparation of a recovery-owned operation; ending a session cannot erase it.
    let oracle =
        rusqlite::Connection::open(owner._serving.layout().directory.join("remote.db")).unwrap();
    oracle
        .execute(
            "UPDATE operations SET phase='uncertain' WHERE id=?1",
            [&uncertain_id],
        )
        .unwrap();
    drop(oracle);
    let third = owner.open();
    assert_eq!(
        super::signed_code(owner.admit(&owner.wire(&first, "3", "capabilities", b"{}"))),
        "REMOTE_SESSION_EVICTED"
    );
    let mut store = owner.store.lock().unwrap();
    let now = tmt_remote::pairing::now_ms().unwrap();
    assert_eq!(
        store.owned(&owner.grant, &first_id, now).unwrap().phase,
        "held"
    );
    assert_eq!(
        store.owned(&owner.grant, &second_id, now).unwrap().phase,
        "held"
    );
    let uncertain = store.owned(&owner.grant, &uncertain_id, now).unwrap();
    assert_eq!(uncertain.phase, "uncertain");
    assert!(uncertain.frozen.is_some());
    assert!(core.calls().is_empty());
    drop(store);
    assert!(
        owner
            .admit(&owner.wire(&third, "1", "capabilities", b"{}"))
            .is_ok()
    );
    owner.sessions.shutdown();
    assert_eq!(
        owner
            .store
            .lock()
            .unwrap()
            .owned(&owner.grant, &second_id, now)
            .unwrap()
            .phase,
        "cancelled"
    );
}

#[test]
fn another_live_tab_recovers_uncertainty_into_the_same_device_stream_without_resending() {
    let owner = OwnerDoor::new();
    let core = Core::new();
    let operations = core.operations();
    let first = owner.open();
    let id = uuid_v4().unwrap();
    let recipient = uuid_v4().unwrap();
    fs::write(core.root.join("lost"), b"").unwrap();
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(&owner, &first, 1, &id, &recipient, "same intent")
        )["state"],
        "uncertain"
    );
    fs::remove_file(core.root.join("lost")).unwrap();
    let second = owner.open();
    let observed = owner.wire(
        &second,
        "1",
        "operation.show",
        json!({"operationId":id}).to_string().as_bytes(),
    );
    assert_eq!(append(&owner, operations, &observed)["state"], "accepted");
    assert_eq!(core.sends(), 1);
    let page = owner
        .store
        .lock()
        .unwrap()
        .page(
            &owner.grant,
            None,
            50,
            tmt_remote::pairing::now_ms().unwrap(),
        )
        .unwrap();
    let entries = page["entries"].as_array().unwrap();
    let last = &entries.last().unwrap()["envelope"];
    assert_eq!(last["sessionId"], second);
    let payload =
        tmt_remote::canonical::base64url_decode(last["payload"].as_str().unwrap()).unwrap();
    let payload: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["state"], "accepted");
    assert_eq!(payload["operationId"], id);
}

#[test]
fn held_bound_remains_per_device_across_session_eviction() {
    let owner = OwnerDoor::with_policy(
        tmt_remote::store::DEFAULT_SCOPES
            .iter()
            .map(|s| (*s).into())
            .collect(),
        "hold",
    );
    let core = Core::new();
    let operations = core.operations();
    let mut ids = Vec::new();
    for _ in 0..16 {
        let session = owner.open();
        let id = uuid_v4().unwrap();
        assert_eq!(
            append(
                &owner,
                Arc::clone(&operations),
                &wire(
                    &owner,
                    &session,
                    1,
                    &id,
                    &uuid_v4().unwrap(),
                    "held across tabs"
                )
            )["state"],
            "held"
        );
        ids.push(id);
    }
    let session = owner.open();
    assert_eq!(
        append(
            &owner,
            Arc::clone(&operations),
            &wire(
                &owner,
                &session,
                1,
                &uuid_v4().unwrap(),
                &uuid_v4().unwrap(),
                "over bound"
            )
        )["error"]["code"],
        "REMOTE_RATE_LIMITED"
    );
    for id in &ids {
        assert_eq!(
            owner
                .store
                .lock()
                .unwrap()
                .owned(&owner.grant, id, tmt_remote::pairing::now_ms().unwrap())
                .unwrap()
                .phase,
            "held"
        );
    }
    assert_eq!(core.sends(), 0);
    owner.sessions.shutdown();
    for id in ids {
        assert_eq!(
            owner
                .store
                .lock()
                .unwrap()
                .owned(&owner.grant, &id, tmt_remote::pairing::now_ms().unwrap())
                .unwrap()
                .phase,
            "cancelled"
        );
    }
}

#[test]
fn authority_loss_cancels_held_work_even_after_its_session_ended() {
    for change in ["revoke", "expiry", "revision"] {
        let mut owner = OwnerDoor::with_policy(
            tmt_remote::store::DEFAULT_SCOPES
                .iter()
                .map(|s| (*s).into())
                .collect(),
            "hold",
        );
        let clock = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
        let fake = Arc::clone(&clock);
        owner.sessions = Arc::new(
            Arc::try_unwrap(owner.sessions)
                .ok()
                .unwrap()
                .with_clock(Arc::new(move || *fake.lock().unwrap())),
        );
        let core = Core::new();
        let session = owner.open();
        let id = uuid_v4().unwrap();
        assert_eq!(
            append(
                &owner,
                core.operations(),
                &wire(&owner, &session, 1, &id, &uuid_v4().unwrap(), "held")
            )["state"],
            "held"
        );
        *clock.lock().unwrap() += std::time::Duration::from_secs(61);
        owner.sessions.maintain().unwrap();
        let oracle =
            rusqlite::Connection::open(owner._serving.layout().directory.join("remote.db"))
                .unwrap();
        let phase = || {
            oracle
                .query_row("SELECT phase FROM operations WHERE id=?1", [&id], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap()
        };
        assert_eq!(phase(), "held");
        assert_eq!(
            oracle
                .execute("DELETE FROM sessions WHERE ended_reason IS NOT NULL", [])
                .unwrap(),
            1
        );
        match change {
            "revoke" => {
                owner
                    .store
                    .lock()
                    .unwrap()
                    .revoke(&owner.grant.client_id)
                    .unwrap();
            }
            "revision" => {
                owner
                    .store
                    .lock()
                    .unwrap()
                    .rename(&owner.grant.client_id, "Changed")
                    .unwrap();
            }
            "expiry" => {
                oracle
                    .execute("UPDATE grants SET expires_at_ms=1", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        *clock.lock().unwrap() += std::time::Duration::from_secs(1);
        owner.sessions.maintain().unwrap();
        assert_eq!(phase(), "cancelled");
        assert_eq!(core.sends(), 0);
    }
}
