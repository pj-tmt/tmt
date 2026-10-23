//! Explicit emulator qualification: ordinary-user ProductionHttp drives the real driver.
//! Run only in an Infra slot with both test-emulator hosts set to loopback addresses.
#![cfg(feature = "test-emulator")]
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
                .timeout_global(Some(Duration::from_secs(20)))
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
    assert_eq!(world.core.sends(), 1);
    wire.report("revoked");
    admin.call(
        "DELETE",
        &format!("/emulator/v1/projects/{PROJECT}/databases/(default)/documents"),
        Value::Null,
    );
}
