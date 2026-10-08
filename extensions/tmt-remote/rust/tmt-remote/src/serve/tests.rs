//! Serve composition tests need no wire-leaf import: successful bus/data proofs
//! remain with object_service/tests, which owns the carrier fixtures.
use super::*;
use std::{
    fs,
    net::TcpStream,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    sync::atomic::AtomicUsize,
};
use tmt_remote::mount::NoSessions;

#[path = "../../tests/support/executable_fixture.rs"]
mod executable_fixture;

const fn declaration(objects: ObjectDeclaration) -> Extension {
    Extension {
        name: "alpha",
        body_bytes: 1024,
        reply_bytes: 1024,
        tunnels: 2,
        tunnel_idle: Duration::from_secs(5),
        objects,
    }
}
static LOCAL: [Extension; 1] = [declaration(ObjectDeclaration::Local)];
static DISABLED: [Extension; 1] = [declaration(ObjectDeclaration::Disabled)];

struct ServeObjectFixture {
    root: PathBuf,
    core: PathBuf,
}
impl ServeObjectFixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(format!(
            "/tmp/t2082-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let core = root.join("core");
        executable_fixture::write_executable(
            &core,
            &format!(
                "printf '%s\\n' \"$*\" >> '{0}/calls'\ninput=$(cat)\ncase \"$input\" in *storage.root*) printf '{{\"dataRoot\":\"{0}/data\"}}';; *) printf '{{\"version\":1,\"limits\":{{\"inputBytes\":1024,\"outputBytes\":4096}}}}';; esac\n",
                root.display()
            ),
        )
        .unwrap();
        Self { root, core }
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    fn socket(&self) -> UnixListener {
        let directory = self.data().join("alpha");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(directory.join("door.sock")).unwrap();
        fs::set_permissions(
            directory.join("door.sock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        listener
    }
    fn released(&self) {
        let layout = Layout::open(&self.data()).unwrap();
        let lease = layout.serve_lock().expect("serve leaked its lease");
        assert!(!layout.directory.join(control::SOCKET).exists());
        drop(lease);
    }
}
impl Drop for ServeObjectFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

/// Own the foreground thread before observing readiness; even an assertion panic
/// cancels and joins it before the fixture may remove its files.
struct ServeObjectRun {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), RemoteError>>>,
    parent: UnixStream,
}
impl ServeObjectRun {
    fn start(fixture: &ServeObjectFixture, extensions: &'static [Extension]) -> Self {
        let (parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let core = CoreClient::at(fixture.core.clone()).unwrap();
        let thread = thread::spawn(move || {
            let mut handoff = Handoff::new(worker, Arc::clone(&flag))?;
            let result =
                foreground_with(core, extensions, Some(0), true, &flag, Some(&mut handoff));
            handoff.finish();
            result
        });
        Self {
            stop,
            thread: Some(thread),
            parent,
        }
    }
    fn ready(&mut self) -> Ready {
        self.ready_after(|| {})
    }
    fn ready_after(&mut self, check: impl FnOnce()) -> Ready {
        let (tag, value) = read_frame(&mut self.parent, Instant::now() + STARTUP, &self.stop)
            .expect("serve did not report readiness");
        assert_eq!(tag, READY);
        let ready = Ready::validate(value).unwrap();
        check();
        self.parent
            .write_all(&[ACCEPT])
            .expect("ready foreground must retain its handoff endpoint");
        ready
    }
    fn finish(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap().unwrap();
    }
}
impl Drop for ServeObjectRun {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.parent.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            assert!(
                thread.join().is_ok(),
                "foreground thread leaked or panicked"
            );
        }
    }
}

fn ordinary_page(ready: &Ready) {
    let authority = ready
        .address
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(format!("GET / HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .expect("ready door is live");
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200 "));
}

#[test]
fn serve_disabled_objects_prepare_nothing_and_connect_nowhere() {
    let fixture = ServeObjectFixture::new();
    let listener = fixture.socket();
    listener.set_nonblocking(true).unwrap();
    let mut run = ServeObjectRun::start(&fixture, &DISABLED);
    let ready = run.ready();
    ordinary_page(&ready);
    assert!(!fixture.data().join("remote/objects.db").exists());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    run.finish();
    fixture.released();
}

/// Join the fixture listener even when readiness or a following assertion fails.
struct ServeObjectPeer {
    path: PathBuf,
    completed: Arc<AtomicBool>,
    thread: Option<JoinHandle<UnixListener>>,
}
impl ServeObjectPeer {
    fn refuse(fixture: &ServeObjectFixture) -> Self {
        let listener = fixture.socket();
        let path = fixture.data().join("alpha/door.sock");
        let completed = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&completed);
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read_exact(&mut byte).is_err() {
                    return listener;
                }
                head.push(byte[0]);
            }
            assert!(head.starts_with(b"GET /.tmt/remote/object-channel-v1 "));
            listener.set_nonblocking(true).unwrap();
            observed.store(true, Ordering::Release);
            let _ = stream.shutdown(std::net::Shutdown::Both);
            listener
        });
        Self {
            path,
            completed,
            thread: Some(thread),
        }
    }
    fn finish(mut self) -> UnixListener {
        self.thread.take().unwrap().join().unwrap()
    }
}
impl Drop for ServeObjectPeer {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // An owned wake releases accept; closing it releases the fixture read.
            if let Ok(stream) = UnixStream::connect(&self.path) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            assert!(thread.join().is_ok(), "extension fixture thread panicked");
        }
    }
}

#[test]
fn serve_one_failed_object_handshake_precedes_ready_and_leaves_door_running() {
    let fixture = ServeObjectFixture::new();
    let peer = ServeObjectPeer::refuse(&fixture);
    let mut run = ServeObjectRun::start(&fixture, &LOCAL);
    let ready = run.ready_after(|| {
        assert!(
            peer.completed.load(Ordering::Acquire),
            "ready preceded object setup"
        );
    });
    let listener = peer.finish();
    ordinary_page(&ready);
    assert!(fixture.data().join("remote/objects.db").exists());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    run.finish();
    fixture.released();
}

#[test]
fn serve_missing_object_listener_is_not_retried_when_it_appears_after_ready() {
    let fixture = ServeObjectFixture::new();
    let mut run = ServeObjectRun::start(&fixture, &LOCAL);
    let ready = run.ready();
    let listener = fixture.socket();
    listener.set_nonblocking(true).unwrap();
    ordinary_page(&ready);
    run.finish();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fixture.released();
}

#[test]
fn serve_object_storage_failure_is_degraded_without_reset_or_door_failure() {
    let fixture = ServeObjectFixture::new();
    let layout = Layout::open(&fixture.data()).unwrap();
    let ledger = layout.directory.join("objects.db");
    let damaged = b"not an object ledger";
    fs::write(&ledger, damaged).unwrap();
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let mut run = ServeObjectRun::start(&fixture, &LOCAL);
    let ready = run.ready();
    ordinary_page(&ready);
    assert_eq!(fs::read(&ledger).unwrap(), damaged);
    run.finish();
    fixture.released();
}

#[test]
fn object_activation_has_typed_failure_and_stop_prevents_a_successor_attempt() {
    let fixture = ServeObjectFixture::new();
    let layout = Layout::open(&fixture.data()).unwrap();
    let serving = layout.serve_lock().unwrap();
    let stop = AtomicBool::new(false);
    let objects = ObjectService::open(
        &serving,
        &LOCAL,
        Quotas::contract(),
        system_clock(),
        ServiceBounds::contract(),
        Origins::default(),
        &IoBudget {
            deadline: Instant::now() + STARTUP,
            cancelled: &stop,
        },
    )
    .unwrap()
    .unwrap();
    let mounts = Mounts::with_extensions(
        fixture.data(),
        "http://127.0.0.1:1",
        "/r/k7qxm4tz2pbwn6rh",
        Arc::new(NoSessions),
        &LOCAL,
    );
    assert_eq!(
        activate_objects(&objects, &mounts, &LOCAL, &stop).unwrap(),
        [ObjectSetupFailure {
            extension: "alpha",
            error: ActivateError::Connect(std::io::ErrorKind::NotFound)
        }]
    );
    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        activate_objects(&objects, &mounts, &LOCAL, &stop)
            .unwrap_err()
            .code,
        "REMOTE_STARTUP_CANCELLED"
    );
    objects.shutdown();
    drop(objects);
    drop(serving);
    fixture.released();
}
