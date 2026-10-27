//! Actual Service composition over injected HTTP, the serving Store and signed browser requests.
use super::*;
use crate::{canonical, store::Grant};
use ed25519_dalek::{Signer, SigningKey};

struct StopOnPanic(Vec<Arc<AtomicBool>>);
impl Drop for StopOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            for stop in &self.0 {
                stop.store(true, Ordering::Release);
            }
        }
    }
}
struct Browser {
    grant: Grant,
    key: SigningKey,
    session: String,
    sequence: u64,
}
impl Browser {
    fn new(owner: &Service) -> Self {
        let key = SigningKey::from_bytes(&[24; 32]);
        let now = crate::pairing::now_ms().unwrap();
        let grant = Grant {
            client_id: crate::store::uuid_v4().unwrap(),
            public_key: key.verifying_key().to_bytes(),
            kind: "browser".into(),
            origin: "http://127.0.0.1:1".into(),
            name: "Owner browser".into(),
            agents: "all".into(),
            scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
            mode: "direct".into(),
            issued_at_ms: now,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let grant = {
            let mut store = owner.store().unwrap();
            store.insert_grant(&grant).unwrap();
            store
                .designate(&grant.client_id, &grant.origin, now)
                .unwrap();
            store.grant(&grant.client_id).unwrap().unwrap()
        };
        let mut browser = Self {
            grant,
            key,
            session: "new".into(),
            sequence: 0,
        };
        let opened = browser.request(
            owner,
            "session.open",
            json!({"clientNonce":crate::store::uuid_v4().unwrap().replace('-', "")}),
        );
        // The response envelope, not the payload, identifies the opened Session.
        browser.session = opened.1;
        browser
    }
    fn request(&mut self, owner: &Service, operation: &str, input: Value) -> (Value, String) {
        let id = input["operationId"]
            .as_str()
            .filter(|_| operation != "remote.management.operation")
            .map(str::to_owned)
            .unwrap_or_else(|| crate::store::uuid_v4().unwrap());
        let payload = input.to_string().into_bytes();
        let sequence = self.sequence.to_string();
        let now = crate::pairing::now_ms().unwrap();
        let envelope = canonical::Envelope {
            kind: if operation == "session.open" {
                "control"
            } else {
                "request"
            },
            id: &id,
            correlation_id: None,
            machine_id: &owner.machine,
            window_id: WINDOW,
            client_id: &self.grant.client_id,
            session_id: &self.session,
            sequence: &sequence,
            timestamp_ms: now,
            origin: &self.grant.origin,
            operation,
            payload: &payload,
        };
        let signature = self
            .key
            .sign(&canonical::envelope(&envelope).unwrap())
            .to_bytes();
        let wire = json!({"version":1,"profile":"local-v1","kind":envelope.kind,"id":id,"correlationId":null,
            "machineId":owner.machine,"windowId":WINDOW,"clientId":self.grant.client_id,"sessionId":self.session,
            "sequence":sequence,"timestampMs":now,"origin":self.grant.origin,"operation":operation,
            "payload":canonical::base64url(&payload),"signature":canonical::base64url(&signature)});
        let bytes = serde_json::to_vec(&wire).unwrap();
        let answer = if operation == "session.open" {
            owner
                .sessions
                .open(Some(&self.grant.origin), &bytes)
                .expect("signed Session open")
                .response
        } else {
            owner
                .messages
                .append(Some(&self.grant.origin), &bytes)
                .unwrap_or_else(|_| panic!("signed {operation} closed"))
        };
        let answer: Value = serde_json::from_slice(&answer).unwrap();
        assert_eq!(answer["correlationId"], id);
        assert_eq!(answer["operation"], operation);
        assert_eq!(answer["clientId"], self.grant.client_id);
        let payload = canonical::base64url_decode(answer["payload"].as_str().unwrap()).unwrap();
        let signature = canonical::base64url_decode(answer["signature"].as_str().unwrap()).unwrap();
        let response = canonical::Envelope {
            kind: "response",
            id: answer["id"].as_str().unwrap(),
            correlation_id: Some(&id),
            machine_id: &owner.machine,
            window_id: WINDOW,
            client_id: &self.grant.client_id,
            session_id: answer["sessionId"].as_str().unwrap(),
            sequence: answer["sequence"].as_str().unwrap(),
            timestamp_ms: answer["timestampMs"].as_u64().unwrap(),
            origin: &self.grant.origin,
            operation,
            payload: &payload,
        };
        crate::crypto::verify_signature(
            &owner.key.public(),
            &canonical::envelope(&response).unwrap(),
            &signature,
        )
        .unwrap();
        self.sequence += 1;
        (
            serde_json::from_slice(&payload).unwrap(),
            answer["sessionId"].as_str().unwrap().into(),
        )
    }
}

#[test]
fn signed_browser_wins_the_live_terminal_offer_and_recovers_one_original() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::browser_ceremony_tests::signed_browser_wins_the_live_terminal_offer_and_recovers_one_original",
    ) {
        return;
    }
    for sending in [false, true] {
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
        let mut browser = Browser::new(&owner);
        let worker = owner.worker().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut decision = None;
        let mut original = None;
        let mut confirmation_input = None;
        let mut candidate_events = 0;
        std::thread::scope(|scope| {
            let _cancel = Cancel(vec![peer.stop.clone()]);
            let inviter = scope.spawn(|| { let _panic = StopOnPanic(vec![owner.stop.clone(), peer.stop.clone()]); owner.invite_selected(PROJECT, config(), |event| {
                match event["event"].as_str() {
                    Some("invite") => tx.send(event["code"].as_str().unwrap().to_owned()).unwrap(),
                    Some("candidate") => {
                        candidate_events += 1;
                        let pending = browser.request(&owner, "remote.invites.pending", json!({})).0;
                        assert_eq!(pending["phase"], "awaiting-owner");
                        assert_eq!(pending["candidate"]["clientId"], peer.machine);
                        assert!(pending.get("code").is_none());
                        let input = json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":pending["offerId"],
                            "snapshot":pending["candidate"]["snapshot"],"matched":true,"sending":sending});
                        for wrong in ["offerId", "snapshot"] {
                            let mut stale = input.clone();
                            stale["operationId"] = json!(crate::store::uuid_v4().unwrap());
                            stale[wrong] = if wrong == "offerId" { json!(crate::store::uuid_v4().unwrap()) }
                                else { json!(canonical::base64url(&[6;32])) };
                            let refused = browser.request(&owner, "remote.invites.confirm", stale.clone()).0;
                            assert_eq!(refused["state"], "refused");
                            assert_eq!(refused["reason"], "REMOTE_INVITATION_INVALID");
                            assert_eq!(browser.request(&owner,"remote.management.operation",json!({"operationId":stale["operationId"]})).0,refused);
                            assert_eq!(owner.store().unwrap().grants().unwrap().len(),1);
                        }
                        // The already-recorded browser outcome survives a lost publication answer.
                        wire.receipt_answers.lose_next.store(sending, Ordering::Release);
                        let result = browser.request(&owner, "remote.invites.confirm", input.clone()).0;
                        assert_eq!(result["state"], "committed");
                        assert_eq!(result["result"]["invitation"]["state"], "confirmed");
                        confirmation_input = Some(input);
                        original = Some(result);
                        // A terminal answer already in flight loses to the browser decision.
                        return Ok(Some((false, false)));
                    }
                    Some("owner-decision") => decision = Some(event),
                    _ => {}
                }
                Ok(None)
            }) });
            let code = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            peer.join(&code, |_| Ok(Some(false))).unwrap();
            inviter.join().unwrap().unwrap();
        });
        let decision = decision.unwrap();
        assert_eq!(decision["outcome"], "confirmed");
        assert_eq!(decision["talk"], sending);
        assert_eq!(candidate_events, 1);
        let granted = owner
            .store()
            .unwrap()
            .grant(&peer.machine)
            .unwrap()
            .unwrap();
        assert_eq!(granted.permits_scope("talk"), sending);
        assert_eq!(owner.store().unwrap().grants().unwrap().len(), 2);
        assert_eq!(peer.store().unwrap().grants().unwrap().len(), 1);
        let commits = wire.commits.lock().unwrap().clone();
        let input = confirmation_input.unwrap();
        let recovered = browser
            .request(
                &owner,
                "remote.management.operation",
                json!({"operationId":input["operationId"]}),
            )
            .0;
        assert_eq!(Some(recovered), original);
        let repeated = browser.request(&owner, "remote.invites.confirm", input).0;
        assert_eq!(Some(repeated), original);
        assert_eq!(
            *wire.commits.lock().unwrap(),
            commits,
            "lookup/repeated ID published again"
        );
        assert_eq!(core.sends(), 0);
        drop(worker);
        drop(peer);
        drop(owner);
        drop(peer_lease);
        drop(owner_lease);
    }
}

#[test]
fn signed_browser_unavailable_offer_settles_definite_refusal() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::browser_ceremony_tests::signed_browser_unavailable_offer_settles_definite_refusal",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let (owner, lease) = service(
        &core.root.join("owner"),
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports,
        },
    );
    let mut browser = Browser::new(&owner);
    assert_eq!(
        browser
            .request(&owner, "remote.invites.pending", json!({}))
            .0,
        json!({"state":"none"})
    );
    let id = crate::store::uuid_v4().unwrap();
    let input = json!({"operationId":id,"offerId":crate::store::uuid_v4().unwrap(),"snapshot":canonical::base64url(&[3;32]),"matched":true,"sending":false});
    let result = browser
        .request(&owner, "remote.invites.confirm", input.clone())
        .0;
    assert_eq!(
        result["state"], "refused",
        "an unavailable offer is a definite no-effect decision"
    );
    assert_eq!(result["reason"], "REMOTE_INVITATION_UNAVAILABLE");
    assert_eq!(
        browser
            .request(
                &owner,
                "remote.management.operation",
                json!({"operationId":id})
            )
            .0,
        result
    );
    assert_eq!(
        browser.request(&owner, "remote.invites.confirm", input).0,
        result
    );
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    assert!(wire.commits.lock().unwrap().is_empty());
    assert_eq!(core.sends(), 0);
    drop(owner);
    drop(lease);
}
#[test]
fn signed_browser_terminal_winner_is_refused_and_refresh_shows_confirmed() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::browser_ceremony_tests::signed_browser_terminal_winner_is_refused_and_refresh_shows_confirmed",
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
    let mut browser = Browser::new(&owner);
    let worker = owner.worker().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let mut input = None;
    std::thread::scope(|scope| {
        let _cancel = Cancel(vec![peer.stop.clone()]);
        let inviter=scope.spawn(||{
            let _panic=StopOnPanic(vec![owner.stop.clone(),peer.stop.clone()]);
            owner.invite_selected(PROJECT,config(),|event|{
                if event["event"]=="invite"{tx.send(event["code"].as_str().unwrap().to_owned()).unwrap();}
                if event["event"]=="candidate"{
                    let pending=browser.request(&owner,"remote.invites.pending",json!({})).0;
                    input=Some(json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":pending["offerId"],"snapshot":pending["candidate"]["snapshot"],"matched":true,"sending":false}));
                    return Ok(Some((true,false)));
                }
                Ok(None)
            })
        });
        let code = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        peer.join(&code, |_| Ok(Some(false))).unwrap();
        inviter.join().unwrap().unwrap();
    });
    let input = input.unwrap();
    let before = wire.commits.lock().unwrap().clone();
    let result = browser
        .request(&owner, "remote.invites.confirm", input.clone())
        .0;
    assert_eq!(
        result["state"], "refused",
        "a later browser decision cannot turn a terminal winner into uncertainty"
    );
    assert_eq!(result["reason"], "REMOTE_INVITATION_DECIDED");
    assert_eq!(
        browser
            .request(
                &owner,
                "remote.management.operation",
                json!({"operationId":input["operationId"]})
            )
            .0,
        result
    );
    let current = browser
        .request(&owner, "remote.invites.pending", json!({}))
        .0;
    assert_eq!(current["phase"], "confirmed");
    assert_eq!(current["localConfirmationKnown"], true);
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 2);
    assert_eq!(
        *wire.commits.lock().unwrap(),
        before,
        "a refused browser replay published again"
    );
    drop(worker);
    drop(peer);
    drop(owner);
    drop(peer_lease);
    drop(owner_lease);
}
