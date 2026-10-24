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
    next: AtomicUsize,
    owner: Mutex<Weak<Service>>,
}
impl Wire {
    fn new() -> Self {
        Self {
            docs: Mutex::new(BTreeMap::new()),
            next: AtomicUsize::new(0),
            owner: Mutex::new(Weak::new()),
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
            let parent = name.strip_suffix(":runQuery").unwrap();
            let q = &input["structuredQuery"];
            let collection = q["from"][0]["collectionId"].as_str().unwrap();
            let prefix = format!("{parent}/{collection}/");
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
            return response(
                200,
                rows.first().map_or_else(
                    || json!([{"readTime":stamp}]),
                    |v| json!([{"document":v,"readTime":stamp}]),
                ),
            );
        }
        let writes = input["writes"].as_array().unwrap();
        for write in writes {
            let name = write["update"]["name"].as_str().unwrap();
            if write["currentDocument"]["exists"] == false && docs.contains_key(name) {
                return response(409, json!({}));
            }
        }
        let mut notify = false;
        for write in writes {
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
