//! Door session acceptance on real sockets: a paired test device opens a
//! session with a signed `session.open`, the door cookie carries its device
//! context to a fixture extension under `<prefix>/x/colab/`, and revocation, a newer
//! session or idle expiry end it.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

use door::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tmt_remote::{
    canonical::{self, Envelope},
    control, crypto,
    pairing::Timing,
    store::uuid_v4,
};

const FAST: Timing = Timing {
    lifetime: Duration::from_secs(30),
    submit_wait: Duration::from_secs(10),
};
const UPGRADE: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n";

/// Pair a browser device through the owner's confirmation; returns its client ID.
fn paired(h: &Harness, device: &Device) -> String {
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(h);
    let body = device.body(&descriptor, &code, &device.key);
    let submitted = submit(h, body, Some(device.origin.clone()));
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("confirm");
    let ended = owner.next();
    assert_eq!(ended["reason"], "paired");
    assert_eq!(submitted.join().unwrap().0, 200);
    ended["clientId"].as_str().unwrap().to_owned()
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// A `session.open` control as the device would send it; `edit` changes the
/// wire after signing.
struct Opening<'a> {
    h: &'a Harness,
    client_id: &'a str,
    key: &'a SigningKey,
    nonce: [u8; 16],
    timestamp_ms: u64,
}
impl<'a> Opening<'a> {
    fn new(h: &'a Harness, client_id: &'a str, key: &'a SigningKey) -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        Self {
            h,
            client_id,
            key,
            nonce,
            timestamp_ms: now_ms(),
        }
    }
    fn wire(&self) -> Value {
        let payload = json!({"clientNonce": hex(&self.nonce)}).to_string();
        let id = uuid_v4().unwrap();
        let signed = canonical::envelope(&Envelope {
            kind: "control",
            id: &id,
            correlation_id: None,
            machine_id: &self.h.machine_id,
            window_id: &self.h.window_id,
            client_id: self.client_id,
            session_id: "new",
            sequence: "0",
            timestamp_ms: self.timestamp_ms,
            origin: &self.h.origin,
            operation: "session.open",
            payload: payload.as_bytes(),
        })
        .unwrap();
        json!({
            "version": 1,
            "profile": "local-v1",
            "kind": "control",
            "id": id,
            "correlationId": null,
            "machineId": self.h.machine_id,
            "windowId": self.h.window_id,
            "clientId": self.client_id,
            "sessionId": "new",
            "sequence": "0",
            "timestampMs": self.timestamp_ms,
            "origin": self.h.origin,
            "operation": "session.open",
            "payload": canonical::base64url(payload.as_bytes()),
            "signature": canonical::base64url(&self.key.sign(&signed).to_bytes()),
        })
    }
}
struct Reply {
    status: u16,
    head: String,
    body: String,
}
impl Reply {
    fn cookie(&self) -> Option<String> {
        self.head.lines().find_map(|line| {
            let (name, value) = line.split_once(": ")?;
            name.eq_ignore_ascii_case("set-cookie")
                .then(|| value.to_owned())
        })
    }
}
fn exchange(h: &Harness, request: &str) -> Reply {
    let mut client = TcpStream::connect(h.addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let mut reply = String::new();
    client.read_to_string(&mut reply).unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    Reply {
        status: head[9..12].parse().unwrap(),
        head: head.to_owned(),
        body: body.to_owned(),
    }
}
/// POST to `<prefix><route>` with optional Origin and Cookie headers.
fn post(h: &Harness, route: &str, body: &str, origin: Option<&str>, cookie: Option<&str>) -> Reply {
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    let cookie = cookie
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    exchange(
        h,
        &format!(
            "POST {}{route} HTTP/1.1\r\nHost: {}\r\n{origin}{cookie}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            h.prefix,
            h.addr,
            body.len()
        ),
    )
}
fn open_session(h: &Harness, wire: &Value) -> Reply {
    post(h, "/append", &wire.to_string(), Some(&h.origin), None)
}
/// The `tmt_door=<token>` pair a browser sends back.
fn pair_of(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_owned()
}

/// Fixture extension on `<root>/colab/door.sock`: records each forwarded
/// device context, answers plain requests with 200 and echoes upgrades.
struct Colab {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    contexts: Arc<Mutex<Vec<Option<Value>>>>,
}
impl Colab {
    fn serve(h: &Harness) -> Self {
        let directory = h.root.join("colab");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("door.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let contexts = Arc::new(Mutex::new(Vec::new()));
        let (flag, seen) = (Arc::clone(&stop), Arc::clone(&contexts));
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while !flag.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let seen = Arc::clone(&seen);
                        connections.push(thread::spawn(move || serve(stream, &seen)));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            stop,
            thread: Some(thread),
            contexts,
        }
    }
    fn last_context(&self) -> Option<Value> {
        self.contexts.lock().unwrap().last().cloned().unwrap()
    }
}
impl Drop for Colab {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn serve(stream: UnixStream, seen: &Mutex<Vec<Option<Value>>>) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let (mut context, mut upgrade) = (None, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        assert!(
            !line.contains("tmt-session"),
            "session marker leaked to extension"
        );
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("tmt-device-context: ") {
            context = serde_json::from_str(line.split_once(": ").unwrap().1.trim()).ok();
        }
        upgrade |= lower.starts_with("upgrade: websocket");
    }
    seen.lock().unwrap().push(context);
    let mut stream = stream;
    if !upgrade {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    let _ = stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n");
    let mut chunk = [0; 1024];
    while let Ok(n) = reader.read(&mut chunk) {
        if n == 0 || stream.write_all(&chunk[..n]).is_err() {
            break;
        }
    }
}
/// A mounted GET carrying `cookie`; returns the context colab received.
fn mounted(h: &Harness, colab: &Colab, cookie: Option<&str>) -> Option<Value> {
    let cookie = cookie
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    let reply = exchange(
        h,
        &format!(
            "GET {}/x/colab/ HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\n{cookie}\r\n",
            h.prefix, h.addr, h.origin
        ),
    );
    assert_eq!(reply.status, 200);
    colab.last_context()
}
/// Upgrade through the mount with `cookie`; returns the echoing client.
fn tunnel(h: &Harness, cookie: &str) -> TcpStream {
    tunnel_for(h, cookie, None)
}
fn tunnel_for(h: &Harness, cookie: &str, session: Option<&str>) -> TcpStream {
    let query = session
        .map(|id| format!("?tmt-session={id}"))
        .unwrap_or_default();
    let mut client = TcpStream::connect(h.addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        client,
        "GET {}/x/colab/sync{query} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nCookie: {cookie}\r\n{UPGRADE}\r\n",
        h.prefix, h.addr, h.origin
    )
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"));
    client.write_all(b"ping").unwrap();
    let mut echoed = [0; 4];
    client.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"ping");
    client
}
/// The door closes the tunnel within a bound (one poll tick in practice).
fn closes(client: &mut TcpStream) {
    let deadline = Instant::now() + Duration::from_secs(5);
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut byte = [0; 1];
    match client.read(&mut byte) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("tunnel stayed open: {other:?}"),
    }
    assert!(Instant::now() < deadline);
}
fn control(h: &Harness, request: Value) -> Value {
    let mut stream = control::connect(&h.root.join("remote")).unwrap();
    stream.write_all(format!("{request}\n").as_bytes()).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn session_open_returns_a_signed_response_and_a_door_cookie() {
    let h = Harness::new(FAST);
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let wire = Opening::new(&h, &client_id, &device.key).wire();
    let reply = open_session(&h, &wire);
    assert_eq!(reply.status, 200);
    // The public root alias adds no cookie; session.open retains the mount path.
    let alias = exchange(
        &h,
        &format!("GET /p/abcd HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
    );
    assert_eq!(alias.status, 302);
    assert!(alias.cookie().is_none());
    // HttpOnly, SameSite=Strict, scoped to mounted pages; only a hash is kept.
    let set_cookie = reply.cookie().unwrap();
    let token = set_cookie
        .strip_prefix("tmt_door=")
        .unwrap()
        .strip_suffix(&format!(
            "; Path={}/x/; HttpOnly; SameSite=Strict",
            h.prefix
        ))
        .unwrap();
    assert_eq!(token.len(), 43);
    assert!(!reply.body.contains(token));
    // The response is machine-signed over the canonical envelope bytes.
    let response: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(response["kind"], "response");
    assert_eq!(response["correlationId"], wire["id"]);
    assert_eq!(response["sequence"], "1");
    assert_eq!(response["origin"], h.origin.as_str());
    let payload = canonical::base64url_decode(response["payload"].as_str().unwrap()).unwrap();
    let text = |name: &str| response[name].as_str().unwrap();
    let signed = canonical::envelope(&Envelope {
        kind: "response",
        id: text("id"),
        correlation_id: Some(text("correlationId")),
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: "1",
        timestamp_ms: response["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: "session.open",
        payload: &payload,
    })
    .unwrap();
    let signature = canonical::base64url_bytes(text("signature"), 64).unwrap();
    crypto::verify_signature(&h.machine_public, &signed, &signature).unwrap();
    let payload: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["sessionId"], response["sessionId"]);
    assert_eq!(payload["grantRevision"], 1);
    assert_eq!(payload["expiresAtMs"], Value::Null);
    // The cookie carries the owner device context to the mounted extension.
    let cookie = pair_of(&reply.cookie().unwrap());
    assert_eq!(
        mounted(&h, &colab, Some(&format!("other=1; {cookie}"))),
        Some(json!({
            "deviceId": client_id,
            "kind": "browser",
            "origin": h.origin,
            "name": "Laptop é",
            "publicKey": canonical::base64url(&device.public()),
            "owner": true,
            "grantRevision": 1,
        }))
    );
    assert_eq!(mounted(&h, &colab, None), None);
    // A duplicated cookie name is ambiguous and carries no context.
    assert_eq!(
        mounted(&h, &colab, Some(&format!("{cookie}; {cookie}"))),
        None
    );
}

#[test]
fn session_open_refusals_are_generic_and_replays_refuse() {
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let impostor = SigningKey::from_bytes(&[3; 32]);
    let mut stale = Opening::new(&h, &client_id, &device.key);
    stale.timestamp_ms -= 61_000;
    let mut future = Opening::new(&h, &client_id, &device.key);
    future.timestamp_ms += 61_000;
    let unknown = uuid_v4().unwrap();
    let good = Opening::new(&h, &client_id, &device.key).wire();
    let edited = |field: &str, value: Value| {
        let mut wire = good.clone();
        wire[field] = value;
        wire
    };
    let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let other_origin = format!(
        "http://127.0.0.1:{}",
        elsewhere.local_addr().unwrap().port()
    );
    let mut extra = good.clone();
    extra["extra"] = json!(1);
    let cases = [
        (
            Opening::new(&h, &client_id, &impostor).wire(),
            Some(h.origin.clone()),
            "other key",
        ),
        (stale.wire(), Some(h.origin.clone()), "stale timestamp"),
        (future.wire(), Some(h.origin.clone()), "future timestamp"),
        (
            Opening::new(&h, &unknown, &device.key).wire(),
            Some(h.origin.clone()),
            "unknown device",
        ),
        (
            edited("windowId", json!(uuid_v4().unwrap())),
            Some(h.origin.clone()),
            "old window",
        ),
        (
            edited("sessionId", json!(uuid_v4().unwrap())),
            Some(h.origin.clone()),
            "not new",
        ),
        (
            edited("payload", json!("e30")),
            Some(h.origin.clone()),
            "no nonce",
        ),
        (extra, Some(h.origin.clone()), "extra field"),
        (good.clone(), None, "missing Origin"),
        (good.clone(), Some(other_origin), "another loopback port"),
    ];
    for (wire, origin, label) in cases {
        let reply = post(&h, "/append", &wire.to_string(), origin.as_deref(), None);
        assert_eq!((reply.status, reply.body.as_str()), (404, "{}"), "{label}");
        assert!(reply.cookie().is_none(), "{label}");
    }
    // None of those consumed the nonce; the valid control opens once, and
    // replaying the same signed bytes refuses.
    assert_eq!(open_session(&h, &good).status, 200);
    let replay = open_session(&h, &good);
    assert_eq!(replay.status, 404);
    assert!(replay.cookie().is_none());
}

#[test]
fn a_session_cookie_alone_cannot_operate_or_pair() {
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let reply = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let cookie = pair_of(&reply.cookie().unwrap());
    // Remote routes refuse any cookie before reading the body, so the cookie
    // can neither append an operation nor submit or confirm an enrollment.
    for route in ["/append", "/subscribe", "/ack", "/pair"] {
        let reply = post(&h, route, "{}", Some(&h.origin), Some(&cookie));
        assert_eq!(reply.status, 400, "{route}");
    }
    // Without its device signature, an operation envelope is refused too.
    let mut unsigned = Opening::new(&h, &client_id, &device.key).wire();
    unsigned["signature"] = json!(canonical::base64url(&[0; 64]));
    assert_eq!(open_session(&h, &unsigned).status, 404);
    assert_eq!(h.grants(), 1);
}

#[test]
fn several_session_cookies_and_tunnels_stay_live_together() {
    let h = Harness::new(FAST);
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let first = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let first = pair_of(&first.cookie().unwrap());
    let mut tunnel = tunnel(&h, &first);
    let second = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let second = pair_of(&second.cookie().unwrap());
    tunnel.write_all(b"pong").unwrap();
    let mut echoed = [0; 4];
    tunnel.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"pong");
    assert_eq!(
        mounted(&h, &colab, Some(&first)).unwrap()["deviceId"],
        client_id
    );
    drop(tunnel);
    assert_eq!(
        mounted(&h, &colab, Some(&second)).unwrap()["deviceId"],
        client_id
    );
}

#[test]
fn revocation_ends_sessions_and_tunnels_before_it_acknowledges() {
    let h = Harness::new(FAST);
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let reply = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let cookie = pair_of(&reply.cookie().unwrap());
    let mut tunnel = tunnel(&h, &cookie);
    let second = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let second_cookie = pair_of(&second.cookie().unwrap());
    let mut second_tunnel = tunnel_for(&h, &second_cookie, None);
    let listed = control(&h, json!({"op":"devices"}));
    assert_eq!(listed["devices"][0]["clientId"], client_id);
    assert_eq!(listed["devices"][0]["revoked"], false);
    assert_eq!(
        listed["devices"][0]["words"],
        canonical::fingerprint_words(&device.public())
            .unwrap()
            .join(" ")
    );
    let revoked = control(&h, json!({"op":"revoke","clientId":client_id}));
    assert_eq!(revoked["device"]["revoked"], true);
    assert_eq!(revoked["device"]["revision"], 2);
    closes(&mut tunnel);
    closes(&mut second_tunnel);
    assert_eq!(mounted(&h, &colab, Some(&second_cookie)), None);
    assert_eq!(mounted(&h, &colab, Some(&cookie)), None);
    // The revoked key cannot open a session again; revoking twice is stable.
    let reopen = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    assert_eq!(reopen.status, 404);
    let again = control(&h, json!({"op":"revoke","clientId":client_id}));
    assert_eq!(again["device"]["revision"], 2);
    let missing = control(&h, json!({"op":"revoke","clientId":uuid_v4().unwrap()}));
    assert_eq!(missing["error"]["code"], "REMOTE_DEVICE_NOT_FOUND");
}

#[test]
fn an_idle_session_ends_and_reopens_silently() {
    let idle = Duration::from_millis(300);
    let now = Arc::new(Mutex::new(Instant::now()));
    let clock = Arc::clone(&now);
    let h = Harness::with_clock(FAST, idle, Arc::new(move || *clock.lock().unwrap()));
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client_id = paired(&h, &device);
    let reply = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    let cookie = pair_of(&reply.cookie().unwrap());
    assert!(mounted(&h, &colab, Some(&cookie)).is_some());
    // Scheduling cannot age the frozen clock. Just before the idle bound the
    // cookie still admits its device, and that mount counts as fresh activity.
    *now.lock().unwrap() += idle - Duration::from_nanos(1);
    assert_eq!(
        mounted(&h, &colab, Some(&cookie)).unwrap()["deviceId"],
        client_id
    );
    // Expire exactly one idle interval after the last mounted request.
    *now.lock().unwrap() += idle;
    assert_eq!(mounted(&h, &colab, Some(&cookie)), None);
    // Reopening is one signed control with no owner step or new pairing.
    let reopened = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
    assert_eq!(reopened.status, 200);
    let fresh = pair_of(&reopened.cookie().unwrap());
    assert!(mounted(&h, &colab, Some(&fresh)).is_some());
    assert_eq!(h.grants(), 1);
}

#[test]
fn rename_preserves_authority_and_reopens_with_the_new_context() {
    for _ in 0..2 {
        let h = Harness::new(FAST);
        let colab = Colab::serve(&h);
        let device = Device::browser(&h, 7);
        let client_id = paired(&h, &device);
        let opened = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
        let cookie = pair_of(&opened.cookie().unwrap());
        let mut socket = tunnel(&h, &cookie);
        let before = h.store.lock().unwrap().grant(&client_id).unwrap().unwrap();
        let renamed = control(
            &h,
            json!({"op":"rename","clientId":client_id,"name":"New laptop"}),
        );
        assert_eq!(renamed["device"]["revision"], 2);
        closes(&mut socket);
        assert_eq!(mounted(&h, &colab, Some(&cookie)), None);
        let fresh = open_session(&h, &Opening::new(&h, &client_id, &device.key).wire());
        assert_eq!(fresh.status, 200);
        let fresh_cookie = pair_of(&fresh.cookie().unwrap());
        let context = mounted(&h, &colab, Some(&fresh_cookie)).unwrap();
        assert_eq!(context["deviceId"], client_id);
        assert_eq!(context["name"], "New laptop");
        assert_eq!(context["grantRevision"], 2);
        let repeated = control(
            &h,
            json!({"op":"rename","clientId":client_id,"name":"New laptop"}),
        );
        assert_eq!(repeated["device"]["revision"], 2);
        assert!(mounted(&h, &colab, Some(&fresh_cookie)).is_some());
        let mut after = h.store.lock().unwrap().grant(&client_id).unwrap().unwrap();
        after.name = before.name.clone();
        after.revision = before.revision;
        assert_eq!(after, before);
    }
}

/// A normal signed message used to observe a session-end reason after its tunnel drops.
fn session_probe(h: &Harness, client: &str, key: &SigningKey, session: &str) -> Reply {
    let mut wire = Opening::new(h, client, key).wire();
    wire["kind"] = json!("request");
    wire["operation"] = json!("capabilities");
    wire["sessionId"] = json!(session);
    wire["sequence"] = json!("1");
    wire["payload"] = json!(canonical::base64url(b"{}"));
    let text = |name: &str| wire[name].as_str().unwrap();
    let bytes = canonical::envelope(&Envelope {
        kind: text("kind"),
        id: text("id"),
        correlation_id: None,
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: text("sequence"),
        timestamp_ms: wire["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: text("operation"),
        payload: b"{}",
    })
    .unwrap();
    wire["signature"] = json!(canonical::base64url(&key.sign(&bytes).to_bytes()));
    open_session(h, &wire)
}
fn payload(reply: &Reply) -> Value {
    assert_eq!(reply.status, 200);
    let wire: Value = serde_json::from_str(&reply.body).unwrap();
    serde_json::from_slice(&canonical::base64url_decode(wire["payload"].as_str().unwrap()).unwrap())
        .unwrap()
}
#[test]
fn explicit_transport_session_survives_shared_cookie_changes_and_ends_on_last_close() {
    let h = Harness::new(FAST);
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let first = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
    let first_id: Value = serde_json::from_str(&first.body).unwrap();
    let first_id = first_id["sessionId"].as_str().unwrap();
    let second = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
    let second_id: Value = serde_json::from_str(&second.body).unwrap();
    let second_id = second_id["sessionId"].as_str().unwrap();
    let cookie = pair_of(&second.cookie().unwrap());
    let one = tunnel_for(&h, &cookie, Some(first_id));
    let mut another = tunnel_for(&h, &cookie, Some(first_id));
    let mut two = tunnel_for(&h, &cookie, Some(second_id));
    drop(one);
    another.write_all(b"live").unwrap();
    let mut echoed = [0; 4];
    another.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"live");
    drop(another);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = payload(&session_probe(&h, &client, &device.key, first_id));
        if result["error"]["code"] == "REMOTE_SESSION_ENDED" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closed transport session stayed live"
        );
        thread::yield_now();
    }
    two.write_all(b"live").unwrap();
    two.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"live");
    assert!(mounted(&h, &colab, Some(&cookie)).is_some());
    // Non-secret ID alone, another device's ID, and a page navigation cannot authorize.
    let other = Device::browser(&h, 8);
    let other_client = paired(&h, &other);
    let other_open = open_session(&h, &Opening::new(&h, &other_client, &other.key).wire());
    let other_cookie = pair_of(&other_open.cookie().unwrap());
    for cookie_header in [String::new(), format!("Cookie: {other_cookie}\r\n")] {
        let reply = exchange(
            &h,
            &format!(
                "GET {}/x/colab/sync?tmt-session={second_id} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\n{cookie_header}{UPGRADE}\r\n",
                h.prefix, h.addr, h.origin
            ),
        );
        assert_eq!(reply.status, 404);
        assert!(!reply.head.contains("Location:"));
    }
    let reply = exchange(
        &h,
        &format!(
            "GET {}/x/colab/home?tmt-session={second_id} HTTP/1.1\r\nHost: {}\r\nCookie: {cookie}\r\n\r\n",
            h.prefix, h.addr
        ),
    );
    assert_eq!(reply.status, 404);
    drop(two);
}

#[test]
fn attached_sessions_keep_the_twelve_hour_idle_limit_and_maintenance_closes_them() {
    let now = Arc::new(Mutex::new(Instant::now()));
    let observed = Arc::clone(&now);
    let h = Harness::with_clock(
        FAST,
        tmt_remote::limits::SESSION_IDLE,
        Arc::new(move || *observed.lock().unwrap()),
    );
    let colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let opened = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
    let cookie = pair_of(&opened.cookie().unwrap());
    let mut transport = tunnel(&h, &cookie);
    *now.lock().unwrap() += tmt_remote::limits::SESSION_UNATTACHED_IDLE;
    assert!(
        mounted(&h, &colab, Some(&cookie)).is_some(),
        "an attached session used the short timeout"
    );
    *now.lock().unwrap() += tmt_remote::limits::SESSION_IDLE;
    // No follow-up request triggers cleanup: the existing door event loop does it.
    closes(&mut transport);
    assert!(mounted(&h, &colab, Some(&cookie)).is_none());
}

#[test]
fn last_transport_close_cancels_its_held_work() {
    let h = Harness::new(FAST);
    let _colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let oracle = rusqlite::Connection::open(h.root.join("remote/remote.db")).unwrap();
    oracle
        .execute(
            "UPDATE grants SET mode='hold' WHERE client_id=?1",
            [&client],
        )
        .unwrap();
    drop(oracle);
    let opened = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
    let session: Value = serde_json::from_str(&opened.body).unwrap();
    let session = session["sessionId"].as_str().unwrap();
    let cookie = pair_of(&opened.cookie().unwrap());
    let transport = tunnel(&h, &cookie);
    let mut wire = Opening::new(&h, &client, &device.key).wire();
    let id = wire["id"].as_str().unwrap().to_owned();
    let intent=json!({"version":1,"operation":"dispatch.create","originator":"anonymous","input":{"operationId":id,"recipientIds":[uuid_v4().unwrap()],"message":"Unconfirmed tab work","kind":"request"}}).to_string();
    wire["kind"] = json!("request");
    wire["sessionId"] = json!(session);
    wire["sequence"] = json!("1");
    wire["operation"] = json!("dispatch.create");
    wire["payload"] = json!(canonical::base64url(intent.as_bytes()));
    let text = |name: &str| wire[name].as_str().unwrap();
    let bytes = canonical::envelope(&Envelope {
        kind: text("kind"),
        id: text("id"),
        correlation_id: None,
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: text("sequence"),
        timestamp_ms: wire["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: text("operation"),
        payload: intent.as_bytes(),
    })
    .unwrap();
    wire["signature"] = json!(canonical::base64url(&device.key.sign(&bytes).to_bytes()));
    let permit = h
        .sessions
        .admit(
            tmt_remote::admission::BindingAction::Append,
            Some(&h.origin),
            wire.to_string().as_bytes(),
            1024,
        )
        .ok()
        .unwrap();
    let held = permit.adopt(Some(intent.as_bytes()), &[]).unwrap();
    assert_eq!(held.phase, "held");
    drop(permit);
    drop(transport);
    let grant = h.store.lock().unwrap().grant(&client).unwrap().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let owned = h
            .store
            .lock()
            .unwrap()
            .owned(&grant, &id, now_ms())
            .unwrap();
        if owned.phase == "cancelled" {
            assert!(owned.frozen.is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "transport close did not cancel held work"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
