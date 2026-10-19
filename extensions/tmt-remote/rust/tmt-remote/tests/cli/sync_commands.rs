//! CLI scenarios use private fixture stores and never invoke a provider.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use tmt_remote::{firebase_auth::presentation, state::Layout};

fn imported(pilot: &Pilot) -> Layout {
    let layout = Layout::open(&pilot.root.join("state")).unwrap();
    let mut descriptor: Value =
        serde_json::from_str(include_str!("../fixtures/remote_link/descriptor-v1.json")).unwrap();
    descriptor.sort_all_objects();
    let link = format!(
        "https://demo-remote-1.web.app/pair#{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&descriptor).unwrap())
    );
    tmt_remote::remote_catalog::add(&layout, "tenant", &link).unwrap();
    layout
}
fn credential(layout: &Layout, end: u64) -> PathBuf {
    let directory = layout.directory.join("firebase-auth");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("demo-remote-1.json");
    let created = end - 365 * 86_400_000;
    let bytes = serde_json::to_vec(&serde_json::json!({"version":1,"projectId":"demo-remote-1","uid":"fixture-long-identity","provider":"anonymous","refreshToken":"fixture-refresh-secret","createdAtMs":created,"expiresAtMs":end,"lastUsedAtMs":created,"state":"active"})).unwrap();
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    path
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}
#[test]
fn imported_active_sign_in_is_reused_offline_without_deployment_or_secret_output() {
    let pilot = Pilot::new();
    let layout = imported(&pilot);
    let path = credential(&layout, now() + 200 * 86_400_000);
    let bytes = fs::read(&path).unwrap();
    let run = pilot
        .command()
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .args(["--remote", "tenant", "sync", "sign-in", "--json"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stdout)
    );
    let answer: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(answer["uid"], "fixture-long-identity");
    assert_eq!(answer["reused"], true);
    assert_eq!(answer["signIn"]["state"], "active");
    assert!(!String::from_utf8_lossy(&run.stdout).contains("fixture-refresh-secret"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!layout.directory.join("deployments").exists());
    let status = pilot
        .command()
        .args(["--remote", "tenant", "status", "--json"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["signIn"], answer["signIn"]);
    let mismatch = pilot
        .command()
        .args([
            "--remote",
            "tenant",
            "sync",
            "sign-in",
            "--project",
            "wrong-project",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!mismatch.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&mismatch.stdout).unwrap()["error"]["code"],
        "REMOTE_SELECTOR_INVALID"
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn ended_status_purges_the_secret_and_an_empty_revoke_is_idempotent() {
    let pilot = Pilot::new();
    let layout = imported(&pilot);
    let path = credential(&layout, now() - 1);
    let run = pilot
        .command()
        .args(["--remote", "tenant", "status"])
        .output()
        .unwrap();
    assert!(run.status.success());
    assert!(String::from_utf8_lossy(&run.stdout).contains(presentation::SIGN_IN_ENDED));
    assert!(
        String::from_utf8_lossy(&run.stdout)
            .contains("Page access for this machine is not available yet.")
    );
    assert!(!path.exists());
    for _ in 0..2 {
        let run = pilot
            .command()
            .args(["--remote", "tenant", "sync", "revoke", "--json"])
            .output()
            .unwrap();
        assert!(run.status.success());
        let answer: Value = serde_json::from_slice(&run.stdout).unwrap();
        assert_eq!(answer["uid"], Value::Null);
        assert_eq!(
            answer["signIn"],
            serde_json::json!({"state":"none","expiresAtMs":null})
        );
        assert!(!path.exists());
    }
}

#[test]
fn diagnostic_status_preserves_door_when_credentials_are_corrupt_or_unsafe() {
    for unsafe_file in [false, true] {
        let pilot = Pilot::new();
        let layout = imported(&pilot);
        let path = credential(&layout, now() + 200 * 86_400_000);
        if unsafe_file {
            // Foreign-owner admission and unsafe path admission both return
            // InvalidCredential; exercise the unsafe path without privileged chown.
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(pilot.root.join("foreign"), &path).unwrap();
        } else {
            fs::write(&path, b"{broken-secret-canary").unwrap();
        }
        let run = pilot
            .command()
            .args(["--remote", "tenant", "status", "--json"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stdout)
        );
        let answer: Value = serde_json::from_slice(&run.stdout).unwrap();
        assert_eq!(answer["running"], false);
        assert_eq!(answer["lastPort"], Value::Null);
        assert_eq!(
            answer["signIn"],
            serde_json::json!({"state":"unreadable","expiresAtMs":null})
        );
        assert!(!String::from_utf8_lossy(&run.stdout).contains("broken-secret-canary"));
        let human = pilot
            .command()
            .args(["--remote", "tenant", "status"])
            .output()
            .unwrap();
        assert!(human.status.success());
        let text = String::from_utf8(human.stdout).unwrap();
        assert!(text.contains("Firestore sign-in file could not be read."));
        let missing = pilot
            .command()
            .args(["--remote", "missing", "status", "--json"])
            .output()
            .unwrap();
        assert!(
            !missing.status.success(),
            "selection failure must still refuse"
        );
    }
}

#[test]
fn diagnostic_status_preserves_door_when_credential_writer_is_busy() {
    let pilot = Pilot::new();
    let layout = imported(&pilot);
    let path = credential(&layout, now() + 200 * 86_400_000);
    let bytes = fs::read(&path).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path.with_extension("lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let run = pilot
        .command()
        .args(["--remote", "tenant", "status", "--json"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stdout)
    );
    let answer: Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(answer["running"], false);
    assert_eq!(
        answer["signIn"],
        serde_json::json!({"state":"busy","expiresAtMs":null})
    );
    let human = pilot
        .command()
        .args(["--remote", "tenant", "status"])
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(
        String::from_utf8_lossy(&human.stdout)
            .contains("Firestore sign-in could not be confirmed.")
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[cfg(not(feature = "test-emulator"))]
#[test]
fn default_sign_in_ignores_emulator_variables_and_reuses_the_identity() {
    let pilot = Pilot::new();
    let layout = imported(&pilot);
    let path = credential(&layout, now() + 200 * 86_400_000);
    let bytes = fs::read(&path).unwrap();
    let result = pilot
        .command()
        .env("FIREBASE_AUTH_EMULATOR_HOST", "example.com:9099")
        .env("FIRESTORE_EMULATOR_HOST", "invalid")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .args(["--remote", "tenant", "sync", "sign-in", "--json"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let answer: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(answer["reused"], true);
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[cfg(feature = "test-emulator")]
#[test]
fn feature_sign_in_reaches_loopback_auth_and_persists_one_native_credential() {
    use std::os::fd::AsFd;
    let pilot = Pilot::new();
    let layout = imported(&pilot);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let claims = serde_json::json!({"aud":"demo-remote-1", "sub":"emulator-cli-user", "firebase":{"sign_in_provider":"anonymous"}});
    let token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    let reply = serde_json::json!({"localId":"emulator-cli-user", "idToken":token,
        "refreshToken":"private-cli-refresh-canary", "expiresIn":"3600"})
    .to_string();
    let (result, request) = std::thread::scope(|scope| {
        let peer = scope.spawn(|| {
            let mut events = [nix::poll::PollFd::new(
                listener.as_fd(),
                nix::poll::PollFlags::POLLIN,
            )];
            assert!(
                nix::poll::poll(&mut events, 15_000u16).unwrap() > 0,
                "sign-in never reached loopback"
            );
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
            }
            assert!(length < 1024);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                serde_json::json!({"returnSecureToken":true})
            );
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
            request
        });
        let result = pilot
            .command()
            .env("FIREBASE_AUTH_EMULATOR_HOST", &address)
            .env("FIRESTORE_EMULATOR_HOST", &address)
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .args(["--remote", "tenant", "sync", "sign-in", "--json"])
            .output()
            .unwrap();
        if !result.status.success() {
            eprintln!(
                "private sign-in refused: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        (result, peer.join().unwrap())
    });
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(request.starts_with("POST /identitytoolkit.googleapis.com/v1/accounts:signUp?key="));
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "one signup request only"
    );
    let answer: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(answer["uid"], "emulator-cli-user");
    assert_eq!(answer["signIn"]["state"], "active");
    assert_eq!(answer["reused"], false);
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-cli-refresh-canary"));
    let path = layout.directory.join("firebase-auth/demo-remote-1.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let stored: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(stored["uid"], "emulator-cli-user");
    assert_eq!(stored["state"], "active");
}
