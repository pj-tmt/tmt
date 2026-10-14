//! Real serve/control/carrier composition against private files and fake HTTP.
//! Protocol JSON stays in this fixture; production imports the carrier owner.
use super::{
    native_factory::NativePorts,
    object_tests::{ServeObjectFixture, ServeObjectRun, ordinary_page},
    test_isolation::isolated_case,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_remote::{
    control,
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, Request, Response, Time,
    },
    firestore_budget::pacific_day,
    limits,
    mount::EXTENSIONS,
    native_sync::NativeBudgetFile,
    remote_catalog, settings,
    state::Layout,
};

const NOW: u64 = 1_800_000_000_000;
fn token() -> String {
    format!("header.{}.signature", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({
        "aud":"demo-remote-1","sub":"fixture-user","firebase":{"sign_in_provider":"anonymous"}
    })).unwrap()))
}
struct FakeHttp(Mutex<Vec<Operation>>);
impl Http for FakeHttp {
    fn send(
        &self,
        request: &Request,
        _: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        assert!(!cancel.cancelled());
        self.0.lock().unwrap().push(request.operation());
        let (status, body) = match request.operation() {
            Operation::Refresh => (
                200,
                json!({"id_token":token(), "expires_in":"3600", "token_type":"Bearer", "refresh_token":"fixture-refresh", "user_id":"fixture-user", "project_id":"123"}),
            ),
            Operation::Read => {
                assert!(request.url().contains(
                    "projects/demo-remote-1/databases/(default)/documents/x/colab/log/probe"
                ));
                assert_eq!(request.bearer(), Some(token().as_str()));
                (404, json!({"error":{"status":"NOT_FOUND"}}))
            }
            _ => panic!("unexpected provider operation"),
        };
        Ok(Response {
            status,
            body: serde_json::to_vec(&body).unwrap(),
            retry_after_ms: None,
        })
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Time {
        Time {
            wall_ms: NOW,
            monotonic_ms: 60_000,
        }
    }
    fn wait_until(&self, _: u64, _: &dyn Cancellation) -> bool {
        panic!("successful read needs no provider wait")
    }
}
impl Jitter for Fixed {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
fn ports(http: Arc<FakeHttp>) -> NativePorts {
    NativePorts {
        http,
        clock: Arc::new(Fixed),
        jitter: Arc::new(Fixed),
    }
}
fn declaration() -> Vec<u8> {
    include_bytes!("../../../../../tmt-colab/contracts/vectors/deploy-declaration-v1.json").to_vec()
}
fn digest() -> String {
    let reply: Value = serde_json::from_slice(&declaration()).unwrap();
    reply["declarationDigest"].as_str().unwrap().into()
}
fn selected(fixture: &ServeObjectFixture) -> Layout {
    let layout = Layout::open(&fixture.data()).unwrap();
    let mut descriptor: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/remote_link/descriptor-v1.json"
    ))
    .unwrap();
    descriptor.sort_all_objects();
    let link = format!(
        "https://demo-remote-1.web.app/pair#{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&descriptor).unwrap())
    );
    remote_catalog::add(&layout, "fixture", &link).unwrap();
    settings::set_default_remote(&fixture.data(), "fixture").unwrap();
    layout
}
fn private_json(directory: &std::path::Path, name: &str, value: Value) {
    fs::create_dir_all(directory).unwrap();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    let file = directory.join(name);
    fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
}
fn signed_in(layout: &Layout, expired: bool) {
    let created = if expired {
        NOW - limits::FIREBASE_CREDENTIAL_LIFETIME_MS - 1
    } else {
        NOW
    };
    private_json(
        &layout.directory.join("firebase-auth"),
        "demo-remote-1.json",
        json!({
            "version":1,"projectId":"demo-remote-1","uid":"fixture-user","provider":"anonymous",
            "refreshToken":"fixture-refresh","createdAtMs":created,
            "expiresAtMs":created + limits::FIREBASE_CREDENTIAL_LIFETIME_MS,
            "lastUsedAtMs":created,"state":"active"
        }),
    );
}
/// Owned listener accepts the ordinary v2 channel and a separate opt-in v3
/// candidate. Kept streams and the accept worker are closed/joined on Drop.
struct Peer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    native: mpsc::Receiver<(UnixStream, String)>,
}
impl Peer {
    fn start(fixture: &ServeObjectFixture) -> Self {
        let directory = fixture.data().join("colab");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("door.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (sender, native) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut legacy = Vec::new();
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    assert!(head.len() < 8192);
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                assert!(head.starts_with("GET /.tmt/remote/object-channel-v1 "));
                let generation = head
                    .lines()
                    .find_map(|line| line.strip_prefix("tmt-object-generation: "))
                    .unwrap()
                    .to_owned();
                let v3 = head.contains("tmt-object-channel: 3\r\n");
                let version = if v3 {
                    "tmt-object-channel: 3\r\n"
                } else {
                    assert!(head.contains("tmt-object-channel: 2\r\n"));
                    ""
                };
                write!(stream, "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tmt-object-channel-v1\r\n{version}tmt-object-generation: {generation}\r\n\r\n").unwrap();
                if v3 {
                    sender.send((stream, generation)).unwrap();
                } else {
                    legacy.push(stream);
                }
            }
        });
        Self {
            path,
            stop,
            worker: Some(worker),
            native,
        }
    }
    fn activated(&self, layout: &Layout) -> Channel {
        match control::activate_native_until(
            &layout.directory,
            "colab",
            Instant::now() + Duration::from_secs(5),
        ) {
            Ok(_) => {}
            Err(_) => panic!("configured native channel must activate"),
        }
        let (stream, generation) = self.native.recv_timeout(Duration::from_secs(5)).unwrap();
        // Observe the real owner-only wire through the existing validated connector.
        let mut socket = control::connect(&layout.directory).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        writeln!(
            socket,
            "{}",
            json!({"op":"sync.activate","extension":"colab","version":3})
        )
        .unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(socket), &mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            response,
            json!({"activated":true,"version":3,"generation":generation,"project":"demo-remote-1"}),
            "the live registered factory supplies the project, not a deployment record"
        );
        assert!(
            !layout.directory.join("deployments").exists(),
            "tenant activation must not need or create deployment records"
        );
        Channel { stream, generation }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(stream) = UnixStream::connect(&self.path) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let joined = self.worker.take().unwrap().join();
        if !thread::panicking() {
            assert!(joined.is_ok(), "fixture accept worker leaked");
        }
    }
}
struct Channel {
    stream: UnixStream,
    generation: String,
}
impl Channel {
    fn request(&mut self, id: u64, method: &str, input: Value) -> Value {
        let bytes = serde_json::to_vec(&json!({"kind":"sync-request","version":1,"generation":self.generation,"requestId":id.to_string(),"call":{"method":method,"input":input}})).unwrap();
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        self.stream.write_all(&bytes).unwrap();
        let mut size = [0; 4];
        self.stream.read_exact(&mut size).unwrap();
        let size = u32::from_be_bytes(size) as usize;
        assert!(size <= limits::NATIVE_SYNC_PAGE_BYTES);
        let mut bytes = vec![0; size];
        self.stream.read_exact(&mut bytes).unwrap();
        let frame: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(frame["kind"], "sync-result");
        assert_eq!(frame["requestId"], id.to_string());
        frame["outcome"].clone()
    }
    fn bind(&mut self) -> Value {
        self.request(1, "sync.bind", json!({"project":"demo-remote-1","extension":"colab","declarationDigest":digest(),"space":"fixture-space"}))
    }
}
#[test]
fn configured_serve_reaches_native_client_only_after_opt_in_and_flushes_on_stop() {
    if isolated_case(
        "serve::native_serve_tests::configured_serve_reaches_native_client_only_after_opt_in_and_flushes_on_stop",
    ) {
        return;
    }
    let fixture = ServeObjectFixture::new();
    let layout = selected(&fixture);
    signed_in(&layout, false);
    let peer = Peer::start(&fixture);
    let http = Arc::new(FakeHttp(Mutex::new(Vec::new())));
    let mut run =
        ServeObjectRun::start_native(&fixture, &EXTENSIONS, declaration(), ports(http.clone()));
    let ready = run.ready();
    ordinary_page(&ready);
    assert!(
        http.0.lock().unwrap().is_empty(),
        "startup must not refresh or contact Firestore"
    );
    assert!(
        matches!(peer.native.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "ordinary startup remains v2"
    );
    let mut channel = peer.activated(&layout);
    let binding = channel.bind();
    assert_eq!(binding["result"], "binding");
    assert!(
        http.0.lock().unwrap().is_empty(),
        "factory construction performs no provider I/O"
    );
    let result = channel.request(
        2,
        "sync.read",
        json!({"binding":binding["context"]["id"],"target":{"resource":"log","id":"probe"}}),
    );
    assert_eq!(
        result,
        json!({"result":"read","document":null}),
        "provider operations: {:?}",
        *http.0.lock().unwrap()
    );
    assert_eq!(
        *http.0.lock().unwrap(),
        vec![Operation::Refresh, Operation::Read]
    );
    run.finish();
    fixture.released();
    let persisted = NativeBudgetFile::open(&layout, "demo-remote-1", NOW, 60_001).unwrap();
    assert_eq!(
        persisted.usage().reads,
        3,
        "stop persists attempted provider reads and releases the sole writer"
    );
    // Shutdown closes the joined native channel, even while the peer still owns it.
    let mut byte = [0];
    assert_eq!(channel.stream.read(&mut byte).unwrap(), 0);
}
#[test]
fn missing_project_sign_in_and_expired_sign_in_refuse_native_without_losing_the_door() {
    if isolated_case(
        "serve::native_serve_tests::missing_project_sign_in_and_expired_sign_in_refuse_native_without_losing_the_door",
    ) {
        return;
    }
    for state in ["project", "sign-in", "expired"] {
        let fixture = ServeObjectFixture::new();
        let layout = if state == "project" {
            Layout::open(&fixture.data()).unwrap()
        } else {
            selected(&fixture)
        };
        if state == "expired" {
            signed_in(&layout, true);
        }
        let peer = Peer::start(&fixture);
        let http = Arc::new(FakeHttp(Mutex::new(Vec::new())));
        let mut run =
            ServeObjectRun::start_native(&fixture, &EXTENSIONS, declaration(), ports(http.clone()));
        let ready = run.ready();
        ordinary_page(&ready);
        let refusal = control::activate_native_until(
            &layout.directory,
            "colab",
            Instant::now() + Duration::from_secs(5),
        );
        match refusal {
            Err(control::ActivationFailure::Refused(error)) => {
                assert_eq!(error.code, "REMOTE_STATE_UNAVAILABLE", "{state}")
            }
            _ => panic!("{state} must be a typed activation refusal"),
        }
        assert!(http.0.lock().unwrap().is_empty());
        assert!(matches!(
            peer.native.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        ordinary_page(&ready);
        run.finish();
        fixture.released();
    }
}
#[test]
fn exhausted_project_budget_refuses_binding_without_provider_work_or_reset() {
    if isolated_case(
        "serve::native_serve_tests::exhausted_project_budget_refuses_binding_without_provider_work_or_reset",
    ) {
        return;
    }
    let fixture = ServeObjectFixture::new();
    let layout = selected(&fixture);
    signed_in(&layout, false);
    private_json(
        &layout.directory.join("native-budget"),
        "demo-remote-1.json",
        json!({"version":1,"day":pacific_day(NOW).0.0,"reads":13_000}),
    );
    let peer = Peer::start(&fixture);
    let http = Arc::new(FakeHttp(Mutex::new(Vec::new())));
    let mut run =
        ServeObjectRun::start_native(&fixture, &EXTENSIONS, declaration(), ports(http.clone()));
    let ready = run.ready();
    let mut channel = peer.activated(&layout);
    let refusal = channel.bind();
    assert_eq!(refusal["result"], "error");
    assert_eq!(refusal["error"]["code"], "REMOTE_BUDGET_EXHAUSTED");
    assert!(refusal["error"]["resetAtMs"].as_u64().unwrap() > NOW);
    assert!(http.0.lock().unwrap().is_empty());
    ordinary_page(&ready);
    run.finish();
    fixture.released();
    assert_eq!(
        NativeBudgetFile::open(&layout, "demo-remote-1", NOW, 60_001)
            .unwrap()
            .usage()
            .reads,
        13_000
    );
}
