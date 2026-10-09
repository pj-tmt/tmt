//! Door session acceptance on real sockets: a paired test device opens a
//! session with a signed `session.open`, the door cookie carries its device
//! context to a fixture extension under `<prefix>/x/colab/`. Last transport close
//! starts a reattach grace; authority loss and idle expiry end its session while
//! ordinary session end preserves held work.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

use door::core_fixture::Core;
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
    mount::{SessionState, Sessions},
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
/// Wait only for the real tunnel owner to release its transport, not for idle time.
fn detached(state: &SessionState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.has_transport() {
        assert!(Instant::now() < deadline, "transport owner did not detach");
        thread::yield_now();
    }
}

/// The door closes the tunnel within a bound (one poll tick in practice).
fn closes(client: &mut TcpStream) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut byte = [0; 1];
    let closed = wait_for_close(deadline, |remaining| {
        client.set_read_timeout(Some(remaining))?;
        client.read(&mut byte)
    });
    assert!(closed.is_ok(), "tunnel stayed open: {closed:?}");
}
/// Read interruptions do not prove closure or renew the original deadline.
fn wait_for_close(
    deadline: Instant,
    mut read: impl FnMut(Duration) -> std::io::Result<usize>,
) -> std::io::Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let result = read(remaining);
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        match result {
            Ok(0) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
            Ok(_) => return Err(std::io::Error::other("received bytes instead of closure")),
        }
    }
}

#[test]
fn closure_wait_retries_interruptions_and_requires_eof_or_reset() {
    use std::io::{Error, ErrorKind};
    for closure in [Ok(0), Err(Error::from(ErrorKind::ConnectionReset))] {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut reads = [
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
            closure,
        ]
        .into_iter();
        let mut previous = Duration::from_secs(5);
        let mut attempts = 0;
        wait_for_close(deadline, |remaining| {
            assert!(remaining <= previous);
            previous = remaining;
            attempts += 1;
            reads.next().unwrap()
        })
        .unwrap();
        assert_eq!(attempts, 3, "interruption was mistaken for closure");
    }
    for unexpected in [
        Ok(1),
        Err(Error::from(ErrorKind::WouldBlock)),
        Err(Error::from(ErrorKind::TimedOut)),
    ] {
        let mut reads = [Err(Error::from(ErrorKind::Interrupted)), unexpected].into_iter();
        assert!(
            wait_for_close(Instant::now() + Duration::from_secs(5), |_| {
                reads.next().unwrap()
            })
            .is_err()
        );
        assert!(reads.next().is_none());
    }
}

#[test]
fn closure_wait_interruptions_do_not_extend_the_deadline() {
    let deadline = Instant::now() + Duration::from_millis(5);
    let mut attempts = 0;
    let error = wait_for_close(deadline, |_| {
        attempts += 1;
        while Instant::now() < deadline {
            thread::yield_now();
        }
        Err(std::io::ErrorKind::Interrupted.into())
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(attempts <= 1, "read was retried past the original deadline");
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
    // The public short entry adds no cookie; session.open retains the mount path.
    let alias = exchange(
        &h,
        &format!("GET /p/abcd HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
    );
    assert_eq!(alias.status, 200);
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
fn explicit_transport_session_survives_shared_cookie_changes_and_reattaches_after_last_close() {
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
    let state = h
        .sessions
        .context_for(Some(&cookie), Some(first_id))
        .unwrap()
        .session;
    drop(another);
    detached(&state);
    assert!(!state.ended());
    let mut reattached = tunnel_for(&h, &cookie, Some(first_id));
    assert!(Arc::ptr_eq(
        &state,
        &h.sessions
            .context_for(Some(&cookie), Some(first_id))
            .unwrap()
            .session
    ));
    reattached.write_all(b"live").unwrap();
    reattached.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"live");
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
    drop(reattached);
    drop(two);
}

#[test]
fn last_transport_close_admits_reload_and_same_session_reattach_then_expires_without_activity() {
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
    let id = payload(&opened)["sessionId"].as_str().unwrap().to_owned();
    let cookie = pair_of(&opened.cookie().unwrap());
    let state = h.sessions.context(Some(&cookie)).unwrap().session;
    let transport = tunnel(&h, &cookie);
    // A quiet live transport uses the long idle limit. Close starts fresh grace.
    *now.lock().unwrap() += tmt_remote::limits::SESSION_UNATTACHED_IDLE;
    drop(transport);
    detached(&state);
    assert!(!state.ended());
    let grace = tmt_remote::limits::SESSION_UNATTACHED_IDLE;
    *now.lock().unwrap() += grace - Duration::from_nanos(1);
    assert_eq!(
        mounted(&h, &colab, Some(&cookie)).unwrap()["deviceId"],
        client
    );
    // HTTP activity renews grace and the upgrade attaches the original session.
    *now.lock().unwrap() += grace - Duration::from_nanos(1);
    let transport = tunnel_for(&h, &cookie, Some(&id));
    assert!(Arc::ptr_eq(
        &state,
        &h.sessions
            .context_for(Some(&cookie), Some(&id))
            .unwrap()
            .session
    ));
    assert!(state.has_transport());
    drop(transport);
    detached(&state);
    *now.lock().unwrap() += grace;
    h.sessions.maintain().unwrap();
    assert!(state.ended());
    assert_eq!(mounted(&h, &colab, Some(&cookie)), None);
    assert!(h.sessions.context_for(Some(&cookie), Some(&id)).is_none());
    assert_eq!(
        payload(&session_probe(&h, &client, &device.key, &id))["error"]["code"],
        "REMOTE_SESSION_ENDED"
    );
    // Live authority is removed; only the bounded signed end notice remains in Store.
    let oracle = rusqlite::Connection::open(h.root.join("remote/remote.db")).unwrap();
    let reason: String = oracle
        .query_row(
            "SELECT ended_reason FROM sessions WHERE session_id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason, "REMOTE_SESSION_ENDED");
}

/// Keep cap ordering independent of scheduler time and the 60-second grace.
fn cap_door(limit: usize) -> (Harness, Arc<Mutex<Instant>>) {
    let now = Arc::new(Mutex::new(Instant::now()));
    let clock = Arc::clone(&now);
    let h = Harness::with_clock(
        FAST,
        tmt_remote::limits::SESSION_IDLE,
        Arc::new(move || *clock.lock().unwrap()),
    );
    tmt_remote::settings::set_sessions_per_device(&h.root, Some(limit)).unwrap();
    (h, now)
}
fn cap_session(h: &Harness, client: &str, key: &SigningKey) -> (String, String, Arc<SessionState>) {
    let opened = open_session(h, &Opening::new(h, client, key).wire());
    let id = payload(&opened)["sessionId"].as_str().unwrap().to_owned();
    let cookie = pair_of(&opened.cookie().unwrap());
    let state = h.sessions.context(Some(&cookie)).unwrap().session;
    (id, cookie, state)
}
fn cap_evicted(h: &Harness, client: &str, key: &SigningKey, id: &str, limit: usize) {
    let reply = session_probe(h, client, key, id);
    let value = payload(&reply);
    assert_eq!(value["error"]["code"], "REMOTE_SESSION_EVICTED");
    assert_eq!(value["error"]["limit"], limit);
    assert!(value["error"].get("settingsUrl").is_none());
    let wire: Value = serde_json::from_str(&reply.body).unwrap();
    let text = |name: &str| wire[name].as_str().unwrap();
    let bytes = canonical::base64url_decode(text("payload")).unwrap();
    let signed = canonical::envelope(&Envelope {
        kind: text("kind"),
        id: text("id"),
        correlation_id: wire["correlationId"].as_str(),
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: text("sequence"),
        timestamp_ms: wire["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: text("operation"),
        payload: &bytes,
    })
    .unwrap();
    crypto::verify_signature(
        &h.machine_public,
        &signed,
        &canonical::base64url_bytes(text("signature"), 64).unwrap(),
    )
    .unwrap();
}
fn cap_echo(transport: &mut TcpStream) {
    transport.write_all(b"live").unwrap();
    let mut echoed = [0; 4];
    transport.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"live");
}

#[test]
fn session_cap_entry_burst_preserves_older_attached_sessions() {
    let (h, now) = cap_door(3);
    let _colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let mut attached = Vec::new();
    for _ in 0..2 {
        let (_, cookie, state) = cap_session(&h, &client, &device.key);
        attached.push((state, tunnel_for(&h, &cookie, None)));
        *now.lock().unwrap() += Duration::from_secs(1);
    }
    // Another device is outside this device's cap even if its session is older.
    let other = Device::browser(&h, 8);
    let other_client = paired(&h, &other);
    let (_, _, other_state) = cap_session(&h, &other_client, &other.key);
    let (mut previous, _, _) = cap_session(&h, &client, &device.key);
    for _ in 0..5 {
        *now.lock().unwrap() += Duration::from_secs(1);
        let (next, _, _) = cap_session(&h, &client, &device.key);
        cap_evicted(&h, &client, &device.key, &previous, 3);
        for (state, transport) in &mut attached {
            assert!(!state.ended());
            assert!(state.has_transport());
            cap_echo(transport);
        }
        assert!(!other_state.ended());
        previous = next;
    }
}

#[test]
fn session_cap_prefers_recently_detached_over_older_attached() {
    let (h, now) = cap_door(2);
    let _colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let (_, cookie, attached) = cap_session(&h, &client, &device.key);
    let mut transport = tunnel_for(&h, &cookie, None);
    *now.lock().unwrap() += Duration::from_secs(10);
    let (id, cookie, state) = cap_session(&h, &client, &device.key);
    let departing = tunnel_for(&h, &cookie, None);
    drop(departing);
    detached(&state);
    let (_, _, replacement) = cap_session(&h, &client, &device.key);
    cap_evicted(&h, &client, &device.key, &id, 2);
    assert!(state.ended());
    assert!(!replacement.ended());
    assert!(!attached.ended());
    cap_echo(&mut transport);
}

#[test]
fn session_cap_lowered_limit_evicts_most_idle_transportless_sessions_first() {
    let (h, now) = cap_door(4);
    let _colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let (_, cookie, attached) = cap_session(&h, &client, &device.key);
    let mut transport = tunnel_for(&h, &cookie, None);
    *now.lock().unwrap() += Duration::from_secs(1);
    let (oldest, _, old_state) = cap_session(&h, &client, &device.key);
    *now.lock().unwrap() += Duration::from_secs(1);
    let (middle, _, middle_state) = cap_session(&h, &client, &device.key);
    *now.lock().unwrap() += Duration::from_secs(1);
    let (_, _, youngest) = cap_session(&h, &client, &device.key);
    // The existing while loop must evict twice, retaining the youngest detached row.
    tmt_remote::settings::set_sessions_per_device(&h.root, Some(3)).unwrap();
    cap_session(&h, &client, &device.key);
    cap_evicted(&h, &client, &device.key, &oldest, 3);
    cap_evicted(&h, &client, &device.key, &middle, 3);
    assert!(old_state.ended());
    assert!(middle_state.ended());
    assert!(!youngest.ended());
    assert!(!attached.ended());
    cap_echo(&mut transport);
}

#[test]
fn session_cap_all_attached_still_evicts_the_most_idle() {
    let (h, now) = cap_door(2);
    let _colab = Colab::serve(&h);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let (oldest, cookie, old_state) = cap_session(&h, &client, &device.key);
    let mut old_transport = tunnel_for(&h, &cookie, None);
    *now.lock().unwrap() += Duration::from_secs(1);
    let (_, cookie, recent) = cap_session(&h, &client, &device.key);
    let mut recent_transport = tunnel_for(&h, &cookie, None);
    cap_session(&h, &client, &device.key);
    cap_evicted(&h, &client, &device.key, &oldest, 2);
    assert!(old_state.ended());
    closes(&mut old_transport);
    assert!(!recent.ended());
    cap_echo(&mut recent_transport);
}

#[test]
fn detached_sessions_count_against_cap_and_authority_loss_remains_immediate() {
    for action in ["evict", "revoke", "revision", "expiry", "stop", "end"] {
        let h = Harness::new(FAST);
        let colab = Colab::serve(&h);
        let device = Device::browser(&h, 7);
        let client = paired(&h, &device);
        tmt_remote::settings::set_sessions_per_device(&h.root, Some(1)).unwrap();
        let opened = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
        let id = payload(&opened)["sessionId"].as_str().unwrap().to_owned();
        let cookie = pair_of(&opened.cookie().unwrap());
        let state = h.sessions.context(Some(&cookie)).unwrap().session;
        let transport = tunnel(&h, &cookie);
        drop(transport);
        detached(&state);
        assert_eq!(
            mounted(&h, &colab, Some(&cookie)).unwrap()["deviceId"],
            client
        );
        match action {
            "evict" => {
                let replacement = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
                assert_eq!(replacement.status, 200);
                let error = payload(&session_probe(&h, &client, &device.key, &id));
                assert_eq!(error["error"]["code"], "REMOTE_SESSION_EVICTED");
                assert_eq!(error["error"]["limit"], 1);
                assert!(
                    mounted(&h, &colab, Some(&pair_of(&replacement.cookie().unwrap()))).is_some()
                );
            }
            "revoke" => {
                control(&h, json!({"op":"revoke","clientId":client}));
            }
            "revision" => {
                control(
                    &h,
                    json!({"op":"rename","clientId":client,"name":"Renamed"}),
                );
            }
            "expiry" => {
                let oracle = rusqlite::Connection::open(h.root.join("remote/remote.db")).unwrap();
                oracle
                    .execute(
                        "UPDATE grants SET expires_at_ms=1 WHERE client_id=?1",
                        [&client],
                    )
                    .unwrap();
            }
            "stop" => h.sessions.shutdown(),
            "end" => state.end(),
            _ => unreachable!(),
        }
        assert_eq!(mounted(&h, &colab, Some(&cookie)), None, "{action}");
        assert!(state.ended(), "{action}");
    }
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
fn held_work_remains_approvable_after_last_transport_close_and_visible_to_later_tabs() {
    for observer in ["other", "reopened", "later"] {
        let now = Arc::new(Mutex::new(Instant::now()));
        let observed = Arc::clone(&now);
        let mut h = Harness::with_clock(
            FAST,
            tmt_remote::limits::SESSION_IDLE,
            Arc::new(move || *observed.lock().unwrap()),
        );
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
        let core = Core::new();
        fs::write(core.root.join("storage-root"), h.root.to_str().unwrap()).unwrap();
        let operations = core.operations();
        h.control.take().unwrap().stop();
        let approval = Arc::new(tmt_remote::approval::Approval::new(
            Arc::clone(&h.store),
            Arc::clone(&h.sessions),
            Arc::clone(&operations),
        ));
        h.control = Some(
            control::Control::start(
                &h._serving,
                Arc::clone(&h.pairing),
                Arc::clone(&h.devices),
                control::Door {
                    origin: h.origin.clone(),
                    prefix: h.prefix.clone(),
                },
                Some(approval),
                Arc::clone(&h.stop),
            )
            .unwrap(),
        );
        let open_observer = || {
            let reply = open_session(&h, &Opening::new(&h, &client, &device.key).wire());
            let wire: Value = serde_json::from_str(&reply.body).unwrap();
            wire["sessionId"].as_str().unwrap().to_owned()
        };
        let mut observed = if observer == "other" {
            Some(open_observer())
        } else {
            None
        };
        let state = h.sessions.context(Some(&cookie)).unwrap().session;
        drop(transport);
        detached(&state);
        let grace = tmt_remote::limits::SESSION_UNATTACHED_IDLE;
        *now.lock().unwrap() += grace - Duration::from_nanos(1);
        if let Some(observer) = &observed {
            // Keep the independent observer live while the originating session expires.
            assert!(
                h.sessions
                    .context_for(Some(&cookie), Some(observer))
                    .is_some()
            );
        }
        *now.lock().unwrap() += Duration::from_nanos(1);
        // Admission checks expiry even inside the maintenance scan interval.
        assert_eq!(
            payload(&session_probe(&h, &client, &device.key, session))["error"]["code"],
            "REMOTE_SESSION_ENDED"
        );
        assert!(state.ended());
        let grant = h.store.lock().unwrap().grant(&client).unwrap().unwrap();
        let held = h
            .store
            .lock()
            .unwrap()
            .owned(&grant, &id, now_ms())
            .unwrap();
        assert_eq!(held.phase, "held");
        assert!(held.frozen.is_some());
        if observer == "reopened" {
            observed = Some(open_observer());
        }
        // Exercise the actual `tmt remote approve --json` executable and its unchanged prompt.
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_tmt-remote"));
        command
            .env_clear()
            .env("HOME", &h.root)
            .env("TMT_EXECUTABLE", core.root.join("tmt"))
            .args(["approve", &id, "--json"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        writeln!(child.stdin.take().unwrap(), "{{\"op\":\"confirm\"}}").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("approve did not finish");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events[0]["event"], "held");
        assert!(events[0].get("sessionId").is_none());
        assert!(events[0].get("tab").is_none());
        assert_eq!(events[1]["state"], "accepted");
        assert_eq!(core.sends(), 1);
        let page = h
            .store
            .lock()
            .unwrap()
            .page(&grant, None, 50, now_ms())
            .unwrap();
        let envelope = &page["entries"].as_array().unwrap().last().unwrap()["envelope"];
        if let Some(observed) = &observed {
            assert_eq!(envelope["sessionId"], observed.as_str());
        }
        let published: Value = serde_json::from_slice(
            &canonical::base64url_decode(envelope["payload"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(published["state"], "accepted");
        assert_eq!(published["operationId"], id);
        crypto::verify_signature(
            &h.machine_public,
            &canonical::envelope(&Envelope {
                kind: "response",
                id: envelope["id"].as_str().unwrap(),
                correlation_id: envelope["correlationId"].as_str(),
                machine_id: envelope["machineId"].as_str().unwrap(),
                window_id: envelope["windowId"].as_str().unwrap(),
                client_id: envelope["clientId"].as_str().unwrap(),
                session_id: envelope["sessionId"].as_str().unwrap(),
                sequence: envelope["sequence"].as_str().unwrap(),
                timestamp_ms: envelope["timestampMs"].as_u64().unwrap(),
                origin: envelope["origin"].as_str().unwrap(),
                operation: "dispatch.create",
                payload: &canonical::base64url_decode(envelope["payload"].as_str().unwrap())
                    .unwrap(),
            })
            .unwrap(),
            &canonical::base64url_decode(envelope["signature"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        if observer == "later" {
            observed = Some(open_observer());
        }
        let mut probe = Opening::new(&h, &client, &device.key).wire();
        let intent = json!({"operationId":id}).to_string();
        probe["kind"] = json!("request");
        probe["operation"] = json!("operation.show");
        probe["sessionId"] = json!(observed.unwrap());
        probe["sequence"] = json!("1");
        probe["payload"] = json!(canonical::base64url(intent.as_bytes()));
        let signed = canonical::envelope(&Envelope {
            kind: "request",
            id: probe["id"].as_str().unwrap(),
            correlation_id: None,
            machine_id: &h.machine_id,
            window_id: &h.window_id,
            client_id: &client,
            session_id: probe["sessionId"].as_str().unwrap(),
            sequence: "1",
            timestamp_ms: probe["timestampMs"].as_u64().unwrap(),
            origin: &h.origin,
            operation: "operation.show",
            payload: intent.as_bytes(),
        })
        .unwrap();
        probe["signature"] = json!(canonical::base64url(&device.key.sign(&signed).to_bytes()));
        let transport =
            tmt_remote::transport::LoopbackTransport::new(Arc::clone(&h.sessions), 65536)
                .with_operations(Arc::clone(&operations));
        use tmt_remote::transport::Transport;
        let reply: Value = serde_json::from_slice(
            &transport
                .append(Some(&h.origin), probe.to_string().as_bytes())
                .unwrap(),
        )
        .unwrap();
        let recovered: Value = serde_json::from_slice(
            &canonical::base64url_decode(reply["payload"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered["state"], "accepted");
        assert_eq!(recovered["operationId"], id);
        assert_eq!(core.sends(), 1);
    }
}

#[test]
fn missing_talk_scope_is_a_signed_real_door_refusal_before_send_ownership() {
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 7);
    let client = paired(&h, &device);
    let narrowed = h.devices.talk(&client, false).unwrap();
    assert!(!narrowed.permits_scope("talk"));
    let opened = payload(&open_session(
        &h,
        &Opening::new(&h, &client, &device.key).wire(),
    ));
    let session = opened["sessionId"].as_str().unwrap();
    let mut wire = Opening::new(&h, &client, &device.key).wire();
    wire["kind"] = json!("request");
    wire["sessionId"] = json!(session);
    wire["sequence"] = json!("1");
    wire["operation"] = json!("dispatch.create");
    wire["payload"] = json!(canonical::base64url(b"{}"));
    let signed = canonical::envelope(&Envelope {
        kind: "request",
        id: wire["id"].as_str().unwrap(),
        correlation_id: None,
        machine_id: &h.machine_id,
        window_id: &h.window_id,
        client_id: &client,
        session_id: session,
        sequence: "1",
        timestamp_ms: wire["timestampMs"].as_u64().unwrap(),
        origin: &h.origin,
        operation: "dispatch.create",
        payload: b"{}",
    })
    .unwrap();
    wire["signature"] = json!(canonical::base64url(&device.key.sign(&signed).to_bytes()));
    let reply = open_session(&h, &wire);
    let error = payload(&reply);
    assert_eq!(error["error"]["code"], "REMOTE_SCOPE_DENIED");
    assert_eq!(error["error"]["scope"], "talk");
    assert_eq!(
        error["error"]["settingsUrl"],
        format!("{}/settings", h.origin)
    );
    let response: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(response["correlationId"], wire["id"]);
    let bytes = canonical::base64url_decode(response["payload"].as_str().unwrap()).unwrap();
    let text = |name: &str| response[name].as_str().unwrap();
    let signed = canonical::envelope(&Envelope {
        kind: "response",
        id: text("id"),
        correlation_id: Some(text("correlationId")),
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: text("sequence"),
        timestamp_ms: response["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: text("operation"),
        payload: &bytes,
    })
    .unwrap();
    crypto::verify_signature(
        &h.machine_public,
        &signed,
        &canonical::base64url_bytes(text("signature"), 64).unwrap(),
    )
    .unwrap();
    let oracle = rusqlite::Connection::open(h.root.join("remote/remote.db")).unwrap();
    let rows: i64 = oracle
        .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "missing sending scope never adopts a send");
}
