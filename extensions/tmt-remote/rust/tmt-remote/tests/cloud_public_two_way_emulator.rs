//! Explicit real-Core acceptance; no normal package test contacts an emulator.
#![cfg(feature = "test-emulator")]

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tmt_remote::{
    cloud_keys::CloudKeys,
    cloud_pairing::service::{Owner, Service, Worker},
    control::{self, Control},
    core::CoreClient,
    deploy_plan::{self, CloudBackend, Enabled, Supplied, Target},
    deploy_record::DeployRecordStore,
    deploy_run::{
        self, DeployBinding, DeployInput, HostingCheckpoint, Run, RunState, SignInProvider,
        StepRecord, StepState,
    },
    firebase_auth::{AuthError, Cancellation, Http, Request, Response},
    hosting::{self, HostingBundle, HostingInventory, HostingManifest},
    message::MessageService,
    native_sync::projects::{ProjectPorts, Projects},
    operations::Operations,
    sealed_sync::AuthPort,
    session::DoorSessions,
    state::{Layout, MachineKey, Serving},
    store::{Store, uuid_v4},
};

const PROJECT: &str = "demo-native-pairing";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn emulator_address(raw: &str) -> Option<SocketAddr> {
    let address: SocketAddr = raw.parse().ok()?;
    (address.ip().is_loopback() && address.port() != 0).then_some(address)
}
fn config() -> Value {
    json!({"apiKey":"public-test","projectId":PROJECT,
        "authDomain":format!("{PROJECT}.firebaseapp.com"),"appId":"1:12345:web:fixture"})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct World {
    root: PathBuf,
    data: PathBuf,
    core: PathBuf,
    name: String,
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    a: World,
    b: World,
    report: PathBuf,
}

fn public(world: &World, args: &[&str], input: &[u8]) -> (bool, Value) {
    let args: Vec<OsString> = args
        .iter()
        .chain(["--json"].iter())
        .map(OsString::from)
        .collect();
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &world.core,
            args: &args,
            input,
            deadline: Instant::now()
                + tmt_remote::limits::CLOUD_RECONCILE_CALL
                + Duration::from_secs(30),
            max_stream_bytes: 1024 * 1024,
            launch: Default::default(),
        },
        None,
    )
    .expect("public Core child must finish and be reaped");
    let value = serde_json::from_slice(&output.stdout).expect("public Core returns one JSON value");
    (output.status.success(), value)
}
fn ok(world: &World, args: &[&str], input: &[u8]) -> Value {
    let (success, value) = public(world, args, input);
    assert!(
        success,
        "public command refused: {}",
        value["error"]["code"]
    );
    value
}
fn unconfirmed(world: &World, answer: &Value) {
    assert_eq!(answer["error"]["code"], "REMOTE_UNCONFIRMED");
    assert_eq!(answer["deliveryState"], "uncertain");
    let detail = ok(
        world,
        &[
            "x",
            "show",
            answer["requestId"].as_str().unwrap(),
            "--identity",
            &world.name,
        ],
        &[],
    );
    assert_eq!(detail["exchange"]["messageState"]["canSendNow"], false);
}

// Synthetic local deployment metadata, admitted through the real record owner.
// It supplies the invite's owned public config, not evidence of a real deployment.
fn bootstrap_record(layout: &Layout) {
    let mut records = DeployRecordStore::open(layout, PROJECT, "fixture bootstrap").unwrap();
    let mut record = records.load_or_draft().unwrap();
    let html = b"<p>synthetic public configuration</p>";
    let manifest = HostingManifest::parse(&json!({"version":1,"files":[{
        "path":"/index.html","sha256":hash(html),"length":html.len(),"contentType":"text/html"}]}))
    .unwrap();
    let mut declaration: Value =
        serde_json::from_slice(include_bytes!("fixtures/declarations/colab-firestore.json"))
            .unwrap();
    declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
    let declaration = serde_json::to_vec(&declaration).unwrap();
    let plan = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: &declaration,
                artifact: include_bytes!("fixtures/declarations/rules/colab-admission.rules"),
            }),
        }],
    )
    .unwrap();
    let bundle = HostingBundle::parse(&manifest, &serde_json::to_vec(&json!({"version":1,
        "manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(html)}]})).unwrap()).unwrap();
    let content = hosting::compose(&[hosting::HostingInput {
        extension: "colab",
        declaration: &declaration,
        bundle: &bundle,
    }])
    .unwrap()
    .unwrap();
    let hosted = hosting::deployment_view(
        &content,
        PROJECT,
        &record.deployment_id,
        &HostingInventory {
            site_exists: false,
            site_app_id: None,
            site_app_config: None,
            web_apps: vec![],
            live: None,
        },
    )
    .unwrap();
    let prepared = deploy_run::prepare_with_hosting(
        &plan,
        &DeployInput {
            account: "fixture@example.test",
            project: PROJECT,
            deployment_id: &record.deployment_id,
            location: "us-central1",
            sign_in: &[SignInProvider::Anonymous],
            rules_body: tmt_remote::cloud_inbox::rules::artifact().as_bytes(),
            live_rules: None,
        },
        Some(&hosted),
    )
    .unwrap();
    let version = format!("sites/{PROJECT}/versions/fixture");
    let publication = serde_json::from_value(json!({"project":PROJECT,"region":"us-central1",
        "deploymentId":record.deployment_id,"planDigest":prepared.digest(),"appId":config()["appId"],
        "siteAppId":config()["appId"],"publicConfig":config(),"entryUrl":format!("https://{PROJECT}.web.app"),
        "version":version,"release":format!("sites/{PROJECT}/releases/fixture"),
        "rulesDigest":prepared.view().rules.digest,"contentDigest":hosted.content["digest"]})).unwrap();
    record.binding = Some(DeployBinding {
        plan_digest: prepared.digest().into(),
        project: PROJECT.into(),
        completed_at_ms: now(),
    });
    record.run = Some(Run {
        plan_digest: prepared.digest().into(),
        account: "fixture@example.test".into(),
        authorized_at_ms: now(),
        state: RunState::Complete,
        rules_attempted: true,
        steps: prepared
            .view()
            .steps
            .iter()
            .map(|id| StepRecord {
                id: id.clone(),
                state: StepState::Done,
                cause: None,
            })
            .collect(),
        hosting: Some(HostingCheckpoint {
            envelope: serde_json::to_value(prepared.view()).unwrap(),
            app_id: Some(config()["appId"].as_str().unwrap().into()),
            operation: None,
            version: Some(version),
            version_created_ms: Some(now()),
            publication: Some(publication),
        }),
    });
    records.bind_target(PROJECT, "us-central1").unwrap();
    records.persist(&record).unwrap();
    let selected = tmt_remote::remote_selector::client(layout, Some(PROJECT), None).unwrap();
    assert!(matches!(
        selected.kind,
        tmt_remote::remote_selector::ClientKind::Owned
    ));
    assert_eq!(selected.public_config, config());
}

struct CountedHttp {
    inner: Arc<dyn Http + Send + Sync>,
    calls: Arc<Mutex<BTreeMap<String, usize>>>,
}
impl Http for CountedHttp {
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
        self.inner.send(request, deadline, cancel)
    }
}

struct RemoteOwner {
    layout: Layout,
    machine: String,
    stop: Arc<AtomicBool>,
    service: Option<Arc<Service>>,
    store: Option<Arc<Mutex<Store>>>,
    worker: Option<Worker>,
    control: Option<Control>,
    lease: Option<Serving>,
}
impl RemoteOwner {
    fn start(world: &World, calls: &Arc<Mutex<BTreeMap<String, usize>>>) -> Self {
        assert!(world.data.starts_with(&world.root) && world.core.starts_with(&world.root));
        let layout = Layout::open(&world.data).unwrap();
        let lease = layout.serve_lock().unwrap();
        let mut store = Store::open(&lease).unwrap();
        let machine = store.machine().unwrap().id;
        let key = MachineKey::open(&layout).unwrap();
        let keys = Arc::new(CloudKeys::new(layout.clone(), machine.clone(), key.clone()));
        keys.maintain(now()).unwrap();
        let store = Arc::new(Mutex::new(store));
        let stop = Arc::new(AtomicBool::new(false));
        let sessions = Arc::new(DoorSessions::new(
            machine.clone(),
            uuid_v4().unwrap(),
            "http://127.0.0.1:1".into(),
            "/r/proof/x/".into(),
            key.clone(),
            store.clone(),
            tmt_remote::session::IDLE,
        ));
        let operations = Arc::new(Operations::new(
            CoreClient::at(world.core.clone()).unwrap(),
            stop.clone(),
            tmt_remote::limits::CLOUD_ENVELOPE_BYTES,
        ));
        let messages = Arc::new(
            MessageService::new(sessions.clone(), tmt_remote::limits::CLOUD_ENVELOPE_BYTES)
                .with_operations(operations),
        );
        let mut ports = ProjectPorts::production(
            &tmt_remote::firebase_auth::Project::from_public_config(&config()).unwrap(),
        )
        .unwrap();
        ports.http = Arc::new(CountedHttp {
            inner: ports.http,
            calls: calls.clone(),
        });
        let projects = Arc::new(Projects::new(layout.clone(), Some(ports)));
        let service = Arc::new(Service::new(Owner {
            layout: layout.clone(),
            store: store.clone(),
            key: key.clone(),
            machine: machine.clone(),
            keys,
            sessions: sessions.clone(),
            messages,
            projects,
            stop: stop.clone(),
        }));
        let pairing = Arc::new(tmt_remote::pairing::Pairing::new(
            machine.clone(),
            uuid_v4().unwrap(),
            key.public(),
            "http://127.0.0.1:1".into(),
            store.clone(),
            tmt_remote::pairing::Timing::CONTRACT,
        ));
        let devices = Arc::new(tmt_remote::devices::Devices::new(
            store.clone(),
            Some(sessions),
        ));
        let control = Control::start_with_views(
            &lease,
            pairing,
            devices,
            control::Door {
                origin: "http://127.0.0.1:1".into(),
                prefix: "/r/proof/".into(),
            },
            None,
            stop.clone(),
            control::StatusViews {
                cloud: Some(service.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        Self {
            layout,
            machine,
            stop,
            service: Some(service),
            store: Some(store),
            worker: None,
            control: Some(control),
            lease: Some(lease),
        }
    }
    fn service(&self) -> &Arc<Service> {
        self.service.as_ref().unwrap()
    }
    fn start_worker(&mut self) {
        self.worker = Some(self.service().worker().unwrap());
    }
}
impl Drop for RemoteOwner {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            control.stop();
        }
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        drop(self.worker.take());
        if let Some(service) = self.service.take() {
            service.projects.stop().unwrap();
            drop(service);
        }
        drop(self.store.take());
        drop(self.lease.take());
        assert!(!self.layout.directory.join("control.sock").exists());
        drop(
            self.layout
                .serve_lock()
                .expect("all owner lease holders released"),
        );
    }
}

fn admin(address: SocketAddr, suffix: &str, body: Value, method: &str) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let request = ureq::http::Request::builder()
        .method(method)
        .uri(format!("http://{address}/{suffix}"))
        .header("Authorization", "Bearer owner")
        .header("Content-Type", "application/json")
        .body(body.to_string().into_bytes())
        .unwrap();
    assert_eq!(
        agent.run(request).unwrap().status().as_u16(),
        200,
        "synthetic emulator bootstrap only"
    );
}
fn pair(owner: &RemoteOwner, joiner: &RemoteOwner) {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::scope(|scope| {
        struct CancelOnFailure<'a> {
            owner: &'a AtomicBool,
            joiner: &'a AtomicBool,
            armed: bool,
        }
        impl Drop for CancelOnFailure<'_> {
            fn drop(&mut self) {
                if self.armed {
                    self.owner.store(true, std::sync::atomic::Ordering::Release);
                    self.joiner
                        .store(true, std::sync::atomic::Ordering::Release);
                }
            }
        }
        let mut cancel = CancelOnFailure {
            owner: &owner.stop,
            joiner: &joiner.stop,
            armed: true,
        };
        let invite = scope.spawn(|| {
            owner.service().invite(Some(PROJECT), |event| {
                if event["event"] == "invite" {
                    tx.send(event["code"].as_str().unwrap().to_owned()).unwrap();
                }
                Ok((event["event"] == "candidate").then_some((true, true)))
            })
        });
        let code = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let mut reverse = 0;
        joiner
            .service()
            .join(&code, |event| {
                if event["event"] == "reverse-permission" {
                    reverse += 1;
                    return Ok(Some(false));
                }
                Ok(None)
            })
            .unwrap();
        invite.join().unwrap().unwrap();
        assert_eq!(reverse, 1, "one default-No reciprocal choice");
        cancel.armed = false;
    });
}
fn wait_value(mut observe: impl FnMut() -> Option<Value>) -> Value {
    // Allow the normal idle cadence to carry the request and the return final;
    // this fixture never wakes the worker to make a send look synchronous.
    let deadline = Instant::now()
        + Duration::from_millis(3 * tmt_remote::limits::CLOUD_POLL_IDLE_MS)
        + 2 * tmt_remote::limits::CORE_CALL;
    loop {
        if let Some(value) = observe() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "causal public observation deadline"
        );
        thread::sleep(Duration::from_millis(250));
    }
}
fn conversation(
    sender: &World,
    receiver: &World,
    receiving: &RemoteOwner,
    response: &str,
) -> Value {
    let marker = format!("public two-way causal {}", uuid_v4().unwrap());
    let (sent_ok, sent) = public(
        sender,
        &[
            "talk",
            &receiver.id,
            &marker,
            "--expect-name",
            &receiver.name,
            "--remote",
            &receiving.machine,
            "--identity",
            &sender.name,
        ],
        &[],
    );
    let original = sent["requestId"].as_str().unwrap();
    let initial_delivery = if sent_ok {
        assert_eq!(sent["status"], "sent");
        "sent"
    } else {
        // The normal peer poll may outlive the synchronous answer window. A
        // truthful uncertainty is still answerable; never issue another talk.
        unconfirmed(sender, &sent);
        "unconfirmed"
    };
    let inbox = wait_value(|| {
        let inbox = ok(receiver, &["inbox", "--identity", &receiver.name], &[]);
        let items = inbox["items"].as_array().unwrap();
        assert!(items.len() <= 1, "one causal recipient dispatch");
        items.first().cloned()
    });
    let adopted = inbox["requestId"].as_str().unwrap();
    let incoming = ok(
        receiver,
        &[
            "x",
            "show",
            adopted,
            "--incoming",
            "--identity",
            &receiver.name,
        ],
        &[],
    );
    assert!(
        incoming["exchange"]["prompt"]["message"]
            .as_str()
            .unwrap()
            .contains(&marker)
    );
    let receipt = incoming["exchange"]["reply"]["receipt"].as_str().unwrap();
    let replied = ok(
        receiver,
        &["reply", adopted, "--receipt", receipt, "--stdin"],
        response.as_bytes(),
    );
    assert_eq!(replied["status"], "submitted");
    assert_eq!(replied["bodyBytes"].as_u64(), Some(response.len() as u64));
    let result = wait_value(|| {
        let (success, result) = public(sender, &["result", original], &[]);
        if success {
            Some(result)
        } else {
            assert_eq!(result["error"]["code"], "RESPONSE_NOT_AVAILABLE");
            None
        }
    });
    assert_eq!(
        result["response"].as_str().unwrap().as_bytes(),
        response.as_bytes()
    );
    // Public observations cannot dispatch again or change the immutable final.
    for _ in 0..2 {
        assert_eq!(ok(sender, &["result", original], &[])["response"], response);
    }
    assert!(
        ok(receiver, &["inbox", "--identity", &receiver.name], &[])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    json!({"bodyBytes":response.len(),"sha256":hash(response.as_bytes()),"initialDelivery":initial_delivery,
        "senderName":sender.name,"original":original,"adopted":adopted,"marker":marker})
}

#[test]
fn unsafe_emulator_routes_refuse_before_state_or_network() {
    for raw in [
        "",
        "firebase.googleapis.com:443",
        "0.0.0.0:8080",
        "127.0.0.1:0",
        "https://127.0.0.1:8080",
    ] {
        assert!(emulator_address(raw).is_none());
    }
    assert!(emulator_address("127.0.0.1:8080").is_some());
}

#[test]
fn bootstrap_metadata_is_typed_owned_config_without_provider_calls() {
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let root =
        Directory(std::env::temp_dir().join(format!("tmt-proof-bootstrap-{}", uuid_v4().unwrap())));
    let layout = Layout::open(&root.0).unwrap();
    bootstrap_record(&layout);
    drop(layout);
}

#[test]
#[ignore = "fresh shared Infra Auth+Firestore slot; Node public sandbox entry point required"]
fn public_core_talk_and_exact_finals_both_directions() {
    let firestore = emulator_address(&std::env::var("FIRESTORE_EMULATOR_HOST").unwrap())
        .expect("numeric loopback only");
    emulator_address(&std::env::var("FIREBASE_AUTH_EMULATOR_HOST").unwrap())
        .expect("numeric loopback only");
    let proof: Proof = serde_json::from_slice(
        &fs::read(std::env::var_os("TMT_CLOUD_TWO_WAY_PROOF").expect("private proof input"))
            .unwrap(),
    )
    .unwrap();
    assert_ne!(proof.a.data, proof.b.data);
    assert!(proof.report.starts_with(&proof.a.root));
    let calls = Arc::new(Mutex::new(BTreeMap::new()));
    let mut a = RemoteOwner::start(&proof.a, &calls);
    let mut b = RemoteOwner::start(&proof.b, &calls);
    bootstrap_record(&b.layout);
    admin(
        firestore,
        &format!("emulator/v1/projects/{PROJECT}:securityRules"),
        json!({"rules":{"files":[{"name":"firestore.rules","content":tmt_remote::cloud_inbox::rules::artifact()}]}}),
        "PUT",
    );
    let auth = b.service().projects.load(&config(), true).unwrap();
    let uid = auth.auth.identity().uid;
    admin(
        firestore,
        &format!("v1/projects/{PROJECT}/databases/(default)/documents:commit"),
        json!({"writes":[{"update":{"name":format!("projects/{PROJECT}/databases/(default)/documents/owners/{uid}"),
            "fields":{"machine":{"stringValue":b.machine}}}}]}),
        "POST",
    );
    pair(&b, &a);
    drop(auth);
    a.start_worker();
    b.start_worker();
    let first = conversation(
        &proof.a,
        &proof.b,
        &b,
        " \r\nBOM:\u{feff} NUL:\0 Unicode: π\n ",
    );
    let refused_marker = format!("reverse must stay disabled {}", uuid_v4().unwrap());
    let (success, refusal) = public(
        &proof.b,
        &[
            "talk",
            &proof.a.id,
            &refused_marker,
            "--expect-name",
            &proof.a.name,
            "--remote",
            &a.machine,
            "--identity",
            &proof.b.name,
        ],
        &[],
    );
    assert!(!success);
    let reverse_initial = if refusal["error"]["code"] == "REMOTE_REFUSED" {
        "refused"
    } else {
        unconfirmed(&proof.b, &refusal);
        "unconfirmed"
    };
    let refused_id = refusal["requestId"]
        .as_str()
        .unwrap()
        .strip_prefix("req_")
        .unwrap();
    // A caller can time out before the normal idle poll. Observe the real durable
    // admission refusal before changing scope; never manufacture a synchronous answer.
    wait_value(|| {
        let db = Connection::open_with_flags(
            a.layout.directory.join("remote.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_secs(1)).unwrap();
        let observed: Option<String> = db.query_row(
            "SELECT refusal FROM cloud_transport WHERE kind='inbox' AND peer_id=?1 AND id=?2 AND length(response)>0",
            [&b.machine,refused_id], |row| row.get(0)).optional().unwrap();
        observed.map(|code| {
            assert_eq!(code, "REMOTE_SCOPE_DENIED");
            json!(true)
        })
    });
    ok(
        &proof.a,
        &["remote", "devices", "talk", &b.machine, "on"],
        &[],
    );
    let second = conversation(&proof.b, &proof.a, &a, "");
    drop(a);
    drop(b);
    // Reopen the same pairing and Core files; do not perform another ceremony or send.
    let a = RemoteOwner::start(&proof.a, &calls);
    let b = RemoteOwner::start(&proof.b, &calls);
    a.service().reconcile().unwrap();
    b.service().reconcile().unwrap();
    for (world, final_body) in [(&proof.a, &first), (&proof.b, &second)] {
        let result = ok(
            world,
            &["result", final_body["original"].as_str().unwrap()],
            &[],
        );
        assert_eq!(
            hash(result["response"].as_str().unwrap().as_bytes()),
            final_body["sha256"]
        );
    }
    drop(a);
    drop(b);
    assert!(!proof.a.data.join("remote/control.sock").exists());
    assert!(!proof.b.data.join("remote/control.sock").exists());
    let report = json!({"completedDirections":2,"reverseDefaultRefused":true,"ownersReleased":true,
        "reverseInitialDelivery":reverse_initial,"refusedMarker":refused_marker,
        "finals":[first,second],"ordinaryUserCalls":*calls.lock().unwrap(),
        "rulesSha256":hash(tmt_remote::cloud_inbox::rules::artifact().as_bytes())});
    fs::write(&proof.report, serde_json::to_vec(&report).unwrap()).unwrap();
}
