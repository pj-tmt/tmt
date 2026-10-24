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
    // Public-sign-in storage limit: an attacker can claim an unregistered random id
    // under its own uid, but the legitimate native owner refuses and never overwrites.
    let (victim, victim_lease) = service(&core.root.join("victim"), &core, emulator.ports());
    let attacker = sender.provider(&config(), &victim.machine, false).unwrap();
    run(
        attacker.register_machine(&sender.key.public(), b"squatted", &sender.io()),
        &sender.io(),
    )
    .unwrap();
    let result = victim.invite_selected(PROJECT, config(), |_| {
        panic!("squatted row must refuse before code output")
    });
    assert_eq!(result.unwrap_err().code, "REMOTE_CLOUD_MACHINE_CONFLICT");
    let row = run(attacker.machine_metadata(&sender.io()), &sender.io())
        .unwrap()
        .unwrap();
    assert_eq!(
        row["fields"]["authenticationKey"]["bytesValue"],
        STANDARD.encode(sender.key.public())
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
