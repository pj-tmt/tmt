//! A serving door on real sockets for pairing and session acceptance: `/pair`,
//! door sessions on `/append`, serve's control socket and an owner-only mount
//! root, plus a Rust test device that signs with a fixed test key.
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
    time::{Duration, Instant},
};
use tmt_remote::{
    canonical::{self, Enrollment},
    control::{self, Control},
    crypto,
    devices::Devices,
    http::{Door, Handler},
    mount::{IdleClock, Mounts},
    pages::Pages,
    pairing::{Pairing, Timing},
    routes::Routes,
    session::{self, DoorSessions},
    site::Site,
    state::{Layout, MachineKey, Serving},
    store::{Store, uuid_v4},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub struct Harness {
    pub root: PathBuf,
    pub addr: SocketAddr,
    pub origin: String,
    pub prefix: String,
    pub machine_public: [u8; 32],
    pub store: Arc<Mutex<Store>>,
    pub pairing: Arc<Pairing>,
    pub sessions: Arc<DoorSessions>,
    pub devices: Arc<Devices>,
    pub machine_id: String,
    pub window_id: String,
    pub control: Option<Control>,
    pub stop: Arc<AtomicBool>,
    pub door: Option<JoinHandle<()>>,
    pub _serving: Serving,
}
impl Harness {
    pub fn new(timing: Timing) -> Self {
        Self::with_clock(timing, session::IDLE, Arc::new(Instant::now))
    }
    /// A serving door with a shared monotonic session clock.
    pub fn with_clock(timing: Timing, idle: Duration, clock: IdleClock) -> Self {
        // Short absolute root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!(
            "/tmp/tmt-1039-pair-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let serving = layout.serve_lock().unwrap();
        let machine_key = MachineKey::open(&layout).unwrap();
        let machine_public = machine_key.public();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let store = Arc::new(Mutex::new(store));
        let door = Door::bind(0).unwrap();
        let addr = door.socket_addr().unwrap();
        let origin = door.origin.clone();
        let window_id = uuid_v4().unwrap();
        let pairing = Arc::new(Pairing::new(
            machine.id.clone(),
            window_id.clone(),
            machine_public,
            origin.clone(),
            Arc::clone(&store),
            timing,
        ));
        let sessions = Arc::new(
            DoorSessions::new(
                machine.id.clone(),
                window_id.clone(),
                origin.clone(),
                format!("{}/x/", machine.route_prefix),
                machine_key,
                Arc::clone(&store),
                idle,
            )
            .with_clock(clock),
        );
        let devices = Arc::new(Devices::new(
            Arc::clone(&store),
            Some(Arc::clone(&sessions)),
        ));
        let routes = Routes::new(1024, machine.route_prefix.clone())
            .unwrap()
            .with_pairing(Arc::clone(&pairing))
            .with_sessions(Arc::clone(&sessions));
        let stop = Arc::new(AtomicBool::new(false));
        let control = Control::start(
            &serving,
            Arc::clone(&pairing),
            Arc::clone(&devices),
            control::Door {
                origin: origin.clone(),
                prefix: machine.route_prefix.clone(),
            },
            None,
            Arc::clone(&stop),
        )
        .unwrap();
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(Mounts::new(
                root.clone(),
                &origin,
                &machine.route_prefix,
                Arc::clone(&sessions) as _,
            )),
            pages: Some(Pages::new(
                &origin,
                machine.id.clone(),
                window_id.clone(),
                &machine.route_prefix,
            )),
        });
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
            sessions,
            devices,
            machine_id: machine.id,
            window_id,
            control: Some(control),
            stop,
            door: Some(door),
            _serving: serving,
        }
    }
    pub fn post(&self, body: &str, origin: Option<&str>) -> (u16, Value) {
        post_to(self.addr, &self.prefix, body, origin)
    }
    pub fn grants(&self) -> i64 {
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
pub struct Owner {
    pub stream: UnixStream,
    pub reader: BufReader<UnixStream>,
}
pub struct Offered {
    pub owner: Owner,
    pub code: [u8; 16],
    pub descriptor: Value,
    pub link: String,
}
impl Owner {
    pub fn next(&mut self) -> Value {
        let mut line = String::new();
        assert!(
            self.reader.read_line(&mut line).unwrap() > 0,
            "control closed"
        );
        serde_json::from_str(&line).unwrap()
    }
    pub fn answer(&mut self, op: &str) {
        writeln!(self.stream, "{}", json!({"op": op})).unwrap();
    }
}
/// Open an offer and decode what the owner was shown.
pub fn open(h: &Harness) -> Offered {
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
pub struct Device {
    pub key: SigningKey,
    pub kind: &'static str,
    pub origin: String,
    pub name: &'static str,
}
impl Device {
    pub fn browser(h: &Harness, seed: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[seed; 32]),
            kind: "browser",
            origin: h.origin.clone(),
            name: "Laptop é",
        }
    }
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    pub fn enrollment(&self, descriptor: &Value) -> Vec<u8> {
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
    pub fn body(&self, descriptor: &Value, code: &[u8; 16], sign: &SigningKey) -> Value {
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
pub fn hex16(text: &str) -> [u8; 16] {
    let mut bytes = [0; 16];
    for (i, pair) in text.as_bytes().chunks(2).enumerate() {
        bytes[i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    bytes
}

pub fn post_to(addr: SocketAddr, prefix: &str, body: &str, origin: Option<&str>) -> (u16, Value) {
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
pub fn submit(h: &Harness, body: Value, origin: Option<String>) -> JoinHandle<(u16, Value)> {
    let (addr, prefix) = (h.addr, h.prefix.clone());
    thread::spawn(move || post_to(addr, &prefix, &body.to_string(), origin.as_deref()))
}
