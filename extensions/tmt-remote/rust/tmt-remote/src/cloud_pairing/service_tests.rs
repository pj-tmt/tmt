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
#[test]
fn serving_sender_preserves_a_typed_owner_refusal_even_with_a_prior_claim() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::serving_sender_preserves_a_typed_owner_refusal_even_with_a_prior_claim",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let root = core.root.join("sender");
    let make_ports = || ProjectPorts {
        http: ports.clone(),
        clock: ports.clone(),
        jitter: ports.clone(),
    };
    let (sender, lease) = service(&root, &core, make_ports());
    let raw = prepared("missing");
    let request = crate::cloud_command::Request::read(&raw).unwrap();
    assert_eq!(
        sender.send(&raw, None).unwrap_err().code,
        "REMOTE_CLOUD_UNAVAILABLE"
    );
    sender
        .store()
        .unwrap()
        .cloud_freeze_outgoing(
            &sender.machine,
            &request.request_id,
            b"frozen-intent",
            b"frozen-carrier",
            request.created_at_ms,
        )
        .unwrap();
    assert_eq!(
        sender.send(&raw, None).unwrap_err().code,
        "REMOTE_CLOUD_UNAVAILABLE",
        "unclaimed bytes prove no mutation was attempted"
    );
    assert!(
        sender
            .store()
            .unwrap()
            .cloud_claim_outgoing(&sender.machine, &request.request_id)
            .unwrap()
    );
    assert_eq!(
        sender.send(&raw, None).unwrap_err().code,
        "REMOTE_CLOUD_UNAVAILABLE",
        "the Service must preserve the typed sender-preparation refusal"
    );
    drop(sender);
    drop(lease);
    let (sender, _lease) = service(&root, &core, make_ports());
    assert_eq!(
        sender.send(&raw, None).unwrap_err().code,
        "REMOTE_CLOUD_UNAVAILABLE",
        "restart must not turn a typed owner refusal into blanket uncertainty"
    );
    assert!(wire.docs.lock().unwrap().is_empty());
    assert!(wire.commits.lock().unwrap().is_empty());
    assert_eq!(
        wire.next.load(Ordering::SeqCst),
        0,
        "no sign-in or credential bootstrap"
    );
    assert_eq!(core.sends(), 0);
}
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
    reads: Mutex<Vec<String>>,
    next: AtomicUsize,
    receivers: Mutex<BTreeMap<String, Weak<Service>>>,
    fail_invite_delete: AtomicBool,
}
impl Wire {
    fn new() -> Self {
        Self {
            docs: Mutex::new(BTreeMap::new()),
            commits: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            next: AtomicUsize::new(0),
            receivers: Mutex::new(BTreeMap::new()),
            fail_invite_delete: AtomicBool::new(false),
        }
    }
    fn route(&self, service: &Arc<Service>) {
        self.receivers
            .lock()
            .unwrap()
            .insert(service.machine.clone(), Arc::downgrade(service));
    }
}
/// Synchronous fixture notification models the worker: a per-operation diagnostic does not
/// turn a successful provider commit into a lost HTTP answer. Shared failures still fail the test.
fn reconcile_notified(service: &Service) {
    let outcome = service.reconcile();
    *service.last_error.lock().unwrap() = outcome.as_ref().err().map(|error| error.code.clone());
    if let Err(error) = outcome {
        assert!(
            crate::cloud_inbox::permanent(&error.code),
            "unexpected reconciliation failure: {}",
            error.code
        );
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
            self.reads.lock().unwrap().push(name.to_owned());
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
            // Fixture-only Rules gate. Native signature/generation admission is
            // still exercised by the real receiving Service below.
            if write["update"]["fields"]["kind"]["stringValue"] == "reroute" {
                use sha2::{Digest, Sha256};
                let parent = name.split("/enroll/").next().unwrap();
                let fields = &write["update"]["fields"];
                let peer = fields["peer"]["stringValue"].as_str().unwrap();
                let token = STANDARD
                    .decode(fields["token"]["bytesValue"].as_str().unwrap())
                    .unwrap();
                let expected = docs
                    .get(&format!("{parent}/reroutes/{peer}"))
                    .map(|row| row["fields"]["tokenHash"]["bytesValue"].clone());
                if expected != Some(json!(STANDARD.encode(Sha256::digest(&token)))) {
                    return response(403, json!({}));
                }
            }
            if write["currentDocument"]["exists"] == false && docs.contains_key(name) {
                return response(409, json!({}));
            }
        }
        let mut notify = std::collections::BTreeSet::new();
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
            if name.contains("/inbox/") || doc["fields"]["kind"]["stringValue"] == "reroute" {
                notify.insert(
                    name.split("/m/")
                        .nth(1)
                        .unwrap()
                        .split('/')
                        .next()
                        .unwrap()
                        .to_owned(),
                );
            }
            docs.insert(name.into(), doc);
        }
        drop(docs);
        for machine in notify {
            let receiver = self
                .receivers
                .lock()
                .unwrap()
                .get(&machine)
                .and_then(Weak::upgrade);
            if let Some(receiver) = receiver {
                reconcile_notified(&receiver);
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
    service_with_identity(root, core, ports, Arc::new(Identity))
}
fn service_with_identity(
    root: &std::path::Path,
    core: &core_fixture::Core,
    ports: ProjectPorts,
    identity: Arc<dyn CloudIdentityQuery>,
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
        .with_cloud_identity(identity),
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
    let operation = crate::store::uuid_v4().unwrap();
    json!({"version":1,"remote":remote,"requestId":operation,"createdAtMs":crate::pairing::now_ms().unwrap(),"from":{"id":"77777777-7777-4777-8777-777777777777","name":"sender"},"target":{"id":TARGET,"name":"target"},"message":"causal cloud pairing effect","reply":{"requestId":format!("req_{operation}"),"receipt":"PRIVATE_PAIRING_RECEIPT"}}).to_string().into_bytes()
}

fn message_control(sender: &Arc<Service>, lease: &Serving) -> crate::control::Control {
    let pairing = Arc::new(crate::pairing::Pairing::new(
        sender.machine.clone(),
        crate::store::uuid_v4().unwrap(),
        sender.key.public(),
        "http://127.0.0.1:1".into(),
        sender.store.clone(),
        crate::pairing::Timing::CONTRACT,
    ));
    let devices = Arc::new(crate::devices::Devices::new(
        sender.store.clone(),
        Some(sender.sessions.clone()),
    ));
    crate::control::Control::start_with_views(
        lease,
        pairing,
        devices,
        crate::control::Door {
            origin: "http://127.0.0.1:1".into(),
            prefix: "/r/fixture/".into(),
        },
        None,
        sender.stop.clone(),
        crate::control::StatusViews {
            cloud: Some(sender.clone()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// The maintained native sandbox owns CLI ancestry, private Core state and child cleanup.
fn public_message(
    core: &core_fixture::Core,
    state: &str,
    capture: &std::path::Path,
    peer: &str,
) -> (Vec<u8>, Value) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let node = tmt_invoke::find_executable(
        std::ffi::OsStr::new("node"),
        &std::env::var_os("PATH").unwrap(),
    )
    .expect("pinned Node must be available");
    let script = root.join(
        "extensions/tmt-remote/rust/tmt-remote/tests/fixtures/cloud-message-core-acceptance.mjs",
    );
    let input=json!({"remoteExecutable":root.join("rust/target/debug/tmt-remote"),"peer":peer,"discovery":core.root.join("tmt"),"capture":capture,"target":TARGET,"state":state}).to_string();
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &node,
            args: &[script.into_os_string()],
            input: input.as_bytes(),
            deadline: Instant::now() + Duration::from_secs(120),
            max_stream_bytes: 65536,
            launch: tmt_invoke::LaunchOptions::default(),
        },
        None,
    )
    .expect("bounded public Core proof and confirmed cleanup");
    assert!(
        output.status.success(),
        "public Core proof failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value =
        serde_json::from_slice(&output.stdout).expect("one content-free proof summary");
    (std::fs::read(capture).unwrap(), summary)
}

#[test]
#[ignore = "explicit built Core/Remote public-command acceptance; offline HTTP fixture"]
fn public_core_message_reaches_the_serving_owner_and_a_lost_answer_stays_uncertain() {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::{UnixListener, UnixStream},
    };
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
    let (sender, sender_lease) = service(&core.root.join("sender"), &core, make_ports());
    let owner_provider = owner.provider(&config(), &owner.machine, true).unwrap();
    let sender_provider = sender.provider(&config(), &owner.machine, true).unwrap();
    let descriptor = owner
        .keys
        .advertised(crate::pairing::now_ms().unwrap())
        .unwrap();
    run(
        owner_provider.register_machine(&owner.key.public(), descriptor.bytes(), None, &owner.io()),
        &owner.io(),
    )
    .unwrap();
    owner
        .store()
        .unwrap()
        .insert_grant(&crate::store::Grant {
            client_id: sender.machine.clone(),
            public_key: sender.key.public(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Sender".into(),
            agents: "all".into(),
            scopes: vec!["results.read".into(), "talk".into()],
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        })
        .unwrap();
    owner
        .store()
        .unwrap()
        .remember_cloud_owner(PROJECT, &config())
        .unwrap();
    owner.store().unwrap().connection.execute("INSERT INTO cloud_peers(project,client_id,owner,owner_key,generation) VALUES(?1,?2,?3,?4,1)",rusqlite::params![PROJECT,sender.machine,owner.machine,owner.key.public().as_slice()]).unwrap();
    sender
        .store()
        .unwrap()
        .save_cloud_connection(&Connection {
            version: 1,
            name: "peer".into(),
            owner: owner.machine.clone(),
            owner_key: owner.key.public(),
            config: config(),
            peer: sender.machine.clone(),
            uid: sender_provider.uid().into(),
            request: vec![],
            confirmed: true,
            descriptor: descriptor.bytes().to_vec(),
            reroute_token: None,
        })
        .unwrap();
    wire.route(&owner);
    let control = message_control(&sender, &sender_lease);
    for (index, (expected_state, lost, narrow)) in [
        ("sent", false, false),
        ("unconfirmed", true, false),
        ("refused", false, false),
        ("refused", false, true),
    ]
    .into_iter()
    .enumerate()
    {
        if expected_state == "refused" {
            owner.store().unwrap().talk(&sender.machine, false).unwrap();
        }
        if narrow {
            owner.store().unwrap().talk(&sender.machine, true).unwrap();
            owner
                .store()
                .unwrap()
                .connection
                .execute(
                    "UPDATE grants SET agents='[]',revision=revision+1 WHERE client_id=?1",
                    [&sender.machine],
                )
                .unwrap();
        }
        let proxy_root = core.root.join(format!("proxy{index}"));
        let directory = proxy_root.join("remote");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(crate::control::SOCKET);
        let socket = UnixListener::bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(core.root.join("storage-root"), proxy_root.to_str().unwrap()).unwrap();
        let actual = sender.layout.directory.clone();
        let capture = core.root.join(format!("public-{index}.json"));
        let (request, summary) = std::thread::scope(|scope| {
            let (cancel, watch) = UnixStream::pair().unwrap();
            let bridge = scope.spawn(move || {
                use std::os::fd::AsFd;
                let mut ready = [
                    nix::poll::PollFd::new(socket.as_fd(), nix::poll::PollFlags::POLLIN),
                    nix::poll::PollFd::new(watch.as_fd(), nix::poll::PollFlags::POLLIN),
                ];
                assert!(
                    nix::poll::poll(&mut ready, 60000u16).unwrap() > 0,
                    "bounded public-command connection"
                );
                if ready[1].revents().is_some_and(|flags| !flags.is_empty()) {
                    return false;
                }
                let (mut client, _) = socket.accept().unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(70)))
                    .unwrap();
                let mut raw = String::new();
                BufReader::new(client.try_clone().unwrap())
                    .read_line(&mut raw)
                    .unwrap();
                let mut destination =
                    UnixStream::connect(actual.join(crate::control::SOCKET)).unwrap();
                destination
                    .set_read_timeout(Some(Duration::from_secs(70)))
                    .unwrap();
                destination.write_all(raw.as_bytes()).unwrap();
                let mut answer = String::new();
                BufReader::new(destination).read_line(&mut answer).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&answer).unwrap()["state"],
                    if expected_state == "refused" {
                        "refused"
                    } else {
                        "sent"
                    }
                );
                if !lost {
                    client.write_all(answer.as_bytes()).unwrap();
                }
                true
            });
            let result = public_message(&core, expected_state, &capture, "peer");
            drop(cancel);
            assert!(
                bridge.join().unwrap(),
                "public command must reach the real owner"
            );
            result
        });
        assert_eq!(
            core.sends(),
            (index + 1).min(2),
            "one effect through the existing inbox/dispatch owner"
        );
        assert_eq!(
            owner.status().unwrap()["error"],
            if narrow {
                json!("REMOTE_SCOPE_DENIED")
            } else {
                Value::Null
            },
            "narrowing the original recipient records the final diagnostic without losing the new refusal"
        );
        assert_eq!(
            summary["deliveryState"],
            if lost {
                json!("uncertain")
            } else {
                Value::Null
            }
        );
        assert_eq!(
            sender.send(&request, None).unwrap()["state"],
            if expected_state == "refused" {
                "refused"
            } else {
                "sent"
            },
            "same ID is observed through the existing recovery owner"
        );
        assert_eq!(
            core.sends(),
            (index + 1).min(2),
            "no second effect or automatic resend"
        );
    }
    control.stop();
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
    let join_error = words_tx.clone();
    let (choice_tx, choice_rx) = mpsc::channel();
    let mut remote = String::new();
    let mut token_text = String::new();
    std::thread::scope(|scope| {
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
                    words_tx.send(Ok(event["words"].clone())).unwrap();
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
            let result = sender_clone
                .join(&token, |event| {
                    events.push(event);
                    Ok(None)
                })
                .map(|()| events);
            if let Err(error) = &result {
                let _ = join_error.send(Err(error.code.clone()));
            }
            result
        });
        let words = words_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
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
            .join(&different.token().unwrap(), |_| Ok(None))
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
    wire.route(&owner);
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
    wire.route(&owner);
    assert_eq!(
        sender.send(&prepared(&remote), Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 2);
    // An exact lost-join observation returns its original receipt, creating no grant.
    sender.join(&token_text, |_| Ok(None)).unwrap();
    assert_eq!(owner.store().unwrap().grants().unwrap().len(), 1);
    let old_uid = sender
        .store()
        .unwrap()
        .cloud_connection(&remote)
        .unwrap()
        .unwrap()
        .uid;
    let sender_key = sender.key.public();
    let machine_row = format!(
        "projects/{PROJECT}/databases/(default)/documents/m/{}",
        sender.machine
    );
    let before_fields = wire.docs.lock().unwrap()[&machine_row]["fields"].clone();
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
    let after_fields = &docs[&machine_row]["fields"];
    assert_eq!(after_fields["uid"]["stringValue"], current_uid);
    for field in ["authenticationKey", "descriptor", "invitingOwner"] {
        assert_eq!(
            after_fields[field], before_fields[field],
            "uid recovery preserves {field}"
        );
    }
    assert!(
        wire.commits.lock().unwrap().iter().any(|commit| {
            let writes = commit["writes"].as_array().unwrap();
            writes.iter().any(|write| {
                write["update"]["name"] == machine_row
                    && write["updateMask"]["fieldPaths"] == json!(["uid"])
            }) && writes.iter().any(|write| {
                write["update"]["name"]
                    .as_str()
                    .is_some_and(|name| name.ends_with(&format!("/peers/{current_uid}")))
            })
        }),
        "verified reroute publishes route and child uid in one commit"
    );
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
            .filter(|(name, _)| name.contains(&format!("/m/{}/peers/", sender.machine)))
            .any(|(_, row)| row["fields"]["revoked"]["booleanValue"] == false),
        "the collaborator retains authority over its independent receiving grant"
    );
    assert!(
        wire.docs
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.contains(&format!("/m/{}/peers/", owner.machine)))
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

fn pair_with_reciprocal_choice(
    owner: &Arc<Service>,
    collaborator: &Arc<Service>,
    reverse_talk: bool,
) -> Vec<Value> {
    let (code_tx, code_rx) = mpsc::channel();
    let mut events = Vec::new();
    std::thread::scope(|scope| {
        let _cancel = Cancel(vec![owner.stop.clone(), collaborator.stop.clone()]);
        let owner_copy = owner.clone();
        let invite = scope.spawn(move || {
            owner_copy.invite_selected(PROJECT, config(), |event| {
                if event["event"] == "invite" {
                    code_tx
                        .send(event["code"].as_str().unwrap().to_owned())
                        .unwrap();
                }
                Ok((event["event"] == "candidate").then_some((true, true)))
            })
        });
        let code = code_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        collaborator
            .join(&code, |event| {
                let choice = (event["event"] == "reverse-permission").then_some(reverse_talk);
                events.push(event);
                Ok(choice)
            })
            .unwrap();
        invite.join().unwrap().unwrap();
    });
    events
}

#[test]
fn missing_peer_machine_discovery_is_bounded_across_polls_and_reopen() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::missing_peer_machine_discovery_is_bounded_across_polls_and_reopen",
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
    let owner_root = core.root.join("owner");
    let (owner, owner_lease) = service(&owner_root, &core, make_ports());
    let (joiner, _joiner_lease) = service(&core.root.join("joiner"), &core, make_ports());
    pair_with_reciprocal_choice(&owner, &joiner, false);
    let original = owner
        .store()
        .unwrap()
        .grant(&joiner.machine)
        .unwrap()
        .unwrap();
    let enrollment = owner
        .store()
        .unwrap()
        .cloud_checkpoint(&owner.machine)
        .unwrap()
        .created_at_ms();
    // A legacy one-way peer has a grant but no reciprocal local connection.
    owner
        .store()
        .unwrap()
        .connection
        .execute(
            "DELETE FROM cloud_remotes WHERE owner=?1",
            [&joiner.machine],
        )
        .unwrap();
    let path = wire
        .docs
        .lock()
        .unwrap()
        .keys()
        .find(|path| path.ends_with(&format!("/m/{}", joiner.machine)))
        .unwrap()
        .clone();
    let machine = wire.docs.lock().unwrap().remove(&path).unwrap();
    wire.reads.lock().unwrap().clear();
    let reads = || {
        wire.reads
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.ends_with(&format!("/m/{}", joiner.machine)))
            .count()
    };
    let now = crate::pairing::now_ms().unwrap();
    owner
        .discover_reciprocal_peers(&config(), now, &owner.io())
        .unwrap();
    assert_eq!(reads(), 1, "one absent-machine observation");
    owner
        .discover_reciprocal_peers(&config(), now + 60_000, &owner.io())
        .unwrap();
    assert_eq!(
        reads(),
        1,
        "a minute poll does not pay for the same absence again"
    );
    drop(owner);
    drop(owner_lease);
    let (owner, _owner_lease) = service(&owner_root, &core, make_ports());
    for minute in 2..1440 {
        owner
            .discover_reciprocal_peers(&config(), now + minute * 60_000, &owner.io())
            .unwrap();
    }
    assert_eq!(
        reads(),
        1,
        "a full day of minute polls, including reopen, costs one discovery read"
    );
    // Later registration is discovered after the bounded negative cache expires.
    wire.docs.lock().unwrap().insert(path, machine);
    owner
        .discover_reciprocal_peers(&config(), now + 86_400_000, &owner.io())
        .unwrap();
    assert_eq!(reads(), 2);
    let connection = owner
        .store()
        .unwrap()
        .cloud_connection(&joiner.machine)
        .unwrap()
        .unwrap();
    assert_eq!(connection.owner_key, joiner.key.public());
    let current = owner
        .store()
        .unwrap()
        .grant(&joiner.machine)
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            current.revision,
            current.scopes,
            current.agents,
            current.mode
        ),
        (
            original.revision,
            original.scopes,
            original.agents,
            original.mode
        )
    );
    assert_eq!(
        owner
            .store()
            .unwrap()
            .cloud_checkpoint(&owner.machine)
            .unwrap()
            .created_at_ms(),
        enrollment
    );
    owner
        .discover_reciprocal_peers(&config(), now + 2 * 86_400_000, &owner.io())
        .unwrap();
    assert_eq!(
        reads(),
        2,
        "a discovered pin needs no further discovery reads"
    );
}

#[test]
fn paired_machines_push_exact_finals_both_ways_and_reply_without_reverse_talk_permission() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::paired_machines_push_exact_finals_both_ways_and_reply_without_reverse_talk_permission",
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
    std::fs::write(
        core.root.join("identities"),
        json!({"identities":[{"id":TARGET,"name":"target","lifetime":"saved"}]}).to_string(),
    )
    .unwrap();
    let query = Arc::new(crate::core::CoreClient::at(core.root.join("tmt")).unwrap());
    let (owner, owner_lease) =
        service_with_identity(&core.root.join("owner"), &core, make_ports(), query.clone());
    let (collaborator, collaborator_lease) =
        service_with_identity(&core.root.join("collaborator"), &core, make_ports(), query);
    let pinned_queries = || {
        core.calls()
            .iter()
            .filter(|call| {
                call["operation"] == "identity" && call["argv"][3] == "--expect-name=target"
            })
            .count()
    };
    let events = pair_with_reciprocal_choice(&owner, &collaborator, false);
    let remote = events
        .iter()
        .find(|event| event["event"] == "connected")
        .unwrap()["remote"]
        .as_str()
        .unwrap()
        .to_owned();
    let choices = events
        .iter()
        .filter(|event| event["event"] == "reverse-permission")
        .count();
    assert_eq!(choices, 1);
    assert!(
        !collaborator
            .store()
            .unwrap()
            .grant(&owner.machine)
            .unwrap()
            .unwrap()
            .permits_scope("talk")
    );
    wire.route(&owner);
    wire.route(&collaborator);
    // Reciprocal local ownership also drives the collaborator's own inbox.
    collaborator.reconcile().unwrap();
    let forward = collaborator
        .store()
        .unwrap()
        .cloud_connection(&owner.machine)
        .unwrap()
        .unwrap()
        .reroute_token
        .unwrap();
    let reverse = owner
        .store()
        .unwrap()
        .cloud_connection(&collaborator.machine)
        .unwrap()
        .unwrap()
        .reroute_token
        .unwrap();
    assert_ne!(forward, reverse);
    // Each wrong-direction capability is refused by the transport gate before
    // the receiving native generation/route state can change.
    for (source, target, wrong) in [
        (&owner, &collaborator, &forward),
        (&collaborator, &owner, &reverse),
    ] {
        let provider = source.provider(&config(), &target.machine, false).unwrap();
        let proof = crate::cloud_pairing::Reroute {
            machine_id: target.machine.clone(),
            owner_key: target.key.public(),
            peer: source.machine.clone(),
            uid: provider.uid().into(),
            generation: 2,
            timestamp_ms: crate::pairing::now_ms().unwrap(),
        };
        let body = wire::route_request(&proof, &source.key).unwrap();
        let id = crate::store::uuid_v4().unwrap();
        let io = source.io();
        assert_eq!(
            run(
                provider.enrollment_create(
                    &id,
                    provider.uid(),
                    "reroute",
                    &body,
                    Some(crate::cloud_pairing::EnrollmentProof::Reroute {
                        peer: &source.machine,
                        token: wrong
                    }),
                    &io
                ),
                &io
            )
            .unwrap_err()
            .code,
            "REMOTE_CLOUD_ACCESS_DENIED"
        );
        assert!(
            !wire
                .docs
                .lock()
                .unwrap()
                .keys()
                .any(|name| name.ends_with(&format!("/enroll/{id}")))
        );
    }
    assert_eq!(core.sends(), 0);
    let first = prepared(&remote);
    let first_request: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(
        collaborator.send(&first, Some(&remote)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 1);
    assert_eq!(
        pinned_queries(),
        1,
        "forward delivery uses the public Core pin"
    );
    let first_operation = first_request["requestId"].as_str().unwrap();
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(core.root.join(format!("receipt-{first_operation}"))).unwrap(),
    )
    .unwrap();
    let mapped = receipt["items"][0]["requestId"].as_str().unwrap();
    let body = "owner final: π\r\n\0 ";
    std::fs::write(
        core.root.join(format!("final-{mapped}")),
        json!({"status":"retained","response":body}).to_string(),
    )
    .unwrap();
    owner.reconcile().unwrap();
    let local_request = format!("req_{first_operation}");
    assert_eq!(
        std::fs::read(core.root.join(format!("reply-{local_request}"))).unwrap(),
        body.as_bytes()
    );
    assert_eq!(
        core.calls()
            .iter()
            .filter(|call| call["operation"] == "reply")
            .count(),
        1
    );
    owner.reconcile().unwrap();
    collaborator.reconcile().unwrap();
    assert_eq!(core.sends(), 1);
    assert_eq!(
        core.calls()
            .iter()
            .filter(|call| call["operation"] == "reply")
            .count(),
        1
    );
    let reverse = collaborator.machine.clone();
    assert_eq!(
        owner.send(&prepared(&reverse), Some(&reverse)).unwrap()["state"],
        "refused"
    );
    assert_eq!(
        core.sends(),
        1,
        "receiving a final does not enable reverse talk"
    );
    collaborator
        .store()
        .unwrap()
        .talk(&owner.machine, true)
        .unwrap();
    let second = prepared(&reverse);
    let second_request: Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(
        owner.send(&second, Some(&reverse)).unwrap()["state"],
        "sent"
    );
    assert_eq!(core.sends(), 2);
    assert_eq!(
        pinned_queries(),
        2,
        "reverse delivery uses the public Core pin too"
    );
    let second_operation = second_request["requestId"].as_str().unwrap();
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(core.root.join(format!("receipt-{second_operation}"))).unwrap(),
    )
    .unwrap();
    let mapped = receipt["items"][0]["requestId"].as_str().unwrap();
    std::fs::write(
        core.root.join(format!("final-{mapped}")),
        json!({"status":"retained","response":""}).to_string(),
    )
    .unwrap();
    collaborator.reconcile().unwrap();
    assert_eq!(
        std::fs::read(core.root.join(format!("reply-req_{second_operation}"))).unwrap(),
        b""
    );
    assert_eq!(
        core.calls()
            .iter()
            .filter(|call| call["operation"] == "reply")
            .count(),
        2
    );
    // A fixed per-operation refusal must not keep a later valid final behind it.
    let bad_operation = "88888888-8888-4888-8888-888888888888";
    let good_operation = "99999999-9999-4999-8999-999999999999";
    let mut pending = Vec::new();
    for operation in [bad_operation, good_operation] {
        let mut request: Value = serde_json::from_slice(&prepared(&remote)).unwrap();
        request["requestId"] = json!(operation);
        request["reply"]["requestId"] = json!(format!("req_{operation}"));
        assert_eq!(
            collaborator
                .send(request.to_string().as_bytes(), Some(&remote))
                .unwrap()["state"],
            "sent"
        );
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(core.root.join(format!("receipt-{operation}"))).unwrap(),
        )
        .unwrap();
        pending.push(
            receipt["items"][0]["requestId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    for (mapped, response) in pending.iter().zip([
        "x".repeat(crate::limits::CLOUD_MESSAGE_BYTES + 1),
        "not blocked by another final".to_owned(),
    ]) {
        std::fs::write(
            core.root.join(format!("final-{mapped}")),
            json!({"status":"retained","response":response}).to_string(),
        )
        .unwrap();
    }
    assert_eq!(
        owner.reconcile().unwrap_err().code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
    assert!(
        !core
            .root
            .join(format!("reply-req_{bad_operation}"))
            .exists()
    );
    assert_eq!(
        std::fs::read(core.root.join(format!("reply-req_{good_operation}"))).unwrap(),
        b"not blocked by another final"
    );
    assert_eq!(
        owner.reconcile().unwrap_err().code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
    assert_eq!(
        core.calls()
            .iter()
            .filter(|call| call["operation"] == "reply")
            .count(),
        3
    );
    assert_eq!(core.sends(), 4);
    let docs = wire.docs.lock().unwrap();
    assert!(!docs.keys().any(|name| name.ends_with(&format!(
        "/inbox/{}",
        crate::cloud_inbox::finals::id(bad_operation).unwrap()
    ))));
    for operation in [first_operation, second_operation, good_operation] {
        let final_id = crate::cloud_inbox::finals::id(operation).unwrap();
        assert_eq!(
            docs.keys()
                .filter(|name| name.ends_with(&format!("/inbox/{final_id}")))
                .count(),
            1
        );
        assert!(
            !docs
                .keys()
                .any(|name| name.ends_with(&format!("/responses/{final_id}"))),
            "a final writes one carrier, without an unused provider response"
        );
    }
    for (name, doc) in docs
        .iter()
        .filter(|(name, _)| name.contains("/inbox/") || name.contains("/responses/"))
    {
        assert_eq!(doc["fields"].as_object().unwrap().len(), 3, "{name}");
        assert!(!doc.to_string().contains("PRIVATE_PAIRING_RECEIPT"));
        assert!(doc["fields"]["carrier"]["bytesValue"].is_string());
    }
    drop(docs);
    drop(owner);
    drop(collaborator);
    drop(owner_lease);
    drop(collaborator_lease);
}
