use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
#[path = "support/executable_fixture.rs"]
mod executable_fixture;

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-remote");
/// Bound on a `serve` child reporting readiness. Startup runs freshly written
/// fake-core executables, whose first exec waits behind the same assessment
/// queue as the fixture probe.
const STARTUP: Duration = executable_fixture::COMPLETION_WINDOW;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Pilot {
    root: PathBuf,
    child: Option<Child>,
}
impl Pilot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-613-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pilot = Self { root, child: None };
        let root = pilot.root.display();
        executable_fixture::write_executable(
            &pilot.root.join("core"),
            &format!(
                "printf '%s\\n' \"$*\" >> '{root}/calls'\ncat > '{root}/input'\nif grep -q storage.root '{root}/input'; then printf '{{\"dataRoot\":\"{root}/state\"}}'; else printf '{{\"version\":1,\"limits\":{{\"inputBytes\":1024,\"outputBytes\":4096}}}}'; fi\n"
            ),
        )
        .unwrap();
        pilot
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("TMT_EXECUTABLE", self.root.join("core"));
        cmd
    }
}
impl Drop for Pilot {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn exchange(socket: &str, request: &str) -> String {
    let mut client = TcpStream::connect(socket).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    response
}
#[test]
fn help_missing_core_and_invalid_options() {
    let pilot = Pilot::new();
    let a = pilot.command().args(["serve", "-h"]).output().unwrap();
    let b = pilot.command().args(["help", "serve"]).output().unwrap();
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert!(String::from_utf8(a.stdout).unwrap().contains("deny-all"));
    assert!(
        !pilot
            .command()
            .args(["serve", "--host", "0.0.0.0"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !pilot
            .command()
            .args(["serve", "--window-seconds", "60"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let missing = pilot
        .command()
        .env_remove("TMT_EXECUTABLE")
        .args(["serve", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        String::from_utf8(missing.stdout)
            .unwrap()
            .contains("REMOTE_CORE_UNAVAILABLE")
    );
    assert!(
        !pilot.root.join("calls").exists(),
        "help/options never start core"
    );
}
#[test]
fn startup_reads_capabilities_and_root_mounts_without_core_calls_and_sigterm_reaps() {
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        let child = pilot
            .command()
            .args(["serve", "--json"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        pilot.child = Some(child);
        let pipe = pilot.child.as_mut().unwrap().stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(pipe).read_line(&mut line).unwrap();
            let _ = tx.send(line);
        });
        let descriptor: Value = serde_json::from_str(&rx.recv_timeout(STARTUP).unwrap()).unwrap();
        reader.join().unwrap();
        assert_eq!(descriptor["state"], "closed");
        assert_eq!(descriptor["startupCoreCalls"], 2);
        let address = descriptor["address"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap();
        let (socket, prefix) = address.split_once('/').unwrap();
        for origin in [
            "",
            "Origin: null\r\n",
            "Origin: https://evil.example\r\n",
            "Origin: chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n",
        ] {
            let mut client = TcpStream::connect(socket).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client,"POST /{prefix}/append HTTP/1.1\r\nHost: {socket}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n{origin}\r\n{{}}").unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 404"));
        }
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n"
        );
        let request: Value =
            serde_json::from_slice(&fs::read(pilot.root.join("input")).unwrap()).unwrap();
        assert_eq!(request["operation"], "storage.root");
        // Startup creates only remote's private subtree beside the extension roots.
        let state: Vec<_> = fs::read_dir(pilot.root.join("state"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(state, ["remote"]);
        let remote = fs::symlink_metadata(pilot.root.join("state/remote")).unwrap();
        assert_eq!(remote.permissions().mode() & 0o777, 0o700);
        for name in ["machine.key", "remote.db", "serve.lock", "key.lock"] {
            let file = fs::symlink_metadata(pilot.root.join("state/remote").join(name)).unwrap();
            assert!(
                file.is_file() && file.permissions().mode() & 0o777 == 0o600,
                "{name}"
            );
        }
        assert_eq!(
            fs::metadata(pilot.root.join("state/remote/machine.key"))
                .unwrap()
                .len(),
            32
        );
        // The running process mounts colab once its owner-only socket appears.
        let get = format!("GET /x/colab/ HTTP/1.1\r\nHost: {socket}\r\n\r\n");
        assert!(exchange(socket, &get).starts_with("HTTP/1.1 404"));
        let directory = pilot.root.join("state/colab");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(directory.join("door.sock")).unwrap();
        fs::set_permissions(
            directory.join("door.sock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let extension = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\ncolab")
                .unwrap();
            String::from_utf8(head).unwrap()
        });
        assert!(exchange(socket, &get).ends_with("\r\n\r\ncolab"));
        assert!(extension.join().unwrap().starts_with("GET / HTTP/1.1\r\n"));
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n",
            "mounted traffic makes no core call"
        );
        let child = pilot.child.as_mut().unwrap();
        assert!(
            Command::new("/bin/kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "foreground process leaked");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(TcpStream::connect(socket).is_err());
        assert!(
            !pilot.root.join("data").exists(),
            "no request adoption/core DB"
        );
    }
}

/// Start `serve --json` and return the child with its descriptor.
fn serve(pilot: &Pilot) -> (Child, Value) {
    let mut child = pilot
        .command()
        .args(["serve", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pipe = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(pipe).read_line(&mut line).unwrap();
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(STARTUP).unwrap();
    (child, serde_json::from_str(&line).unwrap())
}
fn terminate(mut child: Child) {
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "foreground process leaked");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn machine_identity_and_route_prefix_survive_restart_and_one_serve_per_root() {
    let pilot = Pilot::new();
    let (first, before) = serve(&pilot);
    let prefix = |d: &Value| {
        d["address"]
            .as_str()
            .unwrap()
            .split_once("/r/")
            .unwrap()
            .1
            .to_owned()
    };
    // A second serve on the same data root refuses while the first runs.
    let second = pilot.command().args(["serve", "--json"]).output().unwrap();
    assert!(!second.status.success());
    assert!(
        String::from_utf8(second.stdout)
            .unwrap()
            .contains("REMOTE_ALREADY_SERVING")
    );
    let key = fs::read(pilot.root.join("state/remote/machine.key")).unwrap();
    terminate(first);
    let (restarted, after) = serve(&pilot);
    assert_eq!(prefix(&after), prefix(&before));
    assert_eq!(prefix(&before).len(), 32);
    assert_eq!(after["machineId"], before["machineId"]);
    assert_eq!(before["machineId"].as_str().unwrap().len(), 36);
    assert_eq!(
        fs::read(pilot.root.join("state/remote/machine.key")).unwrap(),
        key
    );
    terminate(restarted);
    // Unsafe state fails closed before the door binds.
    fs::set_permissions(
        pilot.root.join("state/remote"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let refused = pilot.command().args(["serve", "--json"]).output().unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8(refused.stdout)
            .unwrap()
            .contains("REMOTE_STATE_UNSAFE")
    );
}
/// Read one JSON line from a child's stdout within a bound.
fn line(reader: &std::sync::mpsc::Receiver<String>) -> Value {
    serde_json::from_str(&reader.recv_timeout(STARTUP).unwrap()).unwrap()
}
#[test]
fn pair_json_confirms_one_device_through_the_running_serve() {
    use ed25519_dalek::{Signer, SigningKey};
    use tmt_remote::{
        canonical::{self, Enrollment},
        crypto,
    };
    let pilot = Pilot::new();
    // Without a running serve there is nothing to pair with.
    let idle = pilot.command().args(["pair", "--json"]).output().unwrap();
    assert!(!idle.status.success());
    assert!(
        String::from_utf8(idle.stdout)
            .unwrap()
            .contains("REMOTE_NOT_RUNNING")
    );
    // A non-terminal without --json cannot show the owner a confirmation.
    let plain = pilot
        .command()
        .arg("pair")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!plain.status.success());
    let (server, descriptor) = serve(&pilot);
    let address = descriptor["address"].as_str().unwrap().to_owned();
    let (origin, prefix) = address.split_at(address.find("/r/").unwrap());
    let socket = origin.strip_prefix("http://").unwrap().to_owned();
    let mut pair = pilot
        .command()
        .args(["pair", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = pair.stdout.take().unwrap();
    let (tx, events) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let offer = line(&events);
    assert_eq!(offer["event"], "offer");
    let d = &offer["descriptor"];
    let code = canonical::pairing_code(offer["code"].as_str().unwrap()).unwrap();
    let key = SigningKey::from_bytes(&[5; 32]);
    let public = key.verifying_key().to_bytes();
    let mut challenge = [0; 16];
    for (i, pair) in d["serverChallenge"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks(2)
        .enumerate()
    {
        challenge[i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    let enrollment = canonical::enrollment(&Enrollment {
        machine_id: d["machineId"].as_str().unwrap(),
        window_id: d["windowId"].as_str().unwrap(),
        offer_id: d["offerId"].as_str().unwrap(),
        server_challenge: &challenge,
        client_nonce: &[1; 16],
        kind: "cli",
        origin: "cli",
        name: "Other machine",
        public_key: &public,
    })
    .unwrap();
    let mac = crypto::enrollment_mac(&code, &enrollment);
    let signature = key.sign(&canonical::possession(&enrollment, &mac).unwrap());
    let body = serde_json::json!({
        "profile": "local-v1", "machineId": d["machineId"], "windowId": d["windowId"],
        "offerId": d["offerId"], "serverChallenge": d["serverChallenge"],
        "clientNonce": "01".repeat(16), "kind": "cli", "origin": "cli", "name": "Other machine",
        "publicKey": canonical::base64url(&public), "mac": canonical::base64url(&mac),
        "signature": canonical::base64url(&signature.to_bytes()),
    })
    .to_string();
    let device = {
        let (socket, prefix) = (socket.clone(), prefix.to_owned());
        std::thread::spawn(move || {
            exchange(
                &socket,
                &format!(
                    "POST {prefix}/pair HTTP/1.1\r\nHost: {socket}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
            )
        })
    };
    let candidate = line(&events);
    assert_eq!(candidate["event"], "candidate");
    assert_eq!(
        candidate["words"],
        serde_json::json!(canonical::fingerprint_words(&public).unwrap())
    );
    pair.stdin.take().unwrap().write_all(b"confirm\n").unwrap();
    let ended = line(&events);
    assert_eq!(ended["reason"], "paired");
    assert!(pair.wait().unwrap().success());
    assert!(device.join().unwrap().starts_with("HTTP/1.1 200"));
    terminate(server);
    let grants: i64 = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(grants, 1);
    assert!(!pilot.root.join("state/remote/control.sock").exists());
}
