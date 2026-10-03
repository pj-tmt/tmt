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
        let mut socket = self.connect();
        socket
            .write_all(Self::get("/sync", &format!("{owner_header}\r\n{UPGRADE}")).as_bytes())
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
    use tmt_colab::assets::{App, POLICY};
    let directory = PathBuf::from(format!("/tmp/tmt-1253-build-{}", std::process::id()));
    fs::create_dir_all(directory.join("assets")).unwrap();
    let files: [(&str, &str, &[u8]); 6] = [
        ("index.html", "text/html; charset=utf-8", br#"<link href="./assets/app.css"><script type="module" src="./assets/app.js"></script>"#),
        ("assets/app.js", "text/javascript; charset=utf-8", b"export {};"),
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
        assert!(head.contains(&format!("Content-Security-Policy: {POLICY}\r\n")));
        assert_eq!(&reply[end..], bytes);
        if path != "/" {
            assert!(
                server
                    .request(&Running::get(&path, ""))
                    .starts_with("HTTP/1.1 403")
            );
        }
    }
    assert!(
        server
            .request(&Running::get("/", ""))
            .contains("This colab space is private")
    );
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
