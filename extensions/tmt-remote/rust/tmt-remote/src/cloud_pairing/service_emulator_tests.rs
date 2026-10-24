//! Ordinary-user qualification. Admin is used only to install Rules in a demo emulator.
use super::*;
use crate::{
    firebase_auth::{ProductionHttp, Project},
    sealed_sync::AuthPort,
};
use std::net::SocketAddr;
struct Emulator {
    http: ProductionHttp,
    clock: Arc<SystemClock>,
    owner: Mutex<Weak<Service>>,
    calls: Mutex<BTreeMap<String, usize>>,
}
impl Jitter for Emulator {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
impl Http for Emulator {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(format!("{:?}", request.operation()))
            .or_default() += 1;
        let response = self.http.send(request, deadline, cancel)?;
        if request.operation() == Operation::Commit && response.status == 200 {
            let input: Value = serde_json::from_slice(request.body()).unwrap();
            if input["writes"].as_array().unwrap().iter().any(|w| {
                w["update"]["name"]
                    .as_str()
                    .is_some_and(|n| n.contains("/inbox/"))
                    || w["update"]["fields"]["kind"]["stringValue"] == "reroute"
            }) {
                let owner = self.owner.lock().unwrap().upgrade();
                if let Some(owner) = owner {
                    owner.reconcile().unwrap();
                }
            }
        }
        Ok(response)
    }
}
impl Emulator {
    fn new() -> Self {
        // Validate BOTH emulator routes before even constructing ProductionHttp.
        let mut firestore = None;
        for name in ["FIRESTORE_EMULATOR_HOST", "FIREBASE_AUTH_EMULATOR_HOST"] {
            let raw = std::env::var(name).expect("explicit emulator host required");
            let address: SocketAddr = raw.parse().expect("numeric loopback emulator host");
            assert!(address.ip().is_loopback() && address.port() != 0);
            if name == "FIRESTORE_EMULATOR_HOST" {
                firestore = Some(address);
            }
        }
        assert!(PROJECT.starts_with("demo-"));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let request = ureq::http::Request::builder().method("PUT")
            .uri(format!("http://{}/emulator/v1/projects/{PROJECT}:securityRules", firestore.unwrap()))
            .header("Authorization", "Bearer owner").header("Content-Type", "application/json")
            .body(json!({"rules":{"files":[{"name":"firestore.rules","content":crate::cloud_inbox::rules::artifact()}]}}).to_string().into_bytes()).unwrap();
        assert_eq!(
            agent.run(request).unwrap().status().as_u16(),
            200,
            "Rules installation"
        );
        let clock = Arc::new(SystemClock::default());
        let http = ProductionHttp::new(
            &Project::from_public_config(&config()).unwrap(),
            clock.clone(),
        )
        .unwrap();
        assert!(http.emulator_active(), "no production fallback");
        Self {
            http,
            clock,
            owner: Mutex::new(Weak::new()),
            calls: Mutex::new(BTreeMap::new()),
        }
    }
    fn ports(self: &Arc<Self>) -> ProjectPorts {
        ProjectPorts {
            http: self.clone(),
            clock: self.clock.clone(),
            jitter: self.clone(),
        }
    }
    fn token(&self, service: &Service) -> String {
        let project = service.projects.load(&config(), true).unwrap();
        complete(project.auth.token(&service.io()), &service.io())
            .unwrap()
            .unwrap()
            .token
    }
    fn raw(&self, token: &str, operation: Operation, suffix: &str, value: Value) -> u16 {
        // Rules probes bypass native accounting intentionally, separately from the measured ceremony.
        let request = Request::firestore(
            operation,
            format!(
                "https://firestore.googleapis.com/v1/projects/{PROJECT}/databases/(default)/documents{suffix}"
            ),
            value.to_string().into_bytes(),
            token.into(),
            crate::limits::CLOUD_ENROLLMENT_BYTES * 4,
        );
        self.http
            .send(
                &request,
                self.clock.now().monotonic_ms + 20_000,
                &AtomicBool::new(false),
            )
            .unwrap()
            .status
    }
    fn seed_owner(&self, service: &Service) {
        let provider = service.provider(&config(), &service.machine, true).unwrap();
        // Demo fixture only: production enrollment uses the authorized deploy step.
        assert_eq!(
            self.commit(
                "owner",
                &format!("owners/{}", provider.uid()),
                json!({"machine":{"stringValue":service.machine}})
            ),
            200
        );
    }
    fn commit(&self, token: &str, path: &str, fields: Value) -> u16 {
        self.raw(token, Operation::Commit, ":commit", json!({"writes":[{"update":{"name":format!("projects/{PROJECT}/databases/(default)/documents/{path}"),"fields":fields}}]}))
    }
}
#[test]
#[ignore = "Infra-approved Auth+Firestore emulator slot; no real project or fallback"]
fn ordinary_user_pairing_rules_restart_reroute_and_revoke() {
    let emulator = Arc::new(Emulator::new());
    let core = core_fixture::Core::new();
    std::fs::write(core.root.join("multi"), "").unwrap();
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, emulator.ports());
    let (sender, sender_lease) = service(&core.root.join("sender"), &core, emulator.ports());
    emulator.seed_owner(&owner);
    let (token_tx, token_rx) = mpsc::channel();
    let (choice_tx, choice_rx) = mpsc::channel();
    let (words_tx, words_rx) = mpsc::channel();
    let (token, remote) = std::thread::scope(|scope| {
        struct Cancel(Vec<Arc<AtomicBool>>);
        impl Drop for Cancel {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    for flag in &self.0 {
                        flag.store(true, Ordering::Release);
                    }
                }
            }
        }
        let _cancel = Cancel(vec![owner.stop.clone(), sender.stop.clone()]);
        let owner_ref = &owner;
        let invite = scope.spawn(move || {
            owner_ref.invite_selected(PROJECT, config(), |event| {
                if event["event"] == "invite" {
                    token_tx
                        .send(event["code"].as_str().unwrap().to_owned())
                        .unwrap();
                }
                if event["event"] == "candidate" {
                    words_tx.send(event["words"].clone()).unwrap();
                    return Ok(Some(
                        choice_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
                    ));
                }
                Ok(None)
            })
        });
        let token = token_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let join_token = token.clone();
        let sender_ref = &sender;
        let join = scope.spawn(move || {
            let mut events = vec![];
            sender_ref
                .join(&join_token, |event| {
                    events.push(event);
                    Ok(())
                })
                .unwrap();
            events
        });
        let words = words_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(owner.store().unwrap().grants().unwrap().is_empty());
        assert_eq!(core.sends(), 0);
        choice_tx.send((true, false)).unwrap();
        invite.join().unwrap().unwrap();
        let events = join.join().unwrap();
        assert_eq!(
            events.iter().find(|e| e["event"] == "joining").unwrap()["words"],
            words
        );
        let connected = events.iter().find(|e| e["event"] == "connected").unwrap();
        assert_eq!(connected["talk"], false);
        (
            token.clone(),
            connected["remote"].as_str().unwrap().to_owned(),
        )
    });
    eprintln!(
        "ceremony wire calls (Rules installation excluded): {:?}",
        emulator.calls.lock().unwrap()
    );
    let mut expired = Invite::read(&token, crate::pairing::now_ms().unwrap()).unwrap();
    expired.challenge.expires_at_ms = crate::pairing::now_ms().unwrap() - 1;
    let before = emulator.calls.lock().unwrap().clone();
    assert_eq!(
        sender
            .join(&expired.token().unwrap(), |_| Ok(()))
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_PAIR_EXPIRED"
    );
    assert_eq!(
        *emulator.calls.lock().unwrap(),
        before,
        "expired code refuses before provider work"
    );
    let (intruder, intruder_lease) = service(&core.root.join("intruder"), &core, emulator.ports());
    assert!(
        intruder.join(&token, |_| Ok(())).is_err(),
        "a competing native key cannot reuse the code"
    );
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    drop(intruder);
    drop(intruder_lease);
    *emulator.owner.lock().unwrap() = Arc::downgrade(&owner);
    let owner_token = emulator.token(&owner);
    let sender_token = emulator.token(&sender);
    let owner_uid = owner
        .projects
        .load(&config(), false)
        .unwrap()
        .auth
        .identity()
        .uid;
    let old_uid = sender
        .projects
        .load(&config(), false)
        .unwrap()
        .auth
        .identity()
        .uid;
    let descriptor = owner
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    let row = json!({"uid":{"stringValue":owner_uid},"authenticationKey":{"bytesValue":STANDARD.encode(owner.key.public())},"descriptor":{"bytesValue":STANDARD.encode(descriptor.bytes())}});
    let path = format!("m/{}", owner.machine);
    assert_eq!(
        emulator.raw(
            &sender_token,
            Operation::Read,
            &format!("/{path}"),
            Value::Null
        ),
        200
    );
    assert_eq!(
        emulator.raw(
            &sender_token,
            Operation::Query,
            ":runQuery",
            json!({"structuredQuery":{"from":[{"collectionId":"m"}],"limit":1}})
        ),
        403
    );
    assert_eq!(
        emulator.commit(&sender_token, &path, row.clone()),
        403,
        "another uid updating owner row"
    );
    let mut changed = row.clone();
    changed["uid"] = json!({"stringValue":old_uid});
    assert_eq!(
        emulator.commit(&owner_token, &path, changed),
        403,
        "uid immutable"
    );
    let mut changed = row.clone();
    changed["authenticationKey"] = json!({"bytesValue":STANDARD.encode([9;32])});
    assert_eq!(
        emulator.commit(&owner_token, &path, changed),
        403,
        "key immutable"
    );
    assert_eq!(
        emulator.commit(&owner_token, &path, row.clone()),
        200,
        "owner descriptor update"
    );
    assert_eq!(
        emulator.commit(
            &sender_token,
            &format!("m/{}", crate::store::uuid_v4().unwrap()),
            row
        ),
        403,
        "cannot create as another uid"
    );
    let route = json!({"peer":{"stringValue":sender.machine},"revoked":{"booleanValue":false},"expiresAt":{"timestampValue":"9999-12-31T23:59:59.999Z"}});
    assert_eq!(
        emulator.commit(&sender_token, &format!("{path}/peers/{old_uid}"), route),
        403,
        "peer cannot write route"
    );
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "refused"
    );
    assert_eq!(core.sends(), 0);
    owner.store().unwrap().talk(&sender.machine, true).unwrap();
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 1);
    owner
        .store()
        .unwrap()
        .connection
        .execute(
            "UPDATE grants SET mode='hold',revision=revision+1 WHERE client_id=?1",
            [&sender.machine],
        )
        .unwrap();
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "held"
    );
    assert_eq!(core.sends(), 1, "held work does not dispatch");
    owner
        .store()
        .unwrap()
        .connection
        .execute(
            "UPDATE grants SET mode='direct',agents='[]',revision=revision+1 WHERE client_id=?1",
            [&sender.machine],
        )
        .unwrap();
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "refused"
    );
    assert_eq!(
        core.sends(),
        1,
        "agent allowlist still fences native ingress"
    );
    owner
        .store()
        .unwrap()
        .connection
        .execute(
            "UPDATE grants SET agents='all',revision=revision+1 WHERE client_id=?1",
            [&sender.machine],
        )
        .unwrap();

    let owner_id = owner.machine.clone();
    let sender_key = sender.key.public();
    drop(owner);
    drop(sender);
    drop(owner_lease);
    drop(sender_lease);
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, emulator.ports());
    let (sender, sender_lease) = service(&core.root.join("sender"), &core, emulator.ports());
    assert_eq!(owner.machine, owner_id);
    assert_eq!(sender.key.public(), sender_key);
    *emulator.owner.lock().unwrap() = Arc::downgrade(&owner);
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    sender.join(&token, |_| Ok(())).unwrap();
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    let credentials = crate::firebase_auth::FileStore::new(&sender.layout, PROJECT).unwrap();
    let cancel = AtomicBool::new(false);
    let mut client = crate::firebase_auth::Client::load(
        Project::from_public_config(&config()).unwrap(),
        &credentials,
        emulator.as_ref(),
        emulator.clock.as_ref(),
        emulator.as_ref(),
        &cancel,
    )
    .unwrap();
    client.revoke().unwrap();
    client.sign_in().unwrap();
    client.stop();
    drop(client);
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 3);
    assert_eq!(sender.key.public(), sender_key);
    assert_ne!(
        sender
            .store()
            .unwrap()
            .cloud_connection(&remote)
            .unwrap()
            .unwrap()
            .uid,
        old_uid
    );
    assert_eq!(
        emulator.raw(
            &sender_token,
            Operation::Read,
            &format!("/{path}/inbox/{}", crate::store::uuid_v4().unwrap()),
            Value::Null
        ),
        403,
        "old uid loses access"
    );
    owner.store().unwrap().revoke(&sender.machine).unwrap();
    owner.flush_routes().unwrap();
    assert_eq!(
        sender
            .send(&prepared(&remote), Some(&remote))
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_ACCESS_DENIED"
    );
    assert_eq!(core.sends(), 3);
    // Anonymous sign-in alone cannot squat a machine, even with a valid native key.
    let (victim, victim_lease) = service(&core.root.join("victim"), &core, emulator.ports());
    let attacker = sender.provider(&config(), &victim.machine, false).unwrap();
    assert_eq!(
        run(
            attacker.register_machine(&sender.key.public(), b"squatted", None, &sender.io()),
            &sender.io(),
        )
        .unwrap_err()
        .code,
        "REMOTE_CLOUD_ACCESS_DENIED"
    );
    assert!(
        run(attacker.machine_metadata(&sender.io()), &sender.io())
            .unwrap()
            .is_none()
    );
    emulator.seed_owner(&victim);
    let result = victim.invite_selected(PROJECT, config(), |event| {
        if event["event"] == "invite" {
            return Err(RemoteError::new("FIXTURE_STOP", "Stop after registration."));
        }
        Ok(None)
    });
    assert_eq!(result.unwrap_err().code, "FIXTURE_STOP");
    let row = run(attacker.machine_metadata(&sender.io()), &sender.io())
        .unwrap()
        .unwrap();
    assert_eq!(
        row["fields"]["authenticationKey"]["bytesValue"],
        STANDARD.encode(victim.key.public())
    );
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    eprintln!(
        "total production wire calls (direct Rules probes excluded): {:?}",
        emulator.calls.lock().unwrap()
    );
    drop(victim);
    drop(victim_lease);
    drop(owner);
    drop(sender);
    drop(owner_lease);
    drop(sender_lease);
}

#[test]
#[ignore = "Infra-approved Auth+Firestore emulator slot; no real project or fallback"]
fn invitation_only_rules_refuse_strangers_and_pin_inviter_uid_recovery() {
    use sha2::{Digest, Sha256};
    let emulator = Arc::new(Emulator::new());
    let core = core_fixture::Core::new();
    let (owner, _owner_lease) = service(&core.root.join("owner"), &core, emulator.ports());
    let (child, _child_lease) = service(&core.root.join("child"), &core, emulator.ports());
    let (stranger, _stranger_lease) = service(&core.root.join("stranger"), &core, emulator.ports());
    emulator.seed_owner(&owner);
    let owner_provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let child_provider = child.provider(&config(), &child.machine, true).unwrap();
    let stranger_provider = stranger
        .provider(&config(), &stranger.machine, true)
        .unwrap();
    let owner_token = emulator.token(&owner);
    let child_token = emulator.token(&child);
    let stranger_token = emulator.token(&stranger);
    let descriptor = owner
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    run(
        owner_provider.register_machine(&owner.key.public(), descriptor.bytes(), None, &owner.io()),
        &owner.io(),
    )
    .unwrap();
    let child_descriptor = child
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    assert_eq!(
        run(
            stranger_provider.register_machine(
                &stranger.key.public(),
                b"valid-bounded-bytes",
                None,
                &stranger.io()
            ),
            &stranger.io()
        )
        .unwrap_err()
        .code,
        "REMOTE_CLOUD_ACCESS_DENIED"
    );
    assert_eq!(
        run(
            child_provider.register_machine(
                &child.key.public(),
                child_descriptor.bytes(),
                Some(&owner.machine),
                &child.io()
            ),
            &child.io()
        )
        .unwrap_err()
        .code,
        "REMOTE_CLOUD_ACCESS_DENIED",
        "an inviter name alone grants nothing"
    );

    let token = [7; 32];
    let hash: [u8; 32] = Sha256::digest(token).into();
    let owner_path = format!("m/{}", owner.machine);
    let gate = json!({"tokenHash":{"bytesValue":STANDARD.encode(hash)}});
    for (path, fields) in [
        (
            format!("owners/{}", stranger_provider.uid()),
            json!({"machine":{"stringValue":stranger.machine}}),
        ),
        (
            format!("{owner_path}/invites/{}", crate::store::uuid_v4().unwrap()),
            json!({"tokenHash":{"bytesValue":STANDARD.encode(hash)},"expiresAt":{"timestampValue":"2035-01-01T00:00:00Z"}}),
        ),
        (
            format!("{owner_path}/reroutes/{}", child.machine),
            gate.clone(),
        ),
    ] {
        assert_eq!(emulator.commit(&stranger_token, &path, fields), 403);
    }

    let candidate = |kind: &str, uid: &str, token: Option<&[u8; 32]>, peer: Option<&str>| {
        let mut fields = json!({"uid":{"stringValue":uid},"kind":{"stringValue":kind},"body":{"bytesValue":STANDARD.encode(b"bounded-native-proof-fixture")}});
        if let Some(token) = token {
            fields["token"] = json!({"bytesValue":STANDARD.encode(token)});
        }
        if let Some(peer) = peer {
            fields["peer"] = json!({"stringValue":peer});
        }
        fields
    };
    let submit = |id: &str, fields: Value| {
        emulator.raw(&stranger_token, Operation::Commit, ":commit", json!({"writes":[{"update":{"name":format!("projects/{PROJECT}/databases/(default)/documents/{owner_path}/enroll/{id}"),"fields":fields},"currentDocument":{"exists":false},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}]}]}))
    };
    let offer = crate::store::uuid_v4().unwrap();
    assert_eq!(
        submit(
            &offer,
            candidate("join", stranger_provider.uid(), None, None)
        ),
        403
    );
    assert_eq!(
        submit(
            &offer,
            candidate("join", stranger_provider.uid(), Some(&token), None)
        ),
        403,
        "no invitation exists"
    );
    assert_eq!(emulator.commit(&owner_token, &format!("{owner_path}/invites/{offer}"), json!({"tokenHash":{"bytesValue":STANDARD.encode(hash)},"expiresAt":{"timestampValue":"2035-01-01T00:00:00Z"}})), 200);
    assert_eq!(
        submit(
            &offer,
            candidate("join", stranger_provider.uid(), Some(&[0; 32]), None)
        ),
        403
    );
    assert_eq!(
        submit(
            &offer,
            candidate("join", stranger_provider.uid(), Some(&token), None)
        ),
        200,
        "the exact capability passes Rules; native proof is still checked separately"
    );
    assert_eq!(
        submit(
            &offer,
            candidate("join", stranger_provider.uid(), Some(&token), None)
        ),
        409,
        "create-only invitation identity cannot be replayed"
    );
    let expired = crate::store::uuid_v4().unwrap();
    assert_eq!(emulator.commit(&owner_token, &format!("{owner_path}/invites/{expired}"), json!({"tokenHash":{"bytesValue":STANDARD.encode(hash)},"expiresAt":{"timestampValue":"2020-01-01T00:00:00Z"}})), 403, "the owner cannot publish an already-expired gate");
    // Seed a gate that expired while a process was stopped; no clock sleeps.
    assert_eq!(emulator.commit("owner", &format!("{owner_path}/invites/{expired}"), json!({"tokenHash":{"bytesValue":STANDARD.encode(hash)},"expiresAt":{"timestampValue":"2020-01-01T00:00:00Z"}})), 200);
    assert_eq!(
        submit(
            &expired,
            candidate("join", stranger_provider.uid(), Some(&token), None)
        ),
        403,
        "an existing expired gate cannot authorize enrollment"
    );
    let reroute_id = crate::store::uuid_v4().unwrap();
    assert_eq!(
        submit(
            &reroute_id,
            candidate(
                "reroute",
                stranger_provider.uid(),
                Some(&token),
                Some(&child.machine)
            )
        ),
        403
    );
    assert_eq!(
        emulator.commit(
            &owner_token,
            &format!("{owner_path}/reroutes/{}", child.machine),
            gate
        ),
        200
    );
    assert_eq!(
        submit(
            &reroute_id,
            candidate(
                "reroute",
                stranger_provider.uid(),
                Some(&[0; 32]),
                Some(&child.machine)
            )
        ),
        403
    );
    assert_eq!(
        submit(
            &reroute_id,
            candidate(
                "reroute",
                stranger_provider.uid(),
                Some(&token),
                Some(&child.machine)
            )
        ),
        200
    );

    let child_route = format!("{owner_path}/peers/{}", child_provider.uid());
    for (peer, expiry) in [
        (&stranger.machine, "2035-01-01T00:00:00Z"),
        (&child.machine, "2020-01-01T00:00:00Z"),
    ] {
        assert_eq!(emulator.commit(&owner_token, &child_route, json!({"peer":{"stringValue":peer},"revoked":{"booleanValue":false},"expiresAt":{"timestampValue":expiry}})), 200);
        assert_eq!(
            run(
                child_provider.register_machine(
                    &child.key.public(),
                    child_descriptor.bytes(),
                    Some(&owner.machine),
                    &child.io()
                ),
                &child.io()
            )
            .unwrap_err()
            .code,
            "REMOTE_CLOUD_ACCESS_DENIED",
            "a wrong-peer or expired route cannot create a child machine"
        );
    }
    let rows = [RouteChange {
        project: PROJECT.into(),
        uid: child_provider.uid().into(),
        peer: child.machine.clone(),
        generation: 1,
        revoked: false,
        reroute_hash: Some(hash),
    }];
    run(
        owner_provider.publish_routes(&rows, None, &owner.io()),
        &owner.io(),
    )
    .unwrap();
    run(
        child_provider.register_machine(
            &child.key.public(),
            child_descriptor.bytes(),
            Some(&owner.machine),
            &child.io(),
        ),
        &child.io(),
    )
    .unwrap();
    let move_uid = |token: &str, uid: &str, extra: Option<(&str, Value)>| {
        let mut fields = json!({"uid":{"stringValue":uid}});
        let mut mask = vec!["uid"];
        if let Some((key, value)) = extra {
            fields[key] = value;
            mask.push(key);
        }
        emulator.raw(token, Operation::Commit, ":commit", json!({"writes":[{"update":{"name":format!("projects/{PROJECT}/databases/(default)/documents/m/{}",child.machine),"fields":fields},"updateMask":{"fieldPaths":mask},"currentDocument":{"exists":true}}]}))
    };
    assert_eq!(
        move_uid(&stranger_token, stranger_provider.uid(), None),
        403
    );
    assert_eq!(move_uid(&child_token, stranger_provider.uid(), None), 403);
    assert_eq!(
        move_uid(&owner_token, stranger_provider.uid(), None),
        403,
        "a matching live route is required"
    );
    let mut next = rows.to_vec();
    next[0].revoked = true;
    next[0].generation = 2;
    next.push(RouteChange {
        project: PROJECT.into(),
        uid: stranger_provider.uid().into(),
        peer: child.machine.clone(),
        generation: 2,
        revoked: false,
        reroute_hash: Some(hash),
    });
    run(
        owner_provider.publish_routes(&next, Some(&child.machine), &owner.io()),
        &owner.io(),
    )
    .unwrap();
    let row = run(child_provider.machine_metadata(&child.io()), &child.io())
        .unwrap()
        .unwrap();
    assert_eq!(row["fields"]["uid"]["stringValue"], stranger_provider.uid());
    assert_eq!(
        row["fields"]["authenticationKey"]["bytesValue"],
        STANDARD.encode(child.key.public())
    );
    assert_eq!(
        row["fields"]["descriptor"]["bytesValue"],
        STANDARD.encode(child_descriptor.bytes())
    );
    assert_eq!(row["fields"]["invitingOwner"]["stringValue"], owner.machine);
    assert_eq!(
        move_uid(
            &owner_token,
            stranger_provider.uid(),
            Some((
                "authenticationKey",
                json!({"bytesValue":STANDARD.encode(owner.key.public())})
            ))
        ),
        403
    );
    assert_eq!(
        move_uid(
            &owner_token,
            stranger_provider.uid(),
            Some(("invitingOwner", json!({"stringValue":stranger.machine})))
        ),
        403
    );
    assert_eq!(
        move_uid(
            &owner_token,
            stranger_provider.uid(),
            Some((
                "descriptor",
                json!({"bytesValue":STANDARD.encode(b"changed")})
            ))
        ),
        403
    );
    for row in &mut next {
        row.revoked = true;
    }
    run(
        owner_provider.publish_routes(&next, None, &owner.io()),
        &owner.io(),
    )
    .unwrap();
    assert_eq!(
        move_uid(&owner_token, child_provider.uid(), None),
        403,
        "revoked routes cannot recover a child uid"
    );
    assert_eq!(
        submit(
            &crate::store::uuid_v4().unwrap(),
            candidate(
                "reroute",
                stranger_provider.uid(),
                Some(&token),
                Some(&child.machine)
            )
        ),
        403,
        "revocation deleted the capability"
    );
    assert_eq!(
        core.sends(),
        0,
        "Rules probes never dispatch a native effect"
    );
    eprintln!(
        "invitation-only Rules wire calls (direct probes included): {:?}",
        emulator.calls.lock().unwrap()
    );
}
