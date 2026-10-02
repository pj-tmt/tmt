//! Pairing acceptance on real sockets: a door with `/pair`, serve's control
//! socket, and a Rust test device that signs with a fixed test key.
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tmt_remote::{
    canonical::{self, Enrollment},
    control::{self, Control},
    crypto,
    http::{Door, Handler},
    mount::{Mounts, NoSessions},
    pairing::{Pairing, Timing},
    routes::Routes,
    site::Site,
    state::{Layout, MachineKey, Serving},
    store::{Grant, Store, uuid_v4},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Harness {
    root: PathBuf,
    addr: SocketAddr,
    origin: String,
    prefix: String,
    machine_public: [u8; 32],
    store: Arc<Mutex<Store>>,
    pairing: Arc<Pairing>,
    control: Option<Control>,
    stop: Arc<AtomicBool>,
    door: Option<JoinHandle<()>>,
    _serving: Serving,
}
impl Harness {
    fn new(timing: Timing) -> Self {
        // Short absolute root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!(
            "/tmp/tmt-1039-pair-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let serving = layout.serve_lock().unwrap();
        let machine_public = MachineKey::open(&layout).unwrap().public();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let store = Arc::new(Mutex::new(store));
        let pairing = Arc::new(Pairing::new(
            machine.id.clone(),
            uuid_v4().unwrap(),
            machine_public,
            Arc::clone(&store),
            timing,
        ));
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        let origin = door.origin.clone();
        let routes = Routes::new(1024, machine.route_prefix.clone())
            .unwrap()
            .with_pairing(Arc::clone(&pairing));
        let control = Control::start(
            &serving,
            Arc::clone(&pairing),
            control::Door {
                origin: origin.clone(),
                prefix: machine.route_prefix.clone(),
            },
        )
        .unwrap();
        let site = Arc::new(Site {
            routes,
            mounts: Mounts::new(root.clone(), &origin, Arc::new(NoSessions)),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let door = thread::spawn(move || door.run(&flag, site as Arc<dyn Handler>).unwrap());
        Self {
            root,
            addr,
            origin,
            prefix: machine.route_prefix,
            machine_public,
            store,
            pairing,
            control: Some(control),
            stop,
            door: Some(door),
            _serving: serving,
        }
    }
    fn post(&self, body: &str, origin: Option<&str>) -> (u16, Value) {
        post_to(self.addr, &self.prefix, body, origin)
    }
    fn grants(&self) -> i64 {
        rusqlite::Connection::open(self.root.join("remote/remote.db"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM grants", [], |r| r.get(0))
            .unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.control.take().unwrap().stop();
        self.stop.store(true, Ordering::Release);
        self.door.take().unwrap().join().unwrap();
        assert!(!self.root.join("remote/control.sock").exists());
        fs::remove_dir_all(&self.root).unwrap();
    }
}
struct Owner {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}
struct Offered {
    owner: Owner,
    code: [u8; 16],
    descriptor: Value,
    link: String,
}
impl Owner {
    fn next(&mut self) -> Value {
        let mut line = String::new();
        assert!(
            self.reader.read_line(&mut line).unwrap() > 0,
            "control closed"
        );
        serde_json::from_str(&line).unwrap()
    }
    fn answer(&mut self, op: &str) {
        writeln!(self.stream, "{}", json!({"op": op})).unwrap();
    }
}
/// Open an offer and decode what the owner was shown.
fn open(h: &Harness) -> Offered {
    let mut stream = control::connect(&h.root.join("remote")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(b"{\"op\":\"pair\"}\n").unwrap();
    let mut owner = Owner {
        reader: BufReader::new(stream.try_clone().unwrap()),
        stream,
    };
    let offer = owner.next();
    assert_eq!(offer["event"], "offer");
    Offered {
        code: canonical::pairing_code(offer["code"].as_str().unwrap()).unwrap(),
        descriptor: offer["descriptor"].clone(),
        link: offer["link"].as_str().unwrap().to_owned(),
        owner,
    }
}
struct Device {
    key: SigningKey,
    kind: &'static str,
    origin: String,
    name: &'static str,
}
impl Device {
    fn browser(h: &Harness, seed: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[seed; 32]),
            kind: "browser",
            origin: h.origin.clone(),
            name: "Laptop é",
        }
    }
    fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    fn enrollment(&self, descriptor: &Value) -> Vec<u8> {
        let challenge = hex16(descriptor["serverChallenge"].as_str().unwrap());
        canonical::enrollment(&Enrollment {
            machine_id: descriptor["machineId"].as_str().unwrap(),
            window_id: descriptor["windowId"].as_str().unwrap(),
            offer_id: descriptor["offerId"].as_str().unwrap(),
            server_challenge: &challenge,
            client_nonce: &[9; 16],
            kind: self.kind,
            origin: &self.origin,
            name: self.name,
            public_key: &self.public(),
        })
        .unwrap()
    }
    /// The wire body; `code` keys the MAC, `sign` chooses the possession signer.
    fn body(&self, descriptor: &Value, code: &[u8; 16], sign: &SigningKey) -> Value {
        let enrollment = self.enrollment(descriptor);
        let mac = crypto::enrollment_mac(code, &enrollment);
        let signature = sign.sign(&canonical::possession(&enrollment, &mac).unwrap());
        json!({
            "profile": "local-v1",
            "machineId": descriptor["machineId"],
            "windowId": descriptor["windowId"],
            "offerId": descriptor["offerId"],
            "serverChallenge": descriptor["serverChallenge"],
            "clientNonce": "09".repeat(16),
            "kind": self.kind,
            "origin": self.origin,
            "name": self.name,
            "publicKey": canonical::base64url(&self.public()),
            "mac": canonical::base64url(&mac),
            "signature": canonical::base64url(&signature.to_bytes()),
        })
    }
}
fn hex16(text: &str) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (i, pair) in text.as_bytes().chunks(2).enumerate() {
        bytes[i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    bytes
}
const FAST: Timing = Timing {
    lifetime: Duration::from_secs(30),
    submit_wait: Duration::from_secs(10),
};

#[test]
fn confirmed_pairing_issues_one_grant_and_a_verifiable_receipt() {
    let h = Harness::new(FAST);
    let Offered {
        mut owner,
        code,
        descriptor,
        link,
    } = open(&h);
    // The link carries the descriptor in its path and the code only in its fragment.
    let (path, fragment) = link.split_once('#').unwrap();
    assert!(path.starts_with(&format!("{}/pair/", h.origin)));
    assert_eq!(canonical::pairing_code(fragment).unwrap(), code);
    let encoded = path.rsplit('/').next().unwrap();
    let shown: Value =
        serde_json::from_slice(&canonical::base64url_decode(encoded).unwrap()).unwrap();
    assert_eq!(shown, descriptor);
    assert_eq!(descriptor["address"], format!("{}{}", h.origin, h.prefix));
    assert_eq!(descriptor["serverChallenge"].as_str().unwrap().len(), 32);
    let device = Device::browser(&h, 7);
    let body = device.body(&descriptor, &code, &device.key).to_string();
    let submit = {
        let (addr, prefix, origin) = (h.addr, h.prefix.clone(), h.origin.clone());
        let body = body.clone();
        thread::spawn(move || post_to(addr, &prefix, &body, Some(&origin)))
    };
    let candidate = owner.next();
    assert_eq!(candidate["event"], "candidate");
    assert_eq!(candidate["kind"], "browser");
    assert_eq!(candidate["origin"], h.origin.as_str());
    assert_eq!(candidate["name"], "Laptop é");
    assert_eq!(
        candidate["words"],
        json!(canonical::fingerprint_words(&device.public()).unwrap())
    );
    assert_eq!(h.grants(), 0, "no grant before confirmation");
    owner.answer("confirm");
    let ended = owner.next();
    assert_eq!(ended["reason"], "paired");
    let (status, reply) = submit.join().unwrap();
    assert_eq!(status, 200);
    let receipt = canonical::base64url_decode(reply["receipt"].as_str().unwrap()).unwrap();
    let proof = canonical::base64url_bytes(reply["serverProof"].as_str().unwrap(), 32).unwrap();
    // The device derives K_response from its code and checks the exact receipt bytes.
    let enrollment = device.enrollment(&descriptor);
    let key = crypto::response_key(&code, &enrollment).unwrap();
    assert!(crypto::verify_server_proof(&key, &receipt, &proof).is_ok());
    assert!(
        crypto::verify_server_proof(
            &crypto::response_key(&[0; 16], &enrollment).unwrap(),
            &receipt,
            &proof
        )
        .is_err()
    );
    let parsed: Value = serde_json::from_slice(&receipt).unwrap();
    let grant = &parsed["grant"];
    assert_eq!(grant["clientId"], ended["clientId"]);
    assert_eq!(grant["machineId"], descriptor["machineId"]);
    assert_eq!(grant["profile"], "local-v1");
    assert_eq!(grant["publicKey"], canonical::base64url(&device.public()));
    assert_eq!(grant["kind"], "browser");
    assert_eq!(grant["origin"], h.origin.as_str());
    assert_eq!(grant["agents"], "all");
    assert_eq!(grant["mode"], "direct");
    assert_eq!(grant["expiresAtMs"], Value::Null);
    assert_eq!(grant["revision"], 1);
    assert_eq!(grant["disabled"], false);
    assert_eq!(
        grant["scopes"],
        json!([
            "agents.read",
            "check.read",
            "results.read",
            "status.read",
            "talk"
        ])
    );
    assert_eq!(
        parsed["machinePublicKey"],
        canonical::base64url(&h.machine_public)
    );
    let stored: Grant = h
        .store
        .lock()
        .unwrap()
        .grant(grant["clientId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.public_key, device.public());
    assert_eq!(h.grants(), 1);
    // An exact retry recovers the same receipt; nothing else is accepted.
    assert_eq!(h.post(&body, Some(&h.origin)), (200, reply.clone()));
    assert_eq!(h.grants(), 1, "retry creates no second grant");
    let other = Device::browser(&h, 8);
    assert_eq!(
        h.post(
            &other.body(&descriptor, &code, &other.key).to_string(),
            Some(&h.origin)
        )
        .0,
        404
    );
}

fn post_to(addr: SocketAddr, prefix: &str, body: &str, origin: Option<&str>) -> (u16, Value) {
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    write!(
        client,
        "POST {prefix}/pair HTTP/1.1\r\nHost: {addr}\r\n{origin}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut reply = String::new();
    client.read_to_string(&mut reply).unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    (
        head[9..12].parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}

/// Submit in the background while the owner side is driven.
fn submit(h: &Harness, body: Value, origin: Option<String>) -> JoinHandle<(u16, Value)> {
    let (addr, prefix) = (h.addr, h.prefix.clone());
    thread::spawn(move || post_to(addr, &prefix, &body.to_string(), origin.as_deref()))
}

#[test]
fn three_failed_code_proofs_cancel_the_offer() {
    let h = Harness::new(FAST);
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let device = Device::browser(&h, 7);
    let mut wrong = code;
    wrong[0] ^= 1;
    let origin = Some(h.origin.as_str());
    for _ in 0..2 {
        let bad = device.body(&descriptor, &wrong, &device.key).to_string();
        assert_eq!(h.post(&bad, origin).0, 404);
    }
    // Two failures leave the offer open: a correct candidate still pins.
    let pending = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("refuse");
    assert_eq!(owner.next()["reason"], "refused");
    assert_eq!(pending.join().unwrap().0, 404);
    // A fresh offer cancels on the third failure; the right code then refuses.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    for _ in 0..3 {
        let bad = device.body(&descriptor, &wrong, &device.key).to_string();
        assert_eq!(h.post(&bad, origin).0, 404);
    }
    assert_eq!(owner.next()["reason"], "too-many-failures");
    let good = device.body(&descriptor, &code, &device.key).to_string();
    assert_eq!(h.post(&good, origin).0, 404);
    assert_eq!(h.grants(), 0);
}

#[test]
fn wrong_possession_origin_and_fields_refuse_without_pinning() {
    let h = Harness::new(Timing {
        lifetime: Duration::from_secs(30),
        submit_wait: Duration::from_millis(300),
    });
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let device = Device::browser(&h, 7);
    let origin = Some(h.origin.as_str());
    // Signed by another key: possession fails and nothing is pinned.
    let impostor = SigningKey::from_bytes(&[3; 32]);
    let forged = device.body(&descriptor, &code, &impostor).to_string();
    assert_eq!(h.post(&forged, origin).0, 404);
    let good = device.body(&descriptor, &code, &device.key);
    let mut cases = vec![
        (good.to_string(), None, "missing Origin"),
        (
            good.to_string(),
            Some("http://127.0.0.1:1".to_owned()),
            "other Origin",
        ),
    ];
    for (field, value) in [
        ("machineId", json!(uuid_v4().unwrap())),
        ("windowId", json!(uuid_v4().unwrap())),
        ("offerId", json!(uuid_v4().unwrap())),
        ("serverChallenge", json!("00".repeat(16))),
        ("profile", json!("cloud-v1")),
        (
            "publicKey",
            json!(format!("{}=", good["publicKey"].as_str().unwrap())),
        ),
        ("mac", json!(1)),
        ("extra", json!("field")),
    ] {
        let mut changed = good.clone();
        changed[field] = value;
        cases.push((changed.to_string(), Some(h.origin.clone()), field));
    }
    cases.push(("not json".into(), Some(h.origin.clone()), "framing"));
    for (body, header, label) in cases {
        assert_eq!(h.post(&body, header.as_deref()).0, 404, "{label}");
    }
    // None of those pinned or ended the offer: the valid candidate pins, then
    // reports pending when the owner has not answered within the wait.
    let (status, body) = h.post(&good.to_string(), origin);
    assert_eq!((status, body), (202, json!({"state": "pending"})));
    assert_eq!(owner.next()["event"], "candidate");
    // A competing candidate cannot replace the pinned one.
    let other = Device::browser(&h, 8);
    assert_eq!(
        h.post(
            &other.body(&descriptor, &code, &other.key).to_string(),
            origin
        )
        .0,
        404
    );
    // The identical candidate coalesces and receives the receipt once confirmed.
    let retry = submit(&h, good.clone(), Some(h.origin.clone()));
    thread::sleep(Duration::from_millis(50));
    owner.answer("confirm");
    assert_eq!(owner.next()["reason"], "paired");
    assert_eq!(retry.join().unwrap().0, 200);
    assert_eq!(h.grants(), 1);
}

#[test]
fn refusal_disconnect_expiry_and_stop_end_the_offer_without_a_grant() {
    let h = Harness::new(Timing {
        lifetime: Duration::from_millis(800),
        submit_wait: Duration::from_secs(5),
    });
    let device = Device::browser(&h, 7);
    // Owner refuses: the waiting device is refused.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("refuse");
    assert_eq!(owner.next()["reason"], "refused");
    assert_eq!(waiting.join().unwrap().0, 404);
    // The pairing command exits: the offer is cancelled.
    let Offered {
        owner,
        code,
        descriptor,
        ..
    } = open(&h);
    drop(owner);
    thread::sleep(Duration::from_millis(200));
    let body = device.body(&descriptor, &code, &device.key).to_string();
    assert_eq!(h.post(&body, Some(&h.origin)).0, 404);
    // No confirmation before the deadline: expiry ends it.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    assert_eq!(owner.next()["reason"], "expired");
    assert_eq!(waiting.join().unwrap().0, 404);
    // A new offer replaces the previous one explicitly.
    let Offered {
        owner: mut first, ..
    } = open(&h);
    let _second = open(&h);
    assert_eq!(first.next()["reason"], "replaced");
    assert_eq!(h.grants(), 0);
}

#[test]
fn enrollment_is_atomic_when_the_grant_cannot_be_written() {
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 7);
    // A live grant for the same device key already exists.
    h.store
        .lock()
        .unwrap()
        .insert_grant(&Grant {
            client_id: uuid_v4().unwrap(),
            public_key: device.public(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Earlier".into(),
            agents: "all".into(),
            scopes: vec!["talk".into()],
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        })
        .unwrap();
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("confirm");
    let ended = owner.next();
    assert_eq!(ended["reason"], "failed");
    assert!(ended["message"].as_str().unwrap().contains("live grant"));
    assert_eq!(waiting.join().unwrap().0, 404);
    assert_eq!(h.grants(), 1, "no partial second grant");
}

#[test]
fn stopping_serve_cancels_a_pending_offer() {
    let mut h = Harness::new(FAST);
    let Offered { mut owner, .. } = open(&h);
    let control = h.control.take().unwrap();
    control.stop();
    assert_eq!(owner.next()["reason"], "cancelled");
    assert!(!h.root.join("remote/control.sock").exists());
    h.control = Some(
        Control::start(
            &h._serving,
            Arc::clone(&h.pairing),
            control::Door {
                origin: h.origin.clone(),
                prefix: h.prefix.clone(),
            },
        )
        .unwrap(),
    );
}
