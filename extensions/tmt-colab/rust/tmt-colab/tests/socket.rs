//! Colab's owner-only socket on real Unix sockets: framing bounds, the device
//! context from the remote door, the colab-sync-v1 handshake, tunnel bounds,
//! capacity and shutdown.
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
        let mut registration =
            Registration::new(store, key, env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap();
        for id in [DEVICE, OTHER] {
            registration
                .register(Some(&context(id)), &registration_body(id), now())
                .unwrap();
        }
        let socket = MountSocket::bind(&layout, &space, tunnels)
            .unwrap()
            .with_registration(&layout, Arc::new(Mutex::new(registration)))
            .unwrap();
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
        let mut socket = self.connect();
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
    assert!(owned.contains(&format!("Colab space {} is running. You are signed in as &lt;b&gt;Laptop&lt;/b&gt;. Co-editing arrives with the next colab slice.", server.space)));
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
        self.request(&format!("POST {path} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{header}\r\n{body}",body.len()))
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
        server.frame("hello", json!({"device":id,"cursors":[]})),
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
        first["membershipHead"],
        json!({"revision":head.revision.to_string(),"statementHash":values::encode_binary(&head.hash)})
    );
    assert_eq!(first["baseline"], Value::Null);
    let mut pages = vec![first];
    for _ in 0..8 {
        let page = receive(peer);
        assert_eq!(page["type"], "catchup");
        let more = page["more"].as_bool().unwrap();
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
    let hello = server.frame("hello", json!({"device":OTHER,"cursors":[],"epoch":"2"}));
    send(&mut reopened, hello);
    let first = receive(&mut reopened);
    assert_eq!(first["membershipHead"]["revision"], "3");
    assert_eq!(first["baseline"], values::encode_binary(&saved.descriptor));
    assert_eq!(receive(&mut reopened)["more"], false);
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
        .frame("hello", json!({"device":DEVICE,"cursors":[]}))
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
