//! Explicit native emulator proof over the shared closed emulator route.
use super::{AuthError, AuthErrorKind};
use crate::emulator_http::EMULATOR_HTTP_TIMEOUT;

const PROJECT: &str = "demo-tmt-colab";

#[test]
fn emulator_routing_admits_only_the_fixed_demo_endpoints() {
    for (url, expected) in [
        (
            "https://identitytoolkit.googleapis.com/v1/accounts:signUp?key=public",
            "http://127.0.0.1:9099/identitytoolkit.googleapis.com/v1/accounts:signUp?key=public",
        ),
        (
            "https://securetoken.googleapis.com/v1/token?key=public",
            "http://127.0.0.1:9099/securetoken.googleapis.com/v1/token?key=public",
        ),
        (
            "https://firestore.googleapis.com/v1/projects/demo-tmt-colab/databases/(default)/documents:commit",
            "http://127.0.0.1:8085/v1/projects/demo-tmt-colab/databases/(default)/documents:commit",
        ),
    ] {
        assert_eq!(
            EmulatorRoute::new(PROJECT, "127.0.0.1:9099", "127.0.0.1:8085")
                .unwrap()
                .url(url)
                .unwrap(),
            expected
        );
    }
    for url in [
        "https://identitytoolkit.googleapis.com.evil/v1/accounts:signUp",
        "http://identitytoolkit.googleapis.com/v1/accounts:signUp",
        "https://firestore.googleapis.com/v1/projects/real-project/databases/(default)/documents:commit",
        "https://firestore.googleapis.com/v1/projects/demo-other/databases/(default)/documents:commit",
        "https://firestore.googleapis.com/v1/projects/demo-tmt-colab/databases/other/documents:commit",
        "https://example.com/redirect",
    ] {
        assert_eq!(
            EmulatorRoute::new(PROJECT, "127.0.0.1:9099", "127.0.0.1:8085")
                .unwrap()
                .url(url)
                .unwrap_err()
                .kind,
            AuthErrorKind::InputInvalid
        );
    }
    assert!(PROJECT.starts_with("demo-"));
}

use super::{
    AuthOwner, Cancellation, Client, Clock, FileStore, Http, Jitter, Operation, Project, Request,
    Response, SystemClock, Time, UreqHttp, emulator_route::EmulatorRoute,
};
use crate::{
    firestore_budget::Model,
    native_sync::{NativeBudgetFile, NativeClient, NativeRuntime},
    objects::IoBudget,
    sealed_sync::{
        AuthPort, Change, Document, Mutation, Published, Query, SealedSyncPort, SyncContext,
        SyncCursor, SyncError, SyncInterest, SyncQueryPage, SyncQueryRequest, Target,
    },
    state::Layout,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::PathBuf,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

#[derive(Default)]
struct EmulatorClock {
    real: SystemClock,
    advance: AtomicU64,
}
impl Clock for EmulatorClock {
    fn now(&self) -> Time {
        let t = self.real.now();
        let advance = self.advance.load(Ordering::SeqCst);
        Time {
            wall_ms: t.wall_ms + advance,
            monotonic_ms: t.monotonic_ms + advance,
        }
    }
    fn wait_until(&self, deadline: u64, cancel: &dyn Cancellation) -> bool {
        if cancel.cancelled() {
            return false;
        }
        let now = self.now().monotonic_ms;
        self.advance
            .fetch_add(deadline.saturating_sub(now), Ordering::SeqCst);
        !cancel.cancelled()
    }
}
struct NoJitter;
impl Jitter for NoJitter {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
struct EmulatorHttp {
    real: UreqHttp,
    route: EmulatorRoute,
    signups: AtomicUsize,
    refreshes: AtomicUsize,
}
impl Http for EmulatorHttp {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        match request.operation() {
            Operation::SignUp => {
                self.signups.fetch_add(1, Ordering::SeqCst);
            }
            Operation::Refresh => {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
        self.route.send(&self.real, request, deadline, cancel)
    }
}
#[test]
fn emulator_url_copy_preserves_every_non_address_request_field() {
    let r = Request::firestore(
        Operation::Commit,
        "https://firestore.googleapis.com/fixed".into(),
        b"sealed".to_vec(),
        "private-test-bearer".into(),
        37,
    );
    let c = r.with_emulator_url("http://127.0.0.1:8085/test".into());
    assert_eq!(
        (
            r.operation(),
            r.method(),
            r.body(),
            r.content_type(),
            r.bearer(),
            r.response_bytes()
        ),
        (
            c.operation(),
            c.method(),
            c.body(),
            c.content_type(),
            c.bearer(),
            c.response_bytes()
        )
    );
    assert!(!format!("{c:?}").contains("private-test-bearer"));
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(std::task::Waker::noop()))
    {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("native synchronous HTTP unexpectedly needs an executor"),
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn fields(row: &[(&str, Value)]) -> Value {
    Value::Object(
        row.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}
fn text(value: &str) -> Value {
    json!({"stringValue":value})
}
fn integer(value: u64) -> Value {
    json!({"integerValue":value.to_string()})
}
struct PrivateRoot(PathBuf);
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn admin(port: u16, path: &str, data: Option<&Value>) {
    // Emulator-only fixture authority, never the production AuthPort.
    let url = format!(
        "http://127.0.0.1:{port}/v1/projects/{PROJECT}/databases/(default)/documents/x/colab/{path}"
    );
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(EMULATOR_HTTP_TIMEOUT))
        .build()
        .new_agent();
    let reply = match data {
        Some(data) => agent
            .patch(&url)
            .header("Authorization", "Bearer owner")
            .header("Content-Type", "application/json")
            .send(json!({"fields":data}).to_string().as_bytes())
            .unwrap(),
        None => agent
            .delete(&url)
            .header("Authorization", "Bearer owner")
            .call()
            .unwrap(),
    };
    assert_eq!(
        reply.status().as_u16(),
        200,
        "private admission fixture failed"
    );
}

#[test]
#[ignore = "requires explicitly selected local Auth+Firestore emulators and checkout vectors"]
fn native_auth_and_sealed_sync_match_the_colab_rules() {
    let required = |key| {
        std::env::var(key).unwrap_or_else(|_| panic!("selected emulator proof requires {key}"))
    };
    let auth_port = required("REMOTE_SYNC_AUTH_PORT").parse::<u16>().unwrap();
    let firestore_port = required("REMOTE_SYNC_FIRESTORE_PORT")
        .parse::<u16>()
        .unwrap();
    let artifacts = PathBuf::from(required("REMOTE_SYNC_COLAB_ARTIFACTS"));
    let expires = required("REMOTE_SYNC_EXPIRES_AT");
    let vector = fs::read(artifacts.join("deploy-declaration-v1.json")).unwrap();
    for name in [
        "deploy-declaration-v1.json",
        "deploy-declaration-v1.firestore.rules",
        "deploy-declaration-v1.firestore.indexes.json",
    ] {
        println!(
            "emulator artifact {name}: {}",
            digest(&fs::read(artifacts.join(name)).unwrap())
        );
    }
    let vector: Value = serde_json::from_slice(&vector).unwrap();
    let declaration = vector["declaration"].as_str().unwrap().as_bytes();
    assert_eq!(
        digest(declaration),
        vector["declarationDigest"].as_str().unwrap()
    );
    let config = json!({"apiKey":"emulator-public-key", "appId":"1:12345:web:emulator", "authDomain":format!("{PROJECT}.firebaseapp.com"), "projectId":PROJECT});
    let clock = Arc::new(EmulatorClock::default());
    let http = Arc::new(EmulatorHttp {
        real: UreqHttp::loopback(clock.clone()),
        route: EmulatorRoute::new(
            PROJECT,
            &format!("127.0.0.1:{auth_port}"),
            &format!("127.0.0.1:{firestore_port}"),
        )
        .unwrap(),
        signups: AtomicUsize::new(0),
        refreshes: AtomicUsize::new(0),
    });
    let project = Project::from_public_config(&config).unwrap();
    let root = PrivateRoot(std::env::temp_dir().join(format!(
        "remote-native-emulator-{}",
        crate::store::uuid_v4().unwrap()
    )));
    fs::create_dir(&root.0).unwrap();
    let layout = Layout::open(&root.0).unwrap();
    let store = Arc::new(FileStore::new(&layout, PROJECT).unwrap());
    let cancelled = AtomicBool::new(false);
    let mut client = Client::load(
        project.clone(),
        store.as_ref(),
        http.as_ref(),
        clock.as_ref(),
        &NoJitter,
        &cancelled,
    )
    .unwrap();
    client.sign_in().unwrap();
    let uid = client.status().uid.unwrap();
    drop(client);
    let auth = Arc::new(
        AuthOwner::load(
            project,
            store,
            http.clone(),
            clock.clone(),
            Arc::new(NoJitter),
        )
        .unwrap(),
    );
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &cancelled,
    };
    assert_eq!(ready(auth.token(&io)).unwrap().identity.uid, uid);
    assert_eq!(http.signups.load(Ordering::SeqCst), 1);
    assert_eq!(
        http.refreshes.load(Ordering::SeqCst),
        1,
        "cold owner must actually refresh the stored credential"
    );
    let space = "a".repeat(32);
    let page = "10000000-0000-4000-8000-000000000001";
    let stream = "20000000-0000-4000-8000-000000000001";
    let page_key = format!("{space}_{page}");
    let member_key = format!("members/{page_key}_{uid}");
    admin(
        firestore_port,
        &format!("spaces/{space}"),
        Some(&fields(&[("space", text(&space)), ("owner", text(&uid))])),
    );
    admin(
        firestore_port,
        &format!("pages/{page_key}"),
        Some(&fields(&[
            ("space", text(&space)),
            ("page", text(page)),
            ("epoch", integer(1)),
            ("state", text("open")),
            ("expiresAt", json!({"timestampValue":expires})),
        ])),
    );
    let member = |role| {
        fields(&[
            ("space", text(&space)),
            ("page", text(page)),
            ("uid", text(&uid)),
            ("role", text(role)),
            ("link", json!({"nullValue":null})),
            ("token", json!({"nullValue":null})),
        ])
    };
    admin(firestore_port, &member_key, Some(&member("editor")));
    let context = SyncContext::bind(
        config,
        declaration,
        vector["declarationDigest"].as_str().unwrap(),
        &space,
        auth.clone(),
    )
    .unwrap();
    let budget = Arc::new(Mutex::new(
        NativeBudgetFile::open(
            &layout,
            PROJECT,
            clock.now().wall_ms,
            clock.now().monotonic_ms,
        )
        .unwrap(),
    ));
    let runtime = Arc::new(NativeRuntime::new(
        http.clone(),
        clock.clone(),
        Arc::new(NoJitter),
        budget,
    ));
    let sync = NativeClient::new(context, runtime.clone(), Model::new(1, 1).unwrap());
    let scope: BTreeMap<_, _> = [
        ("space".into(), text(&space)),
        ("page".into(), text(page)),
        ("epoch".into(), integer(1)),
    ]
    .into();
    let mut interest = ready(sync.subscribe(
        &SyncInterest {
            resource: "log".into(),
            scope: scope.clone(),
        },
        &io,
    ))
    .unwrap();
    assert!(matches!(
        ready(interest.next(&io)).unwrap(),
        Change::Changed
    ));
    let document = |seq| {
        let mut value = fields(&[
            ("space", text(&space)),
            ("page", text(page)),
            ("epoch", integer(1)),
            ("stream", text(stream)),
            ("seq", integer(seq)),
            ("uid", text(&uid)),
            ("hash", text(&"h".repeat(43))),
            ("envelope", text("ZW52ZWxvcGU")),
            ("expiresAt", json!({"timestampValue":expires})),
        ]);
        value.sort_all_objects();
        let bytes = serde_json::to_vec(&value).unwrap();
        Document {
            target: Target {
                resource: "log".into(),
                id: format!("{page_key}_1_{stream}_{seq}"),
            },
            digest: digest(&bytes),
            bytes,
        }
    };
    let first = document(1);
    assert!(matches!(
        ready(sync.publish(&Mutation::Create(first.clone()), &io)).unwrap(),
        Published::Confirmed
    ));
    assert_eq!(
        ready(sync.read(&first.target, &io)).unwrap(),
        Some(first.clone())
    );
    // The first baseline schedules an idle probe later than this call's budget.
    // A deadline retains the interest; advance only the fixture clock to the
    // next due probe, rather than lengthening deadlines or speeding up polling.
    assert!(matches!(
        ready(interest.next(&io)),
        Err(SyncError::Deadline)
    ));
    clock
        .advance
        .fetch_add(crate::limits::NATIVE_POLL_IDLE_MS, Ordering::SeqCst);
    assert!(matches!(
        ready(interest.next(&io)).unwrap(),
        Change::Changed
    ));
    let second = document(2);
    assert!(matches!(
        ready(sync.publish(&Mutation::Create(second.clone()), &io)).unwrap(),
        Published::Confirmed
    ));
    let mut stream_scope = scope.clone();
    stream_scope.insert("stream".into(), text(stream));
    let query = SyncQueryRequest::Stream(Query {
        resource: "log".into(),
        scope: stream_scope,
        stream: stream.into(),
        cursor: Some(SyncCursor {
            stream: stream.into(),
            seq: 1,
        }),
    });
    let SyncQueryPage::Stream(page) = ready(sync.query(&query, &io)).unwrap() else {
        panic!("stream shape changed")
    };
    assert_eq!(page.documents, vec![second]);
    assert_eq!(page.cursor.unwrap().seq, 2);
    admin(firestore_port, &member_key, Some(&member("viewer")));
    let third = document(3);
    assert!(matches!(
        ready(sync.publish(&Mutation::Create(third.clone()), &io)),
        Err(SyncError::Denied)
    ));
    // Colab's get rule reads stored scope metadata: an absent row is denied,
    // not a user-visible 404. Do not treat that refusal as proof of absence.
    assert!(matches!(
        ready(sync.read(&third.target, &io)),
        Err(SyncError::Denied)
    ));
    admin(firestore_port, &member_key, Some(&member("editor")));
    assert!(
        matches!(
            ready(sync.publish(&Mutation::Create(third), &io)).unwrap(),
            Published::Confirmed
        ),
        "exclusive create on the same valid target must be newly confirmed, not already present; the denied attempt stored nothing"
    );
    admin(firestore_port, &member_key, None);
    assert!(matches!(
        ready(sync.read(&first.target, &io)),
        Err(SyncError::Denied)
    ));
    assert!(matches!(
        ready(sync.query(&query, &io)),
        Err(SyncError::Denied)
    ));
    assert!(matches!(ready(interest.next(&io)), Err(SyncError::Denied)));
    assert_eq!(auth.identity().uid, uid);
    assert_eq!(
        http.signups.load(Ordering::SeqCst),
        1,
        "membership loss must not create a replacement identity"
    );
    interest.close();
    cancelled.store(true, Ordering::SeqCst);
    assert!(ready(sync.read(&first.target, &io)).is_err());
    sync.close();
    runtime.finish().unwrap();
    auth.stop();
}
