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
