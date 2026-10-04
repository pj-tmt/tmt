//! Colab's owner-only socket on real Unix sockets: framing bounds, the device
//! context from the remote door, the colab-sync-v1 handshake, tunnel bounds,
//! capacity and shutdown.
mod support;
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_colab::{
    keyring::{Keyring, Layout, StateFault},
    limits,
    registration::Registration,
    socket::{MountSocket, Tunnels},
    store::{Store, StreamScope},
};

const SPACE: &str = "space-1";
const DEVICE: &str = "00000000-0000-4000-8000-000000000004";
const OTHER: &str = "00000000-0000-4000-8000-000000000005";
const PAGE: &str = "00000000-0000-4000-8000-000000000001";
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use tmt_colab_model::{framing, object, values};
use tungstenite::{Message, WebSocket, protocol::Role};
fn context(id: &str) -> String {
    json!({"deviceId":id,"kind":"browser","origin":"http://127.0.0.1:1","name":"<b>Laptop</b>",
        "publicKey":values::encode_binary(SigningKey::from_bytes(&[9;32]).verifying_key().as_bytes()),
        "owner":true,"grantRevision":1}).to_string()
}
fn owner(id: &str) -> String {
    format!("tmt-device-context: {}", context(id))
}
fn signing(id: &str) -> SigningKey {
    SigningKey::from_bytes(&[if id == DEVICE { 10 } else { 11 }; 32])
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn registration_body(id: &str) -> Vec<u8> {
    let issued = now();
    let cert = |purpose: &str, key: &[u8; 32]| {
        let input = framing::frame(&[
            b"tmt-ext-cert-v1",
            b"colab",
            purpose.as_bytes(),
            key,
            issued.to_string().as_bytes(),
        ])
        .unwrap();
        json!({"publicKey":values::encode_binary(key),"issuedAtMs":issued,
            "signature":values::encode_binary(&SigningKey::from_bytes(&[9;32]).sign(&input).to_bytes())})
    };
    serde_json::to_vec(&json!({"deviceId":id,"sign":cert("sign",signing(id).verifying_key().as_bytes()),"enc":cert("enc",&[7;32])})).unwrap()
}

const UPGRADE: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n";

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Running {
    root: PathBuf,
    path: PathBuf,
    space: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Running {
    fn start(tunnels: Tunnels) -> Self {
        Self::start_with_app(tunnels, None)
    }
    fn start_with_app(tunnels: Tunnels, app: Option<tmt_colab::assets::App>) -> Self {
        // Short absolute root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!(
            "/tmp/tmt-1039-colab-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let space = key.space_id.clone();
        let store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        let mut registration = Registration::with_decoder_config(
            store,
            key,
            support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
        )
        .unwrap();
        for id in [DEVICE, OTHER] {
            registration
                .register(Some(&context(id)), &registration_body(id), now())
                .unwrap();
        }
        let socket = MountSocket::bind(&layout, &space, tunnels)
            .unwrap()
            .with_registration(&layout, Arc::new(Mutex::new(registration)))
            .unwrap()
            .with_app(app);
        let path = socket.path.clone();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || socket.run(&flag).unwrap());
        Self {
            root,
            path,
            space,
            stop,
            worker: Some(worker),
        }
    }
    fn connect(&self) -> UnixStream {
        let socket = UnixStream::connect(&self.path).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
    }
    fn request(&self, request: &str) -> String {
        self.request_with_timeout(request, Duration::from_secs(3))
    }
    fn request_with_timeout(&self, request: &str, timeout: Duration) -> String {
        let mut socket = self.connect();
        socket.set_read_timeout(Some(timeout)).unwrap();
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }
    fn get(path: &str, headers: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\n{headers}\r\n")
    }
    /// Upgrade and return the client after the 101 head.
    fn tunnel(&self) -> (UnixStream, String) {
        self.tunnel_for(DEVICE)
    }
    fn tunnel_for(&self, id: &str) -> (UnixStream, String) {
        let owner_header = owner(id);
        self.open_tunnel(&format!("{owner_header}\r\n{UPGRADE}"))
    }
    fn open_tunnel(&self, headers: &str) -> (UnixStream, String) {
        let mut socket = self.connect();
        socket
            .write_all(Self::get("/sync", headers).as_bytes())
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        (socket, String::from_utf8(head).unwrap())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        assert!(!self.path.exists(), "socket left behind");
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn closed(socket: &mut UnixStream) -> bool {
    matches!(socket.read(&mut [0; 1]), Ok(0))
        || matches!(socket.read(&mut [0; 1]), Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset)
}

#[test]
fn pages_follow_the_forwarded_owner_context_within_the_door_bounds() {
    let server = Running::start(Tunnels::PRODUCT);
    let private = server.request(&Running::get("/", ""));
    assert!(private.starts_with("HTTP/1.1 200"));
    assert!(private.contains("This colab space is private. Open it from a browser paired with tmt remote pair, or use a share link."));
    assert!(private.contains("Referrer-Policy: no-referrer"));
    assert!(private.contains("Content-Security-Policy: default-src 'none'"));
    let owner_header = owner(DEVICE);
    let owned = server.request(&Running::get("/", &format!("{owner_header}\r\n")));
    assert!(owned.contains(&format!(
        "Colab space {} is running. You are signed in as &lt;b&gt;Laptop&lt;/b&gt;. {}.",
        server.space,
        tmt_colab::assets::BUILD_HINT
    )));
    for context in [
        "tmt-device-context: {}\r\n",
        "tmt-device-context: not json\r\n",
        "tmt-device-context: {\"deviceId\":\"x\",\"name\":\"n\",\"owner\":false}\r\n",
    ] {
        assert!(
            server
                .request(&Running::get("/", context))
                .starts_with("HTTP/1.1 400"),
            "{context}"
        );
    }
    let fields = (0..31)
        .map(|i| format!("X-Field-{i}: value\r\n"))
        .collect::<String>();
    assert!(
        server
            .request(&Running::get("/", &fields))
            .starts_with("HTTP/1.1 200")
    );
    for (request, status) in [
        (
            Running::get("/", &format!("{fields}X-Overflow: value\r\n")),
            400,
        ),
        ("GET / HTTP/1.1\nHost: x\r\n\r\n".into(), 400),
        (Running::get("/", "Transfer-Encoding: chunked\r\n"), 400),
        (Running::get("/", "Forwarded: host=evil\r\n"), 400),
        (Running::get("/", "Content-Length: 00\r\n"), 400),
        (
            Running::get("/", "Content-Length: 0\r\nContent-Length: 0\r\n"),
            400,
        ),
        (Running::get("http://evil.invalid/", ""), 400),
        (Running::get("/?q=1", ""), 400),
        (Running::get("/api", "Content-Length: 65537\r\n"), 413),
        (
            format!("GET / HTTP/1.1\r\nX-Fill: {}", "x".repeat(8192)),
            413,
        ),
        (Running::get("/other", ""), 404),
    ] {
        assert!(
            server
                .request(&request)
                .starts_with(&format!("HTTP/1.1 {status}")),
            "{request:.60}"
        );
    }
    // A full bounded body is read and the request still answered.
    let body = "x".repeat(limits::HTTP_BODY_BYTES);
    assert!(
        server
            .request(&format!(
                "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
            .starts_with("HTTP/1.1 404")
    );
}

#[test]
fn owner_upgrades_are_accepted_capped_and_closed_when_idle() {
    let server = Running::start(Tunnels {
        cap: 2,
        idle: Duration::from_millis(500),
    });
    let (mut first, head) = server.tunnel();
    assert!(head.starts_with("HTTP/1.1 101"));
    // RFC 6455 section 1.3 sample key and accept value.
    assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    assert!(head.contains("Sec-WebSocket-Protocol: colab-sync-v1\r\n"));
    let (_second, _) = server.tunnel();
    let owner_header = owner(DEVICE);
    let refused = server.request(&Running::get(
        "/sync",
        &format!("{owner_header}\r\n{UPGRADE}"),
    ));
    assert!(refused.starts_with("HTTP/1.1 503"), "{refused:.40}");
    // No frame is sent: the idle timer must fire without inbound readiness.
    for (headers, status) in [
        (UPGRADE.to_owned(), 403),
        (
            format!(
                "{owner_header}\r\n{}",
                UPGRADE.replace("colab-sync-v1", "chat")
            ),
            400,
        ),
        (
            format!(
                "{owner_header}\r\n{}",
                UPGRADE.replace("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n", "")
            ),
            400,
        ),
        (
            format!(
                "{owner_header}\r\n{}",
                UPGRADE.replace("Version: 13", "Version: 8")
            ),
            400,
        ),
        (
            format!(
                "{owner_header}\r\n{}",
                UPGRADE.replace("dGhlIHNhbXBsZSBub25jZQ==", "c2hvcnQ=")
            ),
            400,
        ),
    ] {
        assert!(
            server
                .request(&Running::get("/sync", &headers))
                .starts_with(&format!("HTTP/1.1 {status}")),
            "{headers:.60}"
        );
    }
    // After the idle bound both tunnels close and the cap frees.
    let start = Instant::now();
    assert!(closed(&mut first));
    assert!(start.elapsed() < Duration::from_secs(3));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let (mut again, head) = server.tunnel();
        if head.starts_with("HTTP/1.1 101") {
            drop(again.write_all(b"x"));
            break;
        }
        assert!(Instant::now() < deadline, "cap never freed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn capacity_and_shutdown_close_retained_sockets_and_tunnels_twice() {
    for _ in 0..2 {
        let server = Running::start(Tunnels::PRODUCT);
        let (mut tunnel, head) = server.tunnel();
        assert!(head.starts_with("HTTP/1.1 101"));
        let mut retained = Vec::new();
        for _ in 0..limits::SOCKETS {
            let mut socket = server.connect();
            socket.write_all(b"GET / HTTP/1.1\r\n").unwrap();
            retained.push(socket);
        }
        // A held tunnel does not count against request workers.
        assert!(
            server
                .request(&Running::get("/", ""))
                .starts_with("HTTP/1.1 429")
        );
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(closed(&mut tunnel), "tunnel leaked");
        for mut socket in retained {
            assert!(closed(&mut socket), "socket leaked");
        }
    }
    let server = Running::start(Tunnels::PRODUCT);
    assert!(
        server
            .request("GET / HTTP/1.1\r\n")
            .starts_with("HTTP/1.1 408")
    );
}

#[test]
fn bind_refuses_a_colab_directory_others_can_enter() {
    let root = PathBuf::from(format!(
        "/tmp/tmt-1039-colab-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let layout = Layout::open(&root).unwrap();
    fs::set_permissions(&layout.directory, fs::Permissions::from_mode(0o755)).unwrap();
    let error = MountSocket::bind(&layout, SPACE, Tunnels::PRODUCT)
        .err()
        .expect("a group/other-enterable directory refuses");
    assert_eq!(
        error.downcast_ref::<StateFault>(),
        Some(&StateFault::UnsafeDirectory)
    );
    assert!(!layout.directory.join("door.sock").exists());
    fs::remove_dir_all(&root).unwrap();
}

impl Running {
    fn peer(&self, id: &str) -> WebSocket<UnixStream> {
        let (socket, head) = self.tunnel_for(id);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        WebSocket::from_raw_socket(socket, Role::Client, None)
    }
    fn frame(&self, kind: &str, fields: Value) -> Value {
        let mut frame = json!({"version":1,"type":kind,"space":self.space,"page":PAGE,"epoch":"1"});
        frame
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        frame
    }
    fn append(&self, id: &str, seq: u64, previous: [u8; 32]) -> (Value, Vec<u8>, [u8; 32]) {
        self.append_payload(id, seq, previous, b"opaque update")
    }
    fn append_payload(
        &self,
        id: &str,
        seq: u64,
        previous: [u8; 32],
        payload: &[u8],
    ) -> (Value, Vec<u8>, [u8; 32]) {
        let context = object::Context {
            space: self.space.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: id.into(),
            membership_revision: "1".into(),
            stream_seq: seq.to_string(),
            prev_hash: previous,
        };
        let envelope = object::seal(&context, &[8; 32], &signing(id), payload).unwrap();
        let hash = envelope.hash().unwrap();
        let bytes = envelope.to_json().unwrap();
        (self.frame("append",json!({"streamId":id,"seq":seq.to_string(),"envelopeHash":values::encode_binary(&hash),"envelope":values::encode_binary(&bytes)})),bytes,hash)
    }
    fn event(&self, path: &str, header: &str, body: &str) -> String {
        self.request_with_timeout(&format!("POST {path} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{header}\r\n{body}",body.len()), support::DECODER_DEADLINE)
    }
    fn oracle(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root.join("colab/space.db")).unwrap()
    }
}
fn send(peer: &mut WebSocket<UnixStream>, frame: Value) {
    peer.send(Message::Text(frame.to_string().into())).unwrap();
}
fn receive(peer: &mut WebSocket<UnixStream>) -> Value {
    serde_json::from_str(peer.read().unwrap().to_text().unwrap()).unwrap()
}
fn hello(server: &Running, peer: &mut WebSocket<UnixStream>, id: &str) -> Vec<Value> {
    send(
        peer,
        server.frame(
            "hello",
            json!({"membershipRevision":"0","device":id,"cursors":[]}),
        ),
    );
    let first = receive(peer);
    assert_eq!(first["type"], "catchup");
    let key = Keyring::read(&Layout::open(&server.root).unwrap()).unwrap();
    let head = Store::open(&Layout::open(&server.root).unwrap())
        .unwrap()
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    assert_eq!(
        first["membershipHead"]["statementHash"],
        json!(values::encode_binary(&head.hash))
    );
    assert_eq!(first["baseline"], Value::Null);
    send(peer, server.frame("ack", json!({"cursors":[]})));
    let mut pages = vec![first];
    for _ in 0..8 {
        let page = receive(peer);
        assert_eq!(page["type"], "catchup");
        let more = page["more"].as_bool().unwrap();
        send(peer, server.frame("ack", json!({"cursors":[]})));
        if page.get("wraps").is_some() && more {
            continue;
        }
        pages.push(page);
        if !more {
            return pages;
        }
    }
    panic!("catchup did not finish");
}

#[test]
fn two_owner_tabs_append_broadcast_retry_and_catch_up_over_mounted_socket_twice() {
    for _ in 0..2 {
        let server = Running::start(Tunnels::PRODUCT);
        let mut first = server.peer(DEVICE);
        let mut second = server.peer(OTHER);
        assert_eq!(hello(&server, &mut first, DEVICE).len(), 2);
        assert_eq!(hello(&server, &mut second, OTHER).len(), 2);
        let (append, bytes, hash) = server.append(DEVICE, 1, [0; 32]);
        send(&mut first, append.clone());
        assert_eq!(receive(&mut first)["type"], "receipt");
        let broadcast = receive(&mut first);
        assert_eq!(broadcast["type"], "broadcast");
        assert_eq!(receive(&mut second), broadcast);
        assert_eq!(
            values::binary(broadcast["envelope"].as_str().unwrap(), bytes.len()).unwrap(),
            bytes
        );
        let (other, other_bytes, _) = server.append(OTHER, 1, [0; 32]);
        send(&mut second, other);
        assert_eq!(receive(&mut second)["type"], "receipt");
        assert_eq!(receive(&mut second)["streamId"], OTHER);
        assert_eq!(receive(&mut first)["streamId"], OTHER);
        send(&mut first, append);
        assert_eq!(receive(&mut first)["type"], "receipt");
        // A ping/pong barrier proves the retry produced no extra broadcast.
        first.send(Message::Ping(vec![1].into())).unwrap();
        assert!(matches!(first.read().unwrap(), Message::Pong(_)));
        drop(second);
        let mut reopened = server.peer(OTHER);
        let pages = hello(&server, &mut reopened, OTHER);
        assert_eq!(pages.len(), 4);
        let tails: Vec<_> = pages
            .iter()
            .flat_map(|p| p["streams"].as_array().unwrap())
            .flat_map(|s| s["tail"].as_array().unwrap())
            .collect();
        assert_eq!(tails.len(), 2);
        assert!(
            tails
                .iter()
                .any(|t| t["envelopeHash"] == values::encode_binary(&hash))
        );
        let store = Store::open(&Layout::open(&server.root).unwrap()).unwrap();
        for (id, expected) in [(DEVICE, bytes), (OTHER, other_bytes)] {
            assert_eq!(
                store
                    .payload(
                        StreamScope {
                            page: PAGE,
                            epoch: 1,
                            stream: id
                        },
                        1
                    )
                    .unwrap(),
                Some(expected)
            );
        }
    }
}

#[test]
fn strict_device_events_close_live_and_prehello_tunnels_only_after_durable_revoke() {
    let server = Running::start(Tunnels::PRODUCT);
    let db = server.oracle();
    // Page creation's initial epoch key, seeded only for the real rotation case.
    db.execute(
        "INSERT INTO epoch_secrets VALUES (?,?,?)",
        rusqlite::params![PAGE, format!("{:020}", 1), [8u8; 32].as_slice()],
    )
    .unwrap();
    let mut live = server.peer(DEVICE);
    hello(&server, &mut live, DEVICE);
    let (mut prehello, _) = server.tunnel();
    let mut survivor = server.peer(OTHER);
    hello(&server, &mut survivor, OTHER);
    let revoke = json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":2}).to_string();
    use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
    let doc = Doc::with_client_id(77);
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, "source survives revoke");
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "revoke title");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let (append, _, hash) = server.append_payload(DEVICE, 1, [0; 32], &update);
    send(&mut live, append);
    assert_eq!(receive(&mut live)["type"], "receipt");
    assert_eq!(receive(&mut live)["type"], "broadcast");
    assert_eq!(receive(&mut survivor)["type"], "broadcast");
    let before = rotation_rows(&db);

    db.execute_batch("CREATE TRIGGER reject_revoke BEFORE UPDATE ON device_registrations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        server
            .event(
                "/.tmt/remote/device-events",
                "tmt-device-event: 1\r\n",
                &revoke
            )
            .starts_with("HTTP/1.1 503")
    );
    live.send(Message::Ping(vec![2].into())).unwrap();
    assert!(matches!(live.read().unwrap(), Message::Pong(_)));
    let revoked: bool = db
        .query_row(
            "SELECT revoked FROM device_registrations WHERE device_id=?",
            [DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!revoked);
    assert_eq!(rotation_rows(&db), before);
    db.execute_batch("DROP TRIGGER reject_revoke;").unwrap();
    assert!(
        server
            .event(
                "/.tmt/remote/device-events",
                "tmt-device-event: 1\r\n",
                &revoke
            )
            .starts_with("HTTP/1.1 200")
    );
    assert!(closed(&mut prehello));
    assert!(live.read().is_err(), "revoked tunnel still open");
    let (revoked, binding, revision): (bool, Option<Vec<u8>>, String) = db
        .query_row(
            "SELECT revoked,binding,grant_revision FROM device_registrations WHERE device_id=?",
            [DEVICE],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(revoked);
    assert!(binding.is_none());
    assert_eq!(revision, "00000000000000000002");
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let store = Store::open(&layout).unwrap();
    let saved = store.baseline(PAGE, 2).unwrap().unwrap();
    let descriptor: Value = serde_json::from_slice(&saved.descriptor).unwrap();
    let baseline = object::Envelope::from_json(&saved.envelope).unwrap();
    let context = object::Header::decode(baseline.header()).unwrap().context;
    let secret: Vec<u8> = db
        .query_row(
            "SELECT secret FROM epoch_secrets WHERE page=? AND epoch=?",
            rusqlite::params![PAGE, format!("{:020}", 2)],
            |r| r.get(0),
        )
        .unwrap();
    let secret: [u8; 32] = secret.try_into().unwrap();
    assert_ne!(secret, [8; 32]);
    let body: Value = serde_json::from_slice(
        &object::open(
            &baseline,
            &context,
            &secret,
            &key.management_member().unwrap().signing_key,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(body["source"], "source survives revoke");
    assert_eq!(descriptor["title"], "revoke title");
    assert_eq!(
        descriptor["objectEnvelopeHash"],
        values::encode_binary(&baseline.hash().unwrap())
    );
    let mut head = None;
    let mut query = db
        .prepare("SELECT envelope FROM membership_log ORDER BY revision")
        .unwrap();
    let log = query
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(log.len(), 3);
    for bytes in log {
        let statement = tmt_colab_model::statement::Envelope::from_json(&bytes).unwrap();
        let verified = statement
            .verify_next(&server.space, &key.owner_public(), head.as_ref())
            .unwrap();
        match verified.payload {
            tmt_colab_model::payload::Payload::DeviceRevoke(p) => {
                assert_eq!(p.device_id, DEVICE);
                assert_eq!(p.cuts.as_slice().len(), 2);
            }
            tmt_colab_model::payload::Payload::EpochAdvance(p) => {
                assert_eq!(p.epoch, "2");
                assert_eq!(p.cuts.as_slice().len(), 2);
                for cut in p.cuts.as_slice() {
                    let framed = values::binary(&cut.cut, 2048).unwrap();
                    let c = tmt_colab_model::stream_cut::decode(&framed).unwrap();
                    assert_eq!(c.stream_id, DEVICE);
                    if cut.namespace == "content" {
                        assert_eq!(c.tail_head_seq, "1");
                        assert_eq!(*c.tail_head_hash, hash);
                    }
                }
                assert_eq!(p.wraps.as_slice().len(), 2);
                for wrapped in p.wraps.as_slice() {
                    wrapped.verify_owner(&key.owner_public()).unwrap();
                    let h = wrapped.header().unwrap();
                    assert_eq!(h.epoch, "2");
                    assert_ne!(h.recipient_id, DEVICE);
                    assert!(
                        h.recipient_id == OTHER
                            || h.recipient_id == key.management_member().unwrap().id
                    );
                }
            }
            _ => {}
        }
        head = Some(verified.head);
    }
    let after = rotation_rows(&db);
    // A trigger proves equal/older event delivery does not write again.
    db.execute_batch("CREATE TRIGGER reject_replay BEFORE UPDATE ON device_registrations BEGIN SELECT RAISE(ABORT,'replay wrote'); END;").unwrap();
    for revision in [2, 1] {
        let replay =
            json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":revision}).to_string();
        assert!(
            server
                .event(
                    "/.tmt/remote/device-events",
                    "tmt-device-event: 1\r\n",
                    &replay
                )
                .starts_with("HTTP/1.1 200")
        );
    }
    assert_eq!(rotation_rows(&db), after);
    let denied = server.request(&Running::get(
        "/sync",
        &format!("{}\r\n{UPGRADE}", owner(DEVICE)),
    ));
    assert!(denied.starts_with("HTTP/1.1 403"));
    // Epoch-1 subscriptions lose admission; the surviving identity can reopen.
    let mut reopened = server.peer(OTHER);
    let hello = server.frame(
        "hello",
        json!({"membershipRevision":"0","device":OTHER,"cursors":[],"epoch":"2"}),
    );
    send(&mut reopened, hello);
    let first = receive(&mut reopened);
    assert_eq!(first["membershipHead"]["revision"], "3");
    assert_eq!(first["baseline"], values::encode_binary(&saved.descriptor));
    // The real mounted admission now supplies the stored reset descriptor;
    // native bootstrap must deliver its matching encrypted object as well.
    assert_eq!(
        first["baselineObject"]["envelopeHash"],
        values::encode_binary(&baseline.hash().unwrap())
    );
    assert_eq!(
        first["baselineObject"]["envelope"],
        values::encode_binary(&saved.envelope)
    );
    let final_page = receive(&mut reopened);
    assert!(final_page.get("baselineObject").is_none());
    assert_eq!(final_page["more"], false);
    reopened.send(Message::Ping(vec![3].into())).unwrap();
    assert!(matches!(reopened.read().unwrap(), Message::Pong(_)));
}

#[test]
fn device_event_header_body_and_reserved_path_are_strict_and_rename_has_no_state() {
    let server = Running::start(Tunnels::PRODUCT);
    let valid = json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":2}).to_string();
    for header in [
        "",
        "tmt-device-event: 0\r\n",
        "tmt-device-event: 01\r\n",
        "tmt-device-event: 1\r\ntmt-device-event: 1\r\n",
    ] {
        assert!(
            server
                .event("/.tmt/remote/device-events", header, &valid)
                .starts_with("HTTP/1.1 400")
        );
    }
    for body in [
        valid.replace("device.revoked", "unknown"),
        valid.replace("\"grantRevision\":2", "\"grantRevision\":0"),
        valid.replace("\"grantRevision\":2", "\"grantRevision\":9007199254740992"),
        valid.replace("\"grantRevision\":2", "\"grantRevision\":2.0"),
        valid.replace(DEVICE, "bad"),
        valid.replace(DEVICE, "ABCDEFAB-0000-4000-8000-000000000004"),
        format!("{{\"extra\":1,{}", &valid[1..]),
        format!("{{\"type\":\"device.revoked\",{}", &valid[1..]),
        format!("{{\"deviceId\":\"{DEVICE}\",{}", &valid[1..]),
        format!("{{\"name\":\"n\",{}", &valid[1..]),
        "{}".into(),
    ] {
        assert!(
            server
                .event(
                    "/.tmt/remote/device-events",
                    "tmt-device-event: 1\r\n",
                    &body
                )
                .starts_with("HTTP/1.1 400"),
            "{body}"
        );
    }
    for path in [
        "/.tmt/remote/device-events/",
        "/r/p/x/colab/.tmt/remote/device-events",
    ] {
        assert!(
            server
                .event(path, "tmt-device-event: 1\r\n", &valid)
                .starts_with("HTTP/1.1 404")
        );
    }
    for name in ["", "   ", "bad\nname", &"x".repeat(65)] {
        let body = json!({"type":"device.renamed","deviceId":DEVICE,"grantRevision":2,"name":name})
            .to_string();
        assert!(
            server
                .event(
                    "/.tmt/remote/device-events",
                    "tmt-device-event: 1\r\n",
                    &body
                )
                .starts_with("HTTP/1.1 400")
        );
    }
    for body in [
        json!({"type":"device.renamed","deviceId":DEVICE,"grantRevision":2}).to_string(),
        format!("{{\"grantRevision\":2,{}", &valid[1..]),
    ] {
        assert!(
            server
                .event(
                    "/.tmt/remote/device-events",
                    "tmt-device-event: 1\r\n",
                    &body
                )
                .starts_with("HTTP/1.1 400")
        );
    }
    let db = server.oracle();
    db.execute_batch("CREATE TRIGGER reject_rename BEFORE UPDATE ON device_registrations BEGIN SELECT RAISE(ABORT,'rename wrote'); END;").unwrap();
    let mut peer = server.peer(DEVICE);
    for _ in 0..2 {
        let rename =
            json!({"type":"device.renamed","deviceId":DEVICE,"grantRevision":9,"name":"New name"})
                .to_string();
        assert!(
            server
                .event(
                    "/.tmt/remote/device-events",
                    "tmt-device-event: 1\r\n",
                    &rename
                )
                .starts_with("HTTP/1.1 200")
        );
    }
    peer.send(Message::Ping(vec![4].into())).unwrap();
    assert!(matches!(peer.read().unwrap(), Message::Pong(_)));
    let revision: String = db
        .query_row(
            "SELECT grant_revision FROM device_registrations WHERE device_id=?",
            [DEVICE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, "00000000000000000001");
}

#[test]
fn mounted_sync_rechecks_epoch_revision_registered_key_and_live_owner_projection() {
    let server = Running::start(Tunnels::PRODUCT);
    let mut peer = server.peer(DEVICE);
    hello(&server, &mut peer, DEVICE);
    for (revision, epoch, bad_key, expected) in [
        ("2", "1", false, "DENIED"),
        ("1", "2", false, "STALE_EPOCH"),
        ("1", "1", true, "INVALID"),
    ] {
        let mut peer = server.peer(DEVICE);
        let context = object::Context {
            space: server.space.clone(),
            page: PAGE.into(),
            epoch: epoch.into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: DEVICE.into(),
            membership_revision: revision.into(),
            stream_seq: "1".into(),
            prev_hash: [0; 32],
        };
        let key = if bad_key {
            signing(OTHER)
        } else {
            signing(DEVICE)
        };
        let envelope = object::seal(&context, &[8; 32], &key, b"not admitted").unwrap();
        let mut frame = server.frame("append",json!({"streamId":DEVICE,"seq":"1","envelopeHash":values::encode_binary(&envelope.hash().unwrap()),"envelope":values::encode_binary(&envelope.to_json().unwrap())}));
        frame["epoch"] = json!(epoch);
        send(&mut peer, frame);
        let error = receive(&mut peer);
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], expected);
    }
    assert_eq!(
        server
            .oracle()
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let mut context: Value = serde_json::from_str(&context(DEVICE)).unwrap();
    context["deviceId"] = json!("00000000-0000-4000-8000-000000000006");
    let denied = server.request(&Running::get(
        "/sync",
        &format!("tmt-device-context: {context}\r\n{UPGRADE}"),
    ));
    assert!(denied.starts_with("HTTP/1.1 403"));
    // The durable issuer projection must be checked on silent delivery turns too.
    let db = server.oracle();
    let record: Vec<u8> = db
        .query_row(
            "SELECT record FROM recipients WHERE kind='member'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut recipient: tmt_colab::store::owner::Recipient =
        serde_json::from_slice(&record).unwrap();
    recipient.revoked = true;
    db.execute(
        "UPDATE recipients SET record=? WHERE kind='member'",
        [serde_json::to_vec(&recipient).unwrap()],
    )
    .unwrap();
    assert!(
        matches!(peer.read().unwrap(), Message::Close(_)),
        "revoked issuer did not close subscriber"
    );
}

#[test]
fn upgrade_read_ahead_reaches_sync_and_equal_revoke_has_no_tunnel_effect() {
    let server = Running::start(Tunnels::PRODUCT);
    let mut socket = server.connect();
    let hello = server
        .frame(
            "hello",
            json!({"membershipRevision":"0","device":DEVICE,"cursors":[]}),
        )
        .to_string()
        .into_bytes();
    assert!(hello.len() < 65536);
    let mut request =
        Running::get("/sync", &format!("{}\r\n{UPGRADE}", owner(DEVICE))).into_bytes();
    // Independently frame a masked text message following the HTTP head in one write.
    request.extend([0x81, 0x80 | 126]);
    request.extend((hello.len() as u16).to_be_bytes());
    let mask = [1, 2, 3, 4];
    request.extend(mask);
    request.extend(hello.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    socket.write_all(&request).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"));
    let mut peer = WebSocket::from_raw_socket(socket, Role::Client, None);
    assert_eq!(receive(&mut peer)["membershipHead"]["revision"], "1");
    assert_eq!(receive(&mut peer)["more"], false);
    server.oracle().execute_batch("CREATE TRIGGER reject_equal BEFORE UPDATE ON device_registrations BEGIN SELECT RAISE(ABORT,'equal wrote'); END;").unwrap();
    let equal = json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":1}).to_string();
    assert!(
        server
            .event(
                "/.tmt/remote/device-events",
                "tmt-device-event: 1\r\n",
                &equal
            )
            .starts_with("HTTP/1.1 200")
    );
    peer.send(Message::Ping(vec![5].into())).unwrap();
    assert!(matches!(peer.read().unwrap(), Message::Pong(_)));
}

fn rotation_rows(db: &rusqlite::Connection) -> Vec<i64> {
    [
        "membership_log",
        "epoch_secrets",
        "baselines",
        "wraps",
        "owner_operations",
    ]
    .iter()
    .map(|table| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
    .collect()
}

#[test]
fn owner_discovery_and_paged_log_bootstrap_use_exact_signed_bytes() {
    let server = Running::start(Tunnels::PRODUCT);
    for path in ["/api/session", "/api/pages"] {
        let denied = server.request(&Running::get(path, ""));
        assert!(denied.starts_with("HTTP/1.1 403"));
        assert_eq!(
            denied.split("\r\n\r\n").nth(1).unwrap(),
            "{\"code\":\"DENIED\"}"
        );
        let mut nonowner: Value = serde_json::from_str(&context(DEVICE)).unwrap();
        nonowner["owner"] = false.into();
        let denied = server.request(&Running::get(
            path,
            &format!("tmt-device-context: {nonowner}\r\n"),
        ));
        assert!(denied.starts_with("HTTP/1.1 403"));
    }
    let response = server.request(&Running::get(
        "/api/session",
        &format!("{}\r\n", owner(DEVICE)),
    ));
    let session: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let c: Value = serde_json::from_str(&context(DEVICE)).unwrap();
    assert_eq!(
        session,
        json!({"deviceId":DEVICE,"publicKey":c["publicKey"],"grantRevision":"1","name":c["name"]})
    );
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    let mut expected = Vec::new();
    for n in 2..=130 {
        let head = store
            .owner_head(&server.space, &key.owner_public())
            .unwrap()
            .unwrap();
        let body = serde_json::to_vec(
            &json!({"pageId":PAGE,"mode":if n==130 {"current"} else {"shared"}}),
        )
        .unwrap();
        let envelope = key
            .sign_statement(Some(&head), "page.history", &body)
            .unwrap();
        let bytes = envelope.to_json().unwrap();
        expected.push(bytes.clone());
        store
            .owner_transaction(
                &server.space,
                &key.owner_public(),
                tmt_colab::store::owner::Mutation {
                    operation_id: &format!("00000000-0000-4000-8000-{n:012}"),
                    digest: [n as u8; 32],
                    expected_revision: n - 1,
                },
                |tx| {
                    tx.append_statement(&envelope)?;
                    Ok(bytes)
                },
            )
            .unwrap();
    }
    let response = server.request(&Running::get(
        "/api/pages",
        &format!("{}\r\n", owner(DEVICE)),
    ));
    let pages: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        pages,
        json!({"spaceId":server.space,"ownerKey":values::encode_binary(&key.owner_public()),"revision":"130",
        "pages":[{"pageId":PAGE,"epoch":"1","sharing":"private","history":"current","archived":false}]})
    );
    let mut peer = server.peer(DEVICE);
    send(
        &mut peer,
        server.frame(
            "hello",
            json!({"device":DEVICE,"membershipRevision":"1","cursors":[]}),
        ),
    );
    let first = receive(&mut peer);
    assert_eq!(
        first["membershipHead"]["statements"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(first["membershipHead"]["more"], true);
    let mut received = first["membershipHead"]["statements"]
        .as_array()
        .unwrap()
        .clone();
    send(&mut peer, server.frame("ack", json!({"cursors":[]})));
    for count in [64, 1] {
        let frame = receive(&mut peer);
        assert_eq!(
            frame["membership"]["statements"].as_array().unwrap().len(),
            count
        );
        assert!(frame["streams"].as_array().unwrap().is_empty());
        received.extend(
            frame["membership"]["statements"]
                .as_array()
                .unwrap()
                .clone(),
        );
        send(&mut peer, server.frame("ack", json!({"cursors":[]})));
    }
    assert_eq!(
        received
            .iter()
            .map(|v| values::binary(v.as_str().unwrap(), 65536).unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(receive(&mut peer)["more"], false);
    // A hole is unknown history, never silently truncated or skipped.
    server
        .oracle()
        .execute(
            "DELETE FROM membership_log WHERE revision=?",
            [format!("{:020}", 100)],
        )
        .unwrap();
    let mut peer = server.peer(DEVICE);
    send(
        &mut peer,
        server.frame(
            "hello",
            json!({"device":DEVICE,"membershipRevision":"100","cursors":[]}),
        ),
    );
    assert_eq!(receive(&mut peer)["code"], "RESYNC_REQUIRED");
}

#[test]
fn retained_device_and_member_wraps_are_scoped_paged_and_ordered() {
    use tmt_colab_model::{crypto, wrap};
    let server = Running::start(Tunnels::PRODUCT);
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let member = key.management_member().unwrap();
    let mut store = Store::open(&layout).unwrap();
    let head = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    let change = key
        .sign_statement(
            Some(&head),
            "page.history",
            &serde_json::to_vec(&json!({"pageId":PAGE,"mode":"shared"})).unwrap(),
        )
        .unwrap();
    let genesis: Vec<u8> = server
        .oracle()
        .query_row(
            "SELECT envelope FROM membership_log WHERE revision=?",
            [format!("{:020}", 1)],
            |r| r.get(0),
        )
        .unwrap();
    let mut expected_statements = vec![
        values::encode_binary(&genesis),
        values::encode_binary(&change.to_json().unwrap()),
    ];
    let mut expected = Vec::new();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: OTHER,
                digest: [13; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&change)?;
                for epoch in 1..=65 {
                    tx.put_epoch_secret(PAGE, epoch, &[epoch as u8; 32])?;
                    if epoch > 1 {
                        let revision = tx.head().unwrap().revision + 1;
                        // This transport fixture admits signed policy, without materializing baselines.
                        let descriptor = json!({"pageId":PAGE,"epoch":epoch.to_string(),
                            "sourceDigest":values::encode_binary(&[1;32]),"baselineCommitment":values::encode_binary(&[2;32]),
                            "title":"","objectEnvelopeHash":values::encode_binary(&[3;32]),"membershipRevision":revision.to_string()});
                        let advanced = key.sign_statement(tx.head(), "epoch.advance",
                            &serde_json::to_vec(&json!({"pageId":PAGE,"epoch":epoch.to_string(),"cuts":[],
                                "baseline":descriptor,"wraps":[]}))?)?;
                        tx.append_statement(&advanced)?;
                        tx.advance_epoch(PAGE, epoch - 1)?;
                        expected_statements.push(values::encode_binary(&advanced.to_json()?));
                    }
                    for (kind, id) in [
                        ("device", DEVICE),
                        ("device", OTHER),
                        ("member", member.id.as_str()),
                    ] {
                        let envelope = key.seal_wrap(
                            &wrap::Header {
                                space: server.space.clone(),
                                page: PAGE.into(),
                                epoch: epoch.to_string(),
                                recipient_kind: kind.into(),
                                recipient_id: id.into(),
                                recipient_key: member.encryption_key,
                                signer_key: key.owner_public(),
                                membership_revision: tx.head().unwrap().revision.to_string(),
                            },
                            &[epoch as u8; 32],
                        )?;
                        tx.put_wrap(&envelope)?;
                        if epoch >= 2 && id != OTHER {
                            expected.push(values::encode_binary(&envelope.to_json()?));
                        }
                    }
                }
                Ok(Vec::new())
            },
        )
        .unwrap();
    let mut peer = server.peer(DEVICE);
    let mut hello = server.frame(
        "hello",
        json!({"device":DEVICE,"membershipRevision":"0","cursors":[]}),
    );
    hello["epoch"] = "65".into();
    send(&mut peer, hello);
    let first = receive(&mut peer);
    let mut statements = first["membershipHead"]["statements"]
        .as_array()
        .unwrap()
        .clone();
    let ack = |peer: &mut WebSocket<UnixStream>| {
        let mut frame = server.frame("ack", json!({"cursors":[]}));
        frame["epoch"] = "65".into();
        send(peer, frame);
    };
    ack(&mut peer);
    let mut actual = Vec::new();
    let mut count = 0;
    loop {
        let page = receive(&mut peer);
        assert!(page["streams"].as_array().unwrap().is_empty());
        if let Some(membership) = page.get("membership") {
            statements.extend(membership["statements"].as_array().unwrap().clone());
            ack(&mut peer);
            continue;
        }
        let wraps = page["wraps"].as_array().unwrap();
        assert!(wraps.len() <= 512);
        assert!(page.to_string().len() <= 65536);
        actual.extend(wraps.iter().map(|v| v.as_str().unwrap().to_owned()));
        count += 1;
        ack(&mut peer);
        if page["more"] == false {
            break;
        }
    }
    assert!(count > 1);
    assert_eq!(actual, expected);
    assert_eq!(
        statements,
        expected_statements
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>()
    );
    let root = values::binary(first["membershipHead"]["ownerKey"].as_str().unwrap(), 32).unwrap();
    assert_eq!(
        crypto::space_id(root.as_slice().try_into().unwrap()).unwrap(),
        server.space
    );
}
#[test]
fn revoked_author_chain_is_verifiable_but_signed_log_denies_its_objects() {
    use tmt_colab_model::{certificate, payload, statement};
    let server = Running::start(Tunnels::PRODUCT);
    let mut writer = server.peer(DEVICE);
    hello(&server, &mut writer, DEVICE);
    let (frame, _, _) = server.append(DEVICE, 1, [0; 32]);
    send(&mut writer, frame);
    receive(&mut writer);
    receive(&mut writer);
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    let head = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    let revoke = key
        .sign_statement(
            Some(&head),
            "device.revoke",
            &serde_json::to_vec(&json!({"deviceId":DEVICE,"cuts":[]})).unwrap(),
        )
        .unwrap();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: OTHER,
                digest: [15; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&revoke)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let mut reader = server.peer(OTHER);
    let frames = hello(&server, &mut reader, OTHER);
    let mut verified = None;
    let mut revoked = Vec::new();
    let mut issuer = None;
    for raw in frames[0]["membershipHead"]["statements"]
        .as_array()
        .unwrap()
    {
        let bytes = values::binary(raw.as_str().unwrap(), 65536).unwrap();
        let envelope = statement::Envelope::from_json(&bytes).unwrap();
        let next = envelope
            .verify_next(&server.space, &key.owner_public(), verified.as_ref())
            .unwrap();
        if next.head.revision == 1 {
            issuer = Some(next.head.clone());
        }
        if let payload::Payload::DeviceRevoke(p) = next.payload {
            revoked.push(p.device_id);
        }
        verified = Some(next.head);
    }
    let page = frames
        .iter()
        .find(|p| p["streams"].as_array().is_some_and(|v| !v.is_empty()))
        .unwrap();
    let bytes = values::binary(page["chains"][0]["chain"].as_str().unwrap(), 16384).unwrap();
    let chain = certificate::Chain::from_json(&bytes).unwrap();
    let cert = chain.certificate().unwrap();
    let issuer = issuer.unwrap();
    chain
        .verify(&issuer.hash, &cert, &issuer.owner_member.signing_key)
        .unwrap();
    let entry = &page["streams"][0]["tail"][0];
    let object = object::Envelope::from_json(
        &values::binary(entry["envelope"].as_str().unwrap(), 65536).unwrap(),
    )
    .unwrap();
    tmt_colab_model::crypto::verify_signature(
        cert.signing_key,
        &object.signature_input().unwrap(),
        object.signature(),
    )
    .unwrap();
    // Positive chain/signature control passes; current verified revocation alone
    // denies this author, exactly the consumer's admission requirement.
    assert_eq!(cert.device_id, DEVICE);
    assert!(revoked.iter().any(|id| id == cert.device_id));
}

#[test]
fn large_membership_statement_bootstraps_with_exact_chunks_and_frame_credit() {
    let server = Running::start(Tunnels::PRODUCT);
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    let head = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    let descriptor = json!({"pageId":PAGE,"epoch":"2","sourceDigest":values::encode_binary(&[1;32]),"baselineCommitment":values::encode_binary(&[2;32]),
        "title":"x".repeat(250*1024),"objectEnvelopeHash":values::encode_binary(&[3;32]),"membershipRevision":"2"});
    let statement = key
        .sign_statement(
            Some(&head),
            "epoch.advance",
            &serde_json::to_vec(
                &json!({"pageId":PAGE,"epoch":"2","cuts":[],"baseline":descriptor,"wraps":[]}),
            )
            .unwrap(),
        )
        .unwrap();
    let bytes = statement.to_json().unwrap();
    assert!(bytes.len() > 64 * 1024);
    assert!(bytes.len().div_ceil(limits::CHUNK_BYTES) > limits::SEND_QUEUE_FRAMES);
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: OTHER,
                digest: [14; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&statement)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let second_head = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    let successor = key
        .sign_statement(
            Some(&second_head),
            "page.history",
            &serde_json::to_vec(&json!({"pageId":PAGE,"mode":"current"})).unwrap(),
        )
        .unwrap();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: "00000000-0000-4000-8000-000000000006",
                digest: [15; 32],
                expected_revision: 2,
            },
            |tx| {
                tx.append_statement(&successor)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let genesis_bytes: Vec<u8> = server
        .oracle()
        .query_row(
            "SELECT envelope FROM membership_log WHERE revision=?",
            [format!("{:020}", 1)],
            |r| r.get(0),
        )
        .unwrap();
    let genesis = tmt_colab_model::statement::Envelope::from_json(&genesis_bytes).unwrap();
    let initial = genesis
        .verify_next(&server.space, &key.owner_public(), None)
        .unwrap()
        .head;
    // Fresh and resumed clients exercise a reference in later and first pages.
    for revision in ["0", "1"] {
        let mut peer = server.peer(DEVICE);
        send(
            &mut peer,
            server.frame(
                "hello",
                json!({"device":DEVICE,"membershipRevision":revision,"cursors":[]}),
            ),
        );
        let first = receive(&mut peer);
        assert_eq!(first["membershipHead"]["revision"], "3");
        assert_eq!(
            first["membershipHead"]["statementHash"],
            values::encode_binary(&successor.hash().unwrap())
        );
        let reference = if revision == "0" {
            assert_eq!(
                first["membershipHead"]["statements"],
                json!([values::encode_binary(&genesis.to_json().unwrap())])
            );
            receive(&mut peer)
        } else {
            first
        };
        let membership = if revision == "0" {
            &reference["membership"]
        } else {
            &reference["membershipHead"]
        };
        let hash = values::encode_binary(&statement.hash().unwrap());
        assert_eq!(membership["statements"], json!([{"statementHash":hash}]));
        assert_eq!(membership["more"], true);
        let count = bytes.len().div_ceil(limits::CHUNK_BYTES);
        let mut joined = Vec::new();
        let mut outstanding = if revision == "0" { 2 } else { 1 };
        for index in 0..count {
            let chunk = receive(&mut peer);
            assert_eq!(chunk["type"], "chunk");
            assert_eq!(chunk.as_object().unwrap().len(), 9);
            assert_eq!(chunk["space"], server.space);
            assert_eq!(chunk["page"], PAGE);
            assert_eq!(chunk["epoch"], "1");
            assert_eq!(chunk["statementHash"], hash);
            assert_eq!(chunk["index"], index);
            assert_eq!(chunk["count"], count);
            let part =
                values::binary(chunk["bytes"].as_str().unwrap(), limits::CHUNK_BYTES).unwrap();
            assert!(!part.is_empty());
            if index + 1 < count {
                assert_eq!(part.len(), limits::CHUNK_BYTES);
            }
            joined.extend(part);
            outstanding += 1;
            if outstanding == limits::SEND_QUEUE_FRAMES {
                peer.get_mut()
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                assert!(
                    matches!(peer.read(), Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
                );
                peer.get_mut()
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                for _ in 0..outstanding {
                    send(&mut peer, server.frame("ack", json!({"cursors":[]})));
                }
                outstanding = 0;
            }
        }
        assert_eq!(joined, bytes);
        let decoded = tmt_colab_model::statement::Envelope::from_json(&joined).unwrap();
        assert_eq!(decoded.hash().unwrap(), statement.hash().unwrap());
        let verified = decoded
            .verify_next(&server.space, &key.owner_public(), Some(&initial))
            .unwrap();
        assert_eq!(verified.head, second_head);
        for _ in 0..outstanding {
            send(&mut peer, server.frame("ack", json!({"cursors":[]})));
        }
        let next = receive(&mut peer);
        assert_eq!(
            next["membership"]["statements"],
            json!([values::encode_binary(&successor.to_json().unwrap())])
        );
        assert_eq!(next["membership"]["more"], false);
        send(&mut peer, server.frame("ack", json!({"cursors":[]})));
        assert_eq!(receive(&mut peer)["more"], false);
    }
    let stored: Vec<u8> = server
        .oracle()
        .query_row(
            "SELECT envelope FROM membership_log WHERE revision=?",
            [format!("{:020}", 2)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, bytes);
}

#[test]
fn mounted_owner_rejects_corrupt_oversized_membership_log() {
    let server = Running::start(Tunnels::PRODUCT);
    // Owner admission verifies page policy before transport reads the corrupted log.
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    let head = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap();
    let statement = key
        .sign_statement(
            Some(&head),
            "page.history",
            &serde_json::to_vec(&json!({"pageId":PAGE,"mode":"current"})).unwrap(),
        )
        .unwrap();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: OTHER,
                digest: [16; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&statement)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    server
        .oracle()
        .execute(
            "UPDATE membership_log SET envelope=zeroblob(?) WHERE revision=?",
            rusqlite::params![(limits::STATEMENT_BYTES + 1) as i64, format!("{:020}", 2)],
        )
        .unwrap();
    let mut peer = server.peer(DEVICE);
    send(
        &mut peer,
        server.frame(
            "hello",
            json!({"device":DEVICE,"membershipRevision":"0","cursors":[]}),
        ),
    );
    assert_eq!(receive(&mut peer)["code"], "DENIED");
    let size: i64 = server
        .oracle()
        .query_row(
            "SELECT length(envelope) FROM membership_log WHERE revision=?",
            [format!("{:020}", 2)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(size, (limits::STATEMENT_BYTES + 1) as i64);
}

#[test]
fn owner_static_assets_have_exact_bytes_types_and_no_filesystem_path_resolution() {
    use tmt_colab::assets::{App, POLICY, RENDERER_POLICY};
    let directory = PathBuf::from(format!("/tmp/tmt-1253-build-{}", std::process::id()));
    fs::create_dir_all(directory.join("assets")).unwrap();
    assert!(!POLICY.contains("unsafe-inline"));
    assert!(RENDERER_POLICY.ends_with("sandbox allow-scripts"));
    let files: [(&str, &str, &[u8]); 12] = [
        ("index.html", "text/html; charset=utf-8", br#"<link href="./assets/app.css"><script type="module" src="./assets/app.js"></script>"#),
        ("renderer.html", "text/html; charset=utf-8", b"<!doctype html><title>Renderer</title>"),
        ("reader.html", "text/html; charset=utf-8", br#"<script type="module" src="./assets/reader.js"></script>"#),
        ("assets/reader.js", "text/javascript; charset=utf-8", b"export const reader = true;"),
        ("assets/reader.css", "text/css; charset=utf-8", b"body{color:blue}"),
        ("assets/reader-fold.js", "text/javascript; charset=utf-8", b"export const fold = true;"),
        ("assets/app.js", "text/javascript; charset=utf-8", b"export {};"),
        ("assets/recovery.js", "text/javascript; charset=utf-8", b"export const recovery = true;"),
        ("assets/app.css", "text/css; charset=utf-8", b"body{color:red}"),
        ("assets/font.woff2", "font/woff2", b"wOF2\0\xfffont-test-bytes"),
        ("assets/font.woff", "font/woff", b"wOFF\0\xfffont-test-bytes"),
        ("THIRD-PARTY-NOTICES.txt", "text/plain; charset=utf-8", b"test-only notice"),
    ];
    for (name, _, bytes) in files {
        fs::write(directory.join(name), bytes).unwrap();
    }
    let app = App::load(&directory).unwrap();
    fs::remove_dir_all(&directory).unwrap(); // No request-time file reads are possible.
    let server = Running::start_with_app(Tunnels::PRODUCT, Some(app));
    // The browser must load before Colab registration. This forwarded owner is
    // deliberately absent from the active device registrations above.
    let unregistered = "00000000-0000-4000-8000-000000000006";
    let headers = format!("{}\r\n", owner(unregistered));
    for (name, content_type, bytes) in files {
        let path = if name == "index.html" {
            "/".into()
        } else {
            format!("/{name}")
        };
        let mut socket = server.connect();
        socket
            .write_all(Running::get(&path, &headers).as_bytes())
            .unwrap();
        let mut reply = Vec::new();
        socket.read_to_end(&mut reply).unwrap();
        let end = reply.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        let head = std::str::from_utf8(&reply[..end]).unwrap();
        assert!(head.starts_with("HTTP/1.1 200"));
        assert!(head.contains(&format!("Content-Type: {content_type}\r\n")));
        assert!(head.contains(&format!("Content-Length: {}\r\n", bytes.len())));
        let policy = if name == "renderer.html" {
            RENDERER_POLICY
        } else {
            POLICY
        };
        assert!(head.contains(&format!("Content-Security-Policy: {policy}\r\n")));
        assert_eq!(&reply[end..], bytes);
        if tmt_colab::assets::anonymous_file(&path).is_some() {
            // The reader entry and its files are public static bytes.
            let public = server.request(&Running::get(&path, ""));
            assert!(public.starts_with("HTTP/1.1 200"), "{path}");
            assert!(public.ends_with(std::str::from_utf8(bytes).unwrap()));
            assert!(public.contains(&format!("Content-Security-Policy: {policy}\r\n")));
        } else if path != "/" {
            assert!(
                server
                    .request(&Running::get(&path, ""))
                    .starts_with("HTTP/1.1 403")
            );
        }
    }
    let guidance = server.request(&Running::get("/", ""));
    assert!(guidance.contains("This colab space is private"));
    assert!(guidance.contains("id=\"colab-guidance\" hidden"));
    assert!(guidance.contains("<script type=\"module\" src=\"./assets/recovery.js\"></script>"));
    assert!(guidance.contains(&format!("Content-Security-Policy: {POLICY}\r\n")));
    assert!(!guidance.contains("unsafe-inline"));
    let read = server.request(&Running::get("/read", ""));
    assert!(read.starts_with("HTTP/1.1 200"));
    assert!(read.contains("Content-Type: text/html; charset=utf-8\r\n"));
    assert!(read.contains(&format!("Content-Security-Policy: {POLICY}\r\n")));
    assert!(read.ends_with(r#"<script type="module" src="./assets/reader.js"></script>"#));
    for path in ["/read/", "/read?x=1", "/reader.html"] {
        let reply = server.request(&Running::get(path, ""));
        assert!(!reply.starts_with("HTTP/1.1 200"), "{path}");
    }
    for path in [
        "/../index.html",
        "/assets/../index.html",
        "/assets/./app.js",
        "/assets/%2e%2e/index.html",
        "/assets/app.js?x=1",
        "/assets\\app.js",
        "//assets/app.js",
    ] {
        assert!(
            server
                .request(&Running::get(path, &headers))
                .starts_with("HTTP/1.1 400"),
            "{path}"
        );
    }
    assert!(
        server
            .request(&Running::get("/sync", &format!("{headers}{UPGRADE}")))
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        server
            .request(&Running::get("/assets/missing.js", &headers))
            .starts_with("HTTP/1.1 404")
    );
    // Existing discovery dispatch keeps its JSON policy with an app loaded,
    // and still admits the forwarded owner before extension-key registration.
    for path in ["/api/session", "/api/pages"] {
        let reply = server.request(&Running::get(path, &headers));
        let (head, body) = reply.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200"));
        assert!(head.contains("Content-Type: application/json"));
        assert!(!head.contains("unsafe-inline"));
        let body: Value = serde_json::from_str(body).unwrap();
        if path == "/api/session" {
            assert_eq!(body["deviceId"], unregistered);
            assert_eq!(body["grantRevision"], "1");
        } else {
            assert_eq!(body["spaceId"], server.space);
            assert_eq!(body["pages"][0]["pageId"], PAGE);
        }
        assert!(
            server
                .request(&Running::get(path, ""))
                .starts_with("HTTP/1.1 403")
        );
    }
    assert!(
        server
            .request("POST /assets/app.js HTTP/1.1\r\nHost: x\r\n\r\n")
            .starts_with("HTTP/1.1 403")
    );
    assert!(
        server
            .request(&format!("POST /assets/app.js HTTP/1.1\r\n{headers}\r\n"))
            .starts_with("HTTP/1.1 404")
    );
}

fn management_body(
    server: &Running,
    id: &str,
    revision: u64,
    operation: &str,
    payload: Value,
    sender: &str,
    issued: u64,
) -> String {
    let payload = serde_json::to_vec(&payload).unwrap();
    let request = tmt_colab_model::auth::management_input(&tmt_colab_model::auth::Management {
        space: &server.space,
        page: PAGE,
        expected_revision: &revision.to_string(),
        operation_id: id,
        operation,
        payload: &payload,
        sender_device: sender,
        issued_at: issued,
        expires_at: issued + 60_000,
    })
    .unwrap();
    json!({"request":values::encode_binary(&request),"payload":values::encode_binary(&payload),
        "signature":values::encode_binary(&signing(sender).sign(&request).to_bytes())})
    .to_string()
}
fn local_management(
    server: &Running,
    id: &str,
    revision: u64,
    operation: &str,
    payload: Value,
) -> String {
    json!({"space":server.space,"page":PAGE,"expectedRevision":revision.to_string(),"operationId":id,
        "operation":operation,"payload":values::encode_binary(&serde_json::to_vec(&payload).unwrap())}).to_string()
}
fn management_member(id: &str, seed: u8) -> Value {
    json!({"memberId":id,"role":"viewer","pages":[PAGE],
        "signKey":values::encode_binary(SigningKey::from_bytes(&[seed;32]).verifying_key().as_bytes()),
        "encKey":values::encode_binary(&tmt_colab_model::wrap::RecipientKey::from_seed(&[seed+1;32]).unwrap().public_key())})
}
fn management_recipient(
    server: &Running,
    kind: &str,
    id: &str,
) -> tmt_colab::store::owner::Recipient {
    let bytes: Vec<u8> = server
        .oracle()
        .query_row(
            "SELECT record FROM recipients WHERE kind=? AND id=?",
            [kind, id],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn management_response(server: &Running, raw: &str, revision: &str) -> Value {
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert_eq!(body["membershipHead"]["revision"], revision);
    assert_eq!(
        values::binary(
            body["membershipHead"]["statementHash"].as_str().unwrap(),
            32
        )
        .unwrap()
        .len(),
        32
    );
    let key = Keyring::read(&Layout::existing(&server.root).unwrap().unwrap()).unwrap();
    let db = server.oracle();
    let mut log = db
        .prepare("SELECT envelope FROM membership_log ORDER BY revision")
        .unwrap();
    let mut head = None;
    for bytes in log.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap() {
        let envelope = tmt_colab_model::statement::Envelope::from_json(&bytes.unwrap()).unwrap();
        head = Some(
            envelope
                .verify_next(&server.space, &key.owner_public(), head.as_ref())
                .unwrap()
                .head,
        );
        if head.as_ref().unwrap().revision.to_string() == revision {
            break;
        }
    }
    let head = head.unwrap();
    assert_eq!(head.revision.to_string(), revision);
    assert_eq!(
        values::encode_binary(&head.hash),
        body["membershipHead"]["statementHash"]
    );
    body
}
#[test]
fn management_owner_admission_exact_replay_local_ipc_and_epoch_close_twice() {
    const OP: &str = "40000000-0000-4000-8000-000000000001";
    const LOCAL: &str = "40000000-0000-4000-8000-000000000002";
    const ADVANCE: &str = "40000000-0000-4000-8000-000000000003";
    const STALE: &str = "40000000-0000-4000-8000-000000000004";
    let path = tmt_colab::management::PATH;
    let ipc = tmt_colab::management::LOCAL_PATH;
    for _ in 0..2 {
        let server = Running::start(Tunnels::PRODUCT);
        server
            .oracle()
            .execute(
                "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?, '00000000000000000001', ?)",
                rusqlite::params![PAGE, [8u8; 32].as_slice()],
            )
            .unwrap();
        let issued = now();
        let payload = management_member("60000000-0000-4000-8000-000000000001", 20);
        let body = management_body(
            &server,
            OP,
            1,
            "member.add",
            payload.clone(),
            DEVICE,
            issued,
        );
        let header = format!("{}\r\n", owner(DEVICE));
        let denied = server.event(path, "", &body);
        assert!(
            denied.starts_with("HTTP/1.1 403") && denied.ends_with("DENIED"),
            "{denied}"
        );
        let wrong_device = server.event(path, &format!("{}\r\n", owner(OTHER)), &body);
        assert!(
            wrong_device.starts_with("HTTP/1.1 403") && wrong_device.ends_with("DENIED"),
            "{wrong_device}"
        );
        let mut non_owner: Value = serde_json::from_str(&context(DEVICE)).unwrap();
        non_owner["owner"] = false.into();
        let non_owner_header = format!("tmt-device-context: {non_owner}\r\n");
        assert!(
            server
                .event(path, &non_owner_header, &body)
                .ends_with("DENIED")
        );
        let expired = management_body(
            &server,
            OP,
            1,
            "member.add",
            payload.clone(),
            DEVICE,
            issued - 120_000,
        );
        assert!(server.event(path, &header, &expired).ends_with("EXPIRED"));
        let mut bad_signature: Value = serde_json::from_str(&body).unwrap();
        bad_signature["signature"] = values::encode_binary(&[0; 64]).into();
        assert!(
            server
                .event(path, &header, &bad_signature.to_string())
                .ends_with("DENIED")
        );
        let computed = management_body(
            &server,
            OP,
            1,
            "member.add",
            json!({"pageId":PAGE,"cuts":[]}),
            DEVICE,
            issued,
        );
        assert!(server.event(path, &header, &computed).ends_with("INVALID"));
        let rows = |table| {
            server
                .oracle()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        };
        assert_eq!(rows("membership_log"), 1);
        assert_eq!(rows("owner_operations"), 1);
        let response = server.event(path, &header, &body);
        let result = management_response(&server, &response, "2");
        assert_eq!(result["operationId"], OP);
        let added = management_recipient(&server, "member", "60000000-0000-4000-8000-000000000001");
        assert_eq!(added.role.as_deref(), Some("viewer"));
        assert_eq!(added.pages, [PAGE]);
        assert!(!added.revoked);
        assert_eq!(rows("membership_log"), 2);
        assert_eq!(server.event(path, &header, &body), response);
        assert_eq!(rows("owner_operations"), 2);
        let changed = management_body(
            &server,
            OP,
            1,
            "member.add",
            payload.clone(),
            DEVICE,
            issued - 1,
        );
        assert!(server.event(path, &header, &changed).ends_with("CONFLICT"));
        let stale = management_body(
            &server,
            STALE,
            1,
            "member.add",
            management_member("60000000-0000-4000-8000-000000000003", 24),
            DEVICE,
            issued,
        );
        let stale_response = server.event(path, &header, &stale);
        assert!(stale_response.ends_with("STALE_HEAD"), "{stale_response}");
        assert_eq!(rows("membership_log"), 2);
        let local = local_management(
            &server,
            LOCAL,
            2,
            "member.add",
            management_member("60000000-0000-4000-8000-000000000002", 22),
        );
        // The reserved socket route is root-authorized even without a Remote device.
        let local_added = server.event(ipc, "", &local);
        management_response(&server, &local_added, "3");
        assert!(
            server
                .event(ipc, &non_owner_header, &local)
                .ends_with("DENIED")
        );
        assert_eq!(
            server.event(path, &header, &body),
            response,
            "replay must retain its original head after later commits"
        );
        assert!(
            server
                .request(&Running::get(ipc, ""))
                .starts_with("HTTP/1.1 400")
        );
        assert!(server.event(ipc, "", &body).ends_with("INVALID"));
        let mut peer = server.peer(DEVICE);
        hello(&server, &mut peer, DEVICE);
        let advance =
            local_management(&server, ADVANCE, 3, "epoch.advance", json!({"pageId":PAGE}));
        management_response(&server, &server.event(ipc, "", &advance), "4");
        match peer.read() {
            Ok(Message::Close(Some(close))) => assert_eq!(close.reason, "STALE_EPOCH"),
            other => panic!("epoch advance did not close the subscribed peer: {other:?}"),
        }
        let role = management_body(
            &server,
            "40000000-0000-4000-8000-000000000005",
            4,
            "member.role",
            json!({"memberId":"60000000-0000-4000-8000-000000000001","role":"commenter","pages":[PAGE]}),
            DEVICE,
            now(),
        );
        management_response(&server, &server.event(path, &header, &role), "5");
        assert_eq!(
            management_recipient(&server, "member", "60000000-0000-4000-8000-000000000001")
                .role
                .as_deref(),
            Some("commenter")
        );
        let remove = management_body(
            &server,
            "40000000-0000-4000-8000-000000000006",
            5,
            "member.remove",
            json!({"memberId":"60000000-0000-4000-8000-000000000001","pages":[PAGE]}),
            DEVICE,
            now(),
        );
        management_response(&server, &server.event(path, &header, &remove), "7");
        assert!(
            management_recipient(&server, "member", "60000000-0000-4000-8000-000000000001").revoked
        );
        assert_eq!(rows("membership_log"), 7);
        assert_eq!(rows("owner_operations"), 6);
        server
            .oracle()
            .execute(
                "UPDATE device_registrations SET revoked=1 WHERE device_id=?",
                [DEVICE],
            )
            .unwrap();
        let revoked = server.event(path, &header, &body);
        assert!(revoked.starts_with("HTTP/1.1 403") && revoked.ends_with("DENIED"));
        assert_eq!(rows("owner_operations"), 6);
        drop(peer);
        let socket_path = server.path.clone();
        drop(server);
        assert!(!socket_path.exists());
    }
}
#[test]
fn management_receipt_failure_rolls_back_signed_membership_and_remains_retryable() {
    const OP: &str = "50000000-0000-4000-8000-000000000001";
    let server = Running::start(Tunnels::PRODUCT);
    server
        .oracle()
        .execute(
            "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?, '00000000000000000001', ?)",
            rusqlite::params![PAGE, [8u8; 32].as_slice()],
        )
        .unwrap();
    server.oracle().execute_batch(&format!("CREATE TRIGGER reject_management BEFORE INSERT ON owner_operations WHEN NEW.id='{OP}' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;")).unwrap();
    let body = management_body(
        &server,
        OP,
        1,
        "member.add",
        management_member("60000000-0000-4000-8000-000000000001", 20),
        DEVICE,
        now(),
    );
    let header = format!("{}\r\n", owner(DEVICE));
    let failed = server.event(tmt_colab::management::PATH, &header, &body);
    assert!(
        failed.starts_with("HTTP/1.1 503") && failed.ends_with("UNAVAILABLE"),
        "{failed}"
    );
    assert_eq!(
        server
            .oracle()
            .query_row("SELECT count(*) FROM membership_log", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        server
            .oracle()
            .query_row("SELECT count(*) FROM owner_operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    server
        .oracle()
        .execute_batch("DROP TRIGGER reject_management")
        .unwrap();
    management_response(
        &server,
        &server.event(tmt_colab::management::PATH, &header, &body),
        "2",
    );
}

#[test]
fn management_link_add_reset_remove_fence_scope_replay_and_seed_disclosure() {
    const OLD: &str = "70000000-0000-4000-8000-000000000001";
    const NEW: &str = "70000000-0000-4000-8000-000000000002";
    const ADD: &str = "80000000-0000-4000-8000-000000000001";
    const RESET: &str = "80000000-0000-4000-8000-000000000002";
    const REMOVE: &str = "80000000-0000-4000-8000-000000000003";
    const BAD_SCOPE: &str = "80000000-0000-4000-8000-000000000004";
    let server = Running::start(Tunnels::PRODUCT);
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    // Fixture setup establishes an owner-signed link policy and initial key;
    // all tested management changes go through the mounted socket and runner.
    let mut store = Store::open(&layout).unwrap();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: "90000000-0000-4000-8000-000000000001",
                digest: [19; 32],
                expected_revision: 1,
            },
            |tx| {
                let payload =
                    serde_json::to_vec(&json!({"pageId":PAGE,"mode":"link","epoch":"1"}))?;
                tx.append_statement(&key.sign_statement(tx.head(), "page.share", &payload)?)?;
                tx.put_epoch_secret(PAGE, 1, &[8; 32])?;
                Ok(b"fixture link policy".to_vec())
            },
        )
        .unwrap();
    store.close().unwrap();
    let header = format!("{}\r\n", owner(DEVICE));
    let link = |id, seed| json!({"linkId":id,"role":"viewer","pages":[PAGE],"seed":values::encode_binary(&[seed;32])});
    let old_seed = values::encode_binary(&[31; 32]);
    let new_seed = values::encode_binary(&[33; 32]);
    let add = management_body(&server, ADD, 2, "link.add", link(OLD, 31), DEVICE, now());
    management_response(
        &server,
        &server.event(tmt_colab::management::PATH, &header, &add),
        "3",
    );
    assert!(!management_recipient(&server, "link", OLD).revoked);
    let bad = management_body(
        &server,
        BAD_SCOPE,
        3,
        "link.remove",
        json!({"linkId":OLD,"pages":[PAGE,OTHER],"replacement":null}),
        DEVICE,
        now(),
    );
    assert!(
        server
            .event(tmt_colab::management::PATH, &header, &bad)
            .ends_with("STALE_HEAD")
    );
    let mut peer = server.peer(DEVICE);
    hello(&server, &mut peer, DEVICE);
    let reset_payload = json!({"linkId":OLD,"pages":[PAGE],"replacement":link(NEW,33)});
    let reset = management_body(
        &server,
        RESET,
        3,
        "link.remove",
        reset_payload.clone(),
        DEVICE,
        now(),
    );
    let response = server.event(tmt_colab::management::PATH, &header, &reset);
    management_response(&server, &response, "6");
    assert!(management_recipient(&server, "link", OLD).revoked);
    assert!(!management_recipient(&server, "link", NEW).revoked);
    match peer.read() {
        Ok(Message::Close(Some(close))) => assert_eq!(close.reason, "STALE_EPOCH"),
        other => panic!("Reset left a stale subscription alive: {other:?}"),
    }
    assert_eq!(
        server.event(tmt_colab::management::PATH, &header, &reset),
        response
    );
    let mut changed = reset_payload;
    changed["replacement"]["seed"] = values::encode_binary(&[34; 32]).into();
    let changed = management_body(&server, RESET, 3, "link.remove", changed, DEVICE, now());
    assert!(
        server
            .event(tmt_colab::management::PATH, &header, &changed)
            .ends_with("CONFLICT")
    );
    let remove = management_body(
        &server,
        REMOVE,
        6,
        "link.remove",
        json!({"linkId":NEW,"pages":[PAGE],"replacement":null}),
        DEVICE,
        now(),
    );
    management_response(
        &server,
        &server.event(tmt_colab::management::PATH, &header, &remove),
        "8",
    );
    assert!(management_recipient(&server, "link", NEW).revoked);
    assert_eq!(
        server
            .oracle()
            .query_row("SELECT count(*) FROM membership_log", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
    let db = server.oracle();
    let mut rows = db
        .prepare(
            "SELECT outcome FROM owner_operations UNION ALL SELECT envelope FROM membership_log",
        )
        .unwrap();
    for bytes in rows.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap() {
        let bytes = bytes.unwrap();
        for seed in [
            old_seed.as_bytes(),
            new_seed.as_bytes(),
            [31u8; 32].as_slice(),
            [33u8; 32].as_slice(),
        ] {
            assert!(
                !bytes.windows(seed.len()).any(|part| part == seed),
                "seed leaked into durable public output"
            );
        }
    }
    assert!(!response.contains(&old_seed) && !response.contains(&new_seed));
}
#[test]
fn management_page_policy_lifecycle_keeps_archived_reads_and_closes_deleted_peers() {
    let server = Running::start(Tunnels::PRODUCT);
    server
        .oracle()
        .execute(
            "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?, '00000000000000000001', ?)",
            rusqlite::params![PAGE, [8u8; 32].as_slice()],
        )
        .unwrap();
    let path = tmt_colab::management::PATH;
    let header = format!("{}\r\n", owner(DEVICE));
    for (index, (operation, payload)) in [
        ("page.share", json!({"pageId":PAGE,"mode":"link"})),
        ("page.history", json!({"pageId":PAGE,"mode":"current"})),
        ("retention.set", json!({"pageId":PAGE,"days":30})),
    ]
    .into_iter()
    .enumerate()
    {
        let revision = index as u64 + 1;
        let body = management_body(
            &server,
            &format!("90000000-0000-4000-8000-{revision:012}"),
            revision,
            operation,
            payload,
            DEVICE,
            now(),
        );
        management_response(
            &server,
            &server.event(path, &header, &body),
            &(revision + 1).to_string(),
        );
    }
    let mut peer = server.peer(DEVICE);
    hello(&server, &mut peer, DEVICE);
    let archive = management_body(
        &server,
        "90000000-0000-4000-8000-000000000004",
        4,
        "page.archive",
        json!({"pageId":PAGE}),
        DEVICE,
        now(),
    );
    let archived = server.event(path, &header, &archive);
    management_response(&server, &archived, "5");
    assert_eq!(server.event(path, &header, &archive), archived);
    peer.send(Message::Ping(vec![4].into())).unwrap();
    assert!(
        matches!(peer.read().unwrap(), Message::Pong(_)),
        "archive preserves read access"
    );
    let page_list = server.request(&Running::get("/api/pages", &header));
    let pages: Value = serde_json::from_str(page_list.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(pages["pages"][0]["archived"], true);
    assert_eq!(pages["pages"][0]["sharing"], "link");
    assert_eq!(pages["pages"][0]["history"], "current");
    let delete = local_management(
        &server,
        "90000000-0000-4000-8000-000000000005",
        5,
        "page.delete",
        json!({"pageId":PAGE}),
    );
    let deleted = server.event(tmt_colab::management::LOCAL_PATH, "", &delete);
    management_response(&server, &deleted, "6");
    match peer.read() {
        Ok(Message::Close(Some(close))) => assert_eq!(close.reason, "DENIED"),
        other => panic!("delete left a subscribed peer alive: {other:?}"),
    }
    assert_eq!(
        server.event(tmt_colab::management::LOCAL_PATH, "", &delete),
        deleted
    );
    assert_eq!(
        server.event(path, &header, &archive),
        archived,
        "replay retains its committed head after deletion"
    );
    let db = server.oracle();
    for (table, expected) in [
        ("membership_log", 6),
        ("owner_operations", 6),
        ("epoch_secrets", 0),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected);
    }
    let page_list = server.request(&Running::get("/api/pages", &header));
    let pages: Value = serde_json::from_str(page_list.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(pages["pages"], json!([]));
}
#[test]
fn management_public_share_uses_trusted_loopback_and_owner_published_keys() {
    let server = Running::start(Tunnels::PRODUCT);
    server
        .oracle()
        .execute(
            "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?, '00000000000000000001', ?)",
            rusqlite::params![PAGE, [8u8; 32].as_slice()],
        )
        .unwrap();
    let path = tmt_colab::management::PATH;
    let header = format!("{}\r\n", owner(DEVICE));
    let id = "90000000-0000-4000-8000-000000000006";
    let invalid = management_body(
        &server,
        id,
        1,
        "page.share",
        json!({"pageId":PAGE,"mode":"public","publication":"cloud"}),
        DEVICE,
        now(),
    );
    assert!(server.event(path, &header, &invalid).ends_with("INVALID"));
    let body = management_body(
        &server,
        id,
        1,
        "page.share",
        json!({"pageId":PAGE,"mode":"public"}),
        DEVICE,
        now(),
    );
    management_response(&server, &server.event(path, &header, &body), "3");
    let db = server.oracle();
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT envelope FROM membership_log WHERE revision=?",
            [format!("{:020}", 3)],
            |r| r.get(0),
        )
        .unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    let payload: Value = serde_json::from_slice(
        &values::binary(envelope["payload"].as_str().unwrap(), 65536).unwrap(),
    )
    .unwrap();
    assert_eq!(payload["mode"], "public");
    assert_eq!(payload["epoch"], "2");
    assert_eq!(payload["publishedKeys"].as_array().unwrap().len(), 2);
    for (index, epoch) in [1, 2].into_iter().enumerate() {
        let secret: Vec<u8> = db
            .query_row(
                "SELECT secret FROM epoch_secrets WHERE page=? AND epoch=?",
                rusqlite::params![PAGE, format!("{epoch:020}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(payload["publishedKeys"][index]["epoch"], epoch.to_string());
        assert_eq!(
            payload["publishedKeys"][index]["key"],
            values::encode_binary(&secret)
        );
    }
}
#[test]
fn management_root_ipc_rejects_forwarded_headers_before_parsing_without_effects() {
    let server = Running::start(Tunnels::PRODUCT);
    server
        .oracle()
        .execute(
            "INSERT INTO epoch_secrets(page,epoch,secret) VALUES (?, '00000000000000000001', ?)",
            rusqlite::params![PAGE, [8u8; 32].as_slice()],
        )
        .unwrap();
    let path = tmt_colab::management::LOCAL_PATH;
    let body = local_management(
        &server,
        "40000000-0000-4000-8000-000000000007",
        1,
        "member.add",
        management_member("60000000-0000-4000-8000-000000000001", 20),
    );
    for header in [
        format!("{}\r\n", owner(DEVICE)),
        "tmt-device-context: not json\r\n".into(),
        "tmt-device-context: \r\n".into(),
        "tmt-device-event: 1\r\n".into(),
        "tmt-device-event: forged\r\n".into(),
        "tmt-device-event: \r\n".into(),
        format!("{}\r\ntmt-device-event: 1\r\n", owner(DEVICE)),
    ] {
        for body in [body.as_str(), "not json"] {
            let response = server.event(path, &header, body);
            assert!(
                response.starts_with("HTTP/1.1 403") && response.ends_with("DENIED"),
                "{response}"
            );
        }
    }
    for table in ["membership_log", "owner_operations", "recipients"] {
        let count: i64 = server
            .oracle()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "forwarded local request changed {table}");
    }
    // The same frozen request remains usable by a root-local caller without headers.
    management_response(&server, &server.event(path, "", &body), "2");
}

#[test]
fn root_local_page_write_broadcasts_chain_chunks_and_replays_without_fanout() {
    use tmt_colab::{decoder::Decoder, page};
    let server = Running::start(Tunnels::PRODUCT);
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    // The fixture supplies an existing page key; the CLI must not initialize it.
    server
        .oracle()
        .execute(
            "INSERT INTO epoch_secrets VALUES (?,?,?)",
            rusqlite::params![PAGE, format!("{:020}", 1), [8u8; 32].as_slice()],
        )
        .unwrap();
    let (socket, head) = server.tunnel();
    assert!(head.starts_with("HTTP/1.1 101"));
    let mut peer = WebSocket::from_raw_socket(socket, Role::Client, None);
    hello(&server, &mut peer, DEVICE);
    let store = Store::read(&layout).unwrap();
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let source = "source 🐈\r\n".repeat(6000);
    let prepared = page::prepare(&store, &key, PAGE, &source, None, &mut decoder, now()).unwrap();
    let base = page::prepare(&store, &key, PAGE, "stale", None, &mut decoder, now()).unwrap();
    store.close().unwrap();
    let body = serde_json::to_string(&prepared).unwrap();
    assert!(body.len() > limits::HTTP_BODY_BYTES);
    assert!(body.len() < limits::http_body_bytes(page::ipc::PATH));
    let before = fs::read(layout.directory.join("space.db")).unwrap();
    for header in [
        format!("{}\r\n", owner(DEVICE)),
        "tmt-device-event: 1\r\n".into(),
    ] {
        let denied = server.event(page::ipc::PATH, &header, &body);
        assert!(denied.contains("COLAB_DENIED"));
        assert_eq!(fs::read(layout.directory.join("space.db")).unwrap(), before);
    }
    let receipt = page::ipc::write(&layout, &prepared).unwrap();
    let broadcast = receive(&mut peer);
    assert_eq!(broadcast["type"], "broadcast");
    assert_eq!(broadcast["streamId"], receipt.stream_id);
    assert_eq!(broadcast["chains"][0]["chain"], prepared.chain);
    assert!(broadcast["envelope"].is_object());
    let mut assembled = Vec::new();
    loop {
        let chunk = receive(&mut peer);
        assert_eq!(chunk["type"], "chunk");
        assembled
            .extend(values::binary(chunk["bytes"].as_str().unwrap(), limits::CHUNK_BYTES).unwrap());
        if chunk["index"].as_u64().unwrap() + 1 == chunk["count"].as_u64().unwrap() {
            break;
        }
    }
    assert_eq!(
        assembled,
        values::binary(&prepared.envelope, limits::UPDATE_BYTES).unwrap()
    );
    let before = fs::read(layout.directory.join("space.db")).unwrap();
    let replay = page::ipc::write(&layout, &prepared).unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(&receipt).unwrap()
    );
    assert_eq!(fs::read(layout.directory.join("space.db")).unwrap(), before);
    peer.send(Message::Ping(vec![1].into())).unwrap();
    assert!(
        matches!(peer.read().unwrap(), Message::Pong(_)),
        "replay broadcast again"
    );
    let stale = page::ipc::write(&layout, &base).err().unwrap();
    assert_eq!(
        stale
            .downcast_ref::<page::ipc::WriteError>()
            .unwrap()
            .code(),
        "COLAB_STALE_BASE"
    );
    assert_eq!(fs::read(layout.directory.join("space.db")).unwrap(), before);
    let store = Store::read(&layout).unwrap();
    assert_eq!(
        page::read(&store, &key, PAGE, &mut decoder).unwrap().source,
        source
    );
    // A failed local request never switches writers; removing the socket rejects.
    drop(peer);
    drop(server);
    assert!(page::ipc::write(&layout, &prepared).is_err());
}

impl Running {
    fn publish(&self) {
        use tmt_colab::{
            store::owner::Mutation,
            transitions::{Engine, OwnerAction, OwnerRequest, Publication, ShareMode},
        };
        let layout = Layout::open(&self.root).unwrap();
        let key = Keyring::read(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        let head = store
            .owner_head(&self.space, &key.owner_public())
            .unwrap()
            .unwrap();
        store
            .owner_transaction(
                &self.space,
                &key.owner_public(),
                Mutation {
                    operation_id: "40000000-0000-4000-8000-000000000001",
                    expected_revision: head.revision,
                    digest: [1; 32],
                },
                |tx| {
                    tx.put_epoch_secret(PAGE, 1, &[8; 32])?;
                    tx.append_statement(&key.sign_statement(
                        tx.head(),
                        "retention.set",
                        &serde_json::to_vec(&json!({"pageId":PAGE,"days":null}))?,
                    )?)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        Engine::with_decoder_config(support::decoder_config(
            env!("CARGO_BIN_EXE_tmt-colab").into(),
        ))
        .unwrap()
        .apply(
            &mut store,
            &key,
            OwnerRequest {
                operation_id: "40000000-0000-4000-8000-000000000002",
                expected_revision: head.revision + 1,
                action: OwnerAction::Share {
                    page: PAGE,
                    mode: ShareMode::Public,
                    publication: Publication::Loopback,
                },
                transport_digest: None,
                scope: None,
            },
            now(),
        )
        .unwrap();
    }
    fn reader_session(&self) -> Value {
        let response = self.event(
            tmt_colab::readers::CHALLENGE_PATH,
            "",
            &json!({"kind":"public","space":self.space,"page":PAGE}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let c: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        let response = self.event(
            tmt_colab::readers::SESSION_PATH,
            "",
            &json!({"kind":"public","challengeId":c["challengeId"]}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn reader_peer(&self, s: &Value) -> WebSocket<UnixStream> {
        let carrier = format!(
            "Sec-WebSocket-Protocol: colab-reader-v1.{}, colab-sync-v1\r\n",
            s["token"].as_str().unwrap()
        );
        let headers = UPGRADE.replace("Sec-WebSocket-Protocol: colab-sync-v1\r\n", &carrier);
        let (socket, head) = self.open_tunnel(&headers);
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head.contains("Sec-WebSocket-Protocol: colab-sync-v1\r\n"));
        assert!(!head.contains(s["token"].as_str().unwrap()));
        assert!(!head.contains("colab-reader-v1."));
        WebSocket::from_raw_socket(socket, Role::Client, None)
    }
    fn reader_frame(&self, s: &Value, kind: &str, fields: Value) -> Value {
        let mut v = self.frame(kind, fields);
        v["epoch"] = s["epoch"].clone();
        v
    }
}
#[test]
fn mounted_public_readers_catch_up_and_cannot_publish_or_claim_owner_context_twice() {
    for _ in 0..2 {
        let server = Running::start(Tunnels::PRODUCT);
        server.publish();
        let s = server.reader_session();
        let mut peer = server.reader_peer(&s);
        send(
            &mut peer,
            server.reader_frame(
                &s,
                "hello",
                json!({"device":s["principal"],"membershipRevision":"0","cursors":[]}),
            ),
        );
        let first = receive(&mut peer);
        assert_eq!(first["type"], "catchup");
        assert_eq!(first["membershipHead"]["ownerKey"], s["ownerKey"]);
        send(
            &mut peer,
            server.reader_frame(&s, "ack", json!({"cursors":[]})),
        );
        let wraps = receive(&mut peer);
        assert_eq!(wraps["wraps"], json!([]));
        assert_eq!(wraps["more"], false);
        let count = server
            .oracle()
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap();
        for (kind, fields) in [
            (
                "awareness",
                json!({"device":s["principal"],"data":values::encode_binary(b"presence")}),
            ),
            (
                "append",
                json!({"streamId":s["principal"],"seq":"1","envelopeHash":values::encode_binary(&[0;32]),"envelope":{"objectId":"00".repeat(32)}}),
            ),
            (
                "chunk",
                json!({"objectId":"00".repeat(32),"envelopeHash":values::encode_binary(&[0;32]),"index":0,"count":1,"bytes":values::encode_binary(b"x")}),
            ),
        ] {
            let s = server.reader_session();
            let mut p = server.reader_peer(&s);
            send(
                &mut p,
                server.reader_frame(&s, "subscribe", json!({"cursors":[]})),
            );
            send(&mut p, server.reader_frame(&s, kind, fields));
            let denial = receive(&mut p);
            assert_eq!(denial["code"], "DENIED", "{denial}");
        }
        assert_eq!(
            server
                .oracle()
                .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            count
        );
        let revision = server
            .oracle()
            .query_row("SELECT count(*) FROM membership_log", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        let management = management_body(
            &server,
            "40000000-0000-4000-8000-000000000003",
            revision as u64,
            "page.archive",
            json!({"pageId":PAGE}),
            DEVICE,
            now(),
        );
        let denial = server.event(
            tmt_colab::management::PATH,
            &format!(
                "Sec-WebSocket-Protocol: colab-reader-v1.{}\r\n",
                s["token"].as_str().unwrap()
            ),
            &management,
        );
        assert!(
            denial.starts_with("HTTP/1.1 403") && denial.ends_with("DENIED"),
            "{denial}"
        );
        assert_eq!(
            server
                .oracle()
                .query_row("SELECT count(*) FROM membership_log", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            revision
        );
        for path in ["/api/session", "/api/pages"] {
            assert!(
                server
                    .request(&Running::get(path, ""))
                    .starts_with("HTTP/1.1 403")
            );
        }
        assert!(
            server
                .event(
                    tmt_colab::registration::PATH,
                    "",
                    &registration_body(DEVICE)
                        .into_iter()
                        .map(char::from)
                        .collect::<String>()
                )
                .starts_with("HTTP/1.1 403")
        );
        let wrong = UPGRADE.replace(
            "colab-sync-v1",
            "colab-sync-v1, colab-reader-v1.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        );
        assert!(server.open_tunnel(&wrong).1.starts_with("HTTP/1.1 403"));
        let replay = UPGRADE.replace(
            "colab-sync-v1",
            &format!(
                "colab-sync-v1, colab-reader-v1.{}",
                s["token"].as_str().unwrap()
            ),
        );
        assert!(server.open_tunnel(&replay).1.starts_with("HTTP/1.1 403"));
    }
}

#[test]
fn archived_owner_pages_remain_readable_but_deny_all_publication() {
    let server = Running::start(Tunnels::PRODUCT);
    let mut peer = server.peer(DEVICE);
    hello(&server, &mut peer, DEVICE);
    let awareness = server.frame(
        "awareness",
        json!({"device":DEVICE,"data":values::encode_binary(b"presence")}),
    );
    send(&mut peer, awareness.clone());
    assert_eq!(receive(&mut peer)["type"], "awareness");
    let layout = Layout::open(&server.root).unwrap();
    let key = Keyring::read(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    store
        .owner_transaction(
            &server.space,
            &key.owner_public(),
            tmt_colab::store::owner::Mutation {
                operation_id: OTHER,
                digest: [23; 32],
                expected_revision: 1,
            },
            |tx| {
                tx.append_statement(&key.sign_statement(
                    tx.head(),
                    "page.archive",
                    &serde_json::to_vec(&json!({"pageId":PAGE}))?,
                )?)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    // Archive preserves owner catchup, but freezes presence and uploads too.
    let mut reader = server.peer(DEVICE);
    let catchup = hello(&server, &mut reader, DEVICE);
    assert_eq!(catchup[0]["membershipHead"]["revision"], "2");
    for frame in [
        awareness,
        server.frame("append", json!({"streamId":DEVICE,"seq":"1","envelopeHash":values::encode_binary(&[0;32]),"envelope":{"objectId":"00".repeat(32)}})),
        server.frame("chunk", json!({"objectId":"00".repeat(32),"envelopeHash":values::encode_binary(&[0;32]),"index":0,"count":1,"bytes":values::encode_binary(b"x")})),
    ] {
        send(&mut peer, frame);
        assert_eq!(receive(&mut peer)["code"], "DENIED");
    }
    assert_eq!(
        server
            .oracle()
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
