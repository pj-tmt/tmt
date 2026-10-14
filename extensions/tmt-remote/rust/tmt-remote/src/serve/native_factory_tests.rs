//! Project-owner persistence is independent of provider activity and consumer count.
use super::{native_factory::CheckpointOwner, test_isolation::isolated_case};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tmt_remote::{
    firebase_auth::{AuthError, Cancellation, Clock, Http, Jitter, Request, Response, Time},
    firestore_budget::{Decision, Model, NativeUsage},
    native_sync::{NativeBudgetFile, NativeBudgetPort, NativeRuntime},
    sealed_sync::SyncError,
    state::Layout,
};

const NOW: u64 = 1_800_000_000_000;
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = PathBuf::from(format!(
            "/tmp/t3019-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct IdlePorts;
impl Clock for IdlePorts {
    fn now(&self) -> Time {
        Time {
            wall_ms: NOW,
            monotonic_ms: 60_000,
        }
    }
    fn wait_until(&self, _: u64, _: &dyn Cancellation) -> bool {
        panic!("the owner tick must not borrow a provider wait")
    }
}
impl Jitter for IdlePorts {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
impl Http for IdlePorts {
    fn send(&self, _: &Request, _: u64, _: &dyn Cancellation) -> Result<Response, AuthError> {
        panic!("idle checkpoint must not call HTTP")
    }
}
struct Budget {
    file: Mutex<NativeBudgetFile>,
    checkpoints: mpsc::Sender<Result<(), SyncError>>,
    fail: AtomicBool,
}
impl NativeBudgetPort for Budget {
    fn usage(&self, project: &str) -> Result<NativeUsage, SyncError> {
        assert_eq!(project, "demo-tmt-remote");
        Ok(self.file.lock().unwrap().usage())
    }
    fn charge(
        &self,
        project: &str,
        model: &Model,
        reads: u64,
        time: Time,
    ) -> Result<Decision, SyncError> {
        assert_eq!(project, "demo-tmt-remote");
        self.file
            .lock()
            .unwrap()
            .authorize(model, reads, time.wall_ms, time.monotonic_ms)
            .map_err(|_| SyncError::Unavailable)
    }
    fn flush(&self) -> Result<(), SyncError> {
        NativeBudgetPort::flush(&self.file)
    }
    fn checkpoint(&self, time: Time) -> Result<(), SyncError> {
        let result = if self.fail.load(Ordering::Acquire) {
            Err(SyncError::Unavailable)
        } else {
            self.file
                .lock()
                .unwrap()
                .checkpoint(time.monotonic_ms)
                .map_err(|_| SyncError::Unavailable)
        };
        let _ = self.checkpoints.send(result);
        result
    }
}
fn runtime(
    layout: &Layout,
    fail: bool,
) -> (
    Arc<NativeRuntime>,
    Arc<Budget>,
    mpsc::Receiver<Result<(), SyncError>>,
) {
    let (tx, rx) = mpsc::channel();
    let budget = Arc::new(Budget {
        file: Mutex::new(NativeBudgetFile::open(layout, "demo-tmt-remote", NOW, 0).unwrap()),
        checkpoints: tx,
        fail: AtomicBool::new(fail),
    });
    let ports = Arc::new(IdlePorts);
    let runtime = Arc::new(NativeRuntime::new(
        ports.clone(),
        ports.clone(),
        ports,
        budget.clone(),
    ));
    (runtime, budget, rx)
}

#[test]
fn idle_checkpoint_persists_without_provider_work_and_stop_flushes_the_tail() {
    if isolated_case(
        "serve::native_factory_tests::idle_checkpoint_persists_without_provider_work_and_stop_flushes_the_tail",
    ) {
        return;
    }
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let (runtime, budget, observed) = runtime(&layout, false);
    let model = Model::new(1, 1).unwrap();
    budget
        .charge(
            "demo-tmt-remote",
            &model,
            3,
            Time {
                wall_ms: NOW,
                monotonic_ms: 0,
            },
        )
        .unwrap();
    let saved = layout.directory.join("native-budget/demo-tmt-remote.json");
    assert!(!saved.exists());
    let mut owner =
        CheckpointOwner::start(runtime.clone(), Duration::from_millis(5), Arc::new(|| {})).unwrap();
    observed
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&saved).unwrap()).unwrap()["reads"],
        3
    );
    owner.cancel();
    owner.join().unwrap();
    budget
        .charge(
            "demo-tmt-remote",
            &model,
            3,
            Time {
                wall_ms: NOW,
                monotonic_ms: 60_001,
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&saved).unwrap()).unwrap()["reads"],
        3,
        "joining the cancelled tick must leave the final persistence to finish"
    );
    owner.finish().unwrap();
    drop(runtime);
    drop(budget);
    let restored = NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 60_001).unwrap();
    assert_eq!(
        restored.usage().reads,
        6,
        "normal stop must persist the unsaved tail and release its writer"
    );
}

#[test]
fn checkpoint_failure_fences_native_work_and_ends_the_tick() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let (runtime, _, observed) = runtime(&layout, true);
    let (fenced, fence_observed) = mpsc::channel();
    let owner = CheckpointOwner::start(
        runtime,
        Duration::from_millis(5),
        Arc::new(move || {
            fenced.send(()).unwrap();
        }),
    )
    .unwrap();
    assert_eq!(
        observed.recv_timeout(Duration::from_secs(2)).unwrap(),
        Err(SyncError::Unavailable)
    );
    fence_observed.recv_timeout(Duration::from_secs(2)).unwrap();
    owner.finish().unwrap_err();
    assert!(
        observed.try_recv().is_err(),
        "a failed checkpoint must not run another tick"
    );
}

#[test]
fn cancellation_wakes_the_minute_wait_and_joins_before_finish_returns() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let (runtime, _, observed) = runtime(&layout, false);
    let owner = CheckpointOwner::start(runtime, Duration::from_secs(60), Arc::new(|| {})).unwrap();
    let started = Instant::now();
    owner.cancel();
    owner.finish().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "normal stop slept through its minute tick"
    );
    assert!(
        observed.try_recv().is_err(),
        "cancelled owner must not checkpoint after joining"
    );
}

#[test]
fn production_default_is_the_named_four_member_four_writer_assumption() {
    let (members, writers) = tmt_remote::firestore_budget::NATIVE_DEFAULT_MODEL;
    assert_eq!((members, writers), (4, 4));
    let model = Model::new(members, writers).unwrap();
    assert_eq!(model.reads_per_append(), 13);
    assert_eq!(model.daily_append_limit(), 3_846);
    assert_eq!(model.share(), 961);
    assert_eq!(model.warn_at(), 672);
    assert_eq!(model.refuse_at(), 864);
}

#[test]
fn project_stop_fences_all_consumers_without_provider_work() {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    use tmt_remote::{
        objects::IoBudget,
        sealed_sync::{
            AuthFailure, AuthIdentity, AuthPort, AuthToken, SealedSyncPort, SyncContext, Target,
        },
    };
    struct NoToken;
    impl AuthPort for NoToken {
        fn identity(&self) -> AuthIdentity {
            AuthIdentity {
                project: "demo-tmt-remote".into(),
                uid: "fixture-user".into(),
            }
        }
        fn token<'a>(
            &'a self,
            _: &'a IoBudget<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>>
        {
            panic!("a fenced project must not acquire a token")
        }
    }
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let (runtime, _, _) = runtime(&layout, false);
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/sealed_sync/codec-v1.json"
    ))
    .unwrap();
    let declaration = fixture["declaration"].as_str().unwrap().as_bytes();
    let context = || {
        SyncContext::bind(
        serde_json::json!({"apiKey":"public", "appId":"1:123:web:fixture", "authDomain":"demo-tmt-remote.firebaseapp.com", "projectId":"demo-tmt-remote"}),
        declaration, fixture["declarationDigest"].as_str().unwrap(), "fixture-space", Arc::new(NoToken),
    ).unwrap()
    };
    let first = tmt_remote::native_sync::NativeClient::new(
        context(),
        runtime.clone(),
        Model::new(1, 1).unwrap(),
    );
    let second = tmt_remote::native_sync::NativeClient::new(
        context(),
        runtime.clone(),
        Model::new(1, 1).unwrap(),
    );
    first.close();
    assert_eq!(
        second.budget_state().unwrap(),
        Decision::Allow,
        "closing one binding must not close its project"
    );
    runtime.stop();
    assert_eq!(second.budget_state(), Err(SyncError::Unavailable));
    let cancelled = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(2),
        cancelled: &cancelled,
    };
    let target = Target {
        resource: "rows".into(),
        id: "probe".into(),
    };
    let mut request = pin!(second.read(&target, &io));
    assert!(matches!(
        request
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(SyncError::Unavailable))
    ));
}

#[test]
fn project_checkpoint_persists_while_provider_waits_in_backoff_then_stop_joins() {
    if isolated_case(
        "serve::native_factory_tests::project_checkpoint_persists_while_provider_waits_in_backoff_then_stop_joins",
    ) {
        return;
    }
    use std::{
        future::Future,
        pin::pin,
        sync::atomic::AtomicU64,
        task::{Context, Poll, Waker},
    };
    use tmt_remote::{
        firebase_auth::Operation,
        objects::IoBudget,
        sealed_sync::{
            AuthFailure, AuthIdentity, AuthPort, AuthToken, SealedSyncPort, SyncContext,
            SyncInterest,
        },
    };
    struct Backoff {
        time: AtomicU64,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        calls: AtomicUsize,
    }
    impl Clock for Backoff {
        fn now(&self) -> Time {
            Time {
                wall_ms: NOW,
                monotonic_ms: self.time.load(Ordering::SeqCst),
            }
        }
        fn wait_until(&self, _: u64, cancel: &dyn Cancellation) -> bool {
            self.time.store(60_000, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            !cancel.cancelled()
        }
    }
    impl Jitter for Backoff {
        fn milliseconds(&self, _: u64) -> u64 {
            0
        }
    }
    impl Http for Backoff {
        fn send(
            &self,
            request: &Request,
            _: u64,
            _: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            assert_eq!(request.operation(), Operation::Query);
            assert!(request.url().ends_with(":runAggregationQuery"));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Response {
                status: 503,
                body: b"{}".to_vec(),
                retry_after_ms: Some(60_000),
            })
        }
    }
    struct SignedIn;
    impl AuthPort for SignedIn {
        fn identity(&self) -> AuthIdentity {
            AuthIdentity {
                project: "demo-tmt-remote".into(),
                uid: "fixture-user".into(),
            }
        }
        fn token<'a>(
            &'a self,
            _: &'a IoBudget<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>>
        {
            Box::pin(async move {
                Ok(AuthToken {
                    identity: self.identity(),
                    token: "fixture-token".into(),
                    expires_at_ms: NOW + 3_600_000,
                })
            })
        }
    }
    fn ready<F: Future>(future: F) -> F::Output {
        match pin!(future)
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("injected synchronous effects must resolve"),
        }
    }
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let (_, budget, checkpoints) = runtime(&layout, false);
    let (entered, waiting) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let ports = Arc::new(Backoff {
        time: AtomicU64::new(0),
        entered,
        release: Mutex::new(released),
        calls: AtomicUsize::new(0),
    });
    let runtime = Arc::new(NativeRuntime::new(
        ports.clone(),
        ports.clone(),
        ports.clone(),
        budget.clone(),
    ));
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/sealed_sync/codec-v1.json"
    ))
    .unwrap();
    let context = SyncContext::bind(serde_json::json!({"apiKey":"public","appId":"1:123:web:fixture","authDomain":"demo-tmt-remote.firebaseapp.com","projectId":"demo-tmt-remote"}),
        fixture["declaration"].as_str().unwrap().as_bytes(), fixture["declarationDigest"].as_str().unwrap(), "fixture-space", Arc::new(SignedIn)).unwrap();
    let client = Arc::new(tmt_remote::native_sync::NativeClient::new(
        context,
        runtime.clone(),
        Model::new(1, 1).unwrap(),
    ));
    let worker_client = client.clone();
    struct Worker {
        runtime: Arc<NativeRuntime>,
        release: mpsc::Sender<()>,
        thread: Option<std::thread::JoinHandle<Result<tmt_remote::sealed_sync::Change, SyncError>>>,
    }
    impl Drop for Worker {
        fn drop(&mut self) {
            self.runtime.stop();
            let _ = self.release.send(());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    let mut worker = Worker {
        runtime: runtime.clone(),
        release,
        thread: Some(std::thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            let io = IoBudget {
                // The simulated minute remains within the caller's lifetime and
                // the 120-second provider backoff; fixture Drop always cancels it.
                deadline: Instant::now() + Duration::from_secs(180),
                cancelled: &cancelled,
            };
            let interest = SyncInterest {
                resource: "rows".into(),
                scope: serde_json::from_value(
                    serde_json::json!({"tenant":{"stringValue":"fixture-space"}}),
                )
                .unwrap(),
            };
            let mut subscription = ready(worker_client.subscribe(&interest, &io)).unwrap();
            assert!(matches!(
                ready(subscription.next(&io)),
                Err(SyncError::Unavailable)
            ));
            let result = ready(subscription.next(&io));
            subscription.close();
            result
        })),
    };
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(ports.calls.load(Ordering::SeqCst), 1);
    let tick = CheckpointOwner::start(
        runtime.clone(),
        Duration::from_millis(5),
        Arc::new(|| panic!("checkpoint failed")),
    )
    .unwrap();
    checkpoints
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(layout.directory.join("native-budget/demo-tmt-remote.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved["reads"], 3,
        "the owner persists the precharged probe while its provider is blocked"
    );
    runtime.stop();
    worker.release.send(()).unwrap();
    assert!(matches!(
        worker.thread.take().unwrap().join().unwrap(),
        Err(SyncError::Cancelled)
    ));
    tick.finish().unwrap();
    drop(worker);
    drop(client);
    drop(runtime);
    drop(budget);
    assert_eq!(
        NativeBudgetFile::open(&layout, "demo-tmt-remote", NOW, 60_001)
            .unwrap()
            .usage()
            .reads,
        3
    );
}
