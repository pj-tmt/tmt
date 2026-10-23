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
                assert_eq!(input["structuredQuery"]["limit"], 1);
                assert_eq!(
                    input["structuredQuery"]["from"],
                    json!([{"collectionId":"inbox","allDescendants":true}])
                );
                let after =
                    input["structuredQuery"]["startAt"]["values"][1]["referenceValue"].as_str();
                let rows: Vec<_> = self
                    .documents
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(name, _)| {
                        name.starts_with(&format!("{ROOT}/m/{MACHINE}/"))
                            && name.contains("/inbox/")
                            && after.is_none_or(|after| name.as_str() > after)
                    })
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
    wire: Arc<Wire>,
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
    let window = "22222222-2222-4222-8222-222222222222";
    let session = "55555555-5555-4555-8555-555555555555";
    let payload=json!({"version":1,"operation":"dispatch.create","originator":"anonymous","createdAtMs":NOW,"input":{"operationId":id,"recipientIds":[TARGET],"expectedName":"target","message":"one offline effect","kind":"request"}}).to_string();
    let envelope = Envelope {
        kind: "request",
        id,
        correlation_id: None,
        machine_id: MACHINE,
        window_id: window,
        client_id: PEER,
        session_id: session,
        sequence: "1",
        timestamp_ms: NOW,
        origin: "cli",
        operation: "dispatch.create",
        payload: payload.as_bytes(),
    };
    json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,"machineId":MACHINE,"windowId":window,"clientId":PEER,"sessionId":session,"sequence":"1","timestampMs":NOW,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(payload.as_bytes()),"signature":canonical::base64url(&key.sign(&canonical::envelope(&envelope).unwrap()))}).to_string().into_bytes()
}
#[test]
fn lost_create_answer_then_sender_restart_recovers_by_fresh_id_with_one_core_effect() {
    let mut world = cloud_fixture::World::new(NOW, Arc::new(CurrentIdentity));
    let wire = Arc::new(Wire {
        documents: Mutex::new(BTreeMap::new()),
        owner: Mutex::new(Weak::new()),
        cancel_after_create: Mutex::new(None),
        requests: Mutex::new(vec![]),
        deny_responses: AtomicBool::new(false),
        fail_queries: AtomicBool::new(false),
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
    drop(sender);
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
    drop(sender);
    drop(driver);
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
fn poison_between_good_carriers_is_durable_per_document_and_transient_reads_do_not_advance() {
    use base64::{Engine, engine::general_purpose::STANDARD};
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
        let name = format!("{ROOT}/m/{MACHINE}/peers/{PEER}/inbox/{id}");
        wire.documents.lock().unwrap().insert(name.clone(),json!({"name":name,"createTime":STAMP,"fields":{"peerId":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(carrier)},"createdAt":{"timestampValue":STAMP}}}));
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
                [PEER],
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
