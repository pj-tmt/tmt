//! Ordinary-user Auth behavior under injected time/HTTP and real private files.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tmt_remote::{
    firebase_auth::{
        AuthError, AuthState, Cancellation, Client, Clock, Credential, CredentialStore, FileStore,
        Http, Jitter, Operation, Project, Request, Response, StoreRead, Time,
    },
    state::Layout,
};

// These adapters exercise the thread-safe serving owner with the same injected
// lifecycle fixtures, without moving scenario fakes into production modules.
struct SharedStore(std::sync::Mutex<MemoryStore>);
impl CredentialStore for SharedStore {
    fn load(&self) -> Result<StoreRead, AuthError> {
        self.0.lock().unwrap().load()
    }
    fn replace(&self, expected: Option<&Credential>, next: &Credential) -> Result<(), AuthError> {
        self.0.lock().unwrap().replace(expected, next)
    }
    fn remove(&self, expected: &Credential) -> Result<(), AuthError> {
        self.0.lock().unwrap().remove(expected)
    }
}
struct SharedHttp(std::sync::Mutex<FakeHttp<'static>>);
impl Http for SharedHttp {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        self.0.lock().unwrap().send(request, deadline, cancel)
    }
}
struct SharedClock(std::sync::Mutex<TestClock>);
impl Clock for SharedClock {
    fn now(&self) -> Time {
        self.0.lock().unwrap().now()
    }
    fn wait_until(&self, deadline: u64, cancel: &dyn Cancellation) -> bool {
        self.0.lock().unwrap().wait_until(deadline, cancel)
    }
}
fn immediate<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let waker = std::task::Waker::noop();
    match future
        .as_mut()
        .poll(&mut std::task::Context::from_waker(waker))
    {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => panic!("fixture must complete synchronously"),
    }
}

#[test]
fn serving_auth_keeps_identity_refreshes_early_and_honors_the_caller_budget() {
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use tmt_remote::{firebase_auth::AuthOwner, objects::IoBudget, sealed_sync::AuthPort};
    let store = Arc::new(SharedStore(Mutex::new(MemoryStore::default())));
    let clock = Arc::new(SharedClock(Mutex::new(TestClock::new())));
    let http = Arc::new(SharedHttp(Mutex::new(FakeHttp::new(vec![
        Ok(signup("native-uid")),
        Ok(refreshed("native-uid", "first")),
        Ok(refreshed("native-uid", "second")),
    ]))));
    let jitter = Arc::new(ZeroJitter);
    let cancel = AtomicBool::new(false);
    client(store.as_ref(), http.as_ref(), clock.as_ref(), &cancel)
        .sign_in()
        .unwrap();
    let owner = AuthOwner::load(
        project(),
        store.clone(),
        http.clone(),
        clock.clone(),
        jitter,
    )
    .unwrap();
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &cancel,
    };
    let first = immediate(owner.token(&io)).unwrap();
    assert_eq!(first.identity, owner.identity());
    assert_eq!(first.expires_at_ms, NOW + 3_600_000);
    immediate(owner.token(&io)).unwrap();
    assert_eq!(http.0.lock().unwrap().requests.borrow().len(), 2);
    clock.0.lock().unwrap().advance(3_300_000);
    immediate(owner.token(&io)).unwrap();
    assert_eq!(http.0.lock().unwrap().requests.borrow().len(), 3);
    cancel.store(true, Ordering::SeqCst);
    assert!(immediate(owner.token(&io)).is_err());
    cancel.store(false, Ordering::SeqCst);
    store
        .0
        .lock()
        .unwrap()
        .record
        .borrow_mut()
        .as_mut()
        .unwrap()
        .mark_revoking();
    assert!(immediate(owner.token(&io)).is_err());
    assert_eq!(http.0.lock().unwrap().requests.borrow().len(), 3);
    owner.stop();
    assert!(immediate(owner.token(&io)).is_err());
}

const NOW: u64 = 1_800_000_000_000;
const DAY: u64 = 86_400_000;
const REFRESH: &str = "refresh-canary-DO-NOT-PRINT";

trait JsonResponse {
    fn json(status: u16, value: Value) -> Self;
}
impl JsonResponse for Response {
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
            retry_after_ms: None,
        }
    }
}

fn project() -> Project {
    Project::new("demo-tmt-remote", "123456", "public-test-key").unwrap()
}
fn token(uid: &str) -> String {
    format!(
        "e30.{}.test",
        URL_SAFE_NO_PAD.encode(
            json!({"aud":"demo-tmt-remote","sub":uid,"firebase":{"sign_in_provider":"anonymous"}})
                .to_string()
        )
    )
}
fn signup(uid: &str) -> Response {
    Response::json(
        200,
        json!({"localId":uid,"idToken":token(uid),"refreshToken":REFRESH,"expiresIn":"3600"}),
    )
}
fn refreshed(uid: &str, refresh: &str) -> Response {
    Response::json(
        200,
        json!({"user_id":uid,"project_id":"123456","id_token":token(uid),"refresh_token":refresh,"expires_in":"3600"}),
    )
}

struct TestClock {
    wall: Cell<u64>,
    mono: Cell<u64>,
    waits: RefCell<Vec<u64>>,
}
impl TestClock {
    fn new() -> Self {
        Self {
            wall: Cell::new(NOW),
            mono: Cell::new(0),
            waits: RefCell::new(vec![]),
        }
    }
    fn advance(&self, ms: u64) {
        self.wall.set(self.wall.get() + ms);
        self.mono.set(self.mono.get() + ms);
    }
}
impl Clock for TestClock {
    fn now(&self) -> Time {
        Time {
            wall_ms: self.wall.get(),
            monotonic_ms: self.mono.get(),
        }
    }
    fn wait_until(&self, deadline: u64, cancel: &dyn Cancellation) -> bool {
        self.waits.borrow_mut().push(deadline);
        if cancel.cancelled() {
            return false;
        }
        self.advance(deadline.saturating_sub(self.mono.get()));
        true
    }
}
struct ZeroJitter;
impl Jitter for ZeroJitter {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}

#[derive(Default)]
struct MemoryStore {
    record: RefCell<Option<Credential>>,
    writes: Cell<usize>,
}
impl CredentialStore for MemoryStore {
    fn load(&self) -> Result<StoreRead, AuthError> {
        Ok(StoreRead {
            credential: self.record.borrow().clone(),
            permissions_repaired: false,
        })
    }
    fn replace(&self, expected: Option<&Credential>, next: &Credential) -> Result<(), AuthError> {
        if self.record.borrow().as_ref() != expected {
            return Err(AuthError::state_changed());
        }
        self.record.replace(Some(next.clone()));
        self.writes.set(self.writes.get() + 1);
        Ok(())
    }
    fn remove(&self, expected: &Credential) -> Result<(), AuthError> {
        if self.record.borrow().as_ref() != Some(expected) {
            return Err(AuthError::state_changed());
        }
        self.record.replace(None);
        Ok(())
    }
}
struct FakeHttp<'a> {
    replies: RefCell<VecDeque<Result<Response, AuthError>>>,
    requests: RefCell<Vec<(Operation, Value, u64)>>,
    cancel_after: Option<&'a AtomicBool>,
}
impl<'a> FakeHttp<'a> {
    fn new(replies: Vec<Result<Response, AuthError>>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            requests: RefCell::new(vec![]),
            cancel_after: None,
        }
    }
}
impl Http for FakeHttp<'_> {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        _: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        assert!(request.url().starts_with("https://"));
        assert!(!format!("{request:?}").contains(REFRESH));
        let body = if request.operation() == Operation::Refresh {
            json!({"form":String::from_utf8(request.body().to_vec()).unwrap()})
        } else {
            serde_json::from_slice(request.body()).unwrap()
        };
        self.requests
            .borrow_mut()
            .push((request.operation(), body, deadline));
        if let Some(cancel) = self.cancel_after {
            cancel.store(true, Ordering::SeqCst);
        }
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected HTTP effect")
    }
}
fn client<'a>(
    store: &'a dyn CredentialStore,
    http: &'a dyn Http,
    clock: &'a dyn Clock,
    cancel: &'a dyn Cancellation,
) -> Client<'a> {
    Client::load(project(), store, http, clock, &ZeroJitter, cancel).unwrap()
}

#[test]
fn explicit_signup_persists_only_refresh_and_restart_preserves_uid() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Ok(refreshed("u1", "rotated-secret")),
    ]);
    let mut first = client(&store, &http, &clock, &cancel);
    assert_eq!(first.status().state, AuthState::Unenrolled);
    assert!(first.token().is_err());
    assert!(http.requests.borrow().is_empty());
    first.sign_in().unwrap();
    assert_eq!(first.token().unwrap().uid(), "u1");
    assert_eq!(first.status().expires_at_ms, Some(NOW + 365 * DAY));
    first.stop();
    let mut restart = client(&store, &http, &clock, &cancel);
    assert_eq!(restart.token().unwrap().uid(), "u1");
    assert_eq!(
        store.load().unwrap().credential.unwrap().expires_at_ms(),
        NOW + 365 * DAY
    );
    assert_eq!(
        http.requests
            .borrow()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![Operation::SignUp, Operation::Refresh]
    );
    assert!(
        http.requests.borrow()[1].1["form"]
            .as_str()
            .unwrap()
            .contains("grant_type=refresh_token")
    );
}

#[test]
fn scheduler_uses_monotonic_lead_and_coalesces_refreshes() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1")), Ok(refreshed("u1", REFRESH))]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    clock.advance(3_299_999);
    assert!(!c.refresh_if_due().unwrap());
    clock.wall.set(NOW - 1000);
    clock.mono.set(3_300_000);
    assert!(c.refresh_if_due().unwrap());
    assert!(!c.refresh_if_due().unwrap());
    assert_eq!(http.requests.borrow().len(), 2);
}

#[test]
fn transient_refresh_is_bounded_and_cannot_use_an_expired_token() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Err(AuthError::unavailable()),
        Err(AuthError::unavailable()),
        Err(AuthError::unavailable()),
        Err(AuthError::unavailable()),
    ]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    clock.advance(3_600_000);
    assert!(c.token().is_err());
    assert_eq!(http.requests.borrow().len(), 5);
    assert!(c.token().is_err());
    assert_eq!(
        http.requests.borrow().len(),
        5,
        "backoff must not start another series"
    );
    assert_eq!(clock.waits.borrow().len(), 3);
    assert_eq!(
        c.status().state,
        AuthState::Active,
        "offline does not revoke the user"
    );
}

#[test]
fn definitive_refresh_refusal_requires_explicit_new_signin() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Ok(Response::json(
            400,
            json!({"error":{"message":"USER_DISABLED"}}),
        )),
        Ok(signup("u2")),
    ]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    clock.advance(3_600_000);
    assert!(c.token().is_err());
    assert_eq!(c.status().state, AuthState::ReSignInNeeded);
    assert!(c.token().is_err());
    assert_eq!(http.requests.borrow().len(), 2);
    c.sign_in().unwrap();
    assert_eq!(c.token().unwrap().uid(), "u2");
}

#[test]
fn absolute_expiry_purges_the_secret_without_automatic_signup() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    clock.advance(365 * DAY);
    assert!(c.token().is_err());
    assert_eq!(c.status().state, AuthState::ReSignInNeeded);
    assert!(store.load().unwrap().credential.is_none());
    assert_eq!(http.requests.borrow().len(), 1);
}

#[test]
fn ambiguous_signup_is_not_automatically_repeated_but_explicit_retry_is_allowed() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Err(AuthError::unavailable()), Ok(signup("u2"))]);
    let mut c = client(&store, &http, &clock, &cancel);
    assert!(c.sign_in().unwrap_err().may_have_effect());
    assert!(c.token().is_err());
    assert_eq!(http.requests.borrow().len(), 1);
    c.sign_in().unwrap();
    assert_eq!(c.token().unwrap().uid(), "u2");
}

#[test]
fn cancellation_after_auth_response_never_publishes_a_token_or_credential() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let mut http = FakeHttp::new(vec![Ok(signup("u1"))]);
    http.cancel_after = Some(&cancel);
    let mut c = client(&store, &http, &clock, &cancel);
    assert!(c.sign_in().unwrap_err().may_have_effect());
    assert_eq!(store.writes.get(), 0);
    assert!(c.token().is_err());
}

#[test]
fn revoke_stays_durable_and_retry_deletes_only_the_original_uid() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Err(AuthError::unavailable()),
        Ok(refreshed("u1", REFRESH)),
        Ok(Response::json(
            400,
            json!({"error":{"message":"USER_NOT_FOUND"}}),
        )),
    ]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    assert!(c.revoke().unwrap_err().may_have_effect());
    assert_eq!(c.status().state, AuthState::Revoking);
    assert!(c.token().is_err());
    assert!(c.sign_in().is_err());
    let mut restart = client(&store, &http, &clock, &cancel);
    assert_eq!(restart.status().state, AuthState::Revoking);
    restart.revoke().unwrap();
    assert!(store.load().unwrap().credential.is_none());
    assert_eq!(
        http.requests
            .borrow()
            .iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![
            Operation::SignUp,
            Operation::Delete,
            Operation::Refresh,
            Operation::Delete
        ]
    );
    assert_eq!(http.requests.borrow()[3].1["idToken"], token("u1"));
}

#[test]
fn wrong_uid_project_and_malformed_provider_reply_never_rotate() {
    for response in [
        refreshed("other", REFRESH),
        Response::json(
            200,
            json!({"user_id":"u1","project_id":"wrong","id_token":token("u1"),"refresh_token":REFRESH,"expires_in":"3600"}),
        ),
        Response::json(200, json!({"error":REFRESH})),
    ] {
        let store = MemoryStore::default();
        let clock = TestClock::new();
        let cancel = AtomicBool::new(false);
        let http = FakeHttp::new(vec![Ok(signup("u1")), Ok(response)]);
        let mut c = client(&store, &http, &clock, &cancel);
        c.sign_in().unwrap();
        let before = store.load().unwrap().credential;
        clock.advance(3_600_000);
        let error = c.token().unwrap_err();
        assert!(!format!("{error:?} {error}").contains(REFRESH));
        assert_eq!(store.load().unwrap().credential, before);
    }
}

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-firebase-auth-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn file(&self) -> PathBuf {
        self.0.join("remote/firebase-auth/demo-tmt-remote.json")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn file_store_rotates_atomically_repairs_mode_once_and_never_stores_id_token() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let store = FileStore::new(&layout, "demo-tmt-remote").unwrap();
    assert!(store.load().unwrap().credential.is_none());
    assert!(!root.file().parent().unwrap().exists());
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Ok(refreshed("u1", "rotated-secret")),
    ]);
    {
        let mut c = client(&store, &http, &clock, &cancel);
        c.sign_in().unwrap();
    }
    let bytes = fs::read_to_string(root.file()).unwrap();
    assert!(bytes.contains(REFRESH));
    assert!(!bytes.contains(&token("u1")));
    assert_eq!(
        fs::metadata(root.file()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(root.file(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load().unwrap().permissions_repaired);
    assert!(!store.load().unwrap().permissions_repaired);
    let mut restart = client(&store, &http, &clock, &cancel);
    assert_eq!(restart.token().unwrap().uid(), "u1");
    let after: Value = serde_json::from_slice(&fs::read(root.file()).unwrap()).unwrap();
    assert_eq!(after["refreshToken"], "rotated-secret");
    assert_eq!(after["createdAtMs"], NOW);
    assert_eq!(after["expiresAtMs"], NOW + 365 * DAY);
    assert!(!format!("{:?}", store.load().unwrap().credential).contains("rotated-secret"));
    let names = fs::read_dir(root.file().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 2, "only the credential and its lock remain");
}

#[test]
fn abandoned_owned_staging_secrets_are_removed_on_load_and_purge() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let store = FileStore::new(&layout, "demo-tmt-remote").unwrap();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    client(&store, &http, &clock, &cancel).sign_in().unwrap();
    let stage = root
        .file()
        .parent()
        .unwrap()
        .join(".demo-tmt-remote-00000000-0000-4000-8000-000000000123");
    let other_project = root
        .file()
        .parent()
        .unwrap()
        .join(".another-project-00000000-0000-4000-8000-000000000123");
    fs::copy(root.file(), &stage).unwrap();
    fs::copy(root.file(), &other_project).unwrap();
    let credential = store.load().unwrap().credential.unwrap();
    assert!(
        !stage.exists(),
        "crashed staging must not retain a rotated/revoked refresh secret"
    );
    assert!(other_project.exists());
    fs::copy(root.file(), &stage).unwrap();
    store.remove(&credential).unwrap();
    assert!(!stage.exists());
    assert!(!root.file().exists());
    assert!(other_project.exists());
    assert_eq!(http.requests.borrow().len(), 1);
}

#[test]
fn loading_an_ended_sign_in_for_status_purges_its_secret_without_http() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    client(&store, &http, &clock, &cancel).sign_in().unwrap();
    clock.advance(365 * DAY);
    let loaded = client(&store, &http, &clock, &cancel);
    assert_eq!(loaded.status().state, AuthState::ReSignInNeeded);
    assert!(store.load().unwrap().credential.is_none());
    assert_eq!(http.requests.borrow().len(), 1);
}

#[test]
fn file_store_rejects_symlink_malformed_and_project_disagreement_without_network() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let store = FileStore::new(&layout, "demo-tmt-remote").unwrap();
    fs::create_dir(root.file().parent().unwrap()).unwrap();
    fs::set_permissions(
        root.file().parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let outside = root.0.join("outside");
    fs::write(&outside, REFRESH).unwrap();
    symlink(&outside, root.file()).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), REFRESH);
    fs::remove_file(root.file()).unwrap();
    for bytes in [
        b"{broken".as_slice(),
        br#"{"version":99}"#,
        br#"{"version":1,"projectId":"other"}"#,
    ] {
        fs::write(root.file(), bytes).unwrap();
        fs::set_permissions(root.file(), fs::Permissions::from_mode(0o600)).unwrap();
        let http = FakeHttp::new(vec![]);
        let clock = TestClock::new();
        let cancel = AtomicBool::new(false);
        assert!(Client::load(project(), &store, &http, &clock, &ZeroJitter, &cancel).is_err());
        assert!(http.requests.borrow().is_empty());
        assert_eq!(fs::read(root.file()).unwrap(), bytes);
    }
}

#[test]
fn stale_store_snapshot_cannot_overwrite_revoke_or_another_refresh() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let store = FileStore::new(&layout, "demo-tmt-remote").unwrap();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    client(&store, &http, &clock, &cancel).sign_in().unwrap();
    let old = store.load().unwrap().credential.unwrap();
    let mut revoking = old.clone();
    revoking.mark_revoking();
    store.replace(Some(&old), &revoking).unwrap();
    assert!(store.replace(Some(&old), &old).is_err());
    assert_eq!(store.load().unwrap().credential, Some(revoking));
}

#[test]
fn another_invocation_revoke_fences_a_cached_token_before_use() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    let old = store.load().unwrap().credential.unwrap();
    let mut revoking = old.clone();
    revoking.mark_revoking();
    store.replace(Some(&old), &revoking).unwrap();
    assert!(c.token().is_err());
    assert_eq!(c.status().state, AuthState::Revoking);
    assert!(c.token().is_err());
    assert_eq!(http.requests.borrow().len(), 1);
}

#[test]
fn already_deleted_account_is_purged_when_revoke_refresh_proves_absence() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![
        Ok(signup("u1")),
        Ok(Response::json(
            400,
            json!({"error":{"message":"USER_NOT_FOUND"}}),
        )),
    ]);
    client(&store, &http, &clock, &cancel).sign_in().unwrap();
    let mut restart = client(&store, &http, &clock, &cancel);
    restart.revoke().unwrap();
    assert!(store.load().unwrap().credential.is_none());
    assert_eq!(http.requests.borrow().len(), 2);
}

#[test]
fn provider_reply_at_deadline_does_not_publish_even_a_valid_token() {
    struct LateHttp<'a>(&'a TestClock);
    impl Http for LateHttp<'_> {
        fn send(
            &self,
            _: &Request,
            deadline: u64,
            _: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            self.0.advance(deadline - self.0.mono.get());
            Ok(signup("u1"))
        }
    }
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = LateHttp(&clock);
    let mut c = client(&store, &http, &clock, &cancel);
    assert!(c.sign_in().unwrap_err().may_have_effect());
    assert_eq!(store.writes.get(), 0);
}

#[test]
fn stop_retains_refresh_but_rejects_all_later_work_without_http() {
    let store = MemoryStore::default();
    let clock = TestClock::new();
    let cancel = AtomicBool::new(false);
    let http = FakeHttp::new(vec![Ok(signup("u1"))]);
    let mut c = client(&store, &http, &clock, &cancel);
    c.sign_in().unwrap();
    let before = store.load().unwrap().credential;
    c.stop();
    assert!(c.token().is_err());
    assert!(c.sign_in().is_err());
    assert!(c.revoke().is_err());
    assert_eq!(store.load().unwrap().credential, before);
    assert_eq!(http.requests.borrow().len(), 1);
    assert_eq!(c.next_refresh_monotonic_ms(), None);
}

#[test]
fn unsafe_credential_directory_is_refused_without_repair_or_signup() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let store = FileStore::new(&layout, "demo-tmt-remote").unwrap();
    fs::create_dir(root.file().parent().unwrap()).unwrap();
    fs::set_permissions(
        root.file().parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(store.load().is_err());
    assert_eq!(
        fs::metadata(root.file().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!root.file().exists());
}
