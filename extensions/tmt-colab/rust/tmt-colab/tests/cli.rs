use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
struct Pilot {
    root: PathBuf,
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
impl Pilot {
    fn new(reply: Option<&str>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-847-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pilot = Self {
            root,
            child: None,
            reader: None,
        };
        let response = reply
            .map(str::to_owned)
            .unwrap_or_else(|| json!({"dataRoot":pilot.root.join("selected")}).to_string());
        let payload = format!(
            "#!/bin/sh\nif [ \"$1\" = __fixture_ready ]; then exit 0; fi\n[ \"$#\" = 1 ] && [ \"$1\" = api ] || exit 9\ncd {} || exit 9\nprintf '%s\\n' \"$*\" >> calls\ncat > input\nprintf '%s\\n' {}\n",
            quote(pilot.root.to_str().unwrap()),
            quote(&response)
        );
        fs::write(pilot.root.join("core"), payload).unwrap();
        fs::set_permissions(pilot.root.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        // Probe only the no-effect fixture branch, never retry the product invocation.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match Command::new(pilot.root.join("core"))
                .arg("__fixture_ready")
                .output()
            {
                Ok(output) => {
                    assert!(output.status.success());
                    break;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("fixture publication failed: {e}"),
            }
        }
        assert!(!pilot.root.join("calls").exists());
        pilot
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("TMUX_TEAM_HOME", self.root.join("selected"))
            .env("TMT_EXECUTABLE", self.root.join("core"));
        cmd
    }
    fn call(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.contains(&0x1b));
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn start(&mut self) -> Value {
        self.child = Some(
            self.command()
                .args(["serve", "--port", "0", "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pipe = self.child.as_mut().unwrap().stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        self.reader = Some(thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(pipe).read_line(&mut line).unwrap();
            let _ = tx.send(line);
        }));
        let line = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        self.reader.take().unwrap().join().unwrap();
        assert!(!line.contains('\u{1b}'));
        serde_json::from_str(&line).unwrap()
    }
    fn stop(&mut self) {
        let child = self.child.as_mut().unwrap();
        let pid = Pid::from_raw(child.id() as i32);
        kill(pid, Signal::SIGTERM).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait().unwrap() {
                Some(status) => {
                    assert!(status.success());
                    break;
                }
                None => {
                    assert!(Instant::now() < deadline, "foreground process did not stop");
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        self.child.take();
        assert_eq!(
            kill(pid, None),
            Err(Errno::ESRCH),
            "foreground process leaked"
        );
    }
}
impl Drop for Pilot {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            child.wait().unwrap();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn help_and_invalid_core_fail_without_creating_application_state() {
    let pilot = Pilot::new(None);
    let short = pilot.command().args(["serve", "-h"]).output().unwrap();
    let long = pilot.command().args(["help", "serve"]).output().unwrap();
    assert!(short.status.success() && long.status.success());
    assert_eq!(short.stdout, long.stdout);
    assert!(String::from_utf8_lossy(&short.stdout).contains("default 7341"));
    for args in [["serve", "--bind", "0.0.0.0"], ["serve", "--port", "65536"]] {
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
    let invalid = pilot
        .command()
        .args(["serve", "--port", "65536", "--json"])
        .output()
        .unwrap();
    assert!(!invalid.status.success() && invalid.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    for supplied in [None, Some("relative-core")] {
        let mut cmd = pilot.command();
        match supplied {
            None => {
                cmd.env_remove("TMT_EXECUTABLE");
            }
            Some(path) => {
                cmd.env("TMT_EXECUTABLE", path);
            }
        }
        let output = cmd
            .args(["serve", "--port", "0", "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            "COLAB_UNAVAILABLE"
        );
        assert!(output.stderr.is_empty());
    }
    assert!(!pilot.root.join("calls").exists());
    assert!(!pilot.root.join("selected").exists());
    for (reply, code) in [
        ("{}", "COLAB_UNAVAILABLE"),
        ("{\"dataRoot\":\"relative\"}", "COLAB_ROOT_INVALID"),
        (
            "{\"error\":{\"code\":\"API_OPERATION_UNKNOWN\"}}",
            "COLAB_UNAVAILABLE",
        ),
    ] {
        let bad = Pilot::new(Some(reply));
        let output = bad.command().args(["spaces", "--json"]).output().unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            code
        );
        assert!(!bad.root.join("selected").exists());
    }
}
#[test]
fn listing_is_read_only_and_foreground_shutdown_releases_state_and_sockets_twice() {
    for _ in 0..2 {
        let mut pilot = Pilot::new(None);
        assert_eq!(pilot.call(&["spaces", "--json"]), json!({"spaces":[]}));
        assert!(!pilot.root.join("selected").exists());
        let descriptor = pilot.start();
        let id = descriptor["spaceId"].as_str().unwrap();
        assert_eq!(descriptor["state"], "unauthenticated");
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(pilot.root.join("input")).unwrap()).unwrap(),
            json!({"version":1,"operation":"storage.root","input":{}})
        );
        let address = descriptor["url"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .trim_end_matches('/');
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .write_all(format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .unwrap();
        let mut page = String::new();
        socket.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("TMT Colab"));
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n",
            "door invoked core"
        );
        assert_eq!(
            pilot.call(&["spaces", "--json"]),
            json!({"spaces":[{"spaceId":id,"backend":"local","running":true}]})
        );
        let duplicate = pilot
            .command()
            .args(["serve", "--port", "0", "--json"])
            .output()
            .unwrap();
        assert!(!duplicate.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&duplicate.stdout).unwrap()["error"]["code"],
            "COLAB_ALREADY_SERVING"
        );
        let owner = pilot.root.join("selected/colab/owner.key");
        let saved = fs::read(&owner).unwrap();
        pilot.stop();
        assert!(TcpStream::connect(address).is_err(), "listener leaked");
        assert_eq!(
            pilot.call(&["spaces", "--json"]),
            json!({"spaces":[{"spaceId":id,"backend":"local","running":false}]})
        );
        assert_eq!(fs::read(owner).unwrap(), saved);
        assert_eq!(
            fs::read_dir(pilot.root.join("selected")).unwrap().count(),
            1,
            "created core files"
        );
    }
}

#[test]
fn occupied_port_fails_with_a_port_hint() {
    let pilot = Pilot::new(None);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let output = pilot
        .command()
        .args(["serve", "--port", &port, "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains(&format!("port {port} is busy")));
    assert!(message.contains("--port 0"));
    // Failure releases the service lock and leaves an explicit free-port retry usable.
    let mut pilot = pilot;
    pilot.start();
    pilot.stop();
}

#[test]
fn signin_requires_origin_proof_possession_and_one_live_code() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer, SigningKey};
    use tmt_colab_model::{auth, crypto};
    let mut pilot = Pilot::new(None);
    let descriptor = pilot.start();
    let space = descriptor["spaceId"].as_str().unwrap();
    let signup = descriptor["signInUrl"].as_str().unwrap();
    let (path, secret) = signup.split_once('#').unwrap();
    let code_id = path.rsplit('/').next().unwrap();
    let secret: [u8; 16] = URL_SAFE_NO_PAD.decode(secret).unwrap().try_into().unwrap();
    let device = SigningKey::from_bytes(&[13; 32]);
    let public = device.verifying_key().to_bytes();
    let input = auth::SignIn {
        code_id,
        space,
        device: "abd7fb81-aac1-48d3-8311-50e012825e8b",
        signing_key: &public,
        encryption_key: &[14; 32],
        nonce: &[15; 16],
    };
    let proof = auth::signin_proof(&secret, &input).unwrap();
    let signature = device
        .sign(&auth::signin_possession_input(&input).unwrap())
        .to_bytes();
    let body = |proof: &[u8], signature: &[u8]| {
        serde_json::to_string(&json!({"input":URL_SAFE_NO_PAD.encode(auth::signin_input(&input).unwrap()),"proof":URL_SAFE_NO_PAD.encode(proof),"signature":URL_SAFE_NO_PAD.encode(signature)})).unwrap()
    };
    let url = descriptor["url"].as_str().unwrap();
    let origin = url.trim_end_matches('/');
    let host = origin.strip_prefix("http://").unwrap();
    let post = |body: &str, origin: &str| {
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket,"POST /s/{space}/signin HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        let mut reply = String::new();
        socket.read_to_string(&mut reply).unwrap();
        reply
    };
    let db = rusqlite::Connection::open(pilot.root.join("selected/colab/space.db")).unwrap();
    let sessions = || {
        db.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let good = body(&proof, &signature);
    assert!(post(&good, "http://attacker.invalid").starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    assert!(post(&body(&[0; 32], &signature), origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    assert!(post(&body(&proof, &[0; 64]), origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    assert!(post(&"x".repeat(65537), origin).starts_with("HTTP/1.1 413"));
    assert_eq!(sessions(), 0);
    let mut unknown: Value = serde_json::from_str(&good).unwrap();
    unknown["extra"] = json!(true);
    assert!(post(&unknown.to_string(), origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    let input_binary = URL_SAFE_NO_PAD.encode(auth::signin_input(&input).unwrap());
    let duplicate = format!(
        "{{\"input\":\"{input_binary}\",\"input\":\"{input_binary}\",\"proof\":\"{}\",\"signature\":\"{}\"}}",
        URL_SAFE_NO_PAD.encode(proof),
        URL_SAFE_NO_PAD.encode(signature)
    );
    assert!(post(&duplicate, origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    let expires: i64 = db
        .query_row(
            "SELECT expires FROM signin_codes WHERE code=?",
            [code_id],
            |r| r.get(0),
        )
        .unwrap();
    db.execute("UPDATE signin_codes SET expires=0 WHERE code=?", [code_id])
        .unwrap();
    assert!(post(&good, origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 0);
    db.execute(
        "UPDATE signin_codes SET expires=? WHERE code=?",
        rusqlite::params![expires, code_id],
    )
    .unwrap();
    let padded = format!("{good}{}", " ".repeat(65536 - good.len()));
    let issued = post(&padded, origin);
    assert!(issued.starts_with("HTTP/1.1 200"));
    assert_eq!(sessions(), 1);
    let response: Value = serde_json::from_str(issued.split_once("\r\n\r\n").unwrap().1).unwrap();
    let owner: [u8; 32] =
        tmt_colab_model::values::binary(response["ownerKey"].as_str().unwrap(), 32)
            .unwrap()
            .try_into()
            .unwrap();
    assert_eq!(crypto::space_id(&owner).unwrap(), space);
    let chain = tmt_colab_model::certificate::Chain::from_json(
        &serde_json::to_vec(&response["chain"]).unwrap(),
    )
    .unwrap();
    assert_eq!(chain.certificate().unwrap().device_id, input.device);
    let cookie = issued
        .lines()
        .find(|l| l.starts_with("Set-Cookie: "))
        .unwrap()
        .trim_end();
    assert!(
        cookie.contains(&format!("Path=/s/{space}/"))
            && cookie.contains("HttpOnly; SameSite=Strict")
    );
    assert!(!cookie.contains("Secure"));
    let token = cookie
        .strip_prefix("Set-Cookie: tmt_colab_session=")
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let token = URL_SAFE_NO_PAD.decode(token).unwrap();
    assert_eq!(token.len(), 32);
    let persisted: Vec<u8> = db
        .query_row("SELECT token_hash FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(persisted, crypto::digest(&token));
    let reply: Value = serde_json::from_str(issued.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(reply.get("token").is_none());
    assert!(post(&good, origin).starts_with("HTTP/1.1 403"));
    assert_eq!(sessions(), 1);
    drop(db);
    pilot.stop();
}
