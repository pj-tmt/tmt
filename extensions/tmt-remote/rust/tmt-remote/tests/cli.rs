use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
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
    background: bool,
    pending_launcher: bool,
    cleanup_observed: Option<Arc<AtomicBool>>,
    private_workers: Vec<PrivateWorker>,
}
impl Pilot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-613-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pilot = Self {
            root,
            child: None,
            background: false,
            pending_launcher: false,
            cleanup_observed: None,
            private_workers: Vec::new(),
        };
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
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TMPDIR", &self.root)
            .env("XDG_STATE_HOME", self.root.join("xdg-state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("TMT_EXECUTABLE", self.root.join("core"));
        cmd
    }
    fn background_output(&mut self, options: &[&str]) -> std::process::Output {
        if let Some(child) = &mut self.child {
            assert!(
                child.try_wait().unwrap().is_some(),
                "never replace an unreaped fixture child"
            );
        }
        self.background = true;
        self.pending_launcher = true;
        self.child = Some(
            self.command()
                .args(["serve", "--background", "--json"])
                .args(options)
                .stdout(fs::File::create(self.root.join("launcher.json")).unwrap())
                .stderr(fs::File::create(self.root.join("launcher.stderr")).unwrap())
                .spawn()
                .unwrap(),
        );
        let status = reap_until(
            self.child.as_mut().unwrap(),
            Instant::now() + STARTUP + Duration::from_secs(45),
        )
        .expect("background launcher did not complete within startup/cleanup bound");
        let stdout = fs::read(self.root.join("launcher.json")).unwrap();
        let stderr = fs::read(self.root.join("launcher.stderr")).unwrap();
        let value: Value = serde_json::from_slice(&stdout).unwrap();
        // These completed parent results imply either completed handoff or the
        // worker's cleanup confirmation plus reap. Ambiguity keeps the guard.
        if (status.success() && value["state"] == "ready")
            || matches!(
                value["error"]["code"].as_str(),
                Some(
                    "REMOTE_ALREADY_SERVING"
                        | "REMOTE_STATE_UNSAFE"
                        | "REMOTE_PORT_BUSY"
                        | "REMOTE_STARTUP_CANCELLED"
                )
            )
        {
            self.pending_launcher = false;
        }
        std::process::Output {
            status,
            stdout,
            stderr,
        }
    }
    // A launcher result is proof only after its own production cancellation
    // owner reports confirmed pre-Accept cleanup and the exact child is reaped.
    fn cancel_launcher(&mut self) -> Option<std::process::Output> {
        // Any missing/malformed completion may include an accepted service.
        // Attempt its existing stop owner, while retaining uncertainty.
        let previously_admitted = self.background;
        self.background = true;
        let child = self.child.as_mut()?;
        if matches!(child.try_wait(), Ok(None)) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id() as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        let status = reap_until(child, Instant::now() + Duration::from_secs(45))?;
        let stdout = fs::read(self.root.join("launcher.json")).ok()?;
        let stderr = fs::read(self.root.join("launcher.stderr")).ok()?;
        let value: Value = serde_json::from_slice(&stdout).ok()?;
        if status.success() || value["error"]["code"] != "REMOTE_STARTUP_CANCELLED" {
            // An accepted/ambiguous result is not pre-state cleanup evidence.
            // Stop can handle a possibly admitted service, but uncertainty still
            // retains the root rather than implying all invocations are gone.
            return None;
        }
        eprintln!(
            "owned launcher reaped with {}: {}",
            value["error"]["code"], status
        );
        self.pending_launcher = false;
        self.background = previously_admitted;
        Some(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }
}
impl Drop for Pilot {
    fn drop(&mut self) {
        let mut confirmed = !self.pending_launcher || self.cancel_launcher().is_some();
        for worker in &mut self.private_workers {
            if !worker.accepted {
                confirmed &= worker.cancel_and_reap();
            }
        }
        if self.background {
            // A detached worker is not owned by the reaped launcher PID. Stop
            // only this admitted disposable root and confirm lease/socket cleanup.
            let mut clean = false;
            if let Ok(mut stop) = self
                .command()
                .args(["stop", "--json"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                let deadline = Instant::now() + Duration::from_secs(60);
                loop {
                    match stop.try_wait() {
                        Ok(Some(status)) => {
                            clean = status.success();
                            break;
                        }
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(20))
                        }
                        _ => {
                            let _ = stop.kill();
                            let reap_deadline = Instant::now() + Duration::from_secs(1);
                            while Instant::now() < reap_deadline {
                                if matches!(stop.try_wait(), Ok(Some(_))) {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            break;
                        }
                    }
                }
            }
            let released = tmt_remote::state::Layout::existing(&self.root.join("state"))
                .and_then(|layout| match layout {
                    Some(layout) => layout.existing_serve_lock().map(|lease| {
                        drop(lease);
                        true
                    }),
                    None => Ok(true),
                })
                .unwrap_or(false);
            if !clean || !released || self.root.join("state/remote/control.sock").exists() {
                confirmed = false;
            }
        }
        for worker in &mut self.private_workers {
            if worker.accepted {
                confirmed &= reap_until(&mut worker.child, Instant::now() + Duration::from_secs(2))
                    .is_some();
            }
        }
        if let Some(mut child) = self.child.take() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            confirmed &= reap_until(&mut child, Instant::now() + Duration::from_secs(1)).is_some();
        }
        if confirmed {
            if let Some(observed) = &self.cleanup_observed {
                eprintln!(
                    "fixture cleanup confirmed before root removal: {}",
                    self.root.display()
                );
                observed.store(true, Ordering::SeqCst);
            }
            if fs::remove_dir_all(&self.root).is_ok() {
                return;
            }
        }
        eprintln!(
            "fixture cleanup unconfirmed; retained {}",
            self.root.display()
        );
        if !std::thread::panicking() {
            panic!("owned fixture did not confirm cleanup");
        }
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
    assert!(
        String::from_utf8(a.stdout)
            .unwrap()
            .contains("owner-device")
    );
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
        assert_eq!(descriptor["state"], "ready");
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
        // With no listener or object request, startup opens only Remote's ledger subtree.
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
        let get = format!("GET /{prefix}/x/colab/ HTTP/1.1\r\nHost: {socket}\r\n\r\n");
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
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait().unwrap();
            panic!("foreground process leaked");
        }
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
    assert_eq!(prefix(&before).len(), 16);
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
fn pair_json_confirms_one_device_and_grant_survives_control_stop_restart() {
    use ed25519_dalek::{Signer, SigningKey};
    use tmt_remote::{
        canonical::{self, Enrollment},
        crypto,
    };
    let mut pilot = Pilot::new();
    pilot.background = true;
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
    let descriptor: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--background"], false)).unwrap();
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
    // Device management reaches state through the running serve, and through
    // the serve lock once it stops.
    fn devices(pilot: &Pilot, args: &[&str]) -> (bool, Value) {
        let output = pilot.command().arg("devices").args(args).output().unwrap();
        let answer: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
        (output.status.success(), answer)
    }
    let (listed, running) = devices(&pilot, &["--json"]);
    assert!(listed);
    let client_id = ended["clientId"].as_str().unwrap();
    assert_eq!(running["devices"][0]["clientId"], client_id);
    assert_eq!(running["devices"][0]["kind"], "cli");
    let (renamed, name) = devices(&pilot, &["rename", client_id, "Travel laptop", "--json"]);
    assert!(renamed);
    assert_eq!(name["device"]["name"], "Travel laptop");
    assert_eq!(name["device"]["clientId"], client_id);
    assert_eq!(name["device"]["revision"], 2);
    for field in [
        "kind",
        "origin",
        "words",
        "agents",
        "scopes",
        "mode",
        "issuedAtMs",
        "expiresAtMs",
        "revoked",
    ] {
        assert_eq!(
            name["device"][field], running["devices"][0][field],
            "{field} changed"
        );
    }
    let (repeated, same) = devices(&pilot, &["rename", client_id, "Travel laptop", "--json"]);
    assert!(repeated);
    assert_eq!(same, name);
    for invalid in ["", " \u{feff}", "a\nb", &"é".repeat(33)] {
        let (accepted, bad) = devices(&pilot, &["rename", client_id, invalid, "--json"]);
        assert!(!accepted);
        assert_eq!(bad["error"]["code"], "REMOTE_INPUT_INVALID");
    }
    // A serve names an unknown operation with its own code, so clients match
    // on the code rather than on `REMOTE_INPUT_INVALID` text.
    let mut control = UnixStream::connect(pilot.root.join("state/remote/control.sock")).unwrap();
    control
        .write_all(b"{\"op\":\"from-the-future\"}\n")
        .unwrap();
    let mut reply = String::new();
    BufReader::new(control).read_line(&mut reply).unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["error"]["code"], "REMOTE_CONTROL_UNSUPPORTED");
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
    assert_eq!(status_json(&pilot)["running"], false);
    let (listed, stopped) = devices(&pilot, &["--json"]);
    assert!(listed);
    assert_eq!(stopped["devices"][0], name["device"]);
    // Real enrollment and its complete grant, rather than a seeded count,
    // survive a new window at the same origin.
    let next: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--background"], false)).unwrap();
    assert_eq!(next["address"], descriptor["address"]);
    assert_ne!(next["windowId"], descriptor["windowId"]);
    let (listed, after_restart) = devices(&pilot, &["--json"]);
    assert!(listed);
    assert_eq!(after_restart["devices"][0], name["device"]);
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
    let (renamed, offline) = devices(&pilot, &["rename", client_id, "Home laptop", "--json"]);
    assert!(renamed);
    assert_eq!(offline["device"]["name"], "Home laptop");
    assert_eq!(offline["device"]["revision"], 3);
    let (revoked, answer) = devices(&pilot, &["revoke", client_id, "--json"]);
    assert!(revoked);
    assert_eq!(answer["device"]["revoked"], true);
    assert_eq!(answer["device"]["revision"], 4);
    let (renamed, disabled) = devices(&pilot, &["rename", client_id, "Revived", "--json"]);
    assert!(!renamed);
    assert_eq!(disabled["error"]["code"], "REMOTE_DEVICE_REVOKED");
    let (renamed, missing) = devices(
        &pilot,
        &[
            "rename",
            "00000000-0000-4000-8000-000000000009",
            "Missing",
            "--json",
        ],
    );
    assert!(!renamed);
    assert_eq!(missing["error"]["code"], "REMOTE_DEVICE_NOT_FOUND");
    let (found, missing) = devices(
        &pilot,
        &["revoke", "00000000-0000-4000-8000-000000000009", "--json"],
    );
    assert!(!found);
    assert_eq!(missing["error"]["code"], "REMOTE_DEVICE_NOT_FOUND");
    let grants: i64 = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM grants", [], |r| r.get(0))
        .unwrap();
    assert_eq!(grants, 1);
    assert!(!pilot.root.join("state/remote/control.sock").exists());
}

/// A bounded HTTP callback fixture for actual foreground-process composition.
/// Failed production attempts are retried with up to 30 s backoff. Allow that
/// retry plus scheduler delay; receiving the event, not elapsed time, is readiness.
const DEVICE_EVENT_WINDOW: Duration = Duration::from_secs(60);
struct DeviceCallback {
    events: mpsc::Receiver<Result<Value, String>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl DeviceCallback {
    fn serve(root: &std::path::Path) -> Self {
        let directory = root.join("colab");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.join("door.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let (sent, events) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let result = match listener.accept() {
                    Ok((mut stream, _)) => Self::receive(&mut stream, &flag),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => Err(format!("callback listener failed: {e}")),
                };
                match result {
                    Ok(None) => continue,
                    Ok(Some(event)) => {
                        if sent.send(Ok(event)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sent.send(Err(error));
                        break;
                    }
                }
            }
        });
        Self {
            events,
            stop,
            thread: Some(thread),
        }
    }
    fn receive(stream: &mut UnixStream, stop: &AtomicBool) -> Result<Option<Value>, String> {
        // Readiness waits only check cancellation. Preserve partial framing
        // across them, with one absolute acquisition deadline. Nonblocking I/O
        // also handles a peer that has already closed before accept completes.
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = std::str::from_utf8(&bytes[..end]).map_err(|e| e.to_string())?;
                let mut lines = head.split("\r\n");
                let request = lines.next();
                // This device-event-only peer has no object adapter. Refuse its
                // distinct setup request without treating it as a device event.
                if request == Some("GET /.tmt/remote/object-channel-v1 HTTP/1.1") {
                    let _ =
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    return Ok(None);
                }
                if request != Some("POST /.tmt/remote/device-events HTTP/1.1") {
                    return Err("unexpected device-event request line".into());
                }
                let mut marker = false;
                let mut length = None;
                for line in lines {
                    marker |= line == "tmt-device-event: 1";
                    if let Some(value) = line.strip_prefix("Content-Length: ") {
                        length = Some(value.parse::<usize>().map_err(|e| e.to_string())?);
                    }
                }
                let length = length.ok_or("missing callback Content-Length")?;
                if !marker || length > 4096 || end > 8192 {
                    return Err("invalid device-event marker or size".into());
                }
                let body = end + 4;
                if bytes.len() >= body + length {
                    let event = serde_json::from_slice(&bytes[body..body + length])
                        .map_err(|e| format!("invalid device-event JSON: {e}"))?;
                    // An attempt may expire before the fixture is scheduled.
                    // Its reply can fail; the durable snapshot will be replayed.
                    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
                    return Ok(Some(event));
                }
            } else if bytes.len() > 8192 {
                return Err("device-event head exceeds fixture bound".into());
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(None), // Incomplete attempt; serve retries it.
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    use nix::poll::{PollFd, PollFlags, poll};
                    use std::os::fd::AsFd;
                    let mut ready = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
                    match poll(&mut ready, 100u16) {
                        Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                        Err(e) => return Err(format!("callback readiness failed: {e}")),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(None),
                Err(e) => return Err(format!("callback read failed: {e}")),
            }
        }
        Ok(None)
    }
    fn next_within(&self, window: Duration) -> Result<Value, String> {
        self.events.recv_timeout(window).map_err(|e| {
            format!("foreground did not deliver device event within {window:?}: {e}")
        })?
    }
    fn next(&self) -> Value {
        self.next_within(DEVICE_EVENT_WINDOW)
            .unwrap_or_else(|e| panic!("{e}"))
    }
}
impl Drop for DeviceCallback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // The scenario reports failure through next(). Never panic again
            // while its assertion is unwinding, but always join the worker.
            if thread.join().is_err() {
                eprintln!("device callback worker panicked during cleanup");
            }
        }
    }
}

#[test]
fn device_callback_retries_incomplete_requests_and_reports_protocol_errors() {
    let pilot = Pilot::new();
    let callback = DeviceCallback::serve(&pilot.root);
    let socket = pilot.root.join("colab/door.sock");
    {
        let mut abandoned = UnixStream::connect(&socket).unwrap();
        abandoned.write_all(b"POST /.tmt/").unwrap();
    }
    let event = serde_json::json!({"type":"device.revoked","grantRevision":3});
    let body = event.to_string();
    let mut stream = UnixStream::connect(&socket).unwrap_or_else(|e| {
        panic!(
            "callback connect: {e}; worker: {:?}",
            callback.next_within(Duration::from_millis(100))
        )
    });
    write!(stream, "POST /.tmt/remote/device-events HTTP/1.1\r\ntmt-device-event: 1\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    assert_eq!(callback.next(), event);
    let mut invalid = UnixStream::connect(&socket).unwrap();
    invalid
        .write_all(b"GET /wrong HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .unwrap();
    assert_eq!(
        callback.next_within(DEVICE_EVENT_WINDOW).unwrap_err(),
        "unexpected device-event request line"
    );
    drop(callback);
}

#[test]
fn device_callback_missing_event_and_panicked_worker_fail_without_abort() {
    const CHILD: &str = "TMT_1460_CALLBACK_FAILURE";
    if let Ok(root) = std::env::var(CHILD) {
        let mut callback = DeviceCallback::serve(std::path::Path::new(&root));
        assert!(
            callback
                .next_within(Duration::from_millis(20))
                .unwrap_err()
                .contains("foreground did not deliver device event")
        );
        // Deterministically model the original failed worker, then unwind the
        // scenario. The old Drop join unwrap aborts this isolated test process.
        callback.stop.store(true, Ordering::Release);
        callback.thread.take().unwrap().join().unwrap();
        callback.thread = Some(std::thread::spawn(|| panic!("injected worker failure")));
        panic!("missing device event assertion");
    }
    let pilot = Pilot::new();
    let stderr_path = pilot.root.join("failure.stderr");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "device_callback_missing_event_and_panicked_worker_fail_without_abort",
            "--nocapture",
        ])
        .env(CHILD, &pilot.root)
        .stdout(Stdio::null())
        .stderr(fs::File::create(&stderr_path).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + DEVICE_EVENT_WINDOW;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait().unwrap();
            panic!("callback failure subprocess did not terminate");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stderr = fs::read_to_string(stderr_path).unwrap();
    assert_eq!(
        status.code(),
        Some(101),
        "must fail as a Rust assertion, not a signal: {stderr}"
    );
    assert!(
        UnixStream::connect(pilot.root.join("colab/door.sock")).is_err(),
        "callback worker leaked after the failed scenario"
    );
    assert!(
        stderr.contains("missing device event assertion"),
        "{stderr}"
    );
    assert!(
        stderr.contains("device callback worker panicked during cleanup"),
        "{stderr}"
    );
}

#[test]
fn device_events_follow_cli_rename_and_replay_an_offline_revoke_on_restart() {
    use tmt_remote::{
        state::Layout,
        store::{DEFAULT_SCOPES, Grant, Store, uuid_v4},
    };
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        let root = pilot.root.join("state");
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: [7; 32],
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Laptop".into(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        drop(store);
        drop(serving);
        let callback = DeviceCallback::serve(&root);
        let (server, _) = serve(&pilot);
        pilot.child = Some(server);
        assert_eq!(
            callback.next(),
            serde_json::json!({"type":"device.renamed","deviceId":grant.client_id,"grantRevision":1,"name":"Laptop"})
        );
        let renamed = pilot
            .command()
            .args(["devices", "rename", &grant.client_id, "Travel", "--json"])
            .output()
            .unwrap();
        assert!(renamed.status.success());
        assert_eq!(
            callback.next(),
            serde_json::json!({"type":"device.renamed","deviceId":grant.client_id,"grantRevision":2,"name":"Travel"})
        );
        terminate(pilot.child.take().unwrap());
        let revoked = pilot
            .command()
            .args(["devices", "revoke", &grant.client_id, "--json"])
            .output()
            .unwrap();
        assert!(revoked.status.success());
        let (server, _) = serve(&pilot);
        pilot.child = Some(server);
        assert_eq!(
            callback.next(),
            serde_json::json!({"type":"device.revoked","deviceId":grant.client_id,"grantRevision":3})
        );
        terminate(pilot.child.take().unwrap());
        assert!(!root.join("remote/control.sock").exists());
        drop(callback);
    }
}

/// Keep each started child in the fixture before reading readiness, including
/// on assertion failure. Readiness is an output event, not a fixed delay.
fn start_door(pilot: &mut Pilot, options: &[&str], human: bool) -> String {
    let detached =
        options.contains(&"--background") || (human && !options.contains(&"--foreground"));
    if detached {
        pilot.background = true;
        pilot.pending_launcher = true;
    }
    let mut command = pilot.command();
    command.arg("serve").args(options);
    if !human {
        command.arg("--json");
    }
    pilot.child = Some(
        command
            .stdout(Stdio::piped())
            .stderr(fs::File::create(pilot.root.join("serve.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let pipe = pilot.child.as_mut().unwrap().stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let lines = if human {
        if options.contains(&"--foreground") {
            2
        } else {
            3
        }
    } else {
        1
    };
    let reader = std::thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut output = String::new();
        for _ in 0..lines {
            pipe.read_line(&mut output).unwrap();
        }
        let _ = sender.send(output);
    });
    let result = receiver.recv_timeout(STARTUP);
    if result.is_err() {
        if detached {
            // No completed readiness receipt: request startup cancellation.
            // Human output cannot prove private cleanup, so keep uncertainty.
            let _ = pilot.cancel_launcher();
        }
        let child = pilot.child.as_mut().unwrap();
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
        assert!(
            reap_until(child, Instant::now() + Duration::from_secs(1)).is_some(),
            "readiness failure did not reap the owned child; retain the fixture root"
        );
    }
    reader.join().unwrap();
    let output = result.unwrap();
    if detached
        && ((human && output.contains("Manage this door:"))
            || (!human
                && serde_json::from_str::<Value>(&output)
                    .is_ok_and(|value| value["state"] == "ready")))
    {
        pilot.pending_launcher = false;
    }
    output
}
fn status_json(pilot: &Pilot) -> Value {
    let result = pilot.command().args(["status", "--json"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(result.stderr.is_empty());
    serde_json::from_slice(&result.stdout).unwrap()
}
fn stop_json(pilot: &Pilot) -> Value {
    let result = pilot.command().args(["stop", "--json"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(result.stderr.is_empty());
    serde_json::from_slice(&result.stdout).unwrap()
}
fn wait_stopped(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait().unwrap();
            panic!("stop reported success but the owned foreground leaked");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn default_human_serve_returns_after_readiness() {
    let mut pilot = Pilot::new();
    pilot.background = true;
    let output = start_door(&mut pilot, &["--port", "0"], true);
    assert!(output.contains("http://"));
    let deadline = Instant::now() + Duration::from_secs(2);
    let launcher = pilot.child.as_mut().unwrap();
    loop {
        if let Some(status) = launcher.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            launcher.kill().unwrap();
            launcher.wait().unwrap();
            panic!("default human serve kept the launcher in the foreground");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = status_json(&pilot);
    assert_eq!(status["running"], true);
    assert_eq!(
        output
            .lines()
            .find(|line| line.starts_with("http://"))
            .unwrap(),
        format!("{}/", status["origin"].as_str().unwrap())
    );
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped": true}));
    assert_eq!(status_json(&pilot)["running"], false);
}
#[test]
fn human_serve_link_opens_landing_while_protocol_address_stays_separate() {
    let mut pilot = Pilot::new();
    let output = start_door(&mut pilot, &["--foreground", "--port", "0"], true);
    let url = output
        .lines()
        .find(|line| line.starts_with("http://"))
        .unwrap();
    let status = status_json(&pilot);
    let origin = status["origin"].as_str().unwrap();
    let prefix = status["path"].as_str().unwrap();
    assert_eq!(
        url,
        format!("{origin}/"),
        "human output must name the browser entry"
    );
    let calls_before = fs::read(pilot.root.join("calls")).unwrap();
    let socket = origin.strip_prefix("http://").unwrap();
    let page = exchange(socket, &format!("GET / HTTP/1.1\r\nHost: {socket}\r\n\r\n"));
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("text/html; charset=utf-8"));
    assert!(page.contains("tmt remote pair"));
    let protocol = exchange(
        socket,
        &format!("GET {prefix} HTTP/1.1\r\nHost: {socket}\r\n\r\n"),
    );
    assert!(protocol.starts_with("HTTP/1.1 404"));
    assert!(protocol.ends_with("{}"));
    assert_eq!(
        fs::read(pilot.root.join("calls")).unwrap(),
        calls_before,
        "entry and protocol reads must not invoke core"
    );
    terminate(pilot.child.take().unwrap());
    assert!(!pilot.root.join("state/remote/control.sock").exists());
}
#[test]
fn control_stop_closes_listeners_releases_state_and_is_idempotent() {
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        let help = pilot.command().args(["stop", "--help"]).output().unwrap();
        assert!(help.status.success());
        assert!(!pilot.root.join("calls").exists());
        assert_eq!(stop_json(&pilot), serde_json::json!({"running":false}));
        assert!(!pilot.root.join("state").exists());
        let descriptor: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
        let (origin, _, port) = address_parts(descriptor["address"].as_str().unwrap());
        let mut pending = TcpStream::connect(origin.strip_prefix("http://").unwrap()).unwrap();
        pending
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        pending.write_all(b"GET /").unwrap();
        let before = Instant::now();
        assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
        assert!(before.elapsed() < Duration::from_secs(10));
        wait_stopped(pilot.child.as_mut().unwrap());
        assert!(!pilot.root.join("state/remote/control.sock").exists());
        // The retained partial HTTP request cannot keep a worker/listener alive.
        let mut byte = [0];
        match pending.read(&mut byte) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            other => panic!("stopped door retained a request: {other:?}"),
        }
        let available = TcpListener::bind(("127.0.0.1", port)).unwrap();
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":port})
        );
        let state = stopped_files(&pilot);
        assert_eq!(stop_json(&pilot), serde_json::json!({"running":false}));
        assert_eq!(stopped_files(&pilot), state);
        drop(available);
    }
}
fn address_parts(address: &str) -> (&str, &str, u16) {
    let split = address.find("/r/").unwrap();
    let (origin, path) = address.split_at(split);
    let port = origin
        .strip_prefix("http://127.0.0.1:")
        .unwrap()
        .parse()
        .unwrap();
    (origin, path, port)
}
fn stopped_files(pilot: &Pilot) -> Vec<(std::ffi::OsString, Vec<u8>, std::time::SystemTime)> {
    let mut files: Vec<_> = fs::read_dir(pilot.root.join("state/remote"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name(),
                fs::read(entry.path()).unwrap(),
                entry.metadata().unwrap().modified().unwrap(),
            )
        })
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
#[test]
fn status_does_not_create_remote_state_and_help_needs_no_core() {
    let pilot = Pilot::new();
    let help = pilot.command().args(["status", "--help"]).output().unwrap();
    assert!(help.status.success());
    assert!(!pilot.root.join("calls").exists());
    assert_eq!(
        status_json(&pilot),
        serde_json::json!({"running":false,"lastPort":null})
    );
    assert!(!pilot.root.join("state").exists());
    assert_eq!(
        fs::read_to_string(pilot.root.join("calls")).unwrap(),
        "api\n"
    );
    let missing = pilot
        .command()
        .env_remove("TMT_EXECUTABLE")
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&missing.stdout).unwrap()["error"]["code"],
        "REMOTE_CORE_UNAVAILABLE"
    );
    executable_fixture::write_executable(&pilot.root.join("core"),
        "printf '%s' '{\"error\":{\"code\":\"STORAGE_NOT_WRITABLE\",\"message\":\"fixture root unavailable\"}}'\nexit 1\n").unwrap();
    let failed = pilot.command().args(["status", "--json"]).output().unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stdout).unwrap()["error"]["code"],
        "REMOTE_CORE_UNAVAILABLE"
    );
    assert!(!pilot.root.join("state").exists());
}
#[test]
fn default_port_and_exact_live_status_survive_restart_without_status_mutations() {
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        let first: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
        let address = first["address"].as_str().unwrap();
        let (origin, path, port) = address_parts(address);
        let database = fs::read(pilot.root.join("state/remote/remote.db")).unwrap();
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":true,"origin":origin,"path":path})
        );
        assert_eq!(
            fs::read(pilot.root.join("state/remote/remote.db")).unwrap(),
            database
        );
        terminate(pilot.child.take().unwrap());
        let files = stopped_files(&pilot);
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":port})
        );
        assert_eq!(
            machine_status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":port})
        );
        assert_eq!(
            stopped_files(&pilot),
            files,
            "status changed state bytes, mtimes or inventory"
        );
        let restarted: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
        assert_eq!(restarted["address"], first["address"]);
        assert_eq!(restarted["machineId"], first["machineId"]);
        assert_ne!(restarted["windowId"], first["windowId"]);
        terminate(pilot.child.take().unwrap());
        assert!(TcpStream::connect(format!("127.0.0.1:{port}")).is_err());
        assert!(!pilot.root.join("state/remote/control.sock").exists());
    }
}
#[test]
fn busy_remembered_port_refuses_without_changing_origin_and_recovers_after_freeing_it() {
    for human in [false, true] {
        let mut pilot = Pilot::new();
        let first: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
        let (_, _, port) = address_parts(first["address"].as_str().unwrap());
        terminate(pilot.child.take().unwrap());
        let occupied = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let mut command = pilot.command();
        command.arg("serve");
        if !human {
            command.arg("--json");
        }
        let failed = command.output().unwrap();
        assert!(!failed.status.success());
        let message = if human {
            assert!(failed.stdout.is_empty());
            String::from_utf8(failed.stderr).unwrap()
        } else {
            assert!(failed.stderr.is_empty());
            let answer: Value = serde_json::from_slice(&failed.stdout).unwrap();
            assert_eq!(answer["error"]["code"], "REMOTE_PORT_BUSY");
            answer["error"]["message"].as_str().unwrap().to_owned()
        };
        assert!(message.contains(&format!("Remote's port {port} is in use")));
        assert!(message.contains("stop what is using it to keep this browser paired"));
        assert!(message.contains("tmt remote serve --port <n> and pair again"));
        if human {
            // The refusal is an error line and its own hint line, never one run-on message.
            let lines: Vec<&str> = message.lines().collect();
            assert_eq!(lines.len(), 2, "{message}");
            assert!(lines[0].starts_with("error: ") && !lines[0].contains("--port"));
            assert!(lines[1].starts_with("hint: stop what is using it"));
        }
        assert!(!pilot.root.join("state/remote/control.sock").exists());
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":port})
        );
        drop(occupied);
        let next: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
        assert_eq!(next["address"], first["address"]);
        terminate(pilot.child.take().unwrap());
    }
}
#[test]
fn remote_settings_are_private_persistent_and_do_not_open_remote_storage() {
    let pilot = Pilot::new();
    for (args, expected) in [
        (
            vec!["settings", "--json"],
            serde_json::json!({"open":true,"source":"default","sessionsPerDevice":8,"sessionsPerDeviceSource":"default"}),
        ),
        (
            vec!["settings", "open", "off", "--json"],
            serde_json::json!({"open":false,"source":"settings.json","sessionsPerDevice":8,"sessionsPerDeviceSource":"default"}),
        ),
        (
            vec!["settings", "--json"],
            serde_json::json!({"open":false,"source":"settings.json","sessionsPerDevice":8,"sessionsPerDeviceSource":"default"}),
        ),
        (
            vec!["settings", "open", "on", "--json"],
            serde_json::json!({"open":true,"source":"settings.json","sessionsPerDevice":8,"sessionsPerDeviceSource":"default"}),
        ),
    ] {
        let answer = pilot.command().args(args).output().unwrap();
        assert!(answer.status.success(), "{:?}", answer);
        assert_eq!(
            serde_json::from_slice::<Value>(&answer.stdout).unwrap(),
            expected
        );
    }
    let remote = pilot.root.join("state/remote");
    assert_eq!(
        fs::metadata(remote.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!remote.join("remote.db").exists() && !remote.join("machine.key").exists());
    let mut names = fs::read_dir(&remote)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec!["settings.json", "settings.lock"]);
    for args in [
        vec!["pair", "--open", "--no-open"],
        vec!["settings", "open"],
        vec!["settings", "unknown", "off"],
    ] {
        assert!(
            !pilot
                .command()
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
#[test]
fn explicit_zero_ignores_the_remembered_port_and_explicit_busy_does_not_fallback() {
    let mut pilot = Pilot::new();
    let first: Value = serde_json::from_str(&start_door(&mut pilot, &[], false)).unwrap();
    let (_, _, port) = address_parts(first["address"].as_str().unwrap());
    terminate(pilot.child.take().unwrap());
    let occupied = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let failed = pilot
        .command()
        .args(["serve", "--port", &port.to_string(), "--json"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stdout).unwrap()["error"]["code"],
        "REMOTE_PORT_BUSY"
    );
    assert_eq!(
        status_json(&pilot),
        serde_json::json!({"running":false,"lastPort":port})
    );
    let random: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    let (_, _, random_port) = address_parts(random["address"].as_str().unwrap());
    assert_ne!(random_port, port);
    assert!(
        fs::read(pilot.root.join("serve.stderr"))
            .unwrap()
            .is_empty(),
        "explicit 0 must not attempt remembered port"
    );
    terminate(pilot.child.take().unwrap());
    drop(occupied);
    let fixed: Value = serde_json::from_str(&start_door(
        &mut pilot,
        &["--port", &port.to_string()],
        false,
    ))
    .unwrap();
    assert_eq!(address_parts(fixed["address"].as_str().unwrap()).2, port);
    terminate(pilot.child.take().unwrap());
    assert_eq!(
        status_json(&pilot),
        serde_json::json!({"running":false,"lastPort":port})
    );
}
#[test]
fn status_and_stop_refuse_unsafe_state_and_unresponsive_or_malformed_control() {
    use std::os::unix::fs::symlink;
    let mut pilot = Pilot::new();
    start_door(&mut pilot, &[], false);
    terminate(pilot.child.take().unwrap());
    let directory = pilot.root.join("state/remote");
    let refuse = |expected: &str| {
        for args in [
            vec!["status", "--json"],
            vec!["status", "--machine", "--json"],
            vec!["stop", "--json"],
        ] {
            let command = args[0];
            let result = pilot.command().args(args).output().unwrap();
            assert!(
                !result.status.success(),
                "{command} accepted state expected to fail with {expected}"
            );
            let error: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(error.as_object().unwrap().len(), 1);
            assert_eq!(error["error"]["code"], expected);
            assert!(error["error"]["message"].is_string());
        }
    };
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    refuse("REMOTE_STATE_UNSAFE");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::rename(directory.join("remote.db"), directory.join("original.db")).unwrap();
    symlink(directory.join("original.db"), directory.join("remote.db")).unwrap();
    refuse("REMOTE_STATE_UNSAFE");
    fs::remove_file(directory.join("remote.db")).unwrap();
    fs::rename(directory.join("original.db"), directory.join("remote.db")).unwrap();
    fs::remove_file(directory.join("serve.lock")).unwrap();
    refuse("REMOTE_STATE_UNAVAILABLE");
    let layout = tmt_remote::state::Layout::open(&pilot.root.join("state")).unwrap();
    let held = layout.serve_lock().unwrap();
    refuse("REMOTE_ALREADY_SERVING");
    drop(held);
    fs::write(directory.join("control.sock"), b"not a socket").unwrap();
    refuse("REMOTE_STATE_UNSAFE");
    fs::remove_file(directory.join("control.sock")).unwrap();
    for (reply, expected) in [
        (Some("{}\n"), "REMOTE_IO"),
        (Some("{\"stopping\":true}\n"), "REMOTE_IO"),
        (Some("{\"running\":false,\"lastPort\":null}\n"), "REMOTE_IO"),
        (
            Some("{\"error\":{\"code\":\"REMOTE_NOT_RUNNING\",\"message\":\"reported error\"}}\n"),
            "REMOTE_NOT_RUNNING",
        ),
        (
            Some(
                "{\"running\":true,\"origin\":\"http://127.0.0.1:12345/\",\"path\":\"/r/k7qxm4tz2pbwn6rh\"}\n",
            ),
            "REMOTE_IO",
        ),
        (
            Some(
                "{\"running\":true,\"origin\":\"http://127.0.0.1:12345\",\"path\":\"/r/k7qxm4tz2pbwn6rh\",\"devices\":[]}\n",
            ),
            "REMOTE_IO",
        ),
        (None, "REMOTE_IO"),
        // An alpha.1 serve answers an unknown operation as invalid input; a
        // later serve uses the dedicated code. Both mean the serve is older.
        (
            Some(
                "{\"error\":{\"code\":\"REMOTE_INPUT_INVALID\",\"message\":\"Unknown control operation.\"}}\n",
            ),
            "REMOTE_SERVE_OUTDATED",
        ),
        (
            Some(
                "{\"error\":{\"code\":\"REMOTE_CONTROL_UNSUPPORTED\",\"message\":\"Unknown control operation.\"}}\n",
            ),
            "REMOTE_SERVE_OUTDATED",
        ),
        // Other invalid-input replies are not the legacy unknown-operation reply.
        (
            Some(
                "{\"error\":{\"code\":\"REMOTE_INPUT_INVALID\",\"message\":\"Revoke needs a clientId.\"}}\n",
            ),
            "REMOTE_INPUT_INVALID",
        ),
    ] {
        for command in ["status", "stop"] {
            let expected = if command == "stop" && reply == Some("{\"stopping\":true}\n") {
                "REMOTE_STOP_UNCONFIRMED"
            } else {
                expected
            };
            let result = control_reply(
                &pilot,
                &[command, "--json"],
                serde_json::json!({"op":command}),
                reply,
            );
            assert!(
                !result.status.success(),
                "{command} accepted state expected to fail with {expected}"
            );
            let error: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(error.as_object().unwrap().len(), 1);
            assert_eq!(error["error"]["code"], expected);
            if expected == "REMOTE_SERVE_OUTDATED" {
                let message = error["error"]["message"].as_str().unwrap();
                assert!(message.contains("older") && message.contains("Ctrl-C"));
                assert!(!message.contains("remote stop"), "{message}");
            }
        }
    }
    assert!(!status_json(&pilot)["running"].as_bool().unwrap());
    // An owned stale socket does not claim a live door.
    let listener = UnixListener::bind(directory.join("control.sock")).unwrap();
    fs::set_permissions(
        directory.join("control.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    drop(listener);
    assert!(!status_json(&pilot)["running"].as_bool().unwrap());
    assert_eq!(stop_json(&pilot), serde_json::json!({"running":false}));
    fs::remove_file(directory.join("control.sock")).unwrap();
}

#[test]
fn session_limit_settings_show_source_and_preserve_open() {
    let pilot = Pilot::new();
    for args in [
        ["settings", "open", "off", "--json"],
        ["settings", "sessions-per-device", "2", "--json"],
        ["settings", "open", "on", "--json"],
    ] {
        assert!(
            pilot
                .command()
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let shown = pilot
        .command()
        .args(["settings", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"open":true,"source":"settings.json","sessionsPerDevice":2,"sessionsPerDeviceSource":"settings.json"})
    );
    let plain = pilot.command().arg("settings").output().unwrap();
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(text.contains("sessions-per-device") && text.contains("2 (settings.json)"));
    let off = pilot
        .command()
        .args(["settings", "sessions-per-device", "off", "--json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&off.stdout).unwrap();
    assert_eq!(value["sessionsPerDevice"], Value::Null);
    assert_eq!(value["sessionsPerDeviceSource"], "settings.json");
    assert_eq!(value["open"], true);
    for invalid in ["0", "-1", "on", "1.5", "01"] {
        assert!(
            !pilot
                .command()
                .args(["settings", "sessions-per-device", invalid])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

// Exercise the actual binary-private startup owner, not a replacement algorithm.
struct PrivateWorker {
    child: Child,
    stream: UnixStream,
    failed_cleanup: bool,
    accepted: bool,
}
impl PrivateWorker {
    fn record(&mut self) -> (u8, Value) {
        let (tag, value) = read_startup_record(&mut self.stream, Instant::now() + STARTUP).unwrap();
        if tag == 2 {
            self.failed_cleanup = value["cleanupConfirmed"] == true;
        }
        (tag, value)
    }
    fn cancel_and_reap(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(45);
        let _ = self.stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = self.stream.write_all(&[5]);
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
        while !self.failed_cleanup {
            match read_startup_record(&mut self.stream, deadline) {
                Ok((2, value)) => {
                    self.failed_cleanup = value["cleanupConfirmed"] == true;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let exited = reap_until(&mut self.child, deadline).is_some();
        if !exited && matches!(self.child.try_wait(), Ok(None)) {
            // This is only an unreaped owned child. Forced exit cannot confirm
            // its separately grouped invocation cleanup, so retain the root.
            let _ = self.child.kill();
            let _ = reap_until(&mut self.child, Instant::now() + Duration::from_secs(1));
        }
        eprintln!(
            "private worker {} cleanup receipt={} reaped={}",
            self.child.id(),
            self.failed_cleanup,
            exited
        );
        self.failed_cleanup && exited
    }
}
fn reap_until(child: &mut Child, deadline: Instant) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => return None,
        }
    }
}
fn startup_worker(pilot: &mut Pilot) -> usize {
    let (parent, worker) = UnixStream::pair().unwrap();
    let child = pilot
        .command()
        .args(["serve", "--foreground", "--worker", "--port", "0"])
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(worker)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Store both handles before any assertion or blocking read. Pilot owns
    // cancellation, the terminal Failed receipt and reap before root deletion.
    pilot.private_workers.push(PrivateWorker {
        child,
        stream: parent,
        failed_cleanup: false,
        accepted: false,
    });
    pilot.private_workers.len() - 1
}
fn read_startup_record(stream: &mut UnixStream, deadline: Instant) -> std::io::Result<(u8, Value)> {
    fn read_until(
        stream: &mut UnixStream,
        mut bytes: &mut [u8],
        deadline: Instant,
    ) -> std::io::Result<()> {
        while !bytes.is_empty() {
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::TimedOut))?;
            // Poll bounds the complete frame, including a partial header/body;
            // a socket timeout alone would renew its allowance on each read.
            use std::os::fd::AsFd;
            let mut descriptors = [nix::poll::PollFd::new(
                stream.as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            let timeout =
                nix::poll::PollTimeout::try_from(left.min(Duration::from_millis(100))).unwrap();
            if nix::poll::poll(&mut descriptors, timeout)? == 0 {
                continue;
            }
            let count = stream.read(bytes)?;
            if count == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    let mut header = [0; 5];
    read_until(stream, &mut header, deadline)?;
    let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
    if length + header.len() > 4096 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; length];
    read_until(stream, &mut bytes, deadline)?;
    Ok((header[0], serde_json::from_slice(&bytes)?))
}

#[test]
fn background_json_is_clean_duplicate_does_not_clear_diagnostics_or_stop_original() {
    let mut pilot = Pilot::new();
    pilot.background = true;
    let output = pilot.background_output(&["--port", "0"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_to_string(pilot.root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let ready: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ready["startupCoreCalls"], 2);
    assert_eq!(ready["state"], "ready");
    let diagnostic = pilot.root.join("state/remote/serve-error.json");
    assert_eq!(
        fs::metadata(&diagnostic).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&diagnostic).unwrap(), b"");
    fs::write(&diagnostic, b"previous diagnostic fixture").unwrap();
    let duplicate = pilot.background_output(&[]);
    assert!(!duplicate.status.success());
    assert!(duplicate.stderr.is_empty());
    let error: Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(
        error["error"]["code"],
        "REMOTE_ALREADY_SERVING",
        "{}",
        String::from_utf8_lossy(&duplicate.stderr)
    );
    assert_eq!(
        fs::read(&diagnostic).unwrap(),
        b"previous diagnostic fixture"
    );
    let status = status_json(&pilot);
    let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
    assert_eq!(status["origin"], origin);
    assert_eq!(status["path"], path);
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    let socket = origin.strip_prefix("http://").unwrap();
    assert!(
        exchange(socket, &format!("GET / HTTP/1.1\r\nHost: {socket}\r\n\r\n"))
            .contains("<!doctype html>")
    );
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    assert_eq!(status_json(&pilot)["running"], false);
}

#[test]
fn ready_cancel_closes_owned_resources_and_preserves_machine_key() {
    let mut pilot = Pilot::new();
    let index = startup_worker(&mut pilot);
    let worker = &mut pilot.private_workers[index];
    let (tag, ready) = worker.record();
    assert_eq!(tag, 1);
    let key = fs::read(pilot.root.join("state/remote/machine.key")).unwrap();
    worker.stream.write_all(&[5]).unwrap();
    let (tag, failed) = worker.record();
    assert_eq!(tag, 2);
    assert_eq!(failed["cleanupConfirmed"], true);
    assert_eq!(failed["code"], "REMOTE_STARTUP_CANCELLED");
    let status = reap_until(&mut worker.child, Instant::now() + Duration::from_secs(45)).unwrap();
    assert!(!status.success());
    assert_eq!(status_json(&pilot)["running"], false);
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    let socket = origin.strip_prefix("http://").unwrap();
    assert!(TcpStream::connect(socket).is_err());
    assert_eq!(
        fs::read(pilot.root.join("state/remote/machine.key")).unwrap(),
        key
    );
    let diagnostic: Value = serde_json::from_slice(
        &fs::read(pilot.root.join("state/remote/serve-error.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(diagnostic["phase"], "handoff");
}

#[test]
fn lost_accepted_acknowledgment_keeps_the_actual_service_running() {
    let mut pilot = Pilot::new();
    let index = startup_worker(&mut pilot);
    let startup = &mut pilot.private_workers[index];
    let (_, ready) = startup.record();
    let worker = nix::unistd::Pid::from_raw(startup.child.id() as i32);
    assert_eq!(nix::unistd::getsid(Some(worker)).unwrap(), worker);
    assert_ne!(nix::unistd::getsid(None).unwrap(), worker);
    // Refuse the acknowledgment, but buffer Accept before EOF. No re-execution.
    startup.stream.shutdown(std::net::Shutdown::Read).unwrap();
    startup.stream.write_all(&[3]).unwrap();
    startup.accepted = true;
    pilot.background = true;
    startup.stream.shutdown(std::net::Shutdown::Write).unwrap();
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    let socket = origin.strip_prefix("http://").unwrap();
    assert!(
        exchange(socket, &format!("GET / HTTP/1.1\r\nHost: {socket}\r\n\r\n"))
            .contains("<!doctype html>")
    );
    assert!(
        pilot.private_workers[index]
            .child
            .try_wait()
            .unwrap()
            .is_none()
    );
    let status = status_json(&pilot);
    let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
    assert_eq!(status["origin"], origin);
    assert_eq!(status["path"], path);
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(&mut pilot.private_workers[index].child);
    assert!(!pilot.root.join("state/remote/control.sock").exists());
}

#[test]
fn unsafe_diagnostic_and_busy_port_refuse_without_replacing_files() {
    let mut pilot = Pilot::new();
    // Initialize only by the real foreground owner, then release its lease.
    start_door(&mut pilot, &["--port", "0"], false);
    stop_json(&pilot);
    wait_stopped(pilot.child.as_mut().unwrap());
    let diagnostic = pilot.root.join("state/remote/serve-error.json");
    let foreign = pilot.root.join("foreign");
    fs::write(&foreign, b"must stay exact").unwrap();
    std::os::unix::fs::symlink(&foreign, &diagnostic).unwrap();
    // The launcher guard is installed before an unexpected acceptance can unwind.
    let result = pilot.background_output(&[]);
    assert!(!result.status.success());
    let error: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(error["error"]["code"], "REMOTE_STATE_UNSAFE");
    assert_eq!(fs::read(&foreign).unwrap(), b"must stay exact");
    fs::remove_file(&diagnostic).unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = occupied.local_addr().unwrap().port().to_string();
    let result = pilot.background_output(&["--port", &port]);
    assert!(!result.status.success());
    let error: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(error["error"]["code"], "REMOTE_PORT_BUSY");
    let bytes = fs::read(&diagnostic).unwrap();
    assert!(bytes.len() <= 4096);
    let diagnostic: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(diagnostic["phase"], "bind");
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains(&pilot.root.display().to_string())
    );
    assert_eq!(occupied.local_addr().unwrap().port().to_string(), port);
}

#[test]
fn failed_private_start_record_is_bounded_and_reports_cleanup() {
    let mut pilot = Pilot::new();
    start_door(&mut pilot, &[], false);
    let index = startup_worker(&mut pilot);
    let duplicate = &mut pilot.private_workers[index];
    let (tag, failed) = duplicate.record();
    assert_eq!(tag, 2, "{failed}");
    assert_eq!(failed["code"], "REMOTE_ALREADY_SERVING", "{failed}");
    assert_eq!(failed["cleanupConfirmed"], true, "{failed}");
    assert!(
        !reap_until(&mut duplicate.child, Instant::now() + STARTUP)
            .unwrap()
            .success()
    );
    stop_json(&pilot);
    wait_stopped(pilot.child.as_mut().unwrap());
}

#[test]
fn explicit_modes_refuse_conflict_and_foreground_preserves_terminal_ownership() {
    let mut pilot = Pilot::new();
    let conflict = pilot
        .command()
        .args(["serve", "--foreground", "--background", "--json"])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert!(!pilot.root.join("calls").exists());
    start_door(&mut pilot, &["--foreground", "--port", "0"], true);
    assert!(pilot.child.as_mut().unwrap().try_wait().unwrap().is_none());
    assert!(!pilot.root.join("state/remote/serve-error.json").exists());
    terminate(pilot.child.take().unwrap());
    assert_eq!(status_json(&pilot)["running"], false);
}

#[test]
fn launcher_cancellation_reaps_a_real_pending_discovery_invocation() {
    let mut pilot = Pilot::new();
    let pending = start_pending_launcher(&mut pilot);
    assert!(
        fs::read_to_string(pilot.root.join("input"))
            .unwrap()
            .contains("capabilities")
    );
    let output = pilot
        .cancel_launcher()
        .expect("confirmed launcher cancellation and reap");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        error["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("REMOTE_")
    );
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pending), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert!(!pilot.root.join("state/remote").exists());
}

#[test]
fn startup_endpoint_is_not_inherited_by_actual_core_and_eof_reaps_pending_core() {
    let mut pilot = Pilot::new();
    let barrier = pilot.root.join("barrier");
    nix::unistd::mkfifo(
        &barrier,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    executable_fixture::write_executable(&pilot.root.join("core"), &format!(
        "cat > '{root}/input'\nfor fd in /dev/fd/*; do if test -S \"$fd\"; then printf '%s\\n' \"$fd\" >> '{root}/inherited-sockets'; fi; done\nprintf '%s' \"$$\" > '{root}/pending.pid'\nread -r release < '{root}/barrier'\n",
        root=pilot.root.display())).unwrap();
    // Positive control: this exact inspector detects a socket deliberately
    // attached as fake-core stdin. It must not silently always report no sockets.
    let mut gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&barrier)
        .unwrap();
    let (positive, socket) = UnixStream::pair().unwrap();
    positive.shutdown(std::net::Shutdown::Write).unwrap();
    pilot.child = Some(
        Command::new(pilot.root.join("core"))
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(socket)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + STARTUP;
    while !pilot.root.join("pending.pid").exists() {
        assert!(
            Instant::now() < deadline,
            "socket inspector positive control did not reach barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fs::read_to_string(pilot.root.join("inherited-sockets"))
            .unwrap()
            .contains("/dev/fd/0")
    );
    gate.write_all(b"continue\n").unwrap();
    assert!(
        reap_until(pilot.child.as_mut().unwrap(), Instant::now() + STARTUP)
            .unwrap()
            .success()
    );
    fs::remove_file(pilot.root.join("pending.pid")).unwrap();
    fs::remove_file(pilot.root.join("inherited-sockets")).unwrap();
    let index = startup_worker(&mut pilot);
    let deadline = Instant::now() + STARTUP;
    let pending = loop {
        if let Ok(pid) = fs::read_to_string(pilot.root.join("pending.pid"))
            && let Ok(pid) = pid.parse::<i32>()
        {
            break pid;
        }
        assert!(
            Instant::now() < deadline,
            "actual invocation did not reach EOF barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!pilot.root.join("inherited-sockets").exists());
    let worker = &mut pilot.private_workers[index];
    worker.stream.shutdown(std::net::Shutdown::Write).unwrap();
    let (tag, failed) = worker.record();
    assert_eq!(tag, 2);
    assert_eq!(failed["cleanupConfirmed"], true);
    let status =
        reap_until(&mut worker.child, deadline).expect("EOF cleaned the actual pending worker");
    assert!(!status.success());
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pending), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert!(!pilot.root.join("state/remote").exists());
    // Both exact owned processes are now gone; no detached service was admitted.
    pilot.background = false;
}

#[test]
fn background_serving_survives_departure_of_its_real_terminal() {
    let mut pilot = Pilot::new();
    // script owns a disposable PTY. Its exit closes that terminal, rather than
    // simulating departure solely by closing a pipe or painting a status label.
    pilot.background = true;
    pilot.pending_launcher = true;
    let mut terminal = Command::new("/usr/bin/script");
    terminal.env_clear();
    for (name, value) in pilot.command().get_envs() {
        if let Some(value) = value {
            terminal.env(name, value);
        }
    }
    #[cfg(target_os = "macos")]
    terminal
        .args(["-q"])
        .arg(pilot.root.join("terminal.txt"))
        .args([BINARY, "serve", "--port", "0"]);
    #[cfg(not(target_os = "macos"))]
    terminal
        .args(["-q", "-e", "-c"])
        .arg(format!("'{BINARY}' serve --port 0"))
        .arg(pilot.root.join("terminal.txt"));
    pilot.child = Some(
        terminal
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + STARTUP;
    loop {
        if let Some(status) = pilot.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "disposable terminal did not exit after handoff"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = fs::read_to_string(pilot.root.join("terminal.txt")).unwrap();
    assert!(
        output.contains("tmt remote pair")
            && output.contains("tmt remote status")
            && output.contains("tmt remote stop")
    );
    // The exact PTY launcher exited successfully and published full readiness.
    pilot.pending_launcher = false;
    let status = status_json(&pilot);
    assert_eq!(status["running"], true);
    let socket = status["origin"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap();
    assert!(
        exchange(socket, &format!("GET / HTTP/1.1\r\nHost: {socket}\r\n\r\n"))
            .contains("<!doctype html>")
    );
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    assert_eq!(status_json(&pilot)["running"], false);
}

// Deliberately unwind after actual core discovery has entered its barrier.
// The PID is an observation only, never authority to signal a descendant.
#[test]
fn pending_launcher_assertion_unwind_confirms_cleanup_before_root_removal() {
    let mut pilot = Pilot::new();
    let root = pilot.root.clone();
    let confirmed = Arc::new(AtomicBool::new(false));
    pilot.cleanup_observed = Some(confirmed.clone());
    let pending = start_pending_launcher(&mut pilot);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = pilot;
            panic!("deliberate pending launcher fixture failure");
        }))
        .is_err()
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    while nix::sys::signal::kill(nix::unistd::Pid::from_raw(pending), None)
        != Err(nix::errno::Errno::ESRCH)
    {
        assert!(
            Instant::now() < deadline,
            "pending invocation cleanup unconfirmed; retained evidence at {}",
            root.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        confirmed.load(Ordering::SeqCst),
        "root removal needs confirmed startup cancellation plus owned launcher reap"
    );
    assert!(!root.exists());
}
fn pending_core(pilot: &Pilot) {
    let barrier = pilot.root.join("barrier");
    nix::unistd::mkfifo(
        &barrier,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    executable_fixture::write_executable(&pilot.root.join("core"), &format!(
        "cat > '{root}/input'\nprintf '%s' \"$$\" > '{root}/pending.pid'\nread -r release < '{root}/barrier'\n", root=pilot.root.display())).unwrap();
}
fn start_pending_launcher(pilot: &mut Pilot) -> i32 {
    pending_core(pilot);
    pilot.pending_launcher = true;
    pilot.child = Some(
        pilot
            .command()
            .args(["serve", "--background", "--json"])
            .stdout(fs::File::create(pilot.root.join("launcher.json")).unwrap())
            .stderr(fs::File::create(pilot.root.join("launcher.stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + STARTUP;
    loop {
        if let Ok(pid) = fs::read_to_string(pilot.root.join("pending.pid"))
            && let Ok(pid) = pid.parse::<i32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "actual invocation did not reach its barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn private_worker_assertion_unwind_cancels_pending_core_and_reaps_before_removal() {
    let mut pilot = Pilot::new();
    let root = pilot.root.clone();
    let confirmed = Arc::new(AtomicBool::new(false));
    pilot.cleanup_observed = Some(confirmed.clone());
    pending_core(&pilot);
    let index = startup_worker(&mut pilot);
    let worker_pid = pilot.private_workers[index].child.id() as i32;
    let deadline = Instant::now() + STARTUP;
    let pending = loop {
        if let Ok(pid) = fs::read_to_string(root.join("pending.pid"))
            && let Ok(pid) = pid.parse::<i32>()
        {
            break pid;
        }
        assert!(
            Instant::now() < deadline,
            "private worker did not reach actual core barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = pilot;
            panic!("deliberate private worker fixture failure");
        }))
        .is_err()
    );
    assert!(confirmed.load(Ordering::SeqCst));
    for pid in [pending, worker_pid] {
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH)
        );
    }
    assert!(!root.exists());
}
#[test]
fn duplicate_private_worker_assertion_unwind_reaps_duplicate_and_stops_original() {
    let mut pilot = Pilot::new();
    let root = pilot.root.clone();
    let confirmed = Arc::new(AtomicBool::new(false));
    pilot.cleanup_observed = Some(confirmed.clone());
    let ready: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    let socket = origin.strip_prefix("http://").unwrap().to_owned();
    // Existing admitted root stop owns the original service; the separate
    // private guard owns the duplicate, installed before its first frame read.
    pilot.background = true;
    let index = startup_worker(&mut pilot);
    let duplicate_pid = pilot.private_workers[index].child.id() as i32;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = pilot;
            panic!("deliberate duplicate worker assertion failure");
        }))
        .is_err()
    );
    assert!(confirmed.load(Ordering::SeqCst));
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(duplicate_pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert!(TcpStream::connect(socket).is_err());
    assert!(!root.exists());
}
#[test]
fn unexpected_accepted_background_assertion_unwind_stops_before_removal() {
    let mut pilot = Pilot::new();
    let root = pilot.root.clone();
    let confirmed = Arc::new(AtomicBool::new(false));
    pilot.cleanup_observed = Some(confirmed.clone());
    let output = pilot.background_output(&["--port", "0"]);
    assert!(output.status.success());
    let ready: Value = serde_json::from_slice(&output.stdout).unwrap();
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    let socket = origin.strip_prefix("http://").unwrap().to_owned();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = pilot;
            panic!("deliberate expected-refusal assertion after accepted startup");
        }))
        .is_err()
    );
    assert!(confirmed.load(Ordering::SeqCst));
    assert!(TcpStream::connect(socket).is_err());
    assert!(!root.exists());
}

#[test]
fn missing_private_cleanup_receipt_retains_root_without_double_panic() {
    let mut pilot = Pilot::new();
    let root = pilot.root.clone();
    let observed = Arc::new(AtomicBool::new(false));
    pilot.cleanup_observed = Some(observed.clone());
    let index = startup_worker(&mut pilot);
    let worker = &mut pilot.private_workers[index];
    let (_, ready) = worker.record();
    worker.stream.write_all(&[5]).unwrap();
    // Deliberately consume the real completion outside the guard. Exit alone
    // must not replace a missing guard receipt, even with no control socket.
    let (tag, completion) =
        read_startup_record(&mut worker.stream, Instant::now() + STARTUP).unwrap();
    assert_eq!(tag, 2);
    assert_eq!(completion["cleanupConfirmed"], true);
    assert!(reap_until(&mut worker.child, Instant::now() + STARTUP).is_some());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = pilot;
            panic!("deliberate failure without guard cleanup receipt");
        }))
        .is_err()
    );
    assert!(!observed.load(Ordering::SeqCst));
    assert!(
        root.exists(),
        "missing receipt must retain diagnostic state"
    );
    let (origin, _, _) = address_parts(ready["address"].as_str().unwrap());
    assert!(TcpStream::connect(origin.strip_prefix("http://").unwrap()).is_err());
    // The test, unlike the deliberately deprived guard, retained the actual
    // worker completion plus reap above. Only now remove its retained artifact.
    let layout = tmt_remote::state::Layout::existing(&root.join("state"))
        .unwrap()
        .unwrap();
    drop(layout.existing_serve_lock().unwrap());
    assert!(!root.join("state/remote/control.sock").exists());
    eprintln!(
        "independent real completion and reap verified; removing retained artifact {}",
        root.display()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn machine_projection_uses_the_exact_live_control_owner() {
    let mut pilot = Pilot::new();
    let ready: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
    let mut stream = UnixStream::connect(pilot.root.join("state/remote/control.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(
        stream,
        "{}",
        serde_json::json!({"op":"status","machine":true})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        answer,
        serde_json::json!({"running":true,"origin":origin,"path":path,"machineId":ready["machineId"]})
    );
}

fn machine_status_json(pilot: &Pilot) -> Value {
    let result = pilot
        .command()
        .args(["status", "--machine", "--json"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(result.stderr.is_empty());
    serde_json::from_slice(&result.stdout).unwrap()
}
// The existing one-shot owner-only peer fixture, shared by ordinary and optional
// projections. Scoped joins also cover command/read/assertion unwind.
fn control_reply(
    pilot: &Pilot,
    args: &[&str],
    expected_request: Value,
    reply: Option<&str>,
) -> std::process::Output {
    let socket = pilot.root.join("state/remote/control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let (finish, finished) = mpsc::channel();
    let before = Instant::now();
    let output = std::thread::scope(|scope| {
        let peer = scope.spawn(move || {
            use std::os::fd::AsFd;
            let mut events = [nix::poll::PollFd::new(
                listener.as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            assert!(
                nix::poll::poll(&mut events, 15_000u16).unwrap() > 0,
                "control command never connected"
            );
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&request).unwrap(),
                expected_request
            );
            if let Some(reply) = reply {
                stream.write_all(reply.as_bytes()).unwrap();
            } else {
                let _ = finished.recv_timeout(Duration::from_secs(15));
            }
        });
        let result = pilot.command().args(args).output();
        let _ = finish.send(());
        peer.join().unwrap();
        result.unwrap()
    });
    fs::remove_file(socket).unwrap();
    assert!(before.elapsed() < Duration::from_secs(10));
    output
}
#[test]
fn machine_status_is_root_local_and_never_initializes_a_stopped_machine() {
    let mut machines = Vec::new();
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        assert_eq!(
            machine_status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":null})
        );
        assert!(!pilot.root.join("state").exists());
        assert!(
            fs::read_to_string(pilot.root.join("input"))
                .unwrap()
                .contains("storage.root")
        );
        let ready: Value =
            serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
        let (origin, path, port) = address_parts(ready["address"].as_str().unwrap());
        let database = fs::read(pilot.root.join("state/remote/remote.db")).unwrap();
        let key = fs::read(pilot.root.join("state/remote/machine.key")).unwrap();
        let calls = fs::read_to_string(pilot.root.join("calls"))
            .unwrap()
            .lines()
            .count();
        assert_eq!(
            machine_status_json(&pilot),
            serde_json::json!({"running":true,"origin":origin,"path":path,"machineId":ready["machineId"]})
        );
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls"))
                .unwrap()
                .lines()
                .count(),
            calls + 1
        );
        assert!(
            fs::read_to_string(pilot.root.join("input"))
                .unwrap()
                .contains("storage.root")
        );
        assert_eq!(
            status_json(&pilot),
            serde_json::json!({"running":true,"origin":origin,"path":path})
        );
        assert_eq!(
            fs::read(pilot.root.join("state/remote/remote.db")).unwrap(),
            database
        );
        assert_eq!(
            fs::read(pilot.root.join("state/remote/machine.key")).unwrap(),
            key
        );
        machines.push(ready["machineId"].clone());
        terminate(pilot.child.take().unwrap());
        let files = stopped_files(&pilot);
        assert_eq!(
            machine_status_json(&pilot),
            serde_json::json!({"running":false,"lastPort":port})
        );
        assert_eq!(stopped_files(&pilot), files);
    }
    assert_ne!(machines[0], machines[1]);
    let pilot = Pilot::new();
    let refused = pilot
        .command()
        .args(["status", "--machine"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!pilot.root.join("calls").exists());
    assert!(!pilot.root.join("state").exists());
    let help = pilot.command().args(["status", "--help"]).output().unwrap();
    assert!(
        help.status.success()
            && String::from_utf8(help.stdout)
                .unwrap()
                .contains("--machine")
    );
    assert!(!pilot.root.join("calls").exists());
}
#[test]
fn machine_status_preserves_unsupported_errors_and_strict_projection_shapes() {
    let mut pilot = Pilot::new();
    start_door(&mut pilot, &["--port", "0"], false);
    terminate(pilot.child.take().unwrap());
    let before = stopped_files(&pilot);
    for (code, message) in [
        ("REMOTE_CONTROL_UNSUPPORTED", "Unknown control operation."),
        ("REMOTE_INPUT_INVALID", "Unknown control operation."),
        ("REMOTE_INPUT_INVALID", "Revoke needs a clientId."),
        ("REMOTE_NOT_RUNNING", "reported error"),
    ] {
        let expected = serde_json::json!({"error":{"code":code,"message":message}});
        let reply = format!("{expected}\n");
        let result = control_reply(
            &pilot,
            &["status", "--machine", "--json"],
            serde_json::json!({"op":"status","machine":true}),
            Some(&reply),
        );
        assert!(!result.status.success() && result.stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap(),
            expected
        );
    }
    let valid = serde_json::json!({"running":true,"origin":"http://127.0.0.1:12345","path":"/r/k7qxm4tz2pbwn6rh","machineId":"00000000-0000-4000-8000-000000000001"});
    let reply = format!("{valid}\n");
    let positive = control_reply(
        &pilot,
        &["status", "--machine", "--json"],
        serde_json::json!({"op":"status","machine":true}),
        Some(&reply),
    );
    assert!(positive.status.success() && positive.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&positive.stdout).unwrap(),
        valid
    );
    // The same otherwise valid four-key document is still refused by the legacy
    // ordinary projection, whose exact shape must not be loosened.
    let ordinary = control_reply(
        &pilot,
        &["status", "--json"],
        serde_json::json!({"op":"status"}),
        Some(&reply),
    );
    assert!(!ordinary.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&ordinary.stdout).unwrap()["error"]["code"],
        "REMOTE_IO"
    );
    let mut malformed = Vec::new();
    for id in [
        Value::Null,
        serde_json::json!(""),
        serde_json::json!("00000000-0000-0000-0000-000000000000"),
        serde_json::json!("00000000-0000-1000-8000-000000000001"),
        serde_json::json!("00000000-0000-4000-7000-000000000001"),
        serde_json::json!("00000000-0000-4000-8000-00000000000A"),
        serde_json::json!("00000000-0000-4000-8000-00000000000"),
    ] {
        let mut value = valid.clone();
        value["machineId"] = id;
        malformed.push(value);
    }
    let mut value = valid.clone();
    value.as_object_mut().unwrap().remove("machineId");
    malformed.push(value);
    let mut value = valid.clone();
    value["extra"] = serde_json::json!(true);
    malformed.push(value);
    let mut value = valid.clone();
    value["origin"] = serde_json::json!("http://127.0.0.1:12345/");
    malformed.push(value);
    let mut value = valid.clone();
    value["path"] = serde_json::json!("/r/INVALID");
    malformed.push(value);
    malformed.push(serde_json::json!({"running":false,"lastPort":null}));
    for value in malformed {
        let reply = format!("{value}\n");
        let result = control_reply(
            &pilot,
            &["status", "--machine", "--json"],
            serde_json::json!({"op":"status","machine":true}),
            Some(&reply),
        );
        assert!(!result.status.success(), "accepted {value}");
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
            "REMOTE_IO"
        );
    }
    for reply in [None, Some("{\"running\":true")] {
        let result = control_reply(
            &pilot,
            &["status", "--machine", "--json"],
            serde_json::json!({"op":"status","machine":true}),
            reply,
        );
        assert!(!result.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
            "REMOTE_IO"
        );
    }
    assert_eq!(stopped_files(&pilot), before);
}

#[test]
fn object_status_is_opt_in_live_only_and_preserves_strict_ordinary_discovery() {
    let mut pilot = Pilot::new();
    let stopped = pilot
        .command()
        .args(["status", "--objects", "--json"])
        .output()
        .unwrap();
    assert!(stopped.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap(),
        serde_json::json!({"running":false,"lastPort":null})
    );
    let ready: Value =
        serde_json::from_str(&start_door(&mut pilot, &["--port", "0"], false)).unwrap();
    let (origin, path, _) = address_parts(ready["address"].as_str().unwrap());
    let observed = pilot
        .command()
        .args(["status", "--objects", "--json"])
        .output()
        .unwrap();
    assert!(observed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&observed.stdout).unwrap(),
        serde_json::json!({"running":true,"origin":origin,"path":path,"objectChannels":[{"extension":"colab","state":"unavailable","reason":"setup"}]})
    );
    for arguments in [
        vec!["status", "--objects"],
        vec!["status", "--objects", "--machine", "--json"],
    ] {
        assert!(
            !pilot
                .command()
                .args(arguments)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    terminate(pilot.child.take().unwrap());
    let before = stopped_files(&pilot);
    let valid = serde_json::json!({"running":true,"origin":origin,"path":path,"objectChannels":[{"extension":"alpha","state":"ready"},{"extension":"beta","state":"unavailable","reason":"setup"}]});
    let reply = format!("{valid}\n");
    let result = control_reply(
        &pilot,
        &["status", "--objects", "--json"],
        serde_json::json!({"op":"status","objects":true}),
        Some(&reply),
    );
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        valid
    );
    for channels in [
        Value::Null,
        serde_json::json!([{"extension":"alpha","state":"ready","reason":null}]),
        serde_json::json!([{"extension":"alpha","state":"ready"},{"extension":"alpha","state":"ready"}]),
        serde_json::json!([{"extension":"../alpha","state":"ready"}]),
        serde_json::json!([{"extension":"alpha","state":"unavailable"}]),
        serde_json::json!([{"extension":"alpha","state":"unavailable","reason":"/private/socket"}]),
        serde_json::json!([{"extension":"alpha","state":"ready","extra":true}]),
    ] {
        let mut malformed = valid.clone();
        malformed["objectChannels"] = channels;
        let reply = format!("{malformed}\n");
        let result = control_reply(
            &pilot,
            &["status", "--objects", "--json"],
            serde_json::json!({"op":"status","objects":true}),
            Some(&reply),
        );
        assert!(!result.status.success(), "accepted {malformed}");
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap()["error"]["code"],
            "REMOTE_IO"
        );
    }
    let refusal = serde_json::json!({"error":{"code":"REMOTE_CONTROL_UNSUPPORTED","message":"Unknown control operation."}});
    let reply = format!("{refusal}\n");
    let result = control_reply(
        &pilot,
        &["status", "--objects", "--json"],
        serde_json::json!({"op":"status","objects":true}),
        Some(&reply),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        refusal
    );
    assert!(!result.status.success());
    assert_eq!(stopped_files(&pilot), before);
}

#[path = "cli/object_negative.rs"]
mod object_negative;
fn status_run(pilot: &Pilot, args: &[&str]) -> std::process::Output {
    pilot.command().args(args).output().unwrap()
}
fn status_answer(pilot: &Pilot, args: &[&str]) -> Value {
    let result = status_run(pilot, args);
    assert!(
        result.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn member_keys(value: &Value) -> Vec<String> {
    value.as_object().unwrap().keys().cloned().collect()
}
#[test]
fn status_layers_is_opt_in_and_empty_until_firestore_exists() {
    let mut pilot = Pilot::new();
    start_door(&mut pilot, &[], false);
    let (run, json, keys) = (status_run, status_answer, member_keys);
    let ordinary = json(&pilot, &["status", "--json"]);
    assert_eq!(keys(&ordinary), ["running", "origin", "path"]);
    let layers = json(&pilot, &["status", "--layers", "--json"]);
    assert_eq!(
        keys(&layers),
        ["running", "origin", "path", "firestoreLayers"]
    );
    assert_eq!(layers["firestoreLayers"], serde_json::json!([]));
    assert_eq!(layers["origin"], ordinary["origin"]);
    // Plain human status stays one running line plus the address; --layers adds the layer lines.
    let human = String::from_utf8(run(&pilot, &["status"]).stdout).unwrap();
    assert!(!human.contains("Firestore"), "{human}");
    let with = String::from_utf8(run(&pilot, &["status", "--layers"]).stdout).unwrap();
    assert!(with.starts_with(&human), "{with}");
    assert!(with.contains("Firestore is not configured"), "{with}");
    assert!(!with.contains("deploy"), "{with}");
    // The three optional projections are exclusive.
    for args in [
        ["status", "--layers", "--machine", "--json"],
        ["status", "--layers", "--objects", "--json"],
    ] {
        assert!(!run(&pilot, &args).status.success(), "{args:?}");
    }
    terminate(pilot.child.take().unwrap());
    let stopped = json(&pilot, &["status", "--layers", "--json"]);
    assert_eq!(keys(&stopped), ["running", "lastPort"]);
    assert!(
        String::from_utf8(run(&pilot, &["status", "--layers"]).stdout)
            .unwrap()
            .contains("not running")
    );
}

#[test]
fn status_budget_is_opt_in_and_static() {
    let mut pilot = Pilot::new();
    start_door(&mut pilot, &[], false);
    let ordinary = status_answer(&pilot, &["status", "--json"]);
    let budget = status_answer(&pilot, &["status", "--budget", "--json"]);
    assert_eq!(
        member_keys(&budget),
        ["running", "origin", "path", "firestoreBudget"]
    );
    assert_eq!(budget["origin"], ordinary["origin"]);
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/firestore_budget/limits-member.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(budget["firestoreBudget"], fixture);
    // Plain human status stays one running line plus the address; --budget adds the table.
    let human = String::from_utf8(status_run(&pilot, &["status"]).stdout).unwrap();
    assert!(!human.contains("free-plan"), "{human}");
    let with = String::from_utf8(status_run(&pilot, &["status", "--budget"]).stdout).unwrap();
    assert!(with.starts_with(&human), "{with}");
    assert!(with.contains("Document reads"), "{with}");
    assert!(with.contains("50,000 per day"), "{with}");
    assert!(with.contains("check it in the Firebase console"), "{with}");
    // The optional projections are exclusive.
    for args in [
        ["status", "--budget", "--layers", "--json"],
        ["status", "--budget", "--machine", "--json"],
        ["status", "--budget", "--objects", "--json"],
    ] {
        assert!(!status_run(&pilot, &args).status.success(), "{args:?}");
    }
    terminate(pilot.child.take().unwrap());
    let stopped = status_answer(&pilot, &["status", "--budget", "--json"]);
    assert_eq!(member_keys(&stopped), ["running", "lastPort"]);
    assert!(
        String::from_utf8(status_run(&pilot, &["status", "--budget"]).stdout)
            .unwrap()
            .contains("not running")
    );
}

#[test]
fn status_stop_and_serve_diagnostics_preserve_the_deploy_record_while_its_writer_is_locked() {
    use tmt_remote::{deploy_record::DeployRecordStore, deploy_run::DeployRecord, state::Layout};
    let mut pilot = Pilot::new();
    let layout = Layout::open(&pilot.root.join("state")).unwrap();
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    writer
        .persist(&DeployRecord::new("3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11"))
        .unwrap();
    let path = layout.directory.join("deploy.json");
    let before = fs::read(&path).unwrap();
    assert_eq!(status_json(&pilot)["running"], false);
    assert_eq!(stop_json(&pilot)["running"], false);
    start_door(&mut pilot, &["--background", "--port", "0"], false);
    assert_eq!(status_json(&pilot)["running"], true);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(stop_json(&pilot)["stopped"], true);
    assert_eq!(status_json(&pilot)["running"], false);
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(writer);
    drop(DeployRecordStore::open(&layout).unwrap());
}
