use super::*;
use crate::core_fixture;
use crate::{
    core::CloudIdentityQuery,
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, Request, Response, SystemClock,
        Time,
    },
    native_sync::projects::ProjectPorts,
    state::Serving,
};
use std::sync::{Weak, atomic::AtomicUsize, mpsc};
const PROJECT: &str = "demo-native-pairing";
const TARGET: &str = "66666666-6666-4666-8666-666666666666";
fn config() -> Value {
    json!({"projectId":PROJECT,"appId":"1:12345:web:fixture","apiKey":"public-test","authDomain":format!("{PROJECT}.firebaseapp.com")})
}
struct Identity;
impl CloudIdentityQuery for Identity {
    fn identity_show(&self, id: &str, name: &str, _: &AtomicBool) -> Result<(), RemoteError> {
        if id == TARGET && name == "target" {
            Ok(())
        } else {
            Err(wire::invalid())
        }
    }
}
struct Ports {
    wire: Arc<Wire>,
    clock: SystemClock,
}
impl Clock for Ports {
    fn now(&self) -> Time {
        self.clock.now()
    }
    fn wait_until(&self, until: u64, cancel: &dyn Cancellation) -> bool {
        self.clock.wait_until(until, cancel)
    }
}
impl Jitter for Ports {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
impl Http for Ports {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        self.wire.send(request, deadline, cancel)
    }
}
struct Wire {
    docs: Mutex<BTreeMap<String, Value>>,
    commits: Mutex<Vec<Value>>,
    queries: Mutex<Vec<Value>>,
    next: AtomicUsize,
    owner: Mutex<Weak<Service>>,
    fail_invite_delete: AtomicBool,
    lost_invite_delete_reply: AtomicBool,
    invite_answer: Mutex<Option<Value>>,
}
impl Wire {
    fn new() -> Self {
        Self {
            docs: Mutex::new(BTreeMap::new()),
            commits: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
            next: AtomicUsize::new(0),
            owner: Mutex::new(Weak::new()),
            fail_invite_delete: AtomicBool::new(false),
            lost_invite_delete_reply: AtomicBool::new(false),
            invite_answer: Mutex::new(None),
        }
    }
}
fn token(uid: &str) -> String {
    format!(
        "e30.{}.test",
        canonical::base64url(
            json!({"aud":PROJECT,"sub":uid,"firebase":{"sign_in_provider":"anonymous"}})
                .to_string()
                .as_bytes()
        )
    )
}
impl Http for Wire {
    fn send(&self, request: &Request, _: u64, _: &dyn Cancellation) -> Result<Response, AuthError> {
        let response = |status, body: Value| {
            Ok(Response {
                status,
                retry_after_ms: None,
                body: serde_json::to_vec(&body).unwrap(),
            })
        };
        match request.operation() {
            Operation::SignUp => {
                let uid = format!("uid{}", self.next.fetch_add(1, Ordering::SeqCst));
                return response(
                    200,
                    json!({"localId":uid,"idToken":token(&uid),"refreshToken":format!("refresh-{uid}"),"expiresIn":"3600"}),
                );
            }
            Operation::Refresh => {
                let raw = std::str::from_utf8(request.body()).unwrap();
                let uid = raw.split("refresh-").nth(1).unwrap();
                return response(
                    200,
                    json!({"user_id":uid,"project_id":"12345","id_token":token(uid),"refresh_token":format!("refresh-{uid}"),"expires_in":"3600"}),
                );
            }
            Operation::Delete => return response(200, json!({})),
            _ => {}
        }
        let name = request
            .url()
            .strip_prefix("https://firestore.googleapis.com/v1/")
            .unwrap();
        let now = crate::pairing::now_ms().unwrap();
        let stamp = crate::cloud_inbox::stamp(now);
        let mut docs = self.docs.lock().unwrap();
        if request.operation() == Operation::Read {
            return response(
                if docs.contains_key(name) { 200 } else { 404 },
                docs.get(name).cloned().unwrap_or(json!({})),
            );
        }
        let input: Value = serde_json::from_slice(request.body()).unwrap();
        if request.operation() == Operation::Query {
            self.queries.lock().unwrap().push(input.clone());
            let parent = name.strip_suffix(":runQuery").unwrap();
            let q = &input["structuredQuery"];
            let collection = q["from"][0]["collectionId"].as_str().unwrap();
            let prefix = format!("{parent}/{collection}/");
            if collection == "invites" {
                assert_eq!(q["limit"], 1);
                assert_eq!(q["from"][0]["allDescendants"], false);
                assert_eq!(q["where"]["fieldFilter"]["field"]["fieldPath"], "expiresAt");
                assert_eq!(q["where"]["fieldFilter"]["op"], "LESS_THAN_OR_EQUAL");
                assert_eq!(
                    q["orderBy"],
                    json!([{"field":{"fieldPath":"expiresAt"},"direction":"ASCENDING"}])
                );
                if let Some(answer) = self.invite_answer.lock().unwrap().as_ref() {
                    return response(200, answer.clone());
                }
                let cutoff = q["where"]["fieldFilter"]["value"]["timestampValue"]
                    .as_str()
                    .unwrap();
                let mut rows: Vec<_> = docs
                    .values()
                    .filter(|v| {
                        v["name"]
                            .as_str()
                            .unwrap()
                            .strip_prefix(&prefix)
                            .is_some_and(|id| !id.contains('/'))
                            && v["fields"]["expiresAt"]["timestampValue"].as_str().unwrap()
                                <= cutoff
                    })
                    .collect();
                rows.sort_by_key(|v| {
                    (
                        v["fields"]["expiresAt"]["timestampValue"].as_str().unwrap(),
                        v["name"].as_str().unwrap(),
                    )
                });
                return response(
                    200,
                    rows.first().map_or_else(
                        || json!([{"readTime":stamp}]),
                        |v| json!([{"document":v,"readTime":stamp}]),
                    ),
                );
            }
            let from = q["where"]["fieldFilter"]["value"]["timestampValue"]
                .as_str()
                .unwrap();
            let mut rows: Vec<_> = docs
                .values()
                .filter(|v| {
                    v["name"].as_str().unwrap().starts_with(&prefix)
                        && v["fields"]["createdAt"]["timestampValue"].as_str().unwrap() >= from
                })
                .cloned()
                .collect();
            rows.sort_by_key(|v| {
                (
                    v["fields"]["createdAt"]["timestampValue"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    v["name"].as_str().unwrap().to_owned(),
                )
            });
            if let Some(after) = q.get("startAt") {
                let time = after["values"][0]["timestampValue"].as_str().unwrap();
                let name = after["values"][1]["referenceValue"].as_str().unwrap();
                rows.retain(|v| {
                    (
                        v["fields"]["createdAt"]["timestampValue"].as_str().unwrap(),
                        v["name"].as_str().unwrap(),
                    ) > (time, name)
                });
            }
            let limit = q["limit"].as_u64().unwrap() as usize;
            let page: Vec<_> = rows
                .into_iter()
                .take(limit)
                .map(|document| json!({"document":document,"readTime":stamp}))
                .collect();
            return response(
                200,
                if page.is_empty() {
                    json!([{"readTime":stamp}])
                } else {
                    json!(page)
                },
            );
        }
        self.commits.lock().unwrap().push(input.clone());
        let writes = input["writes"].as_array().unwrap();
        if self.fail_invite_delete.load(Ordering::Acquire)
            && writes.iter().any(|write| {
                write["delete"]
                    .as_str()
                    .is_some_and(|name| name.contains("/invites/"))
            })
        {
            return response(503, json!({}));
        }
        for write in writes {
            if write.get("delete").is_some() {
                continue;
            }
            let name = write["update"]["name"].as_str().unwrap();
            if write["currentDocument"]["exists"] == false && docs.contains_key(name) {
                return response(409, json!({}));
            }
        }
        let mut notify = false;
        for write in writes {
            if let Some(name) = write["delete"].as_str() {
                docs.remove(name);
                continue;
            }
            let name = write["update"]["name"].as_str().unwrap();
            let mut doc = docs
                .get(name)
                .cloned()
                .unwrap_or(json!({"name":name,"fields":{},"createTime":stamp}));
            for (field, value) in write["update"]["fields"].as_object().unwrap() {
                doc["fields"][field] = value.clone();
            }
            if write.get("updateTransforms").is_some() {
                doc["fields"]["createdAt"] = json!({"timestampValue":stamp});
            }
            notify |= name.contains("/inbox/") || doc["fields"]["kind"]["stringValue"] == "reroute";
            docs.insert(name.into(), doc);
        }
        drop(docs);
        if self.lost_invite_delete_reply.swap(false, Ordering::AcqRel)
            && writes.iter().any(|w| {
                w["delete"]
                    .as_str()
                    .is_some_and(|n| n.contains("/invites/"))
            })
        {
            return response(503, json!({}));
        }
        if notify {
            let owner = self.owner.lock().unwrap().upgrade();
            if let Some(owner) = owner {
                owner.reconcile().unwrap();
            }
        }
        response(200, json!({"commitTime":stamp,"writeResults":[]}))
    }
}
fn service(
    root: &std::path::Path,
    core: &core_fixture::Core,
    ports: ProjectPorts,
) -> (Arc<Service>, Serving) {
    let layout = Layout::open(root).unwrap();
    let lease = layout.serve_lock().unwrap();
    let mut store = Store::open(&lease).unwrap();
    let machine = store.machine().unwrap();
    let key = MachineKey::open(&layout).unwrap();
    let store = Arc::new(Mutex::new(store));
    let keys = Arc::new(CloudKeys::new(
        layout.clone(),
        machine.id.clone(),
        key.clone(),
    ));
    keys.maintain(crate::pairing::now_ms().unwrap()).unwrap();
    let sessions = Arc::new(DoorSessions::new(
        machine.id.clone(),
        crate::store::uuid_v4().unwrap(),
        "http://127.0.0.1:1".into(),
        "/r/fixture/x/".into(),
        key.clone(),
        store.clone(),
        crate::session::IDLE,
    ));
    let operations = Arc::new(
        crate::operations::Operations::new(
            crate::core::CoreClient::at(core.root.join("tmt")).unwrap(),
            Arc::new(AtomicBool::new(false)),
            crate::limits::CLOUD_ENVELOPE_BYTES,
        )
        .with_cloud_identity(Arc::new(Identity)),
    );
    let messages = Arc::new(
        MessageService::new(sessions.clone(), crate::limits::CLOUD_ENVELOPE_BYTES)
            .with_operations(operations),
    );
    let projects = Arc::new(Projects::new(layout.clone(), Some(ports)));
    (
        Arc::new(Service::new(Owner {
            layout,
            store,
            key,
            machine: machine.id,
            keys,
            sessions,
            messages,
            projects,
            stop: Arc::new(AtomicBool::new(false)),
        })),
        lease,
    )
}
fn prepared(remote: &str) -> Vec<u8> {
    json!({"version":1,"remote":remote,"requestId":crate::store::uuid_v4().unwrap(),"createdAtMs":crate::pairing::now_ms().unwrap(),"from":{"id":"77777777-7777-4777-8777-777777777777","name":"sender"},"target":{"id":TARGET,"name":"target"},"message":"causal cloud pairing effect"}).to_string().into_bytes()
}

#[test]
fn inviter_registration_and_atomic_route_update_preserve_the_native_pin() {
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
    let (owner, _owner_lease) = service(&core.root.join("owner"), &core, make_ports());
    let (child, _child_lease) = service(&core.root.join("child"), &core, make_ports());
    let provider = child.provider(&config(), &child.machine, true).unwrap();
    let io = child.io();
    let descriptor = child
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    run(
        provider.register_machine(
            &child.key.public(),
            descriptor.bytes(),
            Some(&owner.machine),
            &io,
        ),
        &io,
    )
    .unwrap();
    let name = format!(
        "projects/{PROJECT}/databases/(default)/documents/m/{}",
        child.machine
    );
    let frozen = wire.docs.lock().unwrap()[&name]["fields"].clone();
    assert_eq!(frozen["invitingOwner"]["stringValue"], owner.machine);
    assert_eq!(
        frozen["authenticationKey"]["bytesValue"],
        STANDARD.encode(child.key.public())
    );
    let commits = wire.commits.lock().unwrap().len();
    assert!(
        run(
            provider.register_machine(&child.key.public(), descriptor.bytes(), None, &io),
            &io
        )
        .is_err()
    );
    assert!(
        run(
            provider.register_machine(
                &child.key.public(),
                descriptor.bytes(),
                Some(&child.machine),
                &io
            ),
            &io
        )
        .is_err()
    );
    assert_eq!(
        wire.commits.lock().unwrap().len(),
        commits,
        "provenance cannot be erased or replaced"
    );

    let owner_provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let rows = [
        RouteChange {
            project: PROJECT.into(),
            uid: provider.uid().into(),
            peer: child.machine.clone(),
            generation: 2,
            revoked: true,
            reroute_hash: None,
        },
        RouteChange {
            project: PROJECT.into(),
            uid: "renewed-uid".into(),
            peer: child.machine.clone(),
            generation: 2,
            revoked: false,
            reroute_hash: None,
        },
    ];
    run(
        owner_provider.publish_routes(&rows, Some(&child.machine), &io),
        &io,
    )
    .unwrap();
    let docs = wire.docs.lock().unwrap();
    let fields = &docs[&name]["fields"];
    assert_eq!(fields["uid"]["stringValue"], "renewed-uid");
    for field in ["authenticationKey", "descriptor", "invitingOwner"] {
        assert_eq!(fields[field], frozen[field]);
    }
    drop(docs);
    let commits = wire.commits.lock().unwrap();
    let writes = commits.last().unwrap()["writes"].as_array().unwrap();
    assert_eq!(
        writes.len(),
        3,
        "both routes and child uid must commit atomically"
    );
    assert_eq!(writes[2]["update"]["name"], name);
    assert_eq!(
        writes[2]["update"]["fields"],
        json!({"uid":{"stringValue":"renewed-uid"}})
    );
    assert_eq!(writes[2]["updateMask"]["fieldPaths"], json!(["uid"]));
    assert_eq!(writes[2]["currentDocument"]["exists"], true);
    let count = commits.len();
    drop(commits);
    assert!(
        run(
            owner_provider.publish_routes(&[], Some(&child.machine), &io),
            &io
        )
        .is_err()
    );
    assert!(
        run(
            owner_provider.publish_routes(&rows, Some(&owner.machine), &io),
            &io
        )
        .is_err()
    );
    assert!(
        run(
            owner_provider.publish_routes(&rows[..1], Some(&child.machine), &io),
            &io
        )
        .is_err()
    );
    let cap: Vec<_> = (0..crate::limits::CLOUD_ROUTE_BATCH)
        .map(|i| RouteChange {
            project: PROJECT.into(),
            uid: format!("old-{i}"),
            peer: child.machine.clone(),
            generation: 2,
            revoked: i != 0,
            reroute_hash: None,
        })
        .collect();
    assert!(
        run(
            owner_provider.publish_routes(&cap, Some(&child.machine), &io),
            &io
        )
        .is_err(),
        "the child write also counts toward the commit cap"
    );
    assert_eq!(
        wire.commits.lock().unwrap().len(),
        count,
        "invalid atomic requests do no provider write"
    );

    let mut gated = rows.clone();
    for row in &mut gated {
        row.reroute_hash = Some([7; 32]);
    }
    run(
        owner_provider.publish_routes(&gated, Some(&child.machine), &io),
        &io,
    )
    .unwrap();
    let gate_name = format!(
        "projects/{PROJECT}/databases/(default)/documents/m/{}/reroutes/{}",
        owner.machine, child.machine
    );
    assert_eq!(
        wire.docs.lock().unwrap()[&gate_name]["fields"]["tokenHash"]["bytesValue"],
        STANDARD.encode([7; 32])
    );
    assert_eq!(
        wire.commits.lock().unwrap().last().unwrap()["writes"]
            .as_array()
            .unwrap()
            .len(),
        4,
        "capability and child update share the route commit"
    );
    for row in &mut gated {
        row.revoked = true;
    }
    run(owner_provider.publish_routes(&gated, None, &io), &io).unwrap();
    assert!(!wire.docs.lock().unwrap().contains_key(&gate_name));
    assert_eq!(
        wire.commits.lock().unwrap().last().unwrap()["writes"][2]["delete"],
        gate_name
    );

    // A legacy owner registration remains the same three-field shape.
    let owner_descriptor = owner
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    run(
        owner_provider.register_machine(&owner.key.public(), owner_descriptor.bytes(), None, &io),
        &io,
    )
    .unwrap();
    let owner_name = format!(
        "projects/{PROJECT}/databases/(default)/documents/m/{}",
        owner.machine
    );
    assert_eq!(
        wire.docs.lock().unwrap()[&owner_name]["fields"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn route_batch_refuses_two_live_uids_without_a_provider_write() {
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let (owner, _lease) = service(
        &core.root.join("owner"),
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports,
        },
    );
    let provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let rows: Vec<_> = ["old-uid", "new-uid"]
        .into_iter()
        .map(|uid| RouteChange {
            project: PROJECT.into(),
            uid: uid.into(),
            peer: TARGET.into(),
            generation: 2,
            revoked: false,
            reroute_hash: None,
        })
        .collect();
    assert!(
        run(
            provider.publish_routes(&rows, None, &owner.io()),
            &owner.io()
        )
        .is_err()
    );
    assert!(
        wire.commits.lock().unwrap().is_empty(),
        "invalid routes must refuse before the provider write"
    );
}

#[test]
fn invite_gate_is_published_before_the_code_and_removed_when_the_ceremony_ends() {
    for cleanup_fails in [false, true] {
        let core = core_fixture::Core::new();
        let wire = Arc::new(Wire::new());
        wire.fail_invite_delete
            .store(cleanup_fails, Ordering::Release);
        let ports = Arc::new(Ports {
            wire: wire.clone(),
            clock: SystemClock::default(),
        });
        let (owner, _lease) = service(
            &core.root.join("owner"),
            &core,
            ProjectPorts {
                http: ports.clone(),
                clock: ports.clone(),
                jitter: ports,
            },
        );
        let mut gate = String::new();
        let result = owner.invite_selected(PROJECT, config(), |event| {
            if event["event"] == "invite" {
                let invite = Invite::read(
                    event["code"].as_str().unwrap(),
                    crate::pairing::now_ms().unwrap(),
                )
                .unwrap();
                gate = format!(
                    "projects/{PROJECT}/databases/(default)/documents/m/{}/invites/{}",
                    owner.machine, invite.challenge.offer,
                );
                let docs = wire.docs.lock().unwrap();
                let row = docs
                    .get(&gate)
                    .expect("publish the Rules invite gate before exposing the code");
                assert_eq!(row["fields"].as_object().unwrap().len(), 2);
                assert_eq!(
                    row["fields"]["expiresAt"]["timestampValue"],
                    crate::cloud_inbox::stamp(invite.challenge.expires_at_ms),
                );
                let hash = STANDARD
                    .decode(row["fields"]["tokenHash"]["bytesValue"].as_str().unwrap())
                    .unwrap();
                assert_eq!(hash.len(), 32);
                return Err(RemoteError::new(
                    "FIXTURE_CANCEL",
                    "Stop after observing the invite.",
                ));
            }
            Ok(None)
        });
        assert_eq!(result.unwrap_err().code, "FIXTURE_CANCEL");
        assert!(!gate.is_empty());
        assert_eq!(wire.docs.lock().unwrap().contains_key(&gate), cleanup_fails);
        assert!(owner.store().unwrap().grants().unwrap().is_empty());
        assert_eq!(core.sends(), 0);
    }
}

#[test]
fn native_ceremony_requires_owner_confirmation_and_read_only_default_then_keeps_pairing_on_reopen()
{
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::native_ceremony_requires_owner_confirmation_and_read_only_default_then_keeps_pairing_on_reopen",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    std::fs::write(core.root.join("multi"), "").unwrap();
    let wire = Arc::new(Wire::new());
    wire.fail_invite_delete.store(true, Ordering::Release);
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
    let (sender, sender_lease) = service(&core.root.join("sender"), &core, make_ports());
    let (token_tx, token_rx) = mpsc::channel();
    let (words_tx, words_rx) = mpsc::channel();
    let (choice_tx, choice_rx) = mpsc::channel();
    let mut remote = String::new();
    let mut token_text = String::new();
    std::thread::scope(|scope| {
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
        let owner_clone = owner.clone();
        let invite = scope.spawn(move || {
            owner_clone.invite_selected(PROJECT, config(), |event| {
                if event["event"] == "invite" {
                    token_tx
                        .send(event["code"].as_str().unwrap().to_owned())
                        .unwrap();
                }
                if event["event"] == "candidate" {
                    words_tx.send(event["words"].clone()).unwrap();
                    return Ok(Some(
                        choice_rx.recv_timeout(Duration::from_secs(15)).unwrap(),
                    ));
                }
                Ok(None)
            })
        });
        token_text = token_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let token = token_text.clone();
        let sender_clone = sender.clone();
        let join = scope.spawn(move || {
            let mut events = vec![];
            sender_clone
                .join(&token, |event| {
                    events.push(event);
                    Ok(())
                })
                .map(|()| events)
        });
        let words = words_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(owner.store().unwrap().grants().unwrap().is_empty());
        assert_eq!(core.sends(), 0);
        choice_tx.send((true, false)).unwrap();
        invite.join().unwrap().unwrap();
        assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
        assert!(
            wire.docs
                .lock()
                .unwrap()
                .keys()
                .any(|name| name.contains("/invites/")),
            "the injected cleanup failure must leave the gate, not erase the committed ceremony"
        );
        wire.fail_invite_delete.store(false, Ordering::Release);
        let events = join.join().unwrap().unwrap();
        assert_eq!(
            events.iter().find(|v| v["event"] == "joining").unwrap()["words"],
            words
        );
        let connected = events.iter().find(|v| v["event"] == "connected").unwrap();
        assert_eq!(connected["talk"], false);
        remote = connected["remote"].as_str().unwrap().into();
    });
    // A different offer cannot replace or downgrade a saved confirmed connection.
    let saved = serde_json::to_value(
        sender
            .store()
            .unwrap()
            .cloud_connection(&remote)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let different = Invite::new(
        PROJECT,
        &owner.machine,
        &owner.key.public(),
        config(),
        "Owner",
        crate::pairing::now_ms().unwrap(),
    )
    .unwrap();
    assert!(
        sender
            .join(&different.token().unwrap(), |_| Ok(()))
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(
            sender
                .store()
                .unwrap()
                .cloud_connection(&remote)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        saved
    );
    assert_eq!(wire.next.load(Ordering::SeqCst), 2);
    *wire.owner.lock().unwrap() = Arc::downgrade(&owner);
    let answer = sender.send(&prepared(&remote), Some(&remote)).unwrap();
    assert_eq!(answer["state"], "refused");
    assert_eq!(core.sends(), 0);
    let peer = sender.machine.clone();
    owner.store().unwrap().talk(&peer, true).unwrap();
    let answer = sender.send(&prepared(&remote), Some(&remote)).unwrap();
    assert_eq!(answer["state"], "sent");
    assert_eq!(core.sends(), 1);
    // Reopening both owners retains native keys, grants, credentials and remote pins.
    let owner_id = owner.machine.clone();
    let sender_id = sender.machine.clone();
    drop(owner);
    drop(sender);
    drop(owner_lease);
    drop(sender_lease);
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, make_ports());
    let (sender, sender_lease) = service(&core.root.join("sender"), &core, make_ports());
    assert_eq!(owner.machine, owner_id);
    assert_eq!(sender.machine, sender_id);
    assert_eq!(wire.next.load(Ordering::SeqCst), 2);
    *wire.owner.lock().unwrap() = Arc::downgrade(&owner);
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 2);
    // An exact lost-join observation returns its original receipt, creating no grant.
    sender.join(&token_text, |_| Ok(())).unwrap();
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    let old_uid = sender
        .store()
        .unwrap()
        .cloud_connection(&remote)
        .unwrap()
        .unwrap()
        .uid;
    let sender_key = sender.key.public();
    let layout = Layout::open(&core.root.join("sender")).unwrap();
    let credentials = crate::firebase_auth::FileStore::new(&layout, PROJECT).unwrap();
    let cancel = AtomicBool::new(false);
    let mut client = crate::firebase_auth::Client::load(
        crate::firebase_auth::Project::from_public_config(&config()).unwrap(),
        &credentials,
        ports.as_ref(),
        ports.as_ref(),
        ports.as_ref(),
        &cancel,
    )
    .unwrap();
    client.revoke().unwrap();
    client.sign_in().unwrap();
    client.stop();
    drop(client);
    assert_eq!(sender.key.public(), sender_key);
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 3);
    let current_uid = sender
        .store()
        .unwrap()
        .cloud_connection(&remote)
        .unwrap()
        .unwrap()
        .uid;
    assert_ne!(current_uid, old_uid);
    let docs = wire.docs.lock().unwrap();
    assert!(
        docs.iter()
            .find(|(name, _)| name.ends_with(&format!("/peers/{old_uid}")))
            .unwrap()
            .1["fields"]["revoked"]["booleanValue"]
            == true
    );
    assert!(
        docs.iter()
            .find(|(name, _)| name.ends_with(&format!("/peers/{current_uid}")))
            .unwrap()
            .1["fields"]["revoked"]["booleanValue"]
            == false
    );
    drop(docs);
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    owner.store().unwrap().revoke(&sender.machine).unwrap();
    owner.flush_routes().unwrap();
    assert!(
        wire.docs
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.contains("/peers/"))
            .all(|(_, v)| v["fields"]["revoked"]["booleanValue"] == true)
    );
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "refused"
    );
    assert_eq!(core.sends(), 3);
    drop(owner);
    drop(sender);
    drop(owner_lease);
    drop(sender_lease);
}

#[cfg(feature = "test-emulator")]
#[path = "service_emulator_tests.rs"]
mod emulator;

fn saved_gate(machine: &str, id: &str, expires: u64) -> Value {
    json!({"name":format!("projects/{PROJECT}/databases/(default)/documents/m/{machine}/invites/{id}"),"fields":{
        "tokenHash":{"bytesValue":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,[7;32])},
        "expiresAt":{"timestampValue":crate::cloud_inbox::stamp(expires)}
    }})
}

#[test]
fn expired_invite_gates_are_collected_after_reopen_without_repeating_deletes() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::expired_invite_gates_are_collected_after_reopen_without_repeating_deletes",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let root = core.root.join("owner");
    let (owner, lease) = service(
        &root,
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports.clone(),
        },
    );
    let machine = owner.machine.clone();
    let now = crate::pairing::now_ms().unwrap();
    let mut names = Vec::new();
    for (machine, expires) in [
        (machine.as_str(), now - 1),
        (machine.as_str(), now),
        (machine.as_str(), now + 60_000),
        (TARGET, now - 1),
    ] {
        let doc = saved_gate(machine, &crate::store::uuid_v4().unwrap(), expires);
        let name = doc["name"].as_str().unwrap().to_owned();
        names.push(name.clone());
        wire.docs.lock().unwrap().insert(name, doc);
    }
    drop(owner);
    drop(lease);
    let (owner, _lease) = service(
        &root,
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports.clone(),
        },
    );
    assert_eq!(owner.machine, machine);
    let provider = owner.provider(&config(), &machine, true).unwrap();
    owner
        .cleanup_enrollment(&provider, now, &owner.io())
        .unwrap();
    let docs = wire.docs.lock().unwrap();
    assert!(
        !docs.contains_key(&names[0]),
        "a crashed invite's expired gate must be collected"
    );
    assert!(
        !docs.contains_key(&names[1]),
        "expiry is inclusive at the pass boundary"
    );
    assert!(
        docs.contains_key(&names[2]),
        "the live invite stays authoritative"
    );
    assert!(
        docs.contains_key(&names[3]),
        "another machine is never scanned"
    );
    drop(docs);
    let queries = wire.queries.lock().unwrap().len();
    owner
        .cleanup_enrollment(&provider, now, &owner.io())
        .unwrap();
    assert_eq!(
        wire.queries.lock().unwrap().len(),
        queries,
        "the existing hourly gate suppresses another scan"
    );
    owner.cleanup.lock().unwrap().clear();
    owner
        .cleanup_enrollment(&provider, now, &owner.io())
        .unwrap();
    let deletes: Vec<_> = wire
        .commits
        .lock()
        .unwrap()
        .iter()
        .flat_map(|v| v["writes"].as_array().unwrap().clone())
        .filter_map(|v| v["delete"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        deletes.len(),
        2,
        "a repeated pass never deletes an already absent gate"
    );
    assert!(owner.store().unwrap().active_grants().unwrap().is_empty());
}

#[test]
fn expired_invite_cleanup_is_bounded_and_leaves_the_backlog_for_the_next_pass() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::expired_invite_cleanup_is_bounded_and_leaves_the_backlog_for_the_next_pass",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let (owner, _lease) = service(
        &core.root.join("owner"),
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports.clone(),
        },
    );
    let provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let now = crate::pairing::now_ms().unwrap();
    for i in 0..=crate::journal::OPERATIONS {
        let doc = saved_gate(
            &owner.machine,
            &format!("{i:08x}-0000-4000-8000-000000000000"),
            now - 1,
        );
        wire.docs
            .lock()
            .unwrap()
            .insert(doc["name"].as_str().unwrap().into(), doc);
    }
    let result = owner.cleanup_enrollment(&provider, now, &owner.io());
    assert!(
        result.is_ok(),
        "{result:?}; queries={}, commits={}, remaining={}, signups={}",
        wire.queries.lock().unwrap().len(),
        wire.commits.lock().unwrap().len(),
        wire.docs.lock().unwrap().len(),
        wire.next.load(Ordering::Acquire)
    );
    assert_eq!(
        wire.docs.lock().unwrap().len(),
        1,
        "one pass deletes at most the existing operation cap"
    );
    assert_eq!(
        wire.queries
            .lock()
            .unwrap()
            .iter()
            .filter(|q| q["structuredQuery"]["from"][0]["collectionId"] == "invites")
            .count(),
        crate::journal::OPERATIONS as usize
    );
    owner.cleanup.lock().unwrap().clear();
    owner
        .cleanup_enrollment(&provider, now, &owner.io())
        .unwrap();
    assert!(wire.docs.lock().unwrap().is_empty());
    assert_eq!(
        wire.commits.lock().unwrap().len(),
        crate::journal::OPERATIONS as usize + 1
    );
}

#[test]
fn failed_invite_cleanup_preserves_expiry_and_observes_a_lost_delete_before_another_mutation() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::failed_invite_cleanup_preserves_expiry_and_observes_a_lost_delete_before_another_mutation",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let (owner, _lease) = service(
        &core.root.join("owner"),
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports.clone(),
        },
    );
    let provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let now = crate::pairing::now_ms().unwrap();
    let gate = saved_gate(&owner.machine, &crate::store::uuid_v4().unwrap(), now - 1);
    let name = gate["name"].as_str().unwrap().to_owned();
    wire.docs.lock().unwrap().insert(name.clone(), gate);
    wire.fail_invite_delete.store(true, Ordering::Release);
    assert!(
        owner
            .cleanup_enrollment(&provider, now, &owner.io())
            .is_err()
    );
    assert!(wire.docs.lock().unwrap().contains_key(&name));
    assert!(
        owner.cleanup.lock().unwrap().is_empty(),
        "a failed pass does not record completion"
    );
    wire.fail_invite_delete.store(false, Ordering::Release);
    wire.lost_invite_delete_reply.store(true, Ordering::Release);
    assert!(
        owner
            .cleanup_enrollment(&provider, now, &owner.io())
            .is_err()
    );
    assert!(
        !wire.docs.lock().unwrap().contains_key(&name),
        "the delete effect preceded its missing confirmation"
    );
    let count = wire.commits.lock().unwrap().len();
    owner
        .cleanup_enrollment(&provider, now, &owner.io())
        .unwrap();
    assert_eq!(
        wire.commits.lock().unwrap().len(),
        count,
        "observation, never a blind repeat of the delete"
    );
    assert!(owner.store().unwrap().active_grants().unwrap().is_empty());
}

#[test]
fn malformed_invite_query_evidence_never_selects_a_delete() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::malformed_invite_query_evidence_never_selects_a_delete",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let (owner, _lease) = service(
        &core.root.join("owner"),
        &core,
        ProjectPorts {
            http: ports.clone(),
            clock: ports.clone(),
            jitter: ports.clone(),
        },
    );
    let provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let now = crate::pairing::now_ms().unwrap();
    let valid = saved_gate(&owner.machine, &crate::store::uuid_v4().unwrap(), now - 1);
    let mut cases = Vec::new();
    let mut foreign = valid.clone();
    foreign["name"] =
        saved_gate(TARGET, &crate::store::uuid_v4().unwrap(), now - 1)["name"].clone();
    cases.push(json!([{"document":foreign}]));
    let mut future = valid.clone();
    future["fields"]["expiresAt"]["timestampValue"] = crate::cloud_inbox::stamp(now + 1).into();
    cases.push(json!([{"document":future}]));
    let mut fields = valid.clone();
    fields["fields"]["extra"] = json!({"booleanValue":true});
    cases.push(json!([{"document":fields}]));
    let mut hash = valid.clone();
    hash["fields"]["tokenHash"] = json!({"bytesValue":"AA=="});
    cases.push(json!([{"document":hash}]));
    cases.push(json!([{"document":valid.clone()},{"document":valid.clone()}]));
    let mut nested = valid.clone();
    nested["name"] = format!("{}/child", valid["name"].as_str().unwrap()).into();
    cases.push(json!([{"document":nested}]));
    for answer in cases {
        *wire.invite_answer.lock().unwrap() = Some(answer);
        assert!(
            owner
                .cleanup_enrollment(&provider, now, &owner.io())
                .is_err()
        );
        assert!(wire.commits.lock().unwrap().is_empty());
        assert!(owner.cleanup.lock().unwrap().is_empty());
    }
}

#[test]
fn next_explicit_invite_collects_a_killed_first_invite_best_effort_without_peer_polling() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::next_explicit_invite_collects_a_killed_first_invite_best_effort_without_peer_polling",
    ) {
        return;
    }
    for cleanup_fails in [false, true] {
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
        let root = core.root.join("owner");
        let (owner, lease) = service(&root, &core, make_ports());
        wire.fail_invite_delete.store(true, Ordering::Release);
        assert_eq!(
            owner
                .invite_selected(PROJECT, config(), |event| {
                    assert_eq!(event["event"], "invite");
                    Err(RemoteError::new(
                        "FIXTURE_CANCEL",
                        "Stop before a peer confirms.",
                    ))
                })
                .unwrap_err()
                .code,
            "FIXTURE_CANCEL"
        );
        assert!(
            owner
                .store()
                .unwrap()
                .cloud_owner_configs()
                .unwrap()
                .is_empty()
        );
        assert!(owner.store().unwrap().active_grants().unwrap().is_empty());
        let old_gate = {
            let mut docs = wire.docs.lock().unwrap();
            let (name, doc) = docs
                .iter_mut()
                .find(|(name, _)| name.contains("/invites/"))
                .unwrap();
            // The gate expired while the process was stopped, without a clock sleep.
            doc["fields"]["expiresAt"]["timestampValue"] =
                crate::cloud_inbox::stamp(crate::pairing::now_ms().unwrap() - 1).into();
            name.clone()
        };
        drop(owner);
        drop(lease);
        let signups = wire.next.load(Ordering::Acquire);
        let (owner, _lease) = service(&root, &core, make_ports());
        let queries = wire.queries.lock().unwrap().len();
        let commits = wire.commits.lock().unwrap().len();
        owner.reconcile().unwrap();
        assert_eq!(
            wire.queries.lock().unwrap().len(),
            queries,
            "no enrolled peer means no provider poll"
        );
        assert_eq!(wire.commits.lock().unwrap().len(), commits);
        assert!(
            wire.docs.lock().unwrap().contains_key(&old_gate),
            "expiry is authoritative even if the owner never invites again"
        );
        wire.fail_invite_delete
            .store(cleanup_fails, Ordering::Release);
        let mut emitted = false;
        let result = owner.invite_selected(PROJECT, config(), |event| {
            assert_eq!(event["event"], "invite");
            emitted = true;
            assert_eq!(
                wire.docs.lock().unwrap().contains_key(&old_gate),
                cleanup_fails,
                "the next explicit invite collects a stale first gate before exposing its new code"
            );
            Err(RemoteError::new(
                "FIXTURE_CANCEL",
                "Cancel the new ceremony after its code is ready.",
            ))
        });
        assert_eq!(
            result.unwrap_err().code,
            "FIXTURE_CANCEL",
            "cleanup never replaces the new ceremony result"
        );
        assert!(emitted, "cleanup failure never blocks the new invitation");
        assert_eq!(
            wire.next.load(Ordering::Acquire),
            signups,
            "the saved sign-in survives reopen"
        );
        let attempts = wire.commits.lock().unwrap()[commits..]
            .iter()
            .flat_map(|v| v["writes"].as_array().unwrap())
            .filter(|w| w["delete"] == old_gate)
            .count();
        assert_eq!(
            attempts, 1,
            "no retry of a failed stale-gate deletion in this invocation"
        );
        assert!(owner.store().unwrap().active_grants().unwrap().is_empty());
    }
}
