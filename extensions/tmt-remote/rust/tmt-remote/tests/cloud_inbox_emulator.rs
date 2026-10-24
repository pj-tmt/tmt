//! Explicit emulator qualification: ordinary-user ProductionHttp drives the real driver.
//! Run only in an Infra slot with both test-emulator hosts set to loopback addresses.
#![cfg(feature = "test-emulator")]
#[path = "support/emulator_http.rs"]
mod emulator_http;
use emulator_http::EMULATOR_HTTP_TIMEOUT;
#[allow(dead_code)]
#[path = "support/cloud_inbox.rs"]
mod fixture;
use fixture::{MACHINE, PEER, TARGET, World};
use serde_json::{Value, json};
use std::{
    future::Future,
    net::SocketAddr,
    pin::pin,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tmt_remote::{
    canonical,
    cloud_inbox::{InboxDriver, Provider, Sender},
    error::RemoteError,
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, ProductionHttp, Project, Request,
        Response, SystemClock, Time,
    },
    firestore_budget::{Decision, Model, NativeUsage, charge_native},
    native_sync::{NativeBudgetPort, NativeRuntime},
    objects::IoBudget,
    sealed_sync::{AuthFailure, AuthIdentity, AuthPort, AuthToken, SyncError},
    store::{Store, uuid_v4},
};
const PROJECT: &str = "demo-cloud-inbox";
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    let Poll::Ready(value) = future
        .as_mut()
        .poll(&mut Context::from_waker(std::task::Waker::noop()))
    else {
        panic!("fixture ports complete synchronously")
    };
    value
}
struct Auth(&'static str);
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: PROJECT.into(),
            uid: self.0.into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        Box::pin(async move {
            // Firebase's emulator accepts an unsigned token; production never accepts this.
            let header = canonical::base64url(br#"{"alg":"none","typ":"JWT"}"#);
            let body = canonical::base64url(
                json!({"sub":self.0,"user_id":self.0,
                "aud":PROJECT,"iss":format!("https://securetoken.google.com/{PROJECT}"),
                "firebase":{"sign_in_provider":"anonymous"}})
                .to_string()
                .as_bytes(),
            );
            Ok(AuthToken {
                identity: self.identity(),
                token: format!("{header}.{body}."),
                expires_at_ms: u64::MAX,
            })
        })
    }
}
struct Budget {
    usage: Mutex<NativeUsage>,
    refuse: AtomicBool,
}
impl NativeBudgetPort for Budget {
    fn usage(&self, _: &str) -> Result<NativeUsage, SyncError> {
        Ok(*self.usage.lock().unwrap())
    }
    fn charge(&self, _: &str, model: &Model, cost: u64, time: Time) -> Result<Decision, SyncError> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err(SyncError::BudgetExhausted(
                tmt_remote::firestore_budget::Refusal {
                    reset_at_ms: time.wall_ms + 1_000,
                    retry_after_ms: 1_000,
                },
            ));
        }
        charge_native(model, &mut self.usage.lock().unwrap(), cost, time.wall_ms)
            .map_err(|_| SyncError::Unavailable)
    }
    fn flush(&self) -> Result<(), SyncError> {
        Ok(())
    }
}
struct ZeroJitter;
impl Jitter for ZeroJitter {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
// Retain enough structure to reproduce a codec refusal without logging provider values.
fn sanitized_shape(value: &Value, depth: usize, nodes: &mut usize) -> Value {
    if *nodes == 0 {
        return json!({"type":"truncated"});
    }
    *nodes -= 1;
    match value {
        Value::Object(fields) => {
            let known = [
                "document",
                "readTime",
                "name",
                "fields",
                "createTime",
                "updateTime",
                "peer",
                "carrier",
                "createdAt",
                "timestampValue",
                "stringValue",
                "bytesValue",
                "result",
                "done",
                "transaction",
                "skippedResults",
                "error",
                "code",
                "status",
                "message",
                "details",
            ];
            let rows: Vec<_> = fields
                .iter()
                .take(if depth < 6 { 8 } else { 0 })
                .map(|(name, value)| {
                    let field = if known.contains(&name.as_str()) {
                        name.as_str()
                    } else {
                        "<other>"
                    };
                    let shape = sanitized_shape(value, depth + 1, nodes);
                    json!({"field":field,"shape":shape})
                })
                .collect();
            json!({"type":"object","fields":fields.len(),"members":rows})
        }
        Value::Array(values) => {
            let items: Vec<_> = values
                .iter()
                .take(if depth < 6 { 2 } else { 0 })
                .map(|value| sanitized_shape(value, depth + 1, nodes))
                .collect();
            json!({"type":"array","length":values.len(),"items":items})
        }
        Value::String(text) => json!({"type":"string","bytes":text.len()}),
        Value::Number(_) => json!({"type":"number"}),
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Null => json!({"type":"null"}),
    }
}
#[test]
fn shape_evidence_contains_no_provider_identifier_token_or_carrier_values() {
    let value = json!({"document":{"name":"SECRET_NAME","fields":{"peer":{"stringValue":"SECRET_PEER"},"carrier":{"bytesValue":"SECRET_CARRIER"}}},"SECRET_FIELD":"SECRET_VALUE"});
    let shape = sanitized_shape(&value, 0, &mut 64).to_string();
    assert!(!shape.contains("SECRET"));
    assert!(shape.contains("bytesValue"));
    let mut deep = Value::Null;
    for _ in 0..100 {
        deep = json!({"document":deep});
    }
    assert!(
        sanitized_shape(&deep, 0, &mut 64).to_string().len() < 2048,
        "diagnostic traversal remains bounded"
    );
}
struct Wire {
    http: ProductionHttp,
    clock: Arc<SystemClock>,
    owner: Mutex<Weak<InboxDriver>>,
    counts: Mutex<Vec<(Operation, u16)>>,
}
impl Http for Wire {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        let response = self.http.send(request, deadline, cancel)?;
        if matches!(request.operation(), Operation::Read | Operation::Query) {
            let shape = serde_json::from_slice(&response.body)
                .map(|value| sanitized_shape(&value, 0, &mut 64))
                .unwrap_or_else(|_| json!({"type":"invalid-json","bytes":response.body.len()}));
            println!(
                "cloud response_shape operation={:?} status={} shape={shape}",
                request.operation(),
                response.status
            );
        }

        if response.status >= 400 {
            let value: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
            let known: Option<tmt_remote::error::CloudProviderStatus> =
                serde_json::from_value(value["error"]["status"].clone()).ok();
            println!(
                "cloud HTTP failure operation={:?} status={} error_status={known:?}",
                request.operation(),
                response.status
            );
        }
        self.counts
            .lock()
            .unwrap()
            .push((request.operation(), response.status));
        if request.operation() == Operation::Commit && response.status == 200 {
            let body: Value = serde_json::from_slice(request.body()).unwrap();
            if body["writes"][0]["update"]["name"]
                .as_str()
                .is_some_and(|p| p.contains("/inbox/"))
                && let Some(owner) = self.owner.lock().unwrap().upgrade()
            {
                let cancelled = AtomicBool::new(false);
                let io = IoBudget {
                    deadline: Instant::now() + Duration::from_secs(30),
                    cancelled: &cancelled,
                };
                if let Err(error) = ready(owner.reconcile(self.clock.now().wall_ms, &io)) {
                    self.report("reconciliation-failure");
                    eprintln!("cloud reconciliation refusal {}", error.json_value());
                    panic!("owner reconciliation refused");
                }
            }
        }
        Ok(response)
    }
}
impl Wire {
    fn report(&self, phase: &str) {
        let calls = std::mem::take(&mut *self.counts.lock().unwrap());
        let reads = calls
            .iter()
            .filter(|(op, _)| matches!(op, Operation::Read | Operation::Query))
            .count();
        let writes = calls
            .iter()
            .filter(|(op, _)| *op == Operation::Commit)
            .count();
        println!(
            "cloud phase={phase} wire_read_calls={reads} wire_commit_calls={writes} statuses={:?}",
            calls.iter().map(|(_, status)| status).collect::<Vec<_>>()
        );
    }
}
struct CurrentIdentity;
impl tmt_remote::core::CloudIdentityQuery for CurrentIdentity {
    fn identity_show(&self, id: &str, name: &str, _: &AtomicBool) -> Result<(), RemoteError> {
        assert_eq!((id, name), (TARGET, "target"));
        Ok(())
    }
}
fn provider(wire: Arc<Wire>, uid: &'static str, budget: Arc<Budget>) -> Provider {
    let runtime = Arc::new(NativeRuntime::new(
        wire.clone(),
        wire.clock.clone(),
        Arc::new(ZeroJitter),
        budget,
    ));
    Provider::new(
        Arc::new(Auth(uid)),
        runtime,
        Model::new(4, 4).unwrap(),
        MACHINE,
    )
    .unwrap()
}
struct Admin {
    agent: ureq::Agent,
    base: String,
}
impl Admin {
    fn new() -> Self {
        let host =
            std::env::var("FIRESTORE_EMULATOR_HOST").expect("explicit emulator host required");
        let address: SocketAddr = host.parse().expect("numeric loopback emulator address");
        assert!(address.ip().is_loopback() && address.port() != 0);
        assert!(std::env::var("FIREBASE_AUTH_EMULATOR_HOST").is_ok());
        Self {
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .http_status_as_error(false)
                .timeout_global(Some(EMULATOR_HTTP_TIMEOUT))
                .build()
                .into(),
            base: format!("http://{address}"),
        }
    }
    fn status(&self, method: &str, path: &str, body: Value, bearer: &str) -> (u16, String) {
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&body).unwrap())
            .unwrap();
        let mut response = self.agent.run(request).unwrap();
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap();
        (status, text)
    }
    fn call(&self, method: &str, path: &str, body: Value) -> u16 {
        let (status, text) = self.status(method, path, body, "owner");
        assert_eq!(
            status, 200,
            "fixture setup/cleanup failed: {status}: {text}"
        );
        status
    }
    fn seed(&self, path: &str, fields: Value) {
        self.call(
            "PATCH",
            &format!("/v1/projects/{PROJECT}/databases/(default)/documents/{path}"),
            json!({"fields":fields}),
        );
    }
}
#[test]
#[ignore = "explicit Infra emulator slot; no provider fallback"]
fn ordinary_users_deliver_recover_restart_revoke_and_stop_before_budget_io() {
    let admin = Admin::new();
    admin.call(
        "DELETE",
        &format!("/emulator/v1/projects/{PROJECT}/databases/(default)/documents"),
        Value::Null,
    );
    admin.call("PUT",&format!("/emulator/v1/projects/{PROJECT}:securityRules"),json!({"rules":{"files":[{"name":"firestore.rules","content":tmt_remote::cloud_inbox::rules::artifact()}]}}));
    admin.seed(
        &format!("m/{MACHINE}"),
        json!({"uid":{"stringValue":"owner"}}),
    );
    let peer_row = |revoked| json!({"peer":{"stringValue":PEER},"revoked":{"booleanValue":revoked},"expiresAt":{"timestampValue":"2035-01-01T00:00:00Z"}});
    admin.seed(&format!("m/{MACHINE}/peers/peer"), peer_row(false));
    let clock = Arc::new(SystemClock::default());
    let now = clock.now().wall_ms;
    let mut world = World::new(now, Arc::new(CurrentIdentity));
    let http = ProductionHttp::new(
        &Project::new(PROJECT, "123456", "emulator-key").unwrap(),
        clock.clone(),
    )
    .unwrap();
    assert!(http.emulator_active(), "no production fallback");
    let wire = Arc::new(Wire {
        http,
        clock,
        owner: Mutex::new(Weak::new()),
        counts: Mutex::new(vec![]),
    });
    let budget = Arc::new(Budget {
        usage: Mutex::new(NativeUsage::new(now)),
        refuse: AtomicBool::new(false),
    });
    let driver = world.driver(provider(wire.clone(), "owner", budget.clone()));
    *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
    let store = Arc::new(Mutex::new(world.sender_store.take().unwrap()));
    let mut sender = Sender::new(
        provider(wire.clone(), "peer", budget.clone()),
        store.clone(),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(120),
        cancelled: &stopped,
    };
    let id = uuid_v4().unwrap();
    let raw=json!({"version":1,"remote":"team","createdAtMs":now,"requestId":id,"from":{"id":PEER,"name":"peer"},"target":{"id":TARGET,"name":"target"},"message":"one emulator effect"}).to_string();
    let answer =
        ready(sender.prepared(raw.as_bytes(), &world.descriptor, "team", now, &io)).unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Sent);
    assert_eq!(world.core.sends(), 1);
    wire.report("delivery");
    let auth = Auth("peer");
    let token = ready(auth.token(&io)).unwrap().token;
    let root = format!("projects/{PROJECT}/databases/(default)/documents");
    let path = format!("/v1/{root}/m/{MACHINE}/inbox/{id}");
    // Updates and deletes are never sender capabilities, even for its own ID.
    assert_eq!(
        admin
            .status(
                "PATCH",
                &path,
                json!({"fields":{"carrier":{"bytesValue":"eA=="}}}),
                &token
            )
            .0,
        403
    );
    assert_eq!(admin.status("DELETE", &path, Value::Null, &token).0, 403);
    let fresh = uuid_v4().unwrap();
    let name = format!("{root}/m/{MACHINE}/inbox/{fresh}");
    let write = json!({"update":{"name":name,"fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":"eA=="},"extra":{"booleanValue":true}}},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}],"currentDocument":{"exists":false}});
    assert_eq!(
        admin
            .status(
                "POST",
                &format!("/v1/{root}:commit"),
                json!({"writes":[write]}),
                &token
            )
            .0,
        403,
        "extra fields refused"
    );
    let stranger = ready(Auth("stranger").token(&io)).unwrap().token;
    assert_eq!(
        admin.status("GET", &path, Value::Null, &stranger).0,
        403,
        "other uid cannot read ciphertext"
    );
    let missing = format!("/v1/{root}/m/{MACHINE}/responses/{fresh}");
    assert_eq!(
        admin.status("GET", &missing, Value::Null, &token).0,
        404,
        "routed peer may observe a missing response"
    );
    assert_eq!(
        admin.status("GET", &missing, Value::Null, &stranger).0,
        403,
        "non-routed uid cannot observe even a missing ID"
    );
    // A registered other peer cannot adopt a carrier simply by guessing its ID.
    let other_peer = uuid_v4().unwrap();
    admin.seed(&format!("m/{MACHINE}/peers/stranger"), json!({"peer":{"stringValue":other_peer},"revoked":{"booleanValue":false},"expiresAt":{"timestampValue":"2035-01-01T00:00:00Z"}}));
    assert_eq!(
        admin
            .status(
                "GET",
                &format!("/v1/{root}/m/{MACHINE}/responses/{id}"),
                Value::Null,
                &stranger
            )
            .0,
        403,
        "routed foreign peer cannot read this carrier"
    );
    assert_eq!(
        admin
            .status(
                "POST",
                &format!("/v1/{root}/m/{MACHINE}:runQuery"),
                json!({"structuredQuery":{"from":[{"collectionId":"responses"}],"limit":1}}),
                &token
            )
            .0,
        403,
        "response lists are denied even to the sender"
    );
    println!(
        "cloud rules_controls=8 refused=7 missing=1 (fixture HTTP, excluded from driver wire totals)"
    );

    // A new serving window consumes the same persisted journal and descriptor/keyring.
    drop(driver);
    let keyring_before =
        std::fs::read(world.owner_layout.directory.join("cloud-keys.json")).unwrap();
    world = world.reopen(wire.clock.now().wall_ms, Arc::new(CurrentIdentity));
    assert_eq!(
        std::fs::read(world.owner_layout.directory.join("cloud-keys.json")).unwrap(),
        keyring_before,
        "compatible keyring reopens without user action or rewrite"
    );
    let driver = world.driver(provider(wire.clone(), "owner", budget.clone()));
    *wire.owner.lock().unwrap() = Arc::downgrade(&driver);
    // Lose every in-memory response context and reopen the private sender Store.
    drop(sender);
    drop(store);
    sender = Sender::new(
        provider(wire.clone(), "peer", budget.clone()),
        Arc::new(Mutex::new(Store::open(&world.sender_lease).unwrap())),
        PEER,
        world.key.clone(),
    )
    .unwrap();
    let answer = ready(sender.prepared(
        raw.as_bytes(),
        &world.descriptor,
        "team",
        wire.clock.now().wall_ms,
        &io,
    ))
    .unwrap();
    assert_eq!(answer.state, tmt_remote::cloud_command::State::Sent);
    assert_eq!(world.core.sends(), 1, "recovery cannot dispatch again");
    wire.report("restart-recovery");
    // Separate operation receipts let each target control prove its own effect count.
    std::fs::write(world.core.root.join("multi"), "").unwrap();
    for (rows, state, reason) in [
        (
            json!([{"id":TARGET,"name":"target"}]),
            tmt_remote::cloud_command::State::Sent,
            None,
        ),
        (
            json!([]),
            tmt_remote::cloud_command::State::Refused,
            Some("REMOTE_AGENT_NOT_FOUND"),
        ),
        (
            json!([{"id":TARGET,"name":"target"},{"id":PEER,"name":"target"}]),
            tmt_remote::cloud_command::State::Refused,
            Some("REMOTE_AGENT_AMBIGUOUS"),
        ),
    ] {
        std::fs::write(
            world.core.root.join("identities"),
            json!({"identities":rows}).to_string(),
        )
        .unwrap();
        let named = json!({"version":1,"remote":"team","createdAtMs":wire.clock.now().wall_ms,"requestId":uuid_v4().unwrap(),"from":{"id":PEER,"name":"peer"},"target":{"name":"target"},"message":"emulator name resolution"}).to_string();
        let answer = ready(sender.prepared(
            named.as_bytes(),
            &world.descriptor,
            "team",
            wire.clock.now().wall_ms,
            &io,
        ))
        .unwrap();
        assert_eq!(answer.state, state);
        assert_eq!(answer.reason.as_deref(), reason);
        assert_eq!(
            serde_json::to_value(&answer).unwrap()["target"],
            json!({"name":"target"})
        );
        assert_eq!(
            world.core.sends(),
            2,
            "only the one unambiguous name dispatches"
        );
    }
    wire.report("name-targets");
    budget.refuse.store(true, Ordering::SeqCst);
    let blocked = raw.replace(&id, &uuid_v4().unwrap());
    let error = ready(sender.prepared(
        blocked.as_bytes(),
        &world.descriptor,
        "team",
        wire.clock.now().wall_ms,
        &io,
    ))
    .err()
    .unwrap();
    assert_eq!(error.code, "REMOTE_BUDGET_EXHAUSTED");
    assert!(
        wire.counts.lock().unwrap().is_empty(),
        "guard refusal before any HTTP"
    );
    budget.refuse.store(false, Ordering::SeqCst);
    world.owner_store.lock().unwrap().revoke(PEER).unwrap();
    admin.seed(&format!("m/{MACHINE}/peers/peer"), peer_row(true));
    let denied = raw.replace(&id, &uuid_v4().unwrap());
    let error = ready(sender.prepared(
        denied.as_bytes(),
        &world.descriptor,
        "team",
        wire.clock.now().wall_ms,
        &io,
    ))
    .err()
    .unwrap();
    assert_eq!(error.code, "REMOTE_CLOUD_ACCESS_DENIED");
    assert_eq!(world.core.sends(), 2);
    wire.report("revoked");
    admin.call(
        "DELETE",
        &format!("/emulator/v1/projects/{PROJECT}/databases/(default)/documents"),
        Value::Null,
    );
}

/// Receiver-only comparison with the offline 24-round workload. Setup carrier
/// writes use ordinary peer Auth and are excluded from the receiver's counters.
#[test]
#[ignore = "explicit Infra emulator slot; no provider fallback"]
fn ordinary_user_idle_single_and_burst_poll_costs() {
    let admin = Admin::new();
    for messages in [0, 1, 4] {
        admin.call(
            "DELETE",
            &format!("/emulator/v1/projects/{PROJECT}/databases/(default)/documents"),
            Value::Null,
        );
        admin.call("PUT", &format!("/emulator/v1/projects/{PROJECT}:securityRules"), json!({"rules":{"files":[{"name":"firestore.rules","content":tmt_remote::cloud_inbox::rules::artifact()}]}}));
        admin.seed(
            &format!("m/{MACHINE}"),
            json!({"uid":{"stringValue":"owner"}}),
        );
        admin.seed(&format!("m/{MACHINE}/peers/peer"), json!({"peer":{"stringValue":PEER},"revoked":{"booleanValue":false},"expiresAt":{"timestampValue":"2035-01-01T00:00:00Z"}}));
        let clock = Arc::new(SystemClock::default());
        let now = clock.now().wall_ms;
        let world = World::new(now, Arc::new(CurrentIdentity));
        std::fs::write(world.core.root.join("multi"), b"per-operation receipts").unwrap();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(180),
            cancelled: &stopped,
        };
        let peer = ready(Auth("peer").token(&io)).unwrap().token;
        let root = format!("projects/{PROJECT}/databases/(default)/documents");
        for index in 0..messages {
            let id = format!("00000000-0000-4000-8000-{index:012x}");
            let payload = json!({"version":1,"operation":"dispatch.create","originator":"anonymous","createdAtMs":now,"input":{"operationId":id,"recipientIds":[TARGET],"expectedName":"target","message":"one offline effect","kind":"request"}}).to_string();
            let exact =
                fixture::signed_request_as(PEER, &id, "dispatch.create", &payload, now, |bytes| {
                    world.key.sign(bytes)
                });
            let carrier = tmt_remote::cloud_inbox::seal_native(
                &exact,
                &world.descriptor,
                now,
                tmt_remote::cloud_hpke::Sender {
                    client_id: PEER,
                    authentication_key: &world.key.public(),
                },
                |bytes| world.key.sign(bytes),
            )
            .unwrap()
            .0;
            use base64::{Engine, engine::general_purpose::STANDARD};
            let name = format!("{root}/m/{MACHINE}/inbox/{id}");
            let write = json!({"update":{"name":name,"fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(carrier)}}},"updateTransforms":[{"fieldPath":"createdAt","setToServerValue":"REQUEST_TIME"}],"currentDocument":{"exists":false}});
            assert_eq!(
                admin
                    .status(
                        "POST",
                        &format!("/v1/{root}:commit"),
                        json!({"writes":[write]}),
                        &peer
                    )
                    .0,
                200,
                "ordinary peer creates the setup carrier"
            );
        }
        let http = ProductionHttp::new(
            &Project::new(PROJECT, "123456", "emulator-key").unwrap(),
            clock.clone(),
        )
        .unwrap();
        assert!(http.emulator_active());
        let wire = Arc::new(Wire {
            http,
            clock: clock.clone(),
            owner: Mutex::new(Weak::new()),
            counts: Mutex::new(vec![]),
        });
        let budget = Arc::new(Budget {
            usage: Mutex::new(NativeUsage::new(now)),
            refuse: AtomicBool::new(false),
        });
        let driver = world.driver(provider(wire.clone(), "owner", budget.clone()));
        let mut returned = 0;
        for _ in 0..24 {
            returned += ready(driver.reconcile(clock.now().wall_ms, &io)).unwrap();
        }
        assert!(
            clock.now().wall_ms - now < 120_000,
            "cost observation must stay within the retained overlap"
        );
        assert_eq!(returned, messages * 24);
        assert_eq!(world.core.sends(), messages);
        let calls = wire.counts.lock().unwrap();
        assert!(calls.iter().all(|(_, status)| *status == 200));
        let queries = calls
            .iter()
            .filter(|(operation, _)| *operation == Operation::Query)
            .count();
        let reads = calls
            .iter()
            .filter(|(operation, _)| *operation == Operation::Read)
            .count();
        let writes = calls
            .iter()
            .filter(|(operation, _)| *operation == Operation::Commit)
            .count();
        let guard = budget.usage.lock().unwrap().reads;
        let (query_ceiling, guard_ceiling) = match messages {
            0 => (24, 72),
            1 => (48, 220),
            4 => (49, 522),
            _ => unreachable!(),
        };
        assert!(queries <= query_ceiling && guard <= guard_ceiling);
        assert_eq!(writes, messages);
        println!(
            "cloud poll_cost {}",
            json!({"distinctDeliveries":messages,"returnedDocuments":returned,"queryCalls":queries,"responseReads":reads,"responseWrites":writes,"activeRoundGuardReads":guard,"modeledGuardReadsPerDay":if messages==0 {4320} else {guard+1438*3}})
        );
        drop(calls);
        let owner = ready(Auth("owner").token(&io)).unwrap().token;
        let stranger = ready(Auth("stranger").token(&io)).unwrap().token;
        let query = |collection, limit| json!({"structuredQuery":{"from":[{"collectionId":collection,"allDescendants":false}],"limit":limit}});
        let path = format!("/v1/{root}/m/{MACHINE}:runQuery");
        assert_eq!(
            admin.status("POST", &path, query("inbox", 4), &owner).0,
            200
        );
        assert_eq!(
            admin.status("POST", &path, query("inbox", 5), &owner).0,
            403
        );
        assert_eq!(admin.status("POST", &path, query("inbox", 4), &peer).0, 403);
        assert_eq!(
            admin.status("POST", &path, query("inbox", 4), &stranger).0,
            403
        );
        assert_eq!(
            admin.status("POST", &path, query("enroll", 2), &owner).0,
            403
        );
        println!(
            "cloud poll_rules_controls=5 allowed=1 refused=4 (ordinary Auth, excluded from receiver totals)"
        );
    }
}
