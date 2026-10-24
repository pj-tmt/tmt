//! Real crypto, Store/journal and public Core process; HTTP alone is an offline fixture.
#[allow(dead_code)]
#[path = "support/cloud_inbox.rs"]
mod cloud_fixture;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tmt_remote::{
    canonical::{self, Envelope},
    cloud_inbox::{InboxDriver, Provider, Sender},
    error::RemoteError,
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, Request, Response, Time,
    },
    firestore_budget::{Decision, Model, NativeUsage, charge_native},
    native_sync::{NativeBudgetPort, NativeRuntime},
    objects::IoBudget,
    sealed_sync::{AuthFailure, AuthIdentity, AuthPort, AuthToken, SyncError},
    state::MachineKey,
    store::Store,
};
const NOW: u64 = 1_800_000_000_000;
const MACHINE: &str = "11111111-1111-4111-8111-111111111111";
const PEER: &str = "33333333-3333-4333-8333-333333333333";
const ID: &str = "44444444-4444-4444-8444-444444444444";
const TARGET: &str = "66666666-6666-4666-8666-666666666666";
const ROOT: &str = "projects/demo-cloud-inbox/databases/(default)/documents";
const STAMP: &str = "2027-01-15T08:00:00.000Z";
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(std::task::Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("offline ports must complete synchronously"),
    }
}
struct Auth(&'static str);
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: "demo-cloud-inbox".into(),
            uid: self.0.into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        Box::pin(async {
            Ok(AuthToken {
                identity: self.identity(),
                token: self.0.into(),
                expires_at_ms: NOW + 3_600_000,
            })
        })
    }
}
struct ClockPort;
impl Clock for ClockPort {
    fn now(&self) -> Time {
        Time {
            wall_ms: NOW,
            monotonic_ms: 0,
        }
    }
    fn wait_until(&self, _: u64, cancel: &dyn Cancellation) -> bool {
        !cancel.cancelled()
    }
}
impl Jitter for ClockPort {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
struct Budget(Mutex<NativeUsage>);
impl NativeBudgetPort for Budget {
    fn usage(&self, _: &str) -> Result<NativeUsage, SyncError> {
        Ok(*self.0.lock().unwrap())
    }
    fn charge(&self, _: &str, model: &Model, cost: u64, time: Time) -> Result<Decision, SyncError> {
        charge_native(model, &mut self.0.lock().unwrap(), cost, time.wall_ms)
            .map_err(|_| SyncError::Unavailable)
    }
    fn flush(&self) -> Result<(), SyncError> {
        Ok(())
    }
}
struct Wire {
    documents: Mutex<BTreeMap<String, Value>>,
    owner: Mutex<Weak<InboxDriver>>,
    cancel_after_create: Mutex<Option<Arc<AtomicBool>>>,
    requests: Mutex<Vec<(Operation, String)>>,
    deny_responses: AtomicBool,
    fail_queries: AtomicBool,
    lose_response_for: Mutex<Option<String>>,
}
impl Wire {
    fn seed_inbox(&self, id: &str, carrier: &[u8]) {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let name = format!("{ROOT}/m/{MACHINE}/inbox/{id}");
        self.documents.lock().unwrap().insert(name.clone(), json!({"name":name,"createTime":STAMP,"fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(carrier)},"createdAt":{"timestampValue":STAMP}}}));
    }
}
impl Http for Wire {
    fn send(&self, request: &Request, _: u64, _: &dyn Cancellation) -> Result<Response, AuthError> {
        assert_eq!(
            request.response_bytes(),
            tmt_remote::limits::CLOUD_PROVIDER_BYTES
        );
        let path = request
            .url()
            .strip_prefix("https://firestore.googleapis.com/v1/")
            .unwrap();
        self.requests
            .lock()
            .unwrap()
            .push((request.operation(), path.into()));
        let input: Value = if request.body().is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(request.body()).unwrap()
        };
        let (status, answer) = match request.operation() {
            Operation::Read
                if path.contains("/responses/")
                    && request.bearer() == Some("peer")
                    && self.deny_responses.load(Ordering::SeqCst) =>
            {
                (403, json!({}))
            }
            Operation::Read => match self.documents.lock().unwrap().get(path).cloned() {
                Some(row) => (200, row),
                None => (404, json!({})),
            },
            Operation::Commit => {
                let write = &input["writes"][0];
                if let Some(name) = write["delete"].as_str() {
                    self.documents.lock().unwrap().remove(name);
                    (200, json!({}))
                } else {
                    assert_eq!(write["currentDocument"]["exists"], false);
                    assert_eq!(
                        write["updateTransforms"],
                        json!([{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}])
                    );
                    let mut row = write["update"].clone();
                    let name = row["name"].as_str().unwrap().to_owned();
                    {
                        let mut lost = self.lose_response_for.lock().unwrap();
                        if lost.as_deref() == Some(&name) {
                            *lost = None;
                            return Err(AuthError::unavailable());
                        }
                    }
                    row["createTime"] = json!(STAMP);
                    row["fields"]["createdAt"] = json!({"timestampValue":STAMP});
                    let exists = self.documents.lock().unwrap().contains_key(&name);
                    if exists {
                        (409, json!({}))
                    } else {
                        self.documents.lock().unwrap().insert(name.clone(), row);
                        if name.contains("/inbox/") {
                            // A provider notification drives the real finite receiver, with no
                            // fixture authority or operation handler between it and the journal.
                            if let Some(owner) = self.owner.lock().unwrap().upgrade() {
                                let stopped = AtomicBool::new(false);
                                let io = IoBudget {
                                    deadline: Instant::now() + Duration::from_secs(10),
                                    cancelled: &stopped,
                                };
                                ready(owner.reconcile(NOW, &io)).unwrap();
                            }
                            if let Some(cancel) = self.cancel_after_create.lock().unwrap().take() {
                                cancel.store(true, Ordering::SeqCst);
                            }
                        }
                        (200, json!({"writeResults":[],"commitTime":STAMP}))
                    }
                }
            }
            Operation::Query if self.fail_queries.load(Ordering::SeqCst) => {
                return Err(AuthError::unavailable());
            }
            Operation::Query => {
                assert!(
                    request.url().ends_with(&format!("/m/{MACHINE}:runQuery")),
                    "machine remains the parent"
                );
                assert_eq!(input["structuredQuery"]["limit"], 1);
                assert_eq!(
                    input["structuredQuery"]["from"],
                    json!([{"collectionId":"inbox","allDescendants":false}])
                );
                let query = &input["structuredQuery"];
                let from = query["where"]["fieldFilter"]["value"]["timestampValue"]
                    .as_str()
                    .unwrap();
                let after = query["startAt"]["values"][0]["timestampValue"]
                    .as_str()
                    .zip(query["startAt"]["values"][1]["referenceValue"].as_str());
                let documents = self.documents.lock().unwrap();
                let mut ordered: Vec<_> = documents
                    .iter()
                    .filter(|(name, doc)| {
                        let timestamp = doc["fields"]["createdAt"]["timestampValue"]
                            .as_str()
                            .unwrap();
                        name.starts_with(&format!("{ROOT}/m/{MACHINE}/"))
                            && name.contains("/inbox/")
                            && timestamp >= from
                            && after.is_none_or(|after| (timestamp, name.as_str()) > after)
                    })
                    .collect();
                ordered.sort_by_key(|(name, doc)| {
                    (
                        doc["fields"]["createdAt"]["timestampValue"]
                            .as_str()
                            .unwrap(),
                        name.as_str(),
                    )
                });
                let rows: Vec<_> = ordered
                    .into_iter()
                    .take(1)
                    .map(|(_, doc)| json!({"document":doc,"readTime":STAMP}))
                    .collect();
                (
                    200,
                    if rows.is_empty() {
                        json!([{"readTime":STAMP}])
                    } else {
                        json!(rows)
                    },
                )
            }
            _ => panic!("no Auth/network bootstrap in the fixture"),
        };
        Ok(Response {
            status,
            body: serde_json::to_vec(&answer).unwrap(),
            retry_after_ms: None,
        })
    }
}
struct CurrentIdentity;
impl tmt_remote::core::CloudIdentityQuery for CurrentIdentity {
    fn identity_show(&self, id: &str, name: &str, _: &AtomicBool) -> Result<(), RemoteError> {
        assert_eq!(id, TARGET);
        assert_eq!(name, "target");
        Ok(())
    }
}
fn provider(wire: Arc<Wire>, uid: &'static str) -> Provider {
    provider_clock(wire, uid, Arc::new(ClockPort))
}
fn provider_clock(
    wire: Arc<dyn Http + Send + Sync>,
    uid: &'static str,
    clock: Arc<dyn Clock + Send + Sync>,
) -> Provider {
    let runtime = Arc::new(NativeRuntime::new(
        wire,
        clock,
        Arc::new(ClockPort),
        Arc::new(Budget(Mutex::new(NativeUsage::new(NOW)))),
    ));
    Provider::new(
        Arc::new(Auth(uid)),
        runtime,
        Model::new(4, 4).unwrap(),
        MACHINE,
    )
    .unwrap()
}

fn request(key: &MachineKey, id: &str) -> Vec<u8> {
    let payload=json!({"version":1,"operation":"dispatch.create","originator":"anonymous","createdAtMs":NOW,"input":{"operationId":id,"recipientIds":[TARGET],"expectedName":"target","message":"one offline effect","kind":"request"}}).to_string();
    signed_request(key, id, "dispatch.create", &payload)
}
fn signed_request(key: &MachineKey, id: &str, operation: &str, payload: &str) -> Vec<u8> {
    signed_request_as(PEER, id, operation, payload, |bytes| key.sign(bytes))
}
fn signed_request_as(
    peer: &str,
    id: &str,
    operation: &str,
    payload: &str,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
) -> Vec<u8> {
    let window = "22222222-2222-4222-8222-222222222222";
    let session = "55555555-5555-4555-8555-555555555555";
    let envelope = Envelope {
        kind: "request",
        id,
        correlation_id: None,
        machine_id: MACHINE,
        window_id: window,
        client_id: peer,
        session_id: session,
        sequence: "1",
        timestamp_ms: NOW,
        origin: "cli",
        operation,
        payload: payload.as_bytes(),
    };
    json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":MACHINE,"windowId":window,"clientId":peer,"sessionId":session,"sequence":"1","timestampMs":NOW,"origin":"cli","operation":operation,"payload":canonical::base64url(payload.as_bytes()),"signature":canonical::base64url(&sign(&canonical::envelope(&envelope).unwrap()))}).to_string().into_bytes()
}
#[test]
fn sender_restart_recovers_frozen_outcomes_after_lost_create_answer_or_response_post() {
    for lose_response in [false, true] {
        let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
        let wire = Arc::new(Wire {
            documents: Mutex::new(BTreeMap::new()),
            owner: Mutex::new(Weak::new()),
            cancel_after_create: Mutex::new(None),
            requests: Mutex::new(vec![]),
            deny_responses: AtomicBool::new(false),
            fail_queries: AtomicBool::new(false),
            lose_response_for: Mutex::new(
                lose_response.then(|| format!("{ROOT}/m/{MACHINE}/responses/{ID}")),
            ),
        });
        let driver = world.driver(provider(Arc::clone(&wire), "owner"));
        *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
        let exact = request(&world.key, ID);
        let stopped = Arc::new(AtomicBool::new(false));
        *wire.cancel_after_create.lock().unwrap() = Some(Arc::clone(&stopped));
        let sender = Sender::new(
            provider(Arc::clone(&wire), "peer"),
            Arc::new(Mutex::new(world.sender_store.take().unwrap())),
            PEER,
            world.key.clone(),
        )
        .unwrap();
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        assert_eq!(
            ready(sender.append(&exact, &world.descriptor, NOW, NOW, &io))
                .unwrap_err()
                .code,
            "REMOTE_CLOUD_UNCONFIRMED"
        );
        assert_eq!(world.core.sends(), 1);
        if lose_response {
            assert!(
                !wire
                    .documents
                    .lock()
                    .unwrap()
                    .contains_key(&format!("{ROOT}/m/{MACHINE}/responses/{ID}"))
            );
        }
        let database = world.owner_layout.directory.join("remote.db");
        let frozen = || {
            let connection = rusqlite::Connection::open_with_flags(
                &database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
            .unwrap();
            connection.query_row("SELECT response FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",[PEER,ID],|row|row.get::<_,Vec<u8>>(0)).unwrap()
        };
        let before = frozen();
        drop(sender);
        drop(driver);
        world = world.reopen(NOW, Arc::new(CurrentIdentity));
        let driver = world.driver(provider(Arc::clone(&wire), "owner"));
        *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
        let sender = Sender::new(
            provider(Arc::clone(&wire), "peer"),
            Arc::new(Mutex::new(Store::open(&world.sender_lease).unwrap())),
            PEER,
            MachineKey::open(&world.sender_layout).unwrap(),
        )
        .unwrap();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let recovered = ready(sender.append(&exact, &world.descriptor, NOW, NOW, &io)).unwrap();
        let recovered: Value = serde_json::from_slice(&recovered).unwrap();
        assert_ne!(
            recovered["correlationId"], ID,
            "restart uses a fresh observation context, not the lost one"
        );
        assert_eq!(
            world.core.sends(),
            1,
            "ID recovery must never re-run the effect"
        );
        let payload = canonical::base64url_decode(recovered["payload"].as_str().unwrap()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&payload).unwrap()["state"],
            "accepted"
        );
        let documents = wire.documents.lock().unwrap();
        assert_eq!(
            documents.keys().filter(|p| p.contains("/inbox/")).count(),
            2,
            "one original plus one read-only recovery"
        );
        drop(documents);
        assert_eq!(
            frozen(),
            before,
            "restart never reseals the original frozen response"
        );
        let original_posts = wire
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(operation, path)| {
                *operation == Operation::Commit && path.ends_with(":commit")
            })
            .count();
        assert_eq!(
            original_posts, 4,
            "one request + one response attempt per original/recovery, never a response retry"
        );
        drop(sender);
        drop(driver);
    }
}

struct RenamedIdentity;
impl tmt_remote::core::CloudIdentityQuery for RenamedIdentity {
    fn identity_show(&self, id: &str, name: &str, _: &AtomicBool) -> Result<(), RemoteError> {
        assert_eq!((id, name), (TARGET, "target"));
        Err(RemoteError::new(
            "REMOTE_AGENT_STALE",
            "The identity changed.",
        ))
    }
}
#[test]
fn prepared_reply_is_private_durable_and_frozen_before_publication() {
    use rusqlite::{Connection, OpenFlags};
    use std::{path::PathBuf, sync::atomic::AtomicUsize};
    const RECEIPT: &str = "v2_PRIVATE_REPLY_CANARY";
    struct Inspect {
        wire: Arc<Wire>,
        database: PathBuf,
        creates: Arc<AtomicUsize>,
    }
    impl Http for Inspect {
        fn send(
            &self,
            request: &Request,
            deadline: u64,
            cancel: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            assert!(!String::from_utf8_lossy(request.body()).contains(RECEIPT));
            if request.operation() == Operation::Commit {
                let body: Value = serde_json::from_slice(request.body()).unwrap();
                if body["writes"][0]["update"]["name"] == format!("{ROOT}/m/{MACHINE}/inbox/{ID}") {
                    let db = Connection::open_with_flags(
                        &self.database,
                        OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )
                    .unwrap();
                    let (binding, attempted): (String, bool) = db.query_row(
                        "SELECT binding,attempted FROM cloud_transport WHERE peer_id=?1 AND kind='outbox' AND id=?2",
                        [PEER,ID], |row| Ok((row.get(0)?,row.get(1)?)),
                    ).unwrap();
                    assert!(attempted);
                    let binding: Value = serde_json::from_str(&binding).unwrap();
                    assert_eq!(
                        binding,
                        json!({"machine":MACHINE,"remote":"team","senderId":PEER,
                        "reply":{"requestId":format!("req_{ID}"),"receipt":RECEIPT}})
                    );
                    self.creates.fetch_add(1, Ordering::SeqCst);
                }
            }
            self.wire.send(request, deadline, cancel)
        }
    }
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let driver = world.driver(provider(wire.clone(), "owner"));
    *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
    let creates = Arc::new(AtomicUsize::new(0));
    let http = Arc::new(Inspect {
        wire: wire.clone(),
        database: world.sender_layout.directory.join("remote.db"),
        creates: creates.clone(),
    });
    let store = Arc::new(Mutex::new(world.sender_store.take().unwrap()));
    let sender = Sender::new(
        provider_clock(http.clone(), "peer", Arc::new(ClockPort)),
        store.clone(),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let raw = json!({"version":1,"remote":null,"createdAtMs":NOW,"requestId":ID,
        "from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},
        "message":"one private reply binding","reply":{"requestId":format!("req_{ID}"),"receipt":RECEIPT}}).to_string();
    let answer =
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Sent);
    assert!(!serde_json::to_string(&answer).unwrap().contains(RECEIPT));
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    assert_eq!(world.core.sends(), 1);
    assert!(
        world
            .core
            .calls()
            .iter()
            .all(|call| !call.to_string().contains(RECEIPT))
    );
    drop(sender);
    drop(store);
    let sender = Sender::new(
        provider_clock(http, "peer", Arc::new(ClockPort)),
        Arc::new(Mutex::new(Store::open(&world.sender_lease).unwrap())),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
    let calls = wire.requests.lock().unwrap().len();
    assert_eq!(
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "other", NOW, &io))
            .err()
            .unwrap()
            .code,
        "REMOTE_INTENT_CONFLICT"
    );
    assert_eq!(wire.requests.lock().unwrap().len(), calls);
    assert_eq!(creates.load(Ordering::SeqCst), 1);
    assert_eq!(world.core.sends(), 1);
    for layout in [&world.sender_layout, &world.owner_layout] {
        let db = Connection::open_with_flags(
            layout.directory.join("remote.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let audit: String = db.query_row("SELECT COALESCE(group_concat(client_id||envelope_id||operation||COALESCE(operation_id,'')||resources_json||decision||code,''),'') FROM audit", [], |row| row.get(0)).unwrap();
        assert!(!audit.contains(RECEIPT));
    }
}

#[test]
fn prepared_core_boundary_delivers_or_refuses_the_uuid_query_without_a_name_fallback() {
    use tmt_remote::{cloud_command::State, core::CoreClient};
    let cases: Vec<(
        Arc<dyn tmt_remote::core::CloudIdentityQuery>,
        State,
        Option<&str>,
    )> = vec![
        (Arc::new(CurrentIdentity), State::Sent, None),
        (
            Arc::new(RenamedIdentity),
            State::Refused,
            Some("REMOTE_AGENT_STALE"),
        ),
        (
            Arc::new(CoreClient::at("/never-invoked-cloud-identity".into()).unwrap()),
            State::Refused,
            Some("REMOTE_CLOUD_UPGRADE_REQUIRED"),
        ),
    ];
    for (identity, state, reason) in cases {
        let mut world = cloud_fixture::World::new(NOW, identity);
        let wire = Arc::new(Wire {
            documents: Mutex::new(BTreeMap::new()),
            owner: Mutex::new(Weak::new()),
            cancel_after_create: Mutex::new(None),
            requests: Mutex::new(vec![]),
            deny_responses: AtomicBool::new(false),
            fail_queries: AtomicBool::new(false),
            lose_response_for: Mutex::new(None),
        });
        let driver = world.driver(provider(wire.clone(), "owner"));
        *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
        let sender = Sender::new(
            provider(wire.clone(), "peer"),
            Arc::new(Mutex::new(world.sender_store.take().unwrap())),
            PEER,
            world.key.clone(),
        )
        .unwrap();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let raw = json!({"version":1,"remote":null,"createdAtMs":1800000000000_u64,"requestId":ID,"from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},"message":"prepared effect"}).to_string();
        let answer =
            ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
        assert_eq!(answer.state, state);
        assert_eq!(answer.reason.as_deref(), reason);
        assert_eq!(
            serde_json::to_value(&answer).unwrap()["target"],
            json!({"id":TARGET,"name":"target"})
        );
        assert_eq!(world.core.sends(), usize::from(state == State::Sent));
        let mut local: Value = serde_json::from_str(&raw).unwrap();
        local["remote"] = json!("local");
        let calls = wire.requests.lock().unwrap().len();
        assert_eq!(
            ready(sender.prepared(
                local.to_string().as_bytes(),
                &world.descriptor,
                "local",
                NOW,
                &io
            ))
            .err()
            .unwrap()
            .code,
            "REMOTE_CLOUD_UNAVAILABLE"
        );
        assert_eq!(
            wire.requests.lock().unwrap().len(),
            calls,
            "local selection must never silently switch to cloud"
        );
        let mut changed_time: Value = serde_json::from_str(&raw).unwrap();
        changed_time["createdAtMs"] = json!(NOW + 1);
        assert_eq!(
            ready(sender.prepared(
                changed_time.to_string().as_bytes(),
                &world.descriptor,
                "team",
                NOW,
                &io
            ))
            .err()
            .unwrap()
            .code,
            "REMOTE_CLOUD_CONFLICT"
        );
        let mut old: Value = serde_json::from_str(&raw).unwrap();
        old["createdAtMs"] = json!(NOW - tmt_remote::journal::RECOVERY);
        let calls = wire.requests.lock().unwrap().len();
        assert_eq!(
            ready(sender.prepared(
                old.to_string().as_bytes(),
                &world.descriptor,
                "team",
                NOW,
                &io
            ))
            .err()
            .unwrap()
            .code,
            "REMOTE_CLOUD_MESSAGE_EXPIRED"
        );
        assert_eq!(
            wire.requests.lock().unwrap().len(),
            calls,
            "expired original creation time refuses before HTTP even without a retained row"
        );
        let changed = raw.replace("prepared effect", "different intent");
        let error = ready(sender.prepared(changed.as_bytes(), &world.descriptor, "team", NOW, &io))
            .err()
            .unwrap();
        assert_eq!(error.code, "REMOTE_CLOUD_CONFLICT");
        assert_eq!(world.core.sends(), usize::from(state == State::Sent));
        assert!(
            world.core.calls().iter().all(|c| c["operation"] != "check"),
            "never use a name-addressed capture as a UUID query"
        );
    }
}

#[test]
fn unreadable_response_after_a_confirmed_request_is_unconfirmed_not_refused() {
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(true),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let driver = world.driver(provider(wire.clone(), "owner"));
    *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
    let sender = Sender::new(
        provider(wire.clone(), "peer"),
        Arc::new(Mutex::new(world.sender_store.take().unwrap())),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let raw=json!({"version":1,"remote":"team","createdAtMs":NOW,"requestId":ID,"from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},"message":"confirmed request"}).to_string();
    let answer =
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Unconfirmed);
    assert_eq!(answer.reason.as_deref(), Some("REMOTE_CLOUD_UNCONFIRMED"));
    assert_eq!(world.core.sends(), 1);
}

#[test]
fn dense_backlog_over_the_round_cap_drains_after_restart_without_repeating_effects() {
    use ed25519_dalek::Signer;
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let bound = tmt_remote::journal::OPERATIONS as usize;
    let total = bound + 3;
    // Keep the real per-peer sending budget and transport-storage cap intact. The
    // original peer stays quiet, so its checkpoint cannot pin machine progress.
    for first in (0..total).step_by(tmt_remote::budgets::SENDS as usize) {
        let group = first / tmt_remote::budgets::SENDS as usize;
        let peer = format!("00000000-0000-4000-8000-{:012x}", group + 1);
        let key = ed25519_dalek::SigningKey::from_bytes(&[(group + 100) as u8; 32]);
        let public = key.verifying_key().to_bytes();
        world
            .owner_store
            .lock()
            .unwrap()
            .insert_grant(&tmt_remote::store::Grant {
                client_id: peer.clone(),
                public_key: public,
                kind: "cli".into(),
                origin: "cli".into(),
                name: format!("Backlog peer {group}"),
                agents: "all".into(),
                scopes: vec!["talk".into()],
                mode: "direct".into(),
                issued_at_ms: 1,
                expires_at_ms: None,
                revision: 1,
                disabled: false,
            })
            .unwrap();
        for ordinal in first..(first + tmt_remote::budgets::SENDS as usize).min(total) {
            let id = format!("99999999-9999-4999-8999-{:012x}", ordinal + 1);
            let payload = json!({"version":1,"operation":"dispatch.create","originator":"anonymous","createdAtMs":NOW,"input":{"operationId":id,"recipientIds":[TARGET],"expectedName":"target","message":"one backlog effect","kind":"request"}}).to_string();
            let exact = signed_request_as(&peer, &id, "dispatch.create", &payload, |bytes| {
                key.sign(bytes).to_bytes()
            });
            let (carrier, _) = tmt_remote::cloud_inbox::seal_native(
                &exact,
                &world.descriptor,
                NOW,
                tmt_remote::cloud_hpke::Sender {
                    client_id: &peer,
                    authentication_key: &public,
                },
                |bytes| key.sign(bytes).to_bytes(),
            )
            .unwrap();
            wire.seed_inbox(&id, &carrier);
            // Every document has the exact same indexed/create time. Timestamp-only
            // progress cannot distinguish the prefix from its unprocessed suffix.
            wire.documents
                .lock()
                .unwrap()
                .get_mut(&format!("{ROOT}/m/{MACHINE}/inbox/{id}"))
                .unwrap()["fields"]["peer"] = json!({"stringValue":peer});
        }
    }
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(300),
        cancelled: &stopped,
    };
    let driver = world.driver(provider(wire.clone(), "owner"));
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap_err().code,
        "REMOTE_CLOUD_CAPACITY"
    );
    assert_eq!(
        world.core.sends(),
        bound,
        "the first round performs only its bounded prefix"
    );
    drop(driver);
    world = world.reopen(NOW, Arc::new(CurrentIdentity));
    let driver = world.driver(provider(wire.clone(), "owner"));
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap(),
        total - bound,
        "restart must continue after the durable prefix, including tied timestamps"
    );
    assert_eq!(world.core.sends(), total);
    let mut effects = BTreeMap::new();
    for call in world
        .core
        .calls()
        .into_iter()
        .filter(|call| call["operation"] == "dispatch.create")
    {
        *effects
            .entry(call["input"]["operationId"].as_str().unwrap().to_owned())
            .or_insert(0) += 1;
    }
    assert_eq!(effects.len(), total);
    assert!(
        effects.values().all(|count| *count == 1),
        "every operation has exactly one effect"
    );
}

#[test]
fn transient_stop_resumes_after_unconfirmed_prefix_then_durable_poison() {
    struct FailSecondQuery {
        wire: Arc<Wire>,
        queries: std::sync::atomic::AtomicUsize,
    }
    impl Http for FailSecondQuery {
        fn send(
            &self,
            request: &Request,
            deadline: u64,
            cancelled: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            if request.operation() == Operation::Query
                && self.queries.fetch_add(1, Ordering::SeqCst) == 1
            {
                return Err(AuthError::unavailable());
            }
            self.wire.send(request, deadline, cancelled)
        }
    }
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(Some(format!("{ROOT}/m/{MACHINE}/responses/{ID}"))),
    });
    let bad = "55555555-5555-4555-8555-555555555555";
    let good2 = "77777777-7777-4777-8777-777777777777";
    for id in [ID, bad, good2] {
        let carrier = if id == bad {
            b"{}".to_vec()
        } else {
            tmt_remote::cloud_inbox::seal_native(
                &request(&world.key, id),
                &world.descriptor,
                NOW,
                tmt_remote::cloud_hpke::Sender {
                    client_id: PEER,
                    authentication_key: &world.key.public(),
                },
                |bytes| world.key.sign(bytes),
            )
            .unwrap()
            .0
        };
        wire.seed_inbox(id, &carrier);
    }
    let fault = Arc::new(FailSecondQuery {
        wire: wire.clone(),
        queries: std::sync::atomic::AtomicUsize::new(0),
    });
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let driver = world.driver(provider_clock(fault.clone(), "owner", Arc::new(ClockPort)));
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(fault.queries.load(Ordering::SeqCst), 2);
    assert_eq!(world.core.sends(), 1);
    drop(driver);
    world = world.reopen(NOW, Arc::new(CurrentIdentity));
    let driver = world.driver(provider_clock(fault, "owner", Arc::new(ClockPort)));
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap(),
        2,
        "the resumed query skips its durable unconfirmed prefix"
    );
    assert_eq!(world.core.sends(), 2);
    let database = rusqlite::Connection::open_with_flags(
        world.owner_layout.directory.join("remote.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT refusal FROM cloud_transport WHERE peer_id=?1 AND id=?2",
                [PEER, bad],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "REMOTE_CLOUD_CARRIER_INVALID"
    );
    assert!(
        database
            .query_row(
                "SELECT attempted FROM cloud_transport WHERE peer_id=?1 AND id=?2",
                [PEER, ID],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert!(
        !wire
            .documents
            .lock()
            .unwrap()
            .contains_key(&format!("{ROOT}/m/{MACHINE}/responses/{ID}")),
        "advancing never retries an unconfirmed response mutation"
    );
    let continuation: Vec<u8> = database
        .query_row(
            "SELECT carrier FROM cloud_transport WHERE kind='checkpoint' AND peer_id='inbox-query' AND id=''",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        continuation.is_empty(),
        "only a fully drained query clears the continuation"
    );
}

#[test]
fn poison_between_good_carriers_is_durable_per_document_and_transient_reads_do_not_advance() {
    use rusqlite::OptionalExtension;
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(true),
        lose_response_for: Mutex::new(None),
    });
    let bad = "55555555-5555-4555-8555-555555555555";
    let good2 = "77777777-7777-4777-8777-777777777777";
    for id in [ID, bad, good2] {
        let carrier = if id == bad {
            b"{}".to_vec()
        } else {
            tmt_remote::cloud_inbox::seal_native(
                &request(&world.key, id),
                &world.descriptor,
                NOW,
                tmt_remote::cloud_hpke::Sender {
                    client_id: PEER,
                    authentication_key: &world.key.public(),
                },
                |bytes| world.key.sign(bytes),
            )
            .unwrap()
            .0
        };
        wire.seed_inbox(id, &carrier);
    }
    let inspect = || {
        rusqlite::Connection::open_with_flags(
            world.owner_layout.directory.join("remote.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap()
    };
    let checkpoint = || {
        inspect()
            .query_row(
                "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint'",
                ["inbox-query"],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .unwrap_or(0)
    };
    let driver = world.driver(provider(wire.clone(), "owner"));
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    assert!(ready(driver.reconcile(NOW, &io)).is_err());
    assert_eq!(checkpoint(), 0, "transient read failure cannot advance");
    wire.fail_queries.store(false, Ordering::SeqCst);
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap(),
        3,
        "poison must not stop the machine inbox"
    );
    assert_eq!(
        world.core.sends(),
        2,
        "both good messages have one Core effect"
    );
    assert_eq!(checkpoint(), NOW as i64);
    let refused: String = inspect()
        .query_row(
            "SELECT refusal FROM cloud_transport WHERE peer_id=?1 AND id=?2",
            [PEER, bad],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(refused, "REMOTE_CLOUD_CARRIER_INVALID");
    assert_eq!(
        inspect()
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE envelope_id=?1 AND decision='refused'",
                [bad],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(driver);
    world = world.reopen(NOW, Arc::new(CurrentIdentity));
    let driver = world.driver(provider(wire.clone(), "owner"));
    assert_eq!(ready(driver.reconcile(NOW, &io)).unwrap(), 3);
    assert_eq!(world.core.sends(), 2);
    let inspect = rusqlite::Connection::open_with_flags(
        world.owner_layout.directory.join("remote.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    assert_eq!(
        inspect
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE envelope_id=?1 AND decision='refused'",
                [bad],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1,
        "restart deduplicates refused IDs"
    );
}

struct BackoffClock {
    stopped: Arc<AtomicBool>,
    ticks: std::sync::atomic::AtomicU64,
    waits: Mutex<Vec<u64>>,
}
impl Clock for BackoffClock {
    fn now(&self) -> Time {
        let elapsed = self.ticks.load(Ordering::SeqCst);
        Time {
            wall_ms: NOW + elapsed,
            monotonic_ms: elapsed,
        }
    }
    fn wait_until(&self, until: u64, _: &dyn Cancellation) -> bool {
        let before = self.ticks.swap(until, Ordering::SeqCst);
        let mut waits = self.waits.lock().unwrap();
        waits.push(until - before);
        if waits.len() == 2 {
            self.stopped.store(true, Ordering::SeqCst);
            return false;
        }
        true
    }
}
#[test]
fn transient_poll_failure_backs_off_until_cancellation_without_a_checkpoint() {
    let world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(true),
        lose_response_for: Mutex::new(None),
    });
    let stopped = Arc::new(AtomicBool::new(false));
    let clock = Arc::new(BackoffClock {
        stopped: stopped.clone(),
        ticks: std::sync::atomic::AtomicU64::new(0),
        waits: Mutex::new(vec![]),
    });
    let driver = world.driver(provider_clock(wire.clone(), "owner", clock.clone()));
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: stopped.as_ref(),
    };
    assert!(ready(driver.run(&io)).is_err());
    assert_eq!(
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| *op == Operation::Query)
            .count(),
        2,
        "transient failure does not end the worker"
    );
    assert_eq!(clock.waits.lock().unwrap().len(), 2);
    assert!(
        clock
            .waits
            .lock()
            .unwrap()
            .iter()
            .all(|ms| *ms >= tmt_remote::limits::CLOUD_POLL_MIN_MS
                && *ms <= tmt_remote::limits::CLOUD_POLL_IDLE_MS)
    );
    let inspect = rusqlite::Connection::open_with_flags(
        world.owner_layout.directory.join("remote.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    assert_eq!(
        inspect
            .query_row(
                "SELECT COUNT(*) FROM cloud_transport WHERE kind='checkpoint'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(world.core.sends(), 0);
}

#[test]
fn lost_response_post_does_not_block_later_carriers_or_repeat_after_restart() {
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
    let middle = "55555555-5555-4555-8555-555555555555";
    let last = "77777777-7777-4777-8777-777777777777";
    let response = format!("{ROOT}/m/{MACHINE}/responses/{middle}");
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(Some(response.clone())),
    });
    for id in [ID, middle, last] {
        let carrier = tmt_remote::cloud_inbox::seal_native(
            &request(&world.key, id),
            &world.descriptor,
            NOW,
            tmt_remote::cloud_hpke::Sender {
                client_id: PEER,
                authentication_key: &world.key.public(),
            },
            |bytes| world.key.sign(bytes),
        )
        .unwrap()
        .0;
        wire.seed_inbox(id, &carrier);
    }
    let driver = world.driver(provider(wire.clone(), "owner"));
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap(),
        3,
        "one ambiguous response must not block the later carrier"
    );
    assert_eq!(world.core.sends(), 3, "one effect per distinct original ID");
    assert!(!wire.documents.lock().unwrap().contains_key(&response));
    assert!(
        wire.documents
            .lock()
            .unwrap()
            .contains_key(&format!("{ROOT}/m/{MACHINE}/responses/{last}"))
    );
    let database = world.owner_layout.directory.join("remote.db");
    let inspect = || {
        rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap()
    };
    let before:(Vec<u8>,bool,Option<String>)=inspect().query_row("SELECT response,attempted,refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",[PEER,middle],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert!(
        before.1 && before.2.is_none(),
        "unconfirmed publication is not a refusal"
    );
    assert_eq!(
        inspect()
            .query_row(
                "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint'",
                ["inbox-query"],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        NOW as i64
    );
    drop(driver);
    world = world.reopen(NOW, Arc::new(CurrentIdentity));
    let driver = world.driver(provider(wire.clone(), "owner"));
    let posts = || {
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, _)| *op == Operation::Commit)
            .count()
    };
    let posts_before = posts();
    assert_eq!(ready(driver.reconcile(NOW, &io)).unwrap(), 3);
    assert_eq!(world.core.sends(), 3);
    assert_eq!(
        posts(),
        posts_before,
        "restart never retries an attempted response POST"
    );
    let after:(Vec<u8>,bool,Option<String>)=inspect().query_row("SELECT response,attempted,refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",[PEER,middle],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(
        after, before,
        "restart preserves the exact unconfirmed response state"
    );
}

#[test]
fn response_budget_refusal_before_claim_holds_the_checkpoint_and_retries_without_effect() {
    struct ReplyBudget {
        inner: Budget,
        deny: AtomicBool,
    }
    impl NativeBudgetPort for ReplyBudget {
        fn usage(&self, project: &str) -> Result<NativeUsage, SyncError> {
            self.inner.usage(project)
        }
        fn charge(
            &self,
            project: &str,
            model: &Model,
            cost: u64,
            time: Time,
        ) -> Result<Decision, SyncError> {
            if cost == tmt_remote::firestore_budget::WRITE_LOOKUPS
                && self.deny.load(Ordering::SeqCst)
            {
                return Err(SyncError::BudgetExhausted(
                    tmt_remote::firestore_budget::Refusal {
                        reset_at_ms: NOW + 1000,
                        retry_after_ms: 1000,
                    },
                ));
            }
            self.inner.charge(project, model, cost, time)
        }
        fn flush(&self) -> Result<(), SyncError> {
            self.inner.flush()
        }
    }
    let world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let carrier = tmt_remote::cloud_inbox::seal_native(
        &request(&world.key, ID),
        &world.descriptor,
        NOW,
        tmt_remote::cloud_hpke::Sender {
            client_id: PEER,
            authentication_key: &world.key.public(),
        },
        |bytes| world.key.sign(bytes),
    )
    .unwrap()
    .0;
    wire.seed_inbox(ID, &carrier);
    let budget = Arc::new(ReplyBudget {
        inner: Budget(Mutex::new(NativeUsage::new(NOW))),
        deny: AtomicBool::new(true),
    });
    let runtime = Arc::new(NativeRuntime::new(
        wire.clone(),
        Arc::new(ClockPort),
        Arc::new(ClockPort),
        budget.clone(),
    ));
    let driver = world.driver(
        Provider::new(
            Arc::new(Auth("owner")),
            runtime,
            Model::new(4, 4).unwrap(),
            MACHINE,
        )
        .unwrap(),
    );
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap_err().code,
        "REMOTE_BUDGET_EXHAUSTED"
    );
    let database = world.owner_layout.directory.join("remote.db");
    let inspect = || {
        rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap()
    };
    assert_eq!(
        inspect()
            .query_row(
                "SELECT COUNT(*) FROM cloud_transport WHERE kind='checkpoint'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(
        !inspect()
            .query_row(
                "SELECT attempted FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
                [PEER, ID],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    assert!(
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .all(|(op, _)| *op != Operation::Commit),
        "no publication HTTP before the budget guard"
    );
    assert_eq!(world.core.sends(), 1);
    budget.deny.store(false, Ordering::SeqCst);
    assert_eq!(ready(driver.reconcile(NOW, &io)).unwrap(), 1);
    assert_eq!(
        world.core.sends(),
        1,
        "a pre-claim retry uses the frozen outcome, not a new effect"
    );
    assert_eq!(
        inspect()
            .query_row(
                "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint'",
                ["inbox-query"],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        NOW as i64
    );
}

#[test]
fn unknown_owned_lookup_is_permanent_but_real_store_failure_holds_the_checkpoint() {
    let world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
    let middle = "55555555-5555-4555-8555-555555555555";
    let last = "77777777-7777-4777-8777-777777777777";
    let absent = "99999999-9999-4999-8999-999999999999";
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let seed = |id: &str, exact: Vec<u8>| {
        let carrier = tmt_remote::cloud_inbox::seal_native(
            &exact,
            &world.descriptor,
            NOW,
            tmt_remote::cloud_hpke::Sender {
                client_id: PEER,
                authentication_key: &world.key.public(),
            },
            |bytes| world.key.sign(bytes),
        )
        .unwrap()
        .0;
        wire.seed_inbox(id, &carrier);
    };
    seed(
        middle,
        signed_request(
            &world.key,
            middle,
            "operation.show",
            &json!({"operationId":absent}).to_string(),
        ),
    );
    let driver = world.driver(provider(wire.clone(), "owner"));
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let db = rusqlite::Connection::open(world.owner_layout.directory.join("remote.db")).unwrap();
    db.execute_batch("ALTER TABLE operations RENAME TO unavailable_operations")
        .unwrap();
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE",
        "a genuine SQL read failure stays retryable"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM cloud_transport WHERE kind='checkpoint'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        db.query_row(
            "SELECT refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
            [PEER, middle],
            |row| row.get::<_, Option<String>>(0)
        )
        .unwrap()
        .is_none()
    );
    db.execute_batch("ALTER TABLE unavailable_operations RENAME TO operations")
        .unwrap();
    seed(ID, request(&world.key, ID));
    seed(last, request(&world.key, last));
    assert_eq!(
        ready(driver.reconcile(NOW, &io)).unwrap(),
        3,
        "an unknown ID between good messages cannot stop the inbox"
    );
    assert_eq!(
        world.core.sends(),
        2,
        "both good IDs execute once; the unknown read never has an effect"
    );
    assert_eq!(
        db.query_row(
            "SELECT refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
            [PEER, middle],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(
        db.query_row(
            "SELECT created_ms FROM cloud_transport WHERE peer_id=?1 AND kind='checkpoint'",
            ["inbox-query"],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        NOW as i64
    );
    assert!(
        wire.documents
            .lock()
            .unwrap()
            .contains_key(&format!("{ROOT}/m/{MACHINE}/responses/{last}"))
    );
}

#[test]
fn sender_before_claim_refusal_can_make_its_first_post_without_reading_a_missing_document() {
    struct BeforeClaimBudget(AtomicBool);
    impl NativeBudgetPort for BeforeClaimBudget {
        fn usage(&self, _: &str) -> Result<NativeUsage, SyncError> {
            Ok(NativeUsage::new(NOW))
        }
        fn charge(&self, _: &str, _: &Model, _: u64, _: Time) -> Result<Decision, SyncError> {
            if self.0.load(Ordering::SeqCst) {
                return Err(SyncError::BudgetExhausted(
                    tmt_remote::firestore_budget::Refusal {
                        reset_at_ms: NOW + 1000,
                        retry_after_ms: 1000,
                    },
                ));
            }
            Ok(Decision::Allow)
        }
        fn flush(&self) -> Result<(), SyncError> {
            Ok(())
        }
    }
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let driver = world.driver(provider(wire.clone(), "owner"));
    *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
    let budget = Arc::new(BeforeClaimBudget(AtomicBool::new(true)));
    let runtime = Arc::new(NativeRuntime::new(
        wire.clone(),
        Arc::new(ClockPort),
        Arc::new(ClockPort),
        budget.clone(),
    ));
    let sender = Sender::new(
        Provider::new(
            Arc::new(Auth("peer")),
            runtime,
            Model::new(4, 4).unwrap(),
            MACHINE,
        )
        .unwrap(),
        Arc::new(Mutex::new(world.sender_store.take().unwrap())),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let raw=json!({"version":1,"remote":"team","createdAtMs":NOW,"requestId":ID,"from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},"message":"before-claim effect"}).to_string();
    let error = ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io))
        .err()
        .unwrap();
    assert_eq!(error.code, "REMOTE_BUDGET_EXHAUSTED");
    assert!(wire.requests.lock().unwrap().is_empty());
    let inspect =
        rusqlite::Connection::open(world.sender_layout.directory.join("remote.db")).unwrap();
    let attempted: bool = inspect
        .query_row(
            "SELECT attempted FROM cloud_transport WHERE kind='outbox' AND id=?1",
            [ID],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!attempted);
    budget.0.store(false, Ordering::SeqCst);
    let answer =
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Sent);
    assert_eq!(world.core.sends(), 1);
    let calls = wire.requests.lock().unwrap();
    assert_eq!(
        calls[0].0,
        Operation::Commit,
        "unattempted frozen bytes need no missing-document read"
    );
}

#[test]
fn missing_response_then_owner_reply_within_the_call_returns_the_confirmed_outcome() {
    struct DeferredOwner {
        wire: Arc<Wire>,
        owner: Mutex<Weak<InboxDriver>>,
        notified: AtomicBool,
    }
    impl Http for DeferredOwner {
        fn send(
            &self,
            request: &Request,
            deadline: u64,
            cancel: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            let reply = self.wire.send(request, deadline, cancel)?;
            if request.operation() == Operation::Read
                && request.url().contains("/responses/")
                && reply.status == 404
                && !self.notified.swap(true, Ordering::SeqCst)
            {
                let owner = self.owner.lock().unwrap().upgrade().unwrap();
                let stopped = AtomicBool::new(false);
                let io = IoBudget {
                    deadline: Instant::now() + Duration::from_secs(5),
                    cancelled: &stopped,
                };
                ready(owner.reconcile(NOW, &io)).unwrap();
            }
            Ok(reply)
        }
    }
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
        lose_response_for: Mutex::new(None),
    });
    let deferred = Arc::new(DeferredOwner {
        wire: wire.clone(),
        owner: Mutex::new(Weak::new()),
        notified: AtomicBool::new(false),
    });
    let make_provider = |uid| {
        Provider::new(
            Arc::new(Auth(uid)),
            Arc::new(NativeRuntime::new(
                deferred.clone(),
                Arc::new(ClockPort),
                Arc::new(ClockPort),
                Arc::new(Budget(Mutex::new(NativeUsage::new(NOW)))),
            )),
            Model::new(4, 4).unwrap(),
            MACHINE,
        )
        .unwrap()
    };
    let owner = world.driver(make_provider("owner"));
    *deferred.owner.lock().unwrap() = Arc::downgrade(&owner);
    let sender = Sender::new(
        make_provider("peer"),
        Arc::new(Mutex::new(world.sender_store.take().unwrap())),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let raw=json!({"version":1,"remote":"team","createdAtMs":NOW,"requestId":ID,"from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},"message":"delayed owner effect"}).to_string();
    let answer =
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Sent);
    assert_eq!(world.core.sends(), 1);
    assert!(deferred.notified.load(Ordering::SeqCst));
    assert_eq!(
        wire.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(op, path)| *op == Operation::Read && path.contains("/responses/"))
            .count(),
        3,
        "sender observes missing then present, plus the owner's publication readback"
    );
}

#[test]
fn prepared_name_targets_resolve_once_or_record_a_per_document_refusal() {
    use tmt_remote::cloud_command::State;
    for (rows, expected, reason) in [
        (json!([{"id":TARGET,"name":"target"}]), State::Sent, None),
        (json!([]), State::Refused, Some("REMOTE_AGENT_NOT_FOUND")),
        (
            json!([{"id":TARGET,"name":"target"},{"id":ID,"name":"target"}]),
            State::Refused,
            Some("REMOTE_AGENT_AMBIGUOUS"),
        ),
    ] {
        let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
        std::fs::write(
            world.core.root.join("identities"),
            json!({"identities":rows}).to_string(),
        )
        .unwrap();
        let wire = Arc::new(Wire {
            documents: Mutex::new(BTreeMap::new()),
            owner: Mutex::new(Weak::new()),
            cancel_after_create: Mutex::new(None),
            requests: Mutex::new(vec![]),
            deny_responses: AtomicBool::new(false),
            fail_queries: AtomicBool::new(false),
            lose_response_for: Mutex::new(None),
        });
        let driver = world.driver(provider(wire.clone(), "owner"));
        *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
        let sender = Sender::new(
            provider(wire.clone(), "peer"),
            Arc::new(Mutex::new(world.sender_store.take().unwrap())),
            PEER,
            world.key.clone(),
        )
        .unwrap();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let raw = json!({"version":1,"remote":"team","createdAtMs":NOW,"requestId":ID,"from":{"id":PEER,"name":"peer"},"target":{"name":"target"},"message":"resolve at receiver"}).to_string();
        let answer =
            ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", NOW, &io)).unwrap();
        assert_eq!(answer.state, expected);
        assert_eq!(answer.reason.as_deref(), reason);
        assert_eq!(
            serde_json::to_value(&answer).unwrap()["target"],
            json!({"name":"target"})
        );
        assert_eq!(world.core.sends(), usize::from(expected == State::Sent));
        if expected == State::Sent {
            let calls = world.core.calls();
            let created = calls
                .iter()
                .find(|c| c["operation"] == "dispatch.create")
                .unwrap();
            assert_eq!(created["input"]["recipientIds"], json!([TARGET]));
            assert_eq!(
                created["input"]["message"],
                "[remote: Peer]\nresolve at receiver"
            );
        }
        if let Some(reason) = reason {
            let db =
                rusqlite::Connection::open(world.owner_layout.directory.join("remote.db")).unwrap();
            let stored: String = db
                .query_row(
                    "SELECT refusal FROM cloud_transport WHERE id=?1 AND kind='inbox'",
                    [ID],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored, reason);
        }
        let calls = world.core.calls().len();
        ready(driver.reconcile(NOW, &io)).unwrap();
        assert_eq!(
            world.core.calls().len(),
            calls,
            "duplicates never resolve or dispatch again"
        );
    }
}
