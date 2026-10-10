#[path = "support/core_fixture.rs"]
mod core_fixture;
mod support;
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
struct Pilot {
    root: PathBuf,
    response: String,
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
        let response = reply
            .map(str::to_owned)
            .unwrap_or_else(|| json!({"dataRoot":root.join("selected")}).to_string());
        let pilot = Self {
            root,
            response,
            child: None,
            reader: None,
        };
        core_fixture::link(&pilot.root, "core", core_fixture::Program::Core);
        assert!(!pilot.root.join("calls").exists());
        pilot
    }
    /// A core stand-in that also answers Remote's `status --json`; everything else is the fixture core.
    fn door_core(&self, status: &str, delay: Option<u32>) -> PathBuf {
        let sleep = delay.map_or(String::new(), |s| format!("sleep {s}\n"));
        fs::write(
            self.root.join("door-status"),
            format!("{sleep}printf '%s\\n' {}\nexit 0\n", quote(status)),
        )
        .unwrap();
        core_fixture::link(&self.root, "core-door", core_fixture::Program::Door)
    }
    /// Creation's optional machine projection; ordinary support remains available to detect fallback.
    fn creation_core(&self, status: Option<&str>, delay: Option<u32>) -> PathBuf {
        let path = self.root.join("core-creation");
        let snapshot = status.map_or_else(
            || "exit 9\n".to_owned(),
            |status| {
                let sleep = delay.map_or(String::new(), |s| format!("sleep {s}\n"));
                let (body, exit) = status
                    .strip_prefix('!')
                    .map_or((status, 0), |body| (body, 1));
                format!("{sleep}printf '%s\\n' {}\nexit {exit}\n", quote(body))
            },
        );
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = remote ]; then
printf '%s\n' "$*" >> {calls}
fi
case "$*" in
'remote status --machine --json')
{snapshot};;
'remote status --json')
printf '%s\n' {ordinary}
exit 0
;;
'remote devices --json')
printf '%s\n' '{{"devices":[]}}'
exit 0
;;
esac
exec {core} "$@"
"#,
            calls = quote(self.root.join("creation-remote.calls").to_str().unwrap()),
            ordinary = quote(DOOR),
            core = quote(self.root.join("core").to_str().unwrap()),
        );
        tmt_test_support::write_executable(&path, script.as_bytes(), 0o700).unwrap();
        path
    }
    fn command_with_creation_door(&self, status: &str) -> Command {
        let mut cmd = self.command();
        cmd.env("TMT_EXECUTABLE", self.creation_core(Some(status), None));
        cmd
    }
    fn creation_calls(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("creation-remote.calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn assert_creation_acquisitions(&self, count: usize) {
        let calls = self.creation_calls();
        assert_eq!(
            calls
                .iter()
                .filter(|s| *s == "remote status --machine --json")
                .count(),
            count,
            "{calls:?}"
        );
        assert!(
            calls
                .iter()
                .all(|s| s == "remote status --machine --json" || s == "remote devices --json"),
            "unexpected call or fallback: {calls:?}"
        );
    }
    fn command_with_door(&self, status: &str) -> Command {
        let mut cmd = self.command();
        cmd.env("TMT_EXECUTABLE", self.door_core(status, None));
        cmd
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.env_clear()
            // Automatic browser effects are opt-in in fixtures, never a host opener.
            .env("CI", "1")
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("TMPDIR", &self.root)
            .env("TMT_HOME", self.root.join("selected"))
            .env("TMT_EXECUTABLE", self.root.join("core"))
            .env("TMT_COLAB_TEST_ROOT", &self.root)
            .env("TMT_COLAB_TEST_REPLY", &self.response)
            // A stub opener lives in `bin`; a real one must never be reached by a test.
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            );
        cmd
    }
    /// A stand-in for the platform opener (`open` / `xdg-open`) that records each link it gets.
    fn opener(&self, exit: i32) {
        let name = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        fs::create_dir_all(self.root.join("bin")).unwrap();
        let path = self.root.join("bin").join(name);
        tmt_test_support::write_executable(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\nexit {exit}\n",
                quote(self.root.join("opened").to_str().unwrap())
            )
            .as_bytes(),
            0o700,
        )
        .unwrap();
        assert!(!self.root.join("opened").exists());
    }
    fn opened(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("opened"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
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
                .args(["serve", "--json"])
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
    assert!(String::from_utf8_lossy(&short.stdout).contains("door.sock"));
    // There is no TCP listener to configure any more.
    for args in [["serve", "--bind", "0.0.0.0"], ["serve", "--port", "0"]] {
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
        let output = cmd.args(["serve", "--json"]).output().unwrap();
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
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(pilot.root.join("input")).unwrap()).unwrap(),
            json!({"version":1,"operation":"storage.root","input":{}})
        );
        // The descriptor names the socket under the resolved data root.
        let path = fs::canonicalize(&pilot.root)
            .unwrap()
            .join("selected/colab/door.sock");
        assert_eq!(descriptor["socket"], path.to_str().unwrap());
        assert_eq!(descriptor["state"], "mounted");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut socket = UnixStream::connect(&path).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1:1\r\n\r\n")
            .unwrap();
        let mut page = String::new();
        socket.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("This Colab space is private"));
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n",
            "door invoked core"
        );
        assert_eq!(
            pilot.call(&["spaces", "--json"]),
            json!({"spaces":[{"spaceId":id,"backend":"local","running":true}]})
        );
        let duplicate = pilot.command().args(["serve", "--json"]).output().unwrap();
        assert!(!duplicate.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&duplicate.stdout).unwrap()["error"]["code"],
            "COLAB_ALREADY_SERVING"
        );
        let owner = pilot.root.join("selected/colab/owner.key");
        let saved = fs::read(&owner).unwrap();
        pilot.stop();
        assert!(!path.exists(), "socket left behind");
        assert!(UnixStream::connect(&path).is_err(), "listener leaked");
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
fn only_a_stale_own_socket_is_replaced_and_long_paths_refuse() {
    let mut pilot = Pilot::new(None);
    let directory = pilot.root.join("selected/colab");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("door.sock");
    // A non-socket at the path is never removed.
    fs::write(&path, b"keep").unwrap();
    let output = pilot.command().args(["serve", "--json"]).output().unwrap();
    assert!(!output.status.success() && output.stderr.is_empty());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_STATE_UNSAFE");
    assert_eq!(fs::read(&path).unwrap(), b"keep");
    fs::remove_file(&path).unwrap();
    // A socket left by an earlier serve is replaced, and removed on exit.
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    pilot.start();
    assert!(UnixStream::connect(&path).is_ok());
    pilot.stop();
    assert!(!path.exists());
    // A data root too deep for a Unix socket path refuses with the path named.
    let deep = pilot.root.join("d".repeat(60)).join("e".repeat(40));
    fs::create_dir_all(&deep).unwrap();
    let long = Pilot::new(Some(&json!({ "dataRoot": deep }).to_string()));
    let output = long.command().args(["serve", "--json"]).output().unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_SOCKET_PATH_TOO_LONG");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("door.sock")
    );
}

#[test]
fn invalid_explicit_app_path_fails_before_creating_state() {
    let pilot = Pilot::new(None);
    for directory in [pilot.root.join("missing"), PathBuf::from("relative")] {
        let output = pilot
            .command()
            .args(["serve", "--json", "--app-dir"])
            .arg(directory)
            .output()
            .unwrap();
        assert!(!output.status.success() && output.stderr.is_empty());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "COLAB_APP_UNAVAILABLE");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains(tmt_colab::assets::BUILD_HINT)
        );
        assert!(!pilot.root.join("selected").exists());
    }
}

#[test]
fn package_version_exits_without_core_or_state_access() {
    for flag in ["--version", "-V"] {
        let output = Command::new(BINARY).env_clear().arg(flag).output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout,
            format!("colab {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
    }
}

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const MEMBER: &str = "20000000-0000-4000-8000-000000000001";
const LINK: &str = "20000000-0000-4000-8000-000000000002";
const OPERATION: &str = "40000000-0000-4000-8000-000000000001";
fn seed_page(pilot: &Pilot) {
    seed_page_with_title(pilot, "Encrypted π\u{1b}[31m");
}
fn seed_page_with_title(pilot: &Pilot, title: &str) {
    use ed25519_dalek::{Signer, SigningKey};
    use tmt_colab::{
        keyring::{Keyring, Layout},
        store::{
            Envelope, Namespace, Store, StreamScope,
            owner::{Device, Mutation, Recipient},
        },
    };
    use tmt_colab_model::{certificate, object, values, wrap};
    use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
    let layout = Layout::open(&pilot.root.join("selected")).unwrap();
    let key = Keyring::open(&layout).unwrap();
    let mut store = Store::open(&layout).unwrap();
    store.create_page(PAGE).unwrap();
    let editor = SigningKey::from_bytes(&[7; 32]);
    let device = SigningKey::from_bytes(&[9; 32]);
    let device_id = "30000000-0000-4000-8000-000000000001";
    store.owner_transaction(&key.space_id,&key.owner_public(),Mutation {operation_id:OPERATION,digest:[7;32],expected_revision:0},|tx| {
        let owner=key.management_member()?;
        for r in [Recipient {kind:"member".into(),id:owner.id,role:Some("editor".into()),signing_key:owner.signing_key,encryption_key:owner.encryption_key,pages:vec![],revoked:false},
            Recipient {kind:"member".into(),id:MEMBER.into(),role:Some("editor".into()),signing_key:editor.verifying_key().to_bytes(),encryption_key:wrap::RecipientKey::from_seed(&[8;32])?.public_key(),pages:vec![PAGE.into()],revoked:false}] {
            let payload=serde_json::to_vec(&json!({"memberId":r.id,"role":r.role,"signKey":values::encode_binary(&r.signing_key),"encKey":values::encode_binary(&r.encryption_key),"pages":r.pages}))?;
            let statement=key.sign_statement(tx.head(),"member.add",&payload)?;
            tx.append_statement(&statement)?;tx.put_recipient(&r)?;
            if r.id==MEMBER {
                let cert=certificate::input(&certificate::Certificate {space:&key.space_id,issuer_kind:"member",issuer_id:MEMBER,device_id,
                    signing_key:&device.verifying_key().to_bytes(),encryption_key:&wrap::RecipientKey::from_seed(&[10;32])?.public_key(),membership_revision:"2",issued_at:1,expires_at:9007199254740991})?;
                tx.put_device(&Device {revoked:false,chain:serde_json::to_vec(&json!({"version":1,"issuerStatement":values::encode_binary(&statement.hash()?),"deviceCertificate":values::encode_binary(&cert),"issuerSignature":values::encode_binary(&editor.sign(&cert).to_bytes())}))?})?;
            }
        }
        tx.put_epoch_secret(PAGE,1,&[11;32])?;Ok(b"seeded".to_vec())
    }).unwrap();
    let doc = Doc::new();
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    {
        let mut tx = doc.transact_mut();
        html.insert(&mut tx, 0, "<h1>Encrypted source</h1>");
        meta.insert(&mut tx, "title", title);
    }
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let object = object::seal(
        &object::Context {
            space: key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: device_id.into(),
            membership_revision: "2".into(),
            stream_seq: "1".into(),
            prev_hash: [0; 32],
        },
        &[11; 32],
        &device,
        &update,
    )
    .unwrap();
    store
        .append(&Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: device_id,
            },
            namespace: Namespace::Content,
            seq: 1,
            hash: object.hash().unwrap(),
            previous: [0; 32],
            bytes: &object.to_json().unwrap(),
        })
        .unwrap();
    store.close().unwrap();
}
fn failure(pilot: &Pilot, args: &[&str], code: &str) -> Value {
    let output = pilot.command().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty(), "{:?}", output);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], code, "{result}");
    result
}
/// A refused confirmation names its own consequence and the exact retry, never another
/// command's disclosure.
fn confirmation_required(pilot: &Pilot, args: &[&str], consequence: &str, action: &str) -> Value {
    let result = failure(pilot, args, "COLAB_CONFIRMATION_REQUIRED");
    let message = result["error"]["message"].as_str().unwrap();
    assert!(message.starts_with(consequence), "{message}");
    assert!(
        message.ends_with(&format!(" Run again with --yes to {action}.")),
        "{message}"
    );
    assert!(!message.contains("disclosure in help"), "{message}");
    assert_eq!(
        message.contains("permanent"),
        args.contains(&"delete"),
        "{message}"
    );
    result
}
#[test]
fn management_reads_verify_encrypted_titles_and_preserve_missing_and_existing_state() {
    let pilot = Pilot::new(None);
    assert_eq!(pilot.call(&["ls", "--json"])["pages"], json!([]));
    assert!(!pilot.root.join("selected").exists());
    failure(&pilot, &["show", PAGE, "--json"], "COLAB_PAGE_NOT_FOUND");
    assert!(!pilot.root.join("selected").exists());
    seed_page(&pilot);
    let db = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&db).unwrap();
    let list = pilot.call(&["ls", "--json"]);
    let show = pilot.call(&["show", PAGE, "--json"]);
    assert_eq!(list["pages"][0]["title"], "Encrypted π\u{1b}[31m");
    // `ls` adds each page's `path` and `link`; everything else is the same record.
    let mut listed = list["pages"][0].clone();
    for key in ["path", "link", "shortLink"] {
        listed.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(show["page"], listed);
    assert_eq!(show["members"][0]["id"], MEMBER);
    assert_eq!(show["page"]["warnings"], json!([]));
    let updated = show["page"]["lastUpdateAtMs"].as_u64().unwrap();
    assert_eq!(show["page"]["expiresAtMs"], updated + 30 * 86_400_000);
    assert_eq!(show["discussions"], "not-available");
    let human = pilot.command().args(["ls"]).output().unwrap();
    assert!(human.status.success());
    assert!(!human.stdout.contains(&0x1b));
    assert!(String::from_utf8_lossy(&human.stdout).contains("expires in 29 days"));
    assert_eq!(fs::read(db).unwrap(), before);
}
#[test]
fn management_confirmation_and_input_denials_have_no_state_effects() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let db = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&db).unwrap();
    confirmation_required(
        &pilot,
        &["share", "mode", PAGE, "link", "--json"],
        &format!(
            "Sharing page {PAGE} by link lets anyone with a link read it, and copies can't be recalled later."
        ),
        "share it by link",
    );
    confirmation_required(
        &pilot,
        &["share", "mode", PAGE, "public", "--json"],
        &format!(
            "Making page {PAGE} public lets anyone with the link read it, and copies can't be recalled later."
        ),
        "make it public",
    );
    failure(
        &pilot,
        &["show", "not-a-page", "--json"],
        "COLAB_INPUT_INVALID",
    );
    for args in [
        vec!["retention", PAGE, "0", "--json"],
        vec!["archive", "not-a-page", "--json"],
        vec!["delete", "not-a-page", "--yes", "--json"],
        vec!["share", "members", "list", PAGE, "--json"],
        vec!["share", "history", PAGE, "invalid", "--json"],
    ] {
        failure(&pilot, &args, "COLAB_INPUT_INVALID");
    }
    assert_eq!(fs::read(&db).unwrap(), before);
    let allowed = pilot.call(&["share", "mode", PAGE, "link", "--yes", "--json"]);
    assert!(allowed["operationId"].as_str().is_some());
    assert_eq!(allowed["expectedRevision"], "2");
    assert_eq!(allowed["membershipHead"]["revision"], "3");
    assert_eq!(
        pilot.call(&["show", PAGE, "--json"])["page"]["sharing"],
        "link"
    );
}
#[test]
fn link_add_without_a_seed_file_generates_a_fresh_seed_and_prints_the_reader_link_once() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    pilot.call(&["share", "mode", PAGE, "link", "--yes", "--json"]);
    let mut seeds = Vec::new();
    for _ in 0..2 {
        let outcome = pilot.call(&["share", "link", "add", PAGE, "--yes", "--json"]);
        let path = outcome["readerPath"].as_str().unwrap();
        let (route, fragment) = path.split_once('#').unwrap();
        assert_eq!(
            route,
            format!("/read/{}", outcome["linkId"].as_str().unwrap())
        );
        let pairs: Vec<(&str, &str)> = fragment
            .split('&')
            .map(|p| p.split_once('=').unwrap())
            .collect();
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["v", "space", "page", "link", "rev", "st", "seed"]);
        let get = |k: &str| pairs.iter().find(|(n, _)| *n == k).unwrap().1;
        assert_eq!(get("v"), "1");
        assert_eq!(get("page"), PAGE);
        assert_eq!(get("link"), outcome["linkId"]);
        assert_eq!(get("rev"), outcome["membershipHead"]["revision"]);
        assert_eq!(get("st"), outcome["membershipHead"]["statementHash"]);
        assert_eq!(
            tmt_colab_model::values::binary(get("seed"), 32)
                .unwrap()
                .len(),
            32
        );
        seeds.push(get("seed").to_owned());
        // Neither listing nor the rest of the result carries the seed.
        let listed = pilot
            .call(&["share", "link", "list", PAGE, "--json"])
            .to_string();
        assert!(!listed.contains(get("seed")));
        let mut rest = outcome.clone();
        rest.as_object_mut().unwrap().remove("readerPath");
        assert!(!rest.to_string().contains(get("seed")));
    }
    assert_ne!(seeds[0], seeds[1]);
    let human = pilot
        .command()
        .args(["share", "link", "add", PAGE, "--yes"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    let at = human.find("/read/").unwrap();
    let line = human[at..].lines().next().unwrap().trim();
    let (route, fragment) = line.split_once('#').unwrap();
    let id = route.strip_prefix("/read/").unwrap();
    tmt_colab_model::values::generated_id(id).unwrap();
    let shown = pilot.call(&["show", PAGE, "--json"]);
    let space = shown["spaceId"].as_str().unwrap();
    assert!(fragment.starts_with(&format!("v=1&space={space}&page={PAGE}&link={id}&")));
}
#[test]
fn management_link_cli_uses_serving_and_offline_service_with_durable_replay() {
    use tmt_colab_model::values;
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        seed_page(&pilot);
        // Seed audience policy through the real engine with the shared test decoder budget.
        {
            use tmt_colab::{
                keyring::{Keyring, Layout},
                store::Store,
                transitions::{Engine, OwnerAction, OwnerRequest, Publication, ShareMode},
            };
            let layout = Layout::open(&pilot.root.join("selected")).unwrap();
            let key = Keyring::read(&layout).unwrap();
            let mut store = Store::open(&layout).unwrap();
            let mut engine =
                Engine::with_decoder_config(support::decoder_config(PathBuf::from(BINARY)))
                    .unwrap();
            engine
                .apply(
                    &mut store,
                    &key,
                    OwnerRequest {
                        operation_id: "40000000-0000-4000-8000-000000000009",
                        expected_revision: 2,
                        action: OwnerAction::Share {
                            page: PAGE,
                            mode: ShareMode::Link,
                            publication: Publication::Loopback,
                        },
                        transport_digest: None,
                        scope: None,
                    },
                    1,
                )
                .unwrap();
            store.close().unwrap();
        }
        let path = pilot.root.join("seed");
        fs::write(&path, values::encode_binary(&[17; 32])).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        if serving {
            pilot.start();
        }
        let op = "40000000-0000-4000-8000-000000000002";
        let args = [
            "share",
            "link",
            "add",
            PAGE,
            "--seed-file",
            path.to_str().unwrap(),
            "--link-id",
            LINK,
            "--yes",
            "--operation-id",
            op,
            "--expected-revision",
            "3",
            "--json",
        ];
        let result = pilot.call(&args);
        assert_eq!(result["operationId"], op);
        assert_eq!(result["expectedRevision"], "3");
        assert_eq!(result["membershipHead"]["revision"], "4");
        let space = pilot.call(&["ls", "--json"])["spaceId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            result["readerPath"],
            format!(
                "/read/{LINK}#v=1&space={space}&page={PAGE}&link={LINK}&rev=4&st={}&seed={}",
                result["membershipHead"]["statementHash"].as_str().unwrap(),
                values::encode_binary(&[17; 32])
            )
        );
        if serving {
            pilot.stop();
        }
        let db = pilot.root.join("selected/colab/space.db");
        let committed = fs::read(&db).unwrap();
        assert_eq!(pilot.call(&args), result);
        assert_eq!(fs::read(&db).unwrap(), committed);
        fs::write(&path, values::encode_binary(&[18; 32])).unwrap();
        let conflict = failure(&pilot, &args, "COLAB_CONFLICT");
        assert_eq!(conflict["operationId"], op);
        assert_eq!(fs::read(&db).unwrap(), committed);
        let stale = failure(
            &pilot,
            &[
                "share",
                "link",
                "remove",
                PAGE,
                LINK,
                "--operation-id",
                "40000000-0000-4000-8000-000000000003",
                "--expected-revision",
                "3",
                "--json",
            ],
            "COLAB_STALE_HEAD",
        );
        assert_eq!(stale["expectedRevision"], "3");
        assert_eq!(fs::read(&db).unwrap(), committed);
        let listed = pilot.call(&["share", "link", "list", PAGE, "--json"]);
        assert_eq!(listed, pilot.call(&["share", "link", "ls", PAGE, "--json"]));
        assert!(
            listed["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == LINK && r["role"] == "viewer" && r["revoked"] == false)
        );
        let replacement = "20000000-0000-4000-8000-000000000003";
        let reset = [
            "share",
            "link",
            "reset",
            PAGE,
            LINK,
            "--seed-file",
            path.to_str().unwrap(),
            "--link-id",
            replacement,
            "--json",
        ];
        confirmation_required(
            &pilot,
            &reset,
            &format!(
                "Resetting link {LINK} turns off the old link and creates a new read-only one; anyone with the new link can read page {PAGE}."
            ),
            "reset the link",
        );
        assert_eq!(fs::read(&db).unwrap(), committed);
        let mut confirmed = reset.to_vec();
        confirmed.push("--yes");
        let outcome = pilot.call(&confirmed);
        assert_eq!(outcome["linkId"], replacement);
        assert_eq!(
            outcome["readerPath"],
            format!(
                "/read/{replacement}#v=1&space={space}&page={PAGE}&link={replacement}&rev={}&st={}&seed={}",
                outcome["membershipHead"]["revision"].as_str().unwrap(),
                outcome["membershipHead"]["statementHash"].as_str().unwrap(),
                values::encode_binary(&[18; 32])
            )
        );
        let listed_text = pilot
            .call(&["share", "link", "list", PAGE, "--json"])
            .to_string();
        assert!(!listed_text.contains(&values::encode_binary(&[18; 32])));
        let listed = pilot.call(&["share", "link", "list", PAGE, "--json"]);
        assert!(
            listed["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == LINK && r["revoked"] == true)
        );
        assert!(
            listed["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == replacement && r["role"] == "viewer" && r["revoked"] == false)
        );
        pilot.call(&["share", "link", "remove", PAGE, replacement, "--json"]);
        assert!(
            pilot.call(&["share", "link", "list", PAGE, "--json"])["links"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["revoked"] == true)
        );
        if serving {
            pilot.start();
        }
        let before = fs::read(&db).unwrap();
        failure(
            &pilot,
            &["share", "mode", PAGE, "public", "--json"],
            "COLAB_CONFIRMATION_REQUIRED",
        );
        assert_eq!(fs::read(&db).unwrap(), before);
        pilot.call(&["share", "mode", PAGE, "public", "--yes", "--json"]);
        assert_eq!(
            pilot.call(&["show", PAGE, "--json"])["page"]["sharing"],
            "public"
        );
        // Narrowing needs no widening confirmation, and encrypted content remains inspectable.
        pilot.call(&["share", "mode", PAGE, "private", "--json"]);
        let shown = pilot.call(&["show", PAGE, "--json"]);
        assert_eq!(shown["page"]["sharing"], "private");
        assert_eq!(shown["page"]["title"], "Encrypted π\u{1b}[31m");
        if serving {
            pilot.stop();
        }
    }
}

#[test]
fn management_seed_admission_and_interrupted_ipc_never_fall_back_to_offline_mutation() {
    use tmt_colab::{keyring::Layout, socket::SOCKET};
    use tmt_colab_model::values;
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let db = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&db).unwrap();
    let path = pilot.root.join("seed");
    let encoded = values::encode_binary(&[17; 32]);
    fs::write(&path, &encoded).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let args = [
        "share",
        "link",
        "add",
        PAGE,
        "--seed-file",
        path.to_str().unwrap(),
        "--link-id",
        LINK,
        "--yes",
        "--operation-id",
        "40000000-0000-4000-8000-000000000002",
        "--expected-revision",
        "2",
        "--json",
    ];
    let denial = failure(&pilot, &args, "COLAB_INPUT_INVALID");
    assert!(!denial.to_string().contains(&encoded));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let link = pilot.root.join("seed-link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let mut symlink_args = args;
    symlink_args[5] = link.to_str().unwrap();
    let output = pilot.command().args(symlink_args).output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&encoded));
    let layout = Layout::existing(&pilot.root.join("selected"))
        .unwrap()
        .unwrap();
    let _lock = layout.serve_lock().unwrap();
    let socket = layout.directory.join(SOCKET);
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut header_end = None;
        let mut total = None;
        loop {
            let mut buffer = [0; 4096];
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buffer[..n]);
            if header_end.is_none() {
                header_end = request
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|n| n + 4);
            }
            if let Some(end) = header_end {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                total = Some(end + length);
            }
            if total == Some(request.len()) {
                break;
            }
        }
        assert!(request.starts_with(b"POST /.tmt/colab/management HTTP/1.1"));
        assert!(!String::from_utf8_lossy(&request[..header_end.unwrap()]).contains("tmt-device"));
        let body: Value = serde_json::from_slice(&request[header_end.unwrap()..]).unwrap();
        assert_eq!(body["operation"], "link.add");
        // Drop after acquisition, before acknowledgment. A CLI fallback would
        // apply this valid request through the offline service and alter state.
    });
    let unknown = failure(&pilot, &args, "COLAB_OUTCOME_UNKNOWN");
    server.join().unwrap();
    assert_eq!(unknown["linkId"], LINK);
    assert_eq!(unknown["expectedRevision"], "2");
    assert!(!unknown.to_string().contains(&encoded));
    assert_eq!(fs::read(db).unwrap(), before);
    fs::remove_file(socket).unwrap();
}
#[test]
fn management_read_refuses_unsafe_state_without_changes() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let directory = pilot.root.join("selected/colab");
    let db = directory.join("space.db");
    let before = fs::read(&db).unwrap();
    let key = directory.join("owner.key");
    fs::remove_file(&key).unwrap();
    std::os::unix::fs::symlink("missing", &key).unwrap();
    let output = pilot.command().args(["ls", "--json"]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(db).unwrap(), before);
}

#[test]
fn schema_mismatches_name_recovery_in_human_and_json_without_changing_state() {
    for (version, code, message, hint, command) in [
        (
            4,
            "COLAB_STORE_OUTDATED",
            "This space was saved by an older Colab.",
            "Start or restart tmt colab serve to update it.",
            "tmt colab serve",
        ),
        (
            99,
            "COLAB_STORE_NEWER",
            "This space was saved by a newer Colab.",
            "Run tmt upgrade, then try again.",
            "tmt upgrade",
        ),
    ] {
        let pilot = Pilot::new(None);
        seed_page(&pilot);
        let db = pilot.root.join("selected/colab/space.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let supported: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        if version == 4 {
            // Restore the schema-4 fixture used by state.rs, not just its version stamp.
            conn.execute_batch("ALTER TABLE pages DROP COLUMN last_update_at_ms")
                .unwrap();
        }
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);
        let before = fs::read(&db).unwrap();
        for args in [
            vec!["ls"],
            vec!["show", PAGE],
            vec!["page", "read", PAGE],
            vec!["export", PAGE],
        ] {
            let mut json_args = args.clone();
            json_args.push("--json");
            let result = failure(&pilot, &json_args, code);
            assert_eq!(result["error"]["message"], message);
            assert_eq!(result["error"]["storeSchema"], version);
            assert_eq!(result["error"]["supportedSchema"], supported);
            assert_eq!(result["next"], json!([command]));
            let output = pilot.command().args(&args).output().unwrap();
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!(
                    "error: {}\nhint: {}\n",
                    message.trim_end_matches('.'),
                    hint.trim_end_matches('.')
                )
            );
            assert_eq!(fs::read(&db).unwrap(), before);
        }
        let connection = rusqlite::Connection::open(&db).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            version
        );
        assert!(!pilot.root.join("selected/colab/door.sock").exists());
        assert!(!pilot.root.join("selected/colab/exports").exists());
    }
}

#[test]
fn serve_migrates_an_older_store_and_preserves_the_page() {
    let mut pilot = Pilot::new(None);
    seed_page(&pilot);
    let page = pilot.call(&["page", "read", PAGE, "--json"]);
    let db = pilot.root.join("selected/colab/space.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let supported: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    conn.execute_batch("ALTER TABLE owner_operations RENAME TO current_owner_operations;
        CREATE TABLE owner_operations(id TEXT PRIMARY KEY, digest BLOB NOT NULL, outcome BLOB NOT NULL);
        INSERT INTO owner_operations(id,digest,outcome) SELECT id,digest,outcome FROM current_owner_operations;
        DROP TABLE current_owner_operations;
        ALTER TABLE pages DROP COLUMN last_update_at_ms; PRAGMA user_version=4")
        .unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    failure(&pilot, &["ls", "--json"], "COLAB_STORE_OUTDATED");
    assert_eq!(fs::read(&db).unwrap(), before);
    pilot.start();
    assert_eq!(pilot.call(&["page", "read", PAGE, "--json"]), page);
    assert_eq!(pilot.call(&["ls", "--json"])["pages"][0]["pageId"], PAGE);
    pilot.stop();
    let conn = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        supported
    );
    assert_eq!(
        conn.query_row("SELECT last_update_at_ms FROM pages", [], |row| row
            .get::<_, Option<i64>>(0))
            .unwrap(),
        None
    );
    assert_eq!(pilot.call(&["page", "read", PAGE, "--json"]), page);
}

#[test]
fn future_schema_keeps_recovery_and_correlation_and_serve_refuses_to_migrate() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let db = pilot.root.join("selected/colab/space.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let supported: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    for args in [
        vec!["page", "create", "--title", "No creation", "--json"],
        vec!["serve", "--json"],
    ] {
        let result = failure(&pilot, &args, "COLAB_STORE_NEWER");
        assert_eq!(result["error"]["storeSchema"], 99);
        assert_eq!(result["error"]["supportedSchema"], supported);
        assert_eq!(result["next"], json!(["tmt upgrade"]));
        if args[0] == "page" {
            tmt_colab_model::values::generated_id(result["operationId"].as_str().unwrap()).unwrap();
            tmt_colab_model::values::generated_id(result["pageId"].as_str().unwrap()).unwrap();
        }
        assert_eq!(fs::read(&db).unwrap(), before);
    }
    assert!(!pilot.root.join("selected/colab/door.sock").exists());
}

#[test]
fn create_initializes_fresh_space_then_read_write_and_list_work_offline_and_serving() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        assert_eq!(pilot.call(&["ls", "--json"])["pages"], json!([]));
        if serving {
            pilot.start();
        }
        let source = "<h1>Created 🐈</h1>\r\n";
        let file = pilot.root.join("initial.html");
        fs::write(&file, source).unwrap();
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Fresh 🐈",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let id = created["pageId"].as_str().unwrap();
        tmt_colab_model::values::generated_id(id).unwrap();
        assert_eq!(created["title"], "Fresh 🐈");
        assert_eq!(created["path"], format!("/p/{}", &id[..8]));
        assert_eq!(created["membershipHead"]["revision"], "2");
        let read = pilot.call(&["page", "read", id, "--json"]);
        assert_eq!(read["source"], source);
        assert_eq!(read["title"], "Fresh 🐈");
        assert_eq!(read["epoch"], "1");
        fs::write(&file, "<p>Edited</p>").unwrap();
        pilot.call(&[
            "page",
            "write",
            id,
            "--file",
            file.to_str().unwrap(),
            "--expected-revision",
            read["revision"].as_str().unwrap(),
            "--json",
        ]);
        let edited = pilot.call(&["page", "read", id, "--json"]);
        assert_eq!(edited["source"], "<p>Edited</p>");
        assert_eq!(edited["title"], "Fresh 🐈");
        let listed = pilot.call(&["ls", "--json"]);
        assert_eq!(listed["pages"].as_array().unwrap().len(), 1);
        assert_eq!(listed["pages"][0]["pageId"], id);
        assert_eq!(listed["pages"][0]["title"], "Fresh 🐈");
        assert_eq!(listed["pages"][0]["sharing"], "private");
        if serving {
            pilot.stop();
        }
    }
}
/// Printable text that shares no run longer than a few bytes with any other seed's output, so a
/// replacement is a full rewrite of the page rather than a small diff.
fn varied_source(seed: u64, bytes: usize) -> String {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut text = String::with_capacity(bytes + 64);
    while text.len() < bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push(char::from(b'a' + (state % 26) as u8));
        if state.is_multiple_of(61) {
            text.push_str("\r\n");
        }
    }
    text.truncate(bytes);
    text
}
#[test]
fn a_page_of_one_and_a_half_mib_is_replaced_whole_three_times_offline_and_serving() {
    const SIZE: usize = 1_536 * 1024;
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        if serving {
            pilot.start();
        }
        let file = pilot.root.join("source.html");
        let first = varied_source(1, SIZE);
        fs::write(&file, &first).unwrap();
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Large",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let id = created["pageId"].as_str().unwrap();
        assert_eq!(pilot.call(&["page", "read", id, "--json"])["source"], first);
        let mut revisions = vec![
            pilot.call(&["page", "read", id, "--json"])["revision"]
                .as_str()
                .unwrap()
                .to_owned(),
        ];
        for seed in 2..=4 {
            let source = varied_source(seed, SIZE);
            fs::write(&file, &source).unwrap();
            let output = pilot
                .command()
                .args([
                    "page",
                    "write",
                    id,
                    "--file",
                    file.to_str().unwrap(),
                    "--json",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "write {seed} (serving: {serving}): {output:?}"
            );
            let written: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(written["changed"], true);
            assert_eq!(written["pageId"], id);
            let read = pilot.call(&["page", "read", id, "--json"]);
            // The receipt carries the revision after the write's own combine, so it fences the next write.
            assert_eq!(written["revision"], read["revision"]);
            revisions.push(read["revision"].as_str().unwrap().to_owned());
            assert_eq!(
                read["source"], source,
                "write {seed} did not read back exactly"
            );
            let export = pilot.call(&[
                "export",
                id,
                "--dir",
                pilot.root.to_str().unwrap(),
                "--json",
            ]);
            let directory = PathBuf::from(export["directory"].as_str().unwrap());
            assert_eq!(
                fs::read(directory.join("page.html")).unwrap(),
                source.as_bytes(),
                "export differs after write {seed}"
            );
        }
        revisions.dedup();
        assert_eq!(revisions.len(), 4, "each replacement is its own revision");
        if serving {
            pilot.stop();
        }
    }
}
#[test]
fn a_write_names_its_limit_or_stale_base_and_changes_nothing_offline_and_serving() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        if serving {
            pilot.start();
        }
        let file = pilot.root.join("source.html");
        fs::write(&file, "<p>Original</p>").unwrap();
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Limits",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let id = created["pageId"].as_str().unwrap();
        let before = pilot.call(&["page", "read", id, "--json"]);
        let limit = tmt_colab::decoder::BASELINE_BYTES;
        fs::write(&file, "x".repeat(limit + 1)).unwrap();
        let refused = failure(
            &pilot,
            &[
                "page",
                "write",
                id,
                "--file",
                file.to_str().unwrap(),
                "--json",
            ],
            "COLAB_CAPACITY",
        );
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("{} bytes", limit + 1))
                && message.contains(&format!("at most {limit} bytes (2 MiB)"))
                && message.ends_with("Nothing was written."),
            "{message}"
        );
        // The most a source can be still has to fit the retained tail: replacing a freshly
        // created 2 MiB page with a different 2 MiB source would leave 4 MiB plus overhead.
        let full = pilot.root.join("full.html");
        fs::write(&full, varied_source(7, limit)).unwrap();
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Full",
            "--file",
            full.to_str().unwrap(),
            "--json",
        ]);
        let full_id = created["pageId"].as_str().unwrap();
        let full_before = pilot.call(&["page", "read", full_id, "--json"]);
        fs::write(&full, varied_source(8, limit)).unwrap();
        let refused = failure(
            &pilot,
            &[
                "page",
                "write",
                full_id,
                "--file",
                full.to_str().unwrap(),
                "--json",
            ],
            "COLAB_CAPACITY",
        );
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("is full: this edit would take its changes to")
                && message.contains("more than the 4 MiB (4194304 bytes) one page can hold")
                && message.contains("Nothing was deleted"),
            "{message}"
        );
        assert_eq!(
            pilot.call(&["page", "read", full_id, "--json"]),
            full_before
        );
        // A base the page has moved past is refused before anything is sealed or published.
        fs::write(&file, "<p>Second</p>").unwrap();
        pilot.call(&[
            "page",
            "write",
            id,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        fs::write(&file, "<p>Third</p>").unwrap();
        failure(
            &pilot,
            &[
                "page",
                "write",
                id,
                "--file",
                file.to_str().unwrap(),
                "--expected-revision",
                before["revision"].as_str().unwrap(),
                "--json",
            ],
            "COLAB_STALE_BASE",
        );
        let after = pilot.call(&["page", "read", id, "--json"]);
        assert_eq!(after["source"], "<p>Second</p>");
        if serving {
            pilot.stop();
        }
    }
}
#[test]
fn writing_an_archived_page_is_refused_before_anything_is_recorded() {
    let pilot = Pilot::new(None);
    let file = pilot.root.join("source.html");
    fs::write(&file, "<p>Kept</p>").unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Archived",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    let id = created["pageId"].as_str().unwrap();
    pilot.call(&["archive", id, "--json"]);
    fs::write(&file, "<p>Changed</p>").unwrap();
    let db = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&db).unwrap();
    failure(
        &pilot,
        &[
            "page",
            "write",
            id,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        "COLAB_PAGE_INACTIVE",
    );
    assert_eq!(fs::read(&db).unwrap(), before, "a refused write left a row");
}
#[test]
fn attachment_read_refuses_without_a_serve_or_a_valid_reference_and_creates_nothing() {
    let pilot = Pilot::new(None);
    let file = pilot.root.join("source.html");
    fs::write(&file, "<p>Kept</p>").unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Attachments",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    let id = created["pageId"].as_str().unwrap();
    let output = pilot.root.join("output");
    fs::create_dir(&output).unwrap();
    let reference = pilot.root.join("reference.json");
    let selector = format!(
        r#"{{"kind":"document-current","attachmentId":"00000000-0000-4000-8000-000000000021","descriptorHash":"{}","contentRevision":"v1:{}"}}"#,
        "b".repeat(64),
        "a".repeat(64)
    );
    let read = |path: &std::path::Path| {
        [
            "attachment",
            "read",
            id,
            "--reference",
            path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--json",
        ]
        .map(str::to_owned)
    };
    // A well-formed reference with no serve: nothing can be read, and nothing is created.
    fs::write(&reference, &selector).unwrap();
    let args = read(&reference);
    failure(
        &pilot,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        "COLAB_UNAVAILABLE",
    );
    // Each malformed reference is refused before the serve is asked.
    for malformed in [
        "{}".to_owned(),
        selector.replace(&"b".repeat(64), "short"),
        selector.replace("}", r#","agent":"x"}"#),
        " ".repeat(tmt_colab_model::attachment::REFERENCE_BYTES + 1),
    ] {
        fs::write(&reference, malformed).unwrap();
        let args = read(&reference);
        failure(
            &pilot,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            "COLAB_INPUT_INVALID",
        );
    }
    // A missing file is invalid input too, not a serve problem.
    let args = read(&pilot.root.join("missing.json"));
    failure(
        &pilot,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        "COLAB_INPUT_INVALID",
    );
    assert_eq!(
        fs::read_dir(&output).unwrap().count(),
        0,
        "a refused read left output behind"
    );
}
#[test]
fn attachment_attach_help_keeps_requirements_resume_and_changed_page_on_separate_lines() {
    let pilot = Pilot::new(None);
    let output = pilot
        .command()
        .args(["attachment", "attach", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    let details = help
        .split_once("Details:\n")
        .unwrap()
        .1
        .split_once("\nExamples:\n")
        .unwrap()
        .0;
    assert_eq!(
        details,
        concat!(
            "  Requires a running serve, an editable page and an established object channel.\n",
            "  Pair the browser with tmt remote pair, then open the page once to establish the channel.\n",
            "  The serve copies the file into a private slot, seals it with this device's writer key, uploads it and lists it on the page.\n",
            "  The type defaults from the file extension and is only a label. Files over 8 MiB are refused.\n",
            "  If the reply is lost, rerun with --resume SLOT using the printed slot. It checks the backend before sending and never uploads twice.\n",
            "  If the page changed after the file was sealed, the uploaded original and slot are discarded. Nothing is listed; run attach again for a new slot.\n",
            "  --json prints the exact reference that attachment read takes.\n",
        )
    );
    assert!(!help.contains("stale base") && !help.contains("through tmt remote"));
    assert!(
        !pilot.root.join("selected").exists() && !pilot.root.join("calls").exists(),
        "help invoked core or created application state"
    );
}
#[test]
fn attachment_attach_refuses_what_it_cannot_send_and_creates_nothing_without_a_serve() {
    let pilot = Pilot::new(None);
    let file = pilot.root.join("source.html");
    fs::write(&file, "<p>Kept</p>").unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Attach",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    let id = created["pageId"].as_str().unwrap();
    let notes = pilot.root.join("notes.txt");
    fs::write(&notes, b"notes").unwrap();
    let link = pilot.root.join("link.txt");
    std::os::unix::fs::symlink(&notes, &link).unwrap();
    let big = pilot.root.join("big.bin");
    fs::File::create(&big)
        .unwrap()
        .set_len(tmt_colab_model::attachment::PLAINTEXT_BYTES as u64 + 1)
        .unwrap();
    let attach = |args: &[&str]| {
        let mut all = vec!["attachment", "attach", id];
        all.extend_from_slice(args);
        all.push("--json");
        failure_args(&pilot, &all)
    };
    // A file that cannot be attached is refused before the serve is asked, whatever its state.
    for path in [
        pilot.root.join("absent.txt"),
        link.clone(),
        pilot.root.clone(),
    ] {
        assert_eq!(
            attach(&[path.to_str().unwrap()])["error"]["code"],
            "COLAB_INPUT_INVALID"
        );
    }
    let large = attach(&[big.to_str().unwrap()]);
    assert_eq!(large["error"]["code"], "COLAB_CAPACITY");
    assert!(
        large["error"]["message"]
            .as_str()
            .unwrap()
            .contains("at most 8 MiB"),
        "{large}"
    );
    assert_eq!(
        attach(&[notes.to_str().unwrap(), "--type", "TEXT/plain"])["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    // A valid file with no serve cannot be staged, and nothing is created for it.
    assert_eq!(
        attach(&[notes.to_str().unwrap()])["error"]["code"],
        "COLAB_UNAVAILABLE"
    );
    assert_eq!(
        attach(&["--resume", &"a".repeat(32)])["error"]["code"],
        "COLAB_UNAVAILABLE"
    );
    fn find(dir: &std::path::Path) -> bool {
        fs::read_dir(dir).unwrap().flatten().any(|entry| {
            entry.file_name() == "attach-slots"
                || (entry.file_type().unwrap().is_dir() && find(&entry.path()))
        })
    }
    assert!(!find(&pilot.root), "a refused attach left a slot behind");
    // A file and a resume together are a usage error, not a serve question.
    let output = pilot
        .command()
        .args([
            "attachment",
            "attach",
            id,
            notes.to_str().unwrap(),
            "--resume",
            &"a".repeat(32),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}
fn failure_args(pilot: &Pilot, args: &[&str]) -> Value {
    let output = pilot.command().args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn writing_the_source_a_page_already_has_publishes_nothing_offline_and_serving() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        if serving {
            pilot.start();
        }
        let file = pilot.root.join("source.html");
        fs::write(&file, "<p>Same</p>").unwrap();
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "Same",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let id = created["pageId"].as_str().unwrap();
        let before = pilot.call(&["page", "read", id, "--json"]);
        let written = pilot.call(&[
            "page",
            "write",
            id,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(written["changed"], false);
        assert_eq!(written["revision"], before["revision"]);
        assert!(written.get("operationId").is_none());
        assert_eq!(pilot.call(&["page", "read", id, "--json"]), before);
        if serving {
            pilot.stop();
        }
    }
}
/// Stands in the serve socket's place and forwards each request to the real server, so a test
/// can lose the reply (or the request) the way a dying connection would.
struct Interposer {
    real: PathBuf,
    path: PathBuf,
    forward: bool,
    thread: Option<JoinHandle<()>>,
}
impl Interposer {
    fn start(pilot: &Pilot, forward: bool) -> Self {
        let path = pilot.root.join("selected/colab/door.sock");
        let real = pilot.root.join("selected/colab/real.sock");
        fs::rename(&path, &real).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let target = real.clone();
        let thread = thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 65536];
            let body_start = loop {
                let n = client.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8_lossy(&request[..body_start]).to_lowercase();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while request.len() < body_start + length {
                let n = client.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
            }
            if forward {
                let mut server = UnixStream::connect(target).unwrap();
                server.write_all(&request).unwrap();
                server.shutdown(std::net::Shutdown::Write).unwrap();
                let mut reply = Vec::new();
                server.read_to_end(&mut reply).unwrap();
                assert!(reply.starts_with(b"HTTP/1.1 200"));
            }
            // The reply, if any, is dropped: the writer sees a closed connection.
        });
        Self {
            real,
            path,
            forward,
            thread: Some(thread),
        }
    }
    fn finish(mut self) {
        self.thread.take().unwrap().join().unwrap();
        fs::remove_file(&self.path).unwrap();
        fs::rename(&self.real, &self.path).unwrap();
        let _ = self.forward;
    }
}
#[test]
fn a_lost_reply_is_resolved_by_original_status_and_a_lost_request_names_its_operation() {
    let mut pilot = Pilot::new(None);
    pilot.start();
    let file = pilot.root.join("source.html");
    fs::write(&file, "<p>Start</p>").unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Doubt",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    let id = created["pageId"].as_str().unwrap();
    let args = [
        "page",
        "write",
        id,
        "--file",
        file.to_str().unwrap(),
        "--json",
    ];
    // The server commits, the reply is lost: the status check finds the original outcome.
    fs::write(&file, "<p>Committed behind a lost reply</p>").unwrap();
    let interposer = Interposer::start(&pilot, true);
    let resolved = pilot.call(&args);
    interposer.finish();
    assert_eq!(resolved["changed"], true);
    let operation = resolved["operationId"].as_str().unwrap().to_owned();
    assert_eq!(
        pilot.call(&["page", "read", id, "--json"])["source"],
        "<p>Committed behind a lost reply</p>"
    );
    // The same command again finds nothing to publish: the lost reply cannot double-publish.
    let again = pilot.call(&args);
    assert_eq!(again["changed"], false);
    // The request is lost before the server sees it: nothing exists to resolve, so the result is
    // unknown, names the original operation, and nothing was written or resent.
    fs::write(&file, "<p>Never reached the server</p>").unwrap();
    let interposer = Interposer::start(&pilot, false);
    let unknown = failure(&pilot, &args, "COLAB_OUTCOME_UNKNOWN");
    interposer.finish();
    let message = unknown["error"]["message"].as_str().unwrap();
    assert!(message.contains("operation "), "{message}");
    assert!(!message.contains(&operation), "{message}");
    assert_eq!(
        pilot.call(&["page", "read", id, "--json"])["source"],
        "<p>Committed behind a lost reply</p>"
    );
    // A plain retry is a fresh preparation and publishes exactly once.
    let retried = pilot.call(&args);
    assert_eq!(retried["changed"], true);
    assert_eq!(
        pilot.call(&["page", "read", id, "--json"])["source"],
        "<p>Never reached the server</p>"
    );
    pilot.stop();
}
/// Answers one request with canned bytes in the serve socket's place, as a serve of another
/// build would. `read_body` false answers as soon as the headers arrive and closes, the way an
/// older serve refuses a body over its cap. The real socket is restored by `finish`.
struct Scripted {
    real: PathBuf,
    path: PathBuf,
    thread: JoinHandle<Vec<u8>>,
}
impl Scripted {
    fn start(pilot: &Pilot, reply: Vec<u8>, read_body: bool) -> Self {
        let path = pilot.root.join("selected/colab/door.sock");
        let real = pilot.root.join("selected/colab/real.sock");
        fs::rename(&path, &real).unwrap();
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let thread = thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 65536];
            let body_start = loop {
                let n = client.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            if read_body {
                let head = String::from_utf8_lossy(&request[..body_start]).to_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                while request.len() < body_start + length {
                    let n = client.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..n]);
                }
            }
            client.write_all(&reply).unwrap();
            request
        });
        Self { real, path, thread }
    }
    /// The request line and headers the writer sent, after restoring the real socket.
    fn finish(self) -> String {
        let request = self.thread.join().unwrap();
        fs::remove_file(&self.path).unwrap();
        fs::rename(&self.real, &self.path).unwrap();
        let end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        String::from_utf8_lossy(&request[..end]).into_owned()
    }
}
fn http(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
#[test]
fn a_serve_that_cannot_understand_the_write_is_a_mismatch_not_an_unknown_outcome() {
    let mut pilot = Pilot::new(None);
    pilot.start();
    let file = pilot.root.join("source.html");
    fs::write(&file, "<p>Start</p>").unwrap();
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Skew",
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    let id = created["pageId"].as_str().unwrap();
    let args = [
        "page",
        "write",
        id,
        "--file",
        file.to_str().unwrap(),
        "--json",
    ];
    let typed =
        |code: &str| format!(r#"{{"error":{{"code":"{code}","message":"Fixture refusal."}}}}"#);
    // A serve built before CLI batches has no such route (an untyped 404), refuses a body over
    // its cap before reading it (413, answered while the writer is still sending), or, when it
    // is a newer build, names a write version it does not speak with a typed refusal.
    fs::write(&file, "<p>Never published by a mismatched serve</p>").unwrap();
    let mismatched: Vec<(&str, Vec<u8>, bool)> = vec![
        ("untyped 404", http("404 Not Found", "NOT FOUND"), true),
        (
            "untyped 413",
            http("413 Payload Too Large", "TOO LARGE"),
            true,
        ),
        (
            "typed version refusal",
            http("409 Conflict", &typed("COLAB_SERVER_MISMATCH")),
            true,
        ),
    ];
    for (name, reply, read_body) in mismatched {
        let scripted = Scripted::start(&pilot, reply, read_body);
        let refused = failure(&pilot, &args, "COLAB_SERVER_MISMATCH");
        let head = scripted.finish();
        assert!(
            head.starts_with("POST /.tmt/colab/local/page-publish HTTP/1.1"),
            "{name}"
        );
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(!message.contains("operation "), "{name}: {message}");
        if name.starts_with("untyped") {
            // The CLI's own words; a typed refusal carries the server's.
            assert!(message.contains("Nothing was written"), "{name}: {message}");
            assert!(message.contains("tmt colab stop"), "{name}: {message}");
        }
    }
    // An older serve answers a body over its cap while the writer is still sending it.
    let big: String = (0..1_500_000u32)
        .map(|n| char::from(b'a' + ((n.wrapping_mul(2_654_435_761) >> 13) % 26) as u8))
        .collect();
    fs::write(&file, format!("<p>{big}</p>")).unwrap();
    let scripted = Scripted::start(&pilot, http("413 Payload Too Large", "TOO LARGE"), false);
    failure(&pilot, &args, "COLAB_SERVER_MISMATCH");
    scripted.finish();
    // Typed refusals keep their own codes, even a typed 404.
    fs::write(&file, "<p>Typed refusals are unchanged</p>").unwrap();
    for (status, code) in [
        ("404 Not Found", "COLAB_STATE_MISSING"),
        ("409 Conflict", "COLAB_STALE_BASE"),
    ] {
        let scripted = Scripted::start(&pilot, http(status, &typed(code)), true);
        failure(&pilot, &args, code);
        scripted.finish();
    }
    // Anything else after the request was sent stays in doubt, naming its operation.
    for reply in [
        http("503 Unavailable", "BUSY"),
        http("200 OK", "not an outcome"),
        http("404 Not Found", &typed("COLAB_SOMETHING_NEW")),
    ] {
        let scripted = Scripted::start(&pilot, reply, true);
        let unknown = failure(&pilot, &args, "COLAB_OUTCOME_UNKNOWN");
        scripted.finish();
        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap()
                .contains("operation ")
        );
    }
    // Nothing above published: the page still has its first source, and a real serve takes the
    // same write.
    assert_eq!(
        pilot.call(&["page", "read", id, "--json"])["source"],
        "<p>Start</p>"
    );
    let written = pilot.call(&args);
    assert_eq!(written["changed"], true);
    pilot.stop();
}
#[test]
fn create_supports_empty_source_and_stdin_and_refuses_invalid_input_before_state_creation() {
    let pilot = Pilot::new(None);
    let invalid = pilot
        .command()
        .args(["page", "create", "--title", "", "--json"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    assert!(!pilot.root.join("selected/colab").exists());
    let file = pilot.root.join("invalid.html");
    fs::write(&file, [0xff]).unwrap();
    let invalid = pilot
        .command()
        .args([
            "page",
            "create",
            "--title",
            "Bad",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    assert!(!pilot.root.join("selected/colab").exists());
    let created = pilot.call(&["page", "create", "--title", "Empty", "--json"]);
    let read = pilot.call(&[
        "page",
        "read",
        created["pageId"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(read["source"], "");
    assert_eq!(read["title"], "Empty");
    let mut child = pilot
        .command()
        .args([
            "page", "create", "--title", "Stdin", "--file", "-", "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"<p>From stdin</p>")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let created: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        pilot.call(&[
            "page",
            "read",
            created["pageId"].as_str().unwrap(),
            "--json"
        ])["source"],
        "<p>From stdin</p>"
    );
    let help = pilot
        .command()
        .args(["help", "page", "create"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--title"));
    let human = pilot
        .command()
        .args(["page", "create", "--title", "Human"])
        .output()
        .unwrap();
    assert!(human.status.success(), "{human:?}");
    assert!(human.stderr.is_empty());
    let message = String::from_utf8(human.stdout).unwrap();
    assert!(message.contains("PAGE CREATED"));
    // An unsupported creation projection leaves a neutral relative path.
    assert!(message.contains("/p/"));
    assert!(message.contains("(Remote link unavailable from the creation snapshot"));
    assert!(!message.contains("start tmt remote serve"));
    assert!(!message.contains("tmt remote pair printed"));
}
const CREATION_DOOR: &str = r#"{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/abcdefghijkl2345","machineId":"40000000-0000-4000-8000-000000000001"}"#;
const DOOR: &str = r#"{"running":true,"origin":"http://127.0.0.1:53253","path":"/r/3e2c69f7"}"#;
#[test]
fn reader_links_print_a_full_url_only_while_a_door_runs() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    pilot.call(&["share", "mode", PAGE, "link", "--yes", "--json"]);
    let add = |status: Option<&str>, extra: &[&str]| {
        let mut command = status.map_or_else(|| pilot.command(), |s| pilot.command_with_door(s));
        let out = command
            .args(["share", "link", "add", PAGE, "--yes"])
            .args(extra)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    // No door: the path alone, with how to get a full link.
    let plain: Value = serde_json::from_str(&add(None, &["--json"])).unwrap();
    assert!(plain.get("readerUrl").is_none());
    assert!(add(None, &[]).contains("Browser access needs the Remote extension"));
    let running: Value = serde_json::from_str(&add(Some(DOOR), &["--json"])).unwrap();
    let path = running["readerPath"].as_str().unwrap();
    assert_eq!(
        running["readerUrl"],
        format!("http://127.0.0.1:53253{path}")
    );
    let human = add(Some(DOOR), &[]);
    assert!(human.contains("http://127.0.0.1:53253/read/"), "{human}");
    assert!(!human.contains("start tmt remote serve"));
    // The listing never carries a link, with or without a door.
    let listed = pilot
        .command_with_door(DOOR)
        .args(["share", "link", "list", PAGE, "--json"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("readerUrl"));
}
#[test]
fn created_pages_only_print_short_links_and_root_paths_while_a_door_runs() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    let json = |status: &str| -> Value {
        let before = pilot
            .creation_calls()
            .iter()
            .filter(|s| *s == "remote status --machine --json")
            .count();
        let out = pilot
            .command_with_creation_door(status)
            .args(["page", "create", "--title", "Linked", "--json"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        pilot.assert_creation_acquisitions(before + 1);
        serde_json::from_slice(&out.stdout).unwrap()
    };
    // No door (the fixture core has no remote status): relative path, no url.
    let plain = pilot.call(&["page", "create", "--title", "Plain", "--json"]);
    assert!(plain.get("url").is_none());
    for status in [
        r#"{"running":false}"#,
        "not json",
        r#"{"running":true,"origin":"http://127.0.0.1:1/x","path":"/r/ab"}"#,
    ] {
        let value = json(status);
        assert!(
            value["link"].is_null() && value["shortLink"].is_null(),
            "{status}"
        );
        assert!(value["paired"].is_null());
        assert_eq!(value["next"], json!([]));
    }
    let created = json(CREATION_DOOR);
    assert!(
        pilot
            .call(&[
                "page",
                "read",
                created["pageId"].as_str().unwrap(),
                "--json"
            ])
            .get("creationRecipient")
            .is_none()
    );
    let path = created["path"].as_str().unwrap();
    assert!(path.starts_with("/p/"));
    assert_eq!(
        created["shortLink"],
        format!(
            "http://127.0.0.1:53253/p/{}",
            &created["pageId"].as_str().unwrap()[..8]
        )
    );
    assert!(plain["shortLink"].is_null());
    assert_eq!(created["link"], format!("http://127.0.0.1:53253{path}"));
    let human = pilot
        .command_with_creation_door(CREATION_DOOR)
        .args(["page", "create", "--title", "Human link"])
        .output()
        .unwrap();
    assert!(
        human.status.success() && human.stderr.is_empty(),
        "{human:?}"
    );
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("http://127.0.0.1:53253/p/"), "{text}");
    assert!(!text.contains("start tmt remote serve"));
    // A stopped opt-in is exact; an extra field is malformed rather than evidence of stopping.
    let stopped = pilot
        .command_with_creation_door(r#"{"running":false,"lastPort":53253}"#)
        .args(["page", "create", "--title", "Stopped"])
        .output()
        .unwrap();
    let text = String::from_utf8(stopped.stdout).unwrap();
    assert!(
        text.contains("(run tmt colab serve to get a full link)"),
        "{text}"
    );
    let malformed = pilot
        .command_with_creation_door(r#"{"running":false,"lastPort":53253,"future":1}"#)
        .args(["page", "create", "--title", "Malformed stopped"])
        .output()
        .unwrap();
    assert!(malformed.status.success());
    let text = String::from_utf8(malformed.stdout).unwrap();
    assert!(
        text.contains("Remote link unavailable from the creation snapshot"),
        "{text}"
    );
    assert!(!text.contains("run tmt colab serve"));
    // An older command still offers ordinary status; creation must never fall back to it.
    fs::write(
        pilot.root.join("publisher"),
        r#"{"identity":{"name":"creator","id":"50000000-0000-1000-8000-000000000001"}}"#,
    )
    .unwrap();
    let old = pilot
        .command()
        .env("TMT_EXECUTABLE", pilot.creation_core(None, None))
        .args(["page", "create", "--title", "Old CLI", "--open", "--json"])
        .output()
        .unwrap();
    assert!(old.status.success());
    let old: Value = serde_json::from_slice(&old.stdout).unwrap();
    assert!(old["link"].is_null() && old["shortLink"].is_null() && old["paired"].is_null());
    assert_eq!(old["next"], json!([]));
    assert!(
        pilot
            .call(&["page", "read", old["pageId"].as_str().unwrap(), "--json"])
            .get("creationRecipient")
            .is_none()
    );
    let old_human = pilot
        .command()
        .env("TMT_EXECUTABLE", pilot.creation_core(None, None))
        .args(["page", "create", "--title", "Old CLI human", "--open"])
        .output()
        .unwrap();
    assert!(old_human.status.success());
    let text = String::from_utf8(old_human.stdout).unwrap();
    assert!(
        text.contains("Remote link unavailable from the creation snapshot"),
        "{text}"
    );
    for instruction in [
        "install",
        "upgrade",
        "restart",
        "tmt colab serve",
        "tmt remote pair",
    ] {
        assert!(!text.contains(instruction), "{text}");
    }
    assert!(pilot.opened().is_empty());
    pilot.assert_creation_acquisitions(9);
    assert_eq!(
        pilot
            .creation_calls()
            .iter()
            .filter(|s| *s == "remote devices --json")
            .count(),
        2
    );
}
#[test]
fn a_door_that_does_not_answer_in_time_falls_back_to_the_relative_path() {
    let pilot = Pilot::new(None);
    let mut cmd = pilot.command();
    cmd.env(
        "TMT_EXECUTABLE",
        pilot.creation_core(Some(CREATION_DOOR), Some(8)),
    );
    // Capture to a file and wait on the process itself: a parallel test's child can inherit a pipe
    // end and delay its EOF, which says nothing about this command's own duration.
    let capture = pilot.root.join("slow.out");
    let started = Instant::now();
    let status = cmd
        .args(["page", "create", "--title", "Slow", "--json"])
        .stdout(fs::File::create(&capture).unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    pilot.assert_creation_acquisitions(1);
    assert!(
        pilot
            .creation_calls()
            .iter()
            .all(|s| s != "remote devices --json")
    );
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "opt-in deadline was not exercised"
    );
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
    let out = fs::read(&capture).unwrap();
    let created: Value = serde_json::from_slice(&out).unwrap();
    assert!(created["link"].is_null() && created["shortLink"].is_null());
    assert!(created["paired"].is_null());
    assert_eq!(created["next"], json!([]));
    assert!(created["path"].as_str().unwrap().starts_with("/p/"));
}

#[test]
fn human_output_is_readable_and_the_decoder_note_is_not_repeated_per_command() {
    let pilot = Pilot::new(None);
    let created = pilot.call(&["page", "create", "--title", "Notes", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    let human = |args: &[&str]| {
        let out = pilot.command().args(args).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        (
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    // `show` is a summary, not raw JSON blobs.
    let (shown, warning) = human(&["show", page]);
    assert!(
        shown.contains("audience") && shown.contains("private · shared history"),
        "{shown}"
    );
    assert!(
        shown.contains("membership") && shown.contains("revision "),
        "{shown}"
    );
    assert!(shown.contains("links") && shown.contains('–'), "{shown}");
    assert!(!shown.contains('{') && !shown.contains('['), "{shown}");
    assert!(
        shown.contains("expiry") && shown.contains("in 29 days") && !shown.contains(" UTC"),
        "{shown}"
    );
    assert!(warning.is_empty(), "{warning}");
    // Page read/write keep decoder state in JSON only.
    let (source, metadata) = human(&["page", "read", page]);
    assert_eq!(source, "");
    assert!(
        metadata.contains("PAGE SOURCE") && !metadata.contains("decoder"),
        "{metadata}"
    );
    let revision = pilot.call(&["page", "read", page, "--json"]);
    assert!(revision["memoryLimit"].is_string());
    let (written, _) = human(&[
        "page",
        "write",
        page,
        "--expected-revision",
        revision["revision"].as_str().unwrap(),
        "--file",
        "-",
    ]);
    // (stdin is empty here, so the write is the same empty source and publishes nothing)
    assert!(
        written.contains("PAGE UNCHANGED") && !written.contains("decoder"),
        "{written}"
    );
    // Help examples cover creating a page and sharing it.
    let (root, _) = human(&["help"]);
    assert!(
        root.contains("page create") && root.contains("share mode"),
        "{root}"
    );
    let (group, _) = human(&["help", "page"]);
    assert!(group.contains("page create"), "{group}");
}
#[test]
fn serve_names_the_link_only_when_a_door_runs_and_never_prints_the_decoder_note() {
    for door in [false, true] {
        let pilot = Pilot::new(None);
        let mut cmd = if door {
            pilot.command_with_door(DOOR)
        } else {
            pilot.command()
        };
        let mut child = cmd
            .args(["serve", "--foreground"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut text = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let wanted = "create";
        while !text.contains(wanted) && Instant::now() < deadline {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            text.push_str(&line);
        }
        kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
        child.wait().unwrap();
        assert!(!text.contains("decoder"), "{text}");
        if door {
            assert!(text.contains("http://127.0.0.1:53253"), "{text}");
        } else {
            assert!(text.contains("local only"), "{text}");
        }
    }
}

fn full_management_cases() -> Vec<(&'static str, Vec<String>)> {
    use ed25519_dalek::SigningKey;
    use tmt_colab_model::{values, wrap};
    let sign = values::encode_binary(&SigningKey::from_bytes(&[19; 32]).verifying_key().to_bytes());
    let enc = values::encode_binary(
        &wrap::RecipientKey::from_seed(&[20; 32])
            .unwrap()
            .public_key(),
    );
    let member = "20000000-0000-4000-8000-000000000009";
    vec![
        (
            "member.add",
            vec![
                "share",
                "member",
                "add",
                PAGE,
                member,
                "viewer",
                "--sign-key",
                &sign,
                "--enc-key",
                &enc,
            ],
        ),
        (
            "member.remove",
            vec!["share", "member", "remove", PAGE, MEMBER],
        ),
        (
            "member.role",
            vec!["share", "member", "role", PAGE, MEMBER, "viewer"],
        ),
        ("history.current", vec!["share", "history", PAGE, "current"]),
        ("history.shared", vec!["share", "history", PAGE, "shared"]),
        ("retention.days", vec!["retention", PAGE, "21"]),
        ("retention.forever", vec!["retention", PAGE, "forever"]),
        ("archive", vec!["archive", PAGE]),
        ("delete", vec!["delete", PAGE]),
    ]
    .into_iter()
    .map(|(name, args)| (name, args.into_iter().map(str::to_owned).collect()))
    .collect()
}
fn words(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}
fn frozen_management(base: &[String], operation: &str, revision: &str) -> Vec<String> {
    base.iter()
        .map(String::as_str)
        .chain([
            "--yes",
            "--operation-id",
            operation,
            "--expected-revision",
            revision,
            "--json",
        ])
        .map(str::to_owned)
        .collect()
}

#[test]
fn full_management_cli_serving_and_stopped_commits_policy_and_replays_after_restart() {
    for serving in [false, true] {
        for (name, base) in full_management_cases()
            .into_iter()
            .flat_map(|(name, args)| {
                let short = args
                    .iter()
                    .map(|arg| {
                        if arg == PAGE {
                            PAGE[..8].to_owned()
                        } else {
                            arg.clone()
                        }
                    })
                    .collect();
                [(name, args), (name, short)]
            })
        {
            let mut pilot = Pilot::new(None);
            seed_page(&pilot);
            if serving {
                pilot.start();
            }
            if name == "history.shared" {
                pilot.call(&["share", "history", PAGE, "current", "--json"]);
            }
            let policy = pilot.call(&["retention", PAGE, "--json"]);
            assert_eq!(policy["page"]["retentionDays"], 30);
            let revision = policy["membershipHead"]["revision"].as_str().unwrap();
            let stale_revision = (revision.parse::<u64>().unwrap() - 1).to_string();
            let stale = frozen_management(
                &base,
                "40000000-0000-4000-8000-000000000098",
                &stale_revision,
            );
            failure(&pilot, &words(&stale), "COLAB_STALE_HEAD");
            assert_eq!(pilot.call(&["retention", PAGE, "--json"]), policy);
            let args = frozen_management(&base, "40000000-0000-4000-8000-000000000099", revision);
            let committed = pilot.call(&words(&args));
            assert_eq!(committed["pageId"], PAGE);
            assert_eq!(committed["expectedRevision"], revision);
            assert_eq!(
                pilot.call(&words(&args)),
                committed,
                "{name}: immediate replay"
            );
            if name == "delete" {
                assert_eq!(
                    pilot.call(&["ls", "--archived", "--json"])["pages"],
                    json!([])
                );
                failure(
                    &pilot,
                    &["delete", PAGE, "--yes", "--json"],
                    "COLAB_PAGE_DELETED",
                );
                let new = frozen_management(
                    &base,
                    "40000000-0000-4000-8000-000000000097",
                    committed["membershipHead"]["revision"].as_str().unwrap(),
                );
                failure(&pilot, &words(&new), "COLAB_DENIED");
                let db = rusqlite::Connection::open_with_flags(
                    pilot.root.join("selected/colab/space.db"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                for table in [
                    "streams",
                    "receipts",
                    "checkpoints",
                    "baselines",
                    "wraps",
                    "epoch_secrets",
                ] {
                    let count: i64 = db
                        .query_row(
                            &format!("SELECT count(*) FROM {table} WHERE page=?"),
                            [PAGE],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(count, 0, "deleted {table}");
                }
                assert_eq!(
                    db.query_row("SELECT count(*) FROM pages WHERE page=?", [PAGE], |row| row
                        .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
                assert_eq!(
                    db.query_row(
                        "SELECT count(*) FROM owner_operations WHERE id=?",
                        ["40000000-0000-4000-8000-000000000099"],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                    1
                );
                drop(db);
                pilot.call(&["page", "create", "--title", "Later page", "--json"]);
            } else {
                let detail = pilot.call(&["show", PAGE, "--json"]);
                match name {
                    "member.add" => assert!(detail["members"].as_array().unwrap().iter().any(
                        |m| m["id"] == base[4] && m["role"] == "viewer" && m["revoked"] == false
                    )),
                    "member.remove" => assert!(
                        detail["members"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|m| m["id"] == MEMBER && m["revoked"] == true)
                    ),
                    "member.role" => assert!(
                        detail["members"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|m| m["id"] == MEMBER && m["role"] == "viewer")
                    ),
                    "history.current" => assert_eq!(detail["page"]["history"], "current"),
                    "history.shared" => assert_eq!(detail["page"]["history"], "shared"),
                    "retention.days" => assert_eq!(detail["page"]["retentionDays"], 21),
                    "retention.forever" => assert_eq!(detail["page"]["retentionDays"], Value::Null),
                    "archive" => {
                        assert_eq!(detail["page"]["archived"], true);
                        assert_eq!(detail["page"]["title"], Value::Null);
                        assert_eq!(pilot.call(&["ls", "--json"])["pages"], json!([]));
                        assert_eq!(
                            pilot.call(&["ls", "--archived", "--json"])["pages"]
                                .as_array()
                                .unwrap()
                                .len(),
                            1
                        );
                        failure(
                            &pilot,
                            &["share", "history", PAGE, "shared", "--yes", "--json"],
                            "COLAB_DENIED",
                        );
                    }
                    _ => unreachable!(),
                }
                pilot.call(&["retention", PAGE, "42", "--json"]);
                assert_eq!(
                    pilot.call(&["retention", PAGE, "--json"])["page"]["retentionDays"],
                    42
                );
            }
            let later = pilot.call(&["ls", "--archived", "--json"])["membershipHead"].clone();
            assert_ne!(later, committed["membershipHead"]);
            if serving {
                pilot.stop();
            }
            pilot.start();
            assert_eq!(
                pilot.call(&words(&args)),
                committed,
                "{name}: durable original head after restart"
            );
            let mut conflict = args.clone();
            let position = conflict
                .iter()
                .position(|v| v == "--expected-revision")
                .unwrap();
            conflict[position + 1] = later["revision"].as_str().unwrap().into();
            failure(&pilot, &words(&conflict), "COLAB_CONFLICT");
            assert_eq!(
                pilot.call(&["ls", "--archived", "--json"])["membershipHead"],
                later
            );
            pilot.stop();
        }
    }
}

#[test]
fn full_management_confirmation_input_and_retention_reads_preserve_state() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        seed_page(&pilot);
        if serving {
            pilot.start();
        }
        pilot.call(&["share", "member", "role", PAGE, MEMBER, "viewer", "--json"]);
        pilot.call(&["share", "history", PAGE, "current", "--json"]);
        let before = pilot.call(&["retention", PAGE, "--json"]);
        let add = full_management_cases().remove(0).1;
        let mut unconfirmed_add = add.clone();
        unconfirmed_add.push("--json".into());
        confirmation_required(
            &pilot,
            &words(&unconfirmed_add),
            &format!(
                "Adding member 20000000-0000-4000-8000-000000000009 as viewer gives them access to page {PAGE}, and copies they make can't be recalled later."
            ),
            "add the member",
        );
        for (base, consequence, action) in [
            (
                vec!["share", "member", "role", PAGE, MEMBER, "editor", "--json"],
                format!("Making member {MEMBER} an editor lets them edit page {PAGE}."),
                "change the role",
            ),
            (
                vec!["share", "history", PAGE, "shared", "--json"],
                format!(
                    "Sharing the full history of page {PAGE} lets readers see deleted text, snapshots, comments and agent replies, and copies can't be recalled later."
                ),
                "share the history",
            ),
            (
                vec!["delete", PAGE, "--json"],
                format!(
                    "Deleting page {PAGE} is permanent; copied text and earlier public history can't be recalled."
                ),
                "delete",
            ),
        ] {
            confirmation_required(&pilot, &base, &consequence, action);
        }
        for days in ["0", "-1", "01", "9007199254740992", "1.5", "Forever"] {
            failure(
                &pilot,
                &["retention", PAGE, days, "--json"],
                "COLAB_INPUT_INVALID",
            );
        }
        for (flag, invalid) in [
            ("--sign-key", "AA"),
            ("--enc-key", "AA"),
            ("--enc-key", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ] {
            let mut args = frozen_management(
                &add,
                "40000000-0000-4000-8000-000000000099",
                before["membershipHead"]["revision"].as_str().unwrap(),
            );
            let position = args.iter().position(|v| v == flag).unwrap();
            args[position + 1] = invalid.into();
            failure(&pilot, &words(&args), "COLAB_INPUT_INVALID");
        }
        assert_eq!(pilot.call(&["retention", PAGE, "--json"]), before);
        let human = pilot.command().args(["retention", PAGE]).output().unwrap();
        assert!(human.status.success());
        let shown = String::from_utf8(human.stdout).unwrap();
        assert!(shown.contains(PAGE));
        assert!(shown.contains("retention") && shown.contains("30 days"));
        assert!(shown.contains("membership") && shown.contains("revision"));
        assert!(!shown.contains("retentionDays") && !shown.contains('{'));
        assert!(!shown.contains("audience") && !shown.contains("epoch"));
        assert!(human.stderr.is_empty());
        assert!(
            shown.contains("last edit") && shown.contains("expiry") && shown.contains("in 29 days")
        );
        assert!(!shown.contains(" UTC"));
        let updated = before["page"]["lastUpdateAtMs"].as_u64().unwrap();
        assert_eq!(before["page"]["expiresAtMs"], updated + 30 * 86_400_000);
        assert_eq!(before["page"]["warnings"], json!([]));
        let catalog = pilot.call(&["ls", "--json"]);
        for field in ["retentionDays", "lastUpdateAtMs", "expiresAtMs", "warnings"] {
            assert_eq!(before["page"][field], catalog["pages"][0][field]);
        }
        pilot.call(&[
            "share", "member", "role", PAGE, MEMBER, "editor", "--yes", "--json",
        ]);
        pilot.call(&["share", "history", PAGE, "shared", "--yes", "--json"]);
        for mut args in [
            add,
            vec![
                "share".into(),
                "member".into(),
                "role".into(),
                PAGE.into(),
                MEMBER.into(),
                "viewer".into(),
            ],
            vec![
                "share".into(),
                "member".into(),
                "remove".into(),
                PAGE.into(),
                MEMBER.into(),
            ],
            vec!["retention".into(), PAGE.into(), "forever".into()],
            vec!["archive".into(), PAGE.into()],
            vec!["delete".into(), PAGE.into()],
        ] {
            args.push("--yes".into());
            let human = pilot.command().args(&args).output().unwrap();
            assert!(human.status.success(), "{human:?}");
            assert!(human.stderr.is_empty());
            let shown = String::from_utf8(human.stdout).unwrap();
            assert!(shown.contains("operation"));
            assert!(shown.contains("expected revision"));
            assert!(shown.contains("membership") && shown.contains("revision"));
            assert!(!shown.contains("operationId") && !shown.contains("expectedRevision"));
            assert!(!shown.contains('{'));
            if args[0] == "retention" {
                let read = pilot.command().args(["retention", PAGE]).output().unwrap();
                assert!(read.status.success());
                let shown = String::from_utf8(read.stdout).unwrap();
                assert!(shown.contains("retention") && shown.contains("forever"));
                assert_eq!(
                    pilot.call(&["retention", PAGE, "--json"])["page"]["retentionDays"],
                    Value::Null
                );
            }
        }
        if serving {
            pilot.stop();
        }
    }
    for command in [
        vec!["share", "member", "add"],
        vec!["share", "member", "remove"],
        vec!["share", "member", "role"],
        vec!["share", "history"],
        vec!["retention"],
        vec!["archive"],
        vec!["delete"],
    ] {
        let pilot = Pilot::new(None);
        let mut args = vec!["help"];
        args.extend(command.iter().copied());
        let help = pilot.command().args(&args).output().unwrap();
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("tmt colab"));
        if command == ["share", "member", "add"] {
            assert!(help.contains("Advanced or scripted use"));
            assert!(help.contains("Member invitation flows come with the Firestore stage"));
        }
        assert!(!pilot.root.join("selected").exists());
    }
}

#[test]
fn every_full_management_mutation_preserves_correlation_and_never_falls_back_after_ipc() {
    use tmt_colab::keyring::Layout;
    for (name, base) in full_management_cases() {
        for response in [Some("DENIED"), None, Some("MALFORMED"), Some("STATUS")] {
            let pilot = Pilot::new(None);
            seed_page(&pilot);
            let db = pilot.root.join("selected/colab/space.db");
            let before = fs::read(&db).unwrap();
            let layout = Layout::existing(&pilot.root.join("selected"))
                .unwrap()
                .unwrap();
            let _lock = layout.serve_lock().unwrap();
            let socket = layout.directory.join(tmt_colab::socket::SOCKET);
            let listener = UnixListener::bind(&socket).unwrap();
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                stream.read_to_end(&mut request).unwrap();
                let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                assert!(request.starts_with(b"POST /.tmt/colab/management HTTP/1.1"));
                assert!(!String::from_utf8_lossy(&request[..offset]).contains("tmt-device"));
                let request: Value = serde_json::from_slice(&request[offset..]).unwrap();
                assert_eq!(
                    request["operationId"],
                    "40000000-0000-4000-8000-000000000099"
                );
                assert_eq!(request["expectedRevision"], "2");
                let operation = match name {
                    "history.current" | "history.shared" => "page.history",
                    "retention.days" | "retention.forever" => "retention.set",
                    "archive" => "page.archive",
                    "delete" => "page.delete",
                    v => v,
                };
                assert_eq!(request["operation"], operation);
                if let Some(response) = response {
                    let (status, body) = if response == "DENIED" {
                        (403, "DENIED")
                    } else if response == "STATUS" {
                        (409, "DENIED")
                    } else {
                        (200, "{invalid")
                    };
                    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let args = frozen_management(&base, "40000000-0000-4000-8000-000000000099", "2");
            let error = failure(
                &pilot,
                &words(&args),
                if response == Some("DENIED") {
                    "COLAB_DENIED"
                } else {
                    "COLAB_OUTCOME_UNKNOWN"
                },
            );
            server.join().unwrap();
            assert_eq!(error["operationId"], "40000000-0000-4000-8000-000000000099");
            assert_eq!(error["expectedRevision"], "2");
            assert_eq!(
                fs::read(&db).unwrap(),
                before,
                "{name}: no offline fallback"
            );
            fs::remove_file(socket).unwrap();
        }
    }
}

#[test]
fn full_management_rejected_inputs_and_missing_confirmations_send_no_ipc() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    pilot.call(&["share", "member", "role", PAGE, MEMBER, "viewer", "--json"]);
    pilot.call(&["share", "history", PAGE, "current", "--json"]);
    let layout = tmt_colab::keyring::Layout::existing(&pilot.root.join("selected"))
        .unwrap()
        .unwrap();
    let _lock = layout.serve_lock().unwrap();
    let socket = layout.directory.join(tmt_colab::socket::SOCKET);
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let mut add = full_management_cases().remove(0).1;
    add.push("--json".into());
    let mut cases = vec![(add.clone(), "COLAB_CONFIRMATION_REQUIRED")];
    for args in [
        vec!["share", "member", "role", PAGE, MEMBER, "editor", "--json"],
        vec!["share", "history", PAGE, "shared", "--json"],
        vec!["delete", PAGE, "--json"],
        vec!["retention", PAGE, "0", "--json"],
        vec!["retention", PAGE, "9007199254740992", "--json"],
    ] {
        let code = if args[0] == "retention" {
            "COLAB_INPUT_INVALID"
        } else {
            "COLAB_CONFIRMATION_REQUIRED"
        };
        cases.push((args.into_iter().map(str::to_owned).collect(), code));
    }
    add.push("--yes".into());
    let key = add.iter().position(|v| v == "--enc-key").unwrap();
    add[key + 1] = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into();
    cases.push((add, "COLAB_INPUT_INVALID"));
    for (args, code) in cases {
        failure(&pilot, &words(&args), code);
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "refused CLI request reached IPC"
        );
    }
    fs::remove_file(socket).unwrap();
}

#[test]
fn cli_member_changes_capture_complete_verified_assignments() {
    use ed25519_dalek::SigningKey;
    use tmt_colab::{
        keyring::{Keyring, Layout},
        store::{Store, owner::Recipient},
        transitions::{Engine, MemberAction, MemberRequest},
    };
    use tmt_colab_model::wrap;
    for serving in [false, true] {
        for action in ["role", "remove"] {
            let mut pilot = Pilot::new(None);
            seed_page(&pilot);
            let second = pilot.call(&["page", "create", "--title", "Second assignment", "--json"]);
            let second_id = second["pageId"].as_str().unwrap();
            let target = "20000000-0000-4000-8000-000000000009";
            let layout = Layout::existing(&pilot.root.join("selected"))
                .unwrap()
                .unwrap();
            {
                let _lock = layout.serve_lock().unwrap();
                let key = Keyring::read(&layout).unwrap();
                let mut store = Store::open(&layout).unwrap();
                let mut pages = vec![PAGE.to_owned(), second_id.to_owned()];
                pages.sort();
                let head = store
                    .owner_head(&key.space_id, &key.owner_public())
                    .unwrap()
                    .unwrap();
                let mut engine =
                    Engine::with_decoder_config(support::decoder_config(PathBuf::from(BINARY)))
                        .unwrap();
                engine
                    .member(
                        &mut store,
                        &key,
                        MemberRequest {
                            operation_id: "40000000-0000-4000-8000-000000000099",
                            expected_revision: head.revision,
                            action: MemberAction::Add(Recipient {
                                kind: "member".into(),
                                id: target.into(),
                                role: Some("editor".into()),
                                signing_key: SigningKey::from_bytes(&[19; 32])
                                    .verifying_key()
                                    .to_bytes(),
                                encryption_key: wrap::RecipientKey::from_seed(&[20; 32])
                                    .unwrap()
                                    .public_key(),
                                pages,
                                revoked: false,
                            }),
                        },
                        1,
                    )
                    .unwrap();
                store.close().unwrap();
            }
            if serving {
                pilot.start();
            }
            let first_before = pilot.call(&["show", PAGE, "--json"]);
            let second_before = pilot.call(&["show", second_id, "--json"]);
            let mut args = vec!["share", "member", action, PAGE, target];
            if action == "role" {
                args.push("viewer");
            }
            args.push("--json");
            pilot.call(&args);
            for (id, before) in [(PAGE, first_before), (second_id, second_before)] {
                let detail = pilot.call(&["show", id, "--json"]);
                if action == "remove" {
                    assert_ne!(detail["page"]["epoch"], before["page"]["epoch"]);
                } else {
                    // Role reduction retains read keys; the engine cuts writer sequences.
                    assert_eq!(detail["page"]["epoch"], before["page"]["epoch"]);
                }
                let recipient = detail["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["id"] == target)
                    .unwrap();
                assert_eq!(recipient["pages"].as_array().unwrap().len(), 2);
                if action == "role" {
                    assert_eq!(recipient["role"], "viewer");
                } else {
                    assert_eq!(recipient["revoked"], true);
                }
            }
            if serving {
                pilot.stop();
            }
        }
    }
}

const READY: &str = r#"{"profile":"local-v1","binding":"loopback-http","state":"ready","address":"http://127.0.0.1:53253/r/3e2c69f7","machineId":"m","windowId":"w","startupCoreCalls":2}"#;
const STOPPED: &str = r#"{"running":false,"lastPort":53253}"#;
const PAGE_LINK: &str = "http://127.0.0.1:53253/colab/";
// Remote's JSON envelope joins its message and hint without introducing a hint field (#1734).
const PORT_BUSY: &str = "Remote's port 53253 is in use: stop what is using it to keep this browser paired, or run tmt remote serve --port <n> and pair again";
/// What the stand-in for `tmt remote serve --json` does.
#[derive(Clone, Copy)]
enum Serve {
    /// Exits at once, like a missing extension or a held lock.
    Fail,
    /// Prints an error envelope on either stream, then exits without a descriptor.
    Error { stderr: bool },
    /// Prints its descriptor, then runs until SIGTERM.
    Hold,
    /// Prints its descriptor and more stderr than a pipe holds, then runs until SIGTERM.
    Diagnostics,
    /// Prints its descriptor, then dies on its own.
    Crash,
    /// Runs until SIGTERM without ever printing a descriptor.
    Silent,
    /// Like `Hold`, but its real work is a grandchild that outlives a leader exiting on SIGTERM,
    /// as a wrapper process in front of the door does.
    Wrapped,
}
impl Pilot {
    /// A core stand-in with a scripted Remote: `status` is its `remote status --json` answer
    /// (`None` fails, as an absent extension does); every other command is the fixture core.
    fn remote_core(&self, status: Option<&str>, serve: Serve) -> PathBuf {
        // A leading `!` is an error envelope: printed on stdout, exit 1, like Remote does.
        let status = status.map_or("exit 1".to_owned(), |s| match s.strip_prefix('!') {
            Some(envelope) => format!("printf '%s\\n' {}\nexit 1", quote(envelope)),
            None => format!("printf '%s\\n' {}\nexit 0", quote(s)),
        });
        let hold = "trap 'touch serve.term; exit 0' TERM\nwhile :; do sleep 0.1; done";
        let serve = match serve {
            Serve::Fail => "exit 2".to_owned(),
            Serve::Error { stderr } => {
                let envelope = json!({"error":{"code":"REMOTE_PORT_BUSY","message":PORT_BUSY}});
                format!(
                    "printf '%s\\n' {}{}\nexit 2",
                    quote(&envelope.to_string()),
                    if stderr { " >&2" } else { "" }
                )
            }
            Serve::Hold => format!("printf '%s\\n' {}\n{hold}", quote(READY)),
            Serve::Diagnostics => format!(
                "printf '%s\\n' {}\nprintf 'first diagnostic\\n' >&2\ni=0\nwhile [ \"$i\" -lt 2048 ]; do printf 'a continuing Remote diagnostic line\\n' >&2; i=$((i+1)); done\nprintf 'last diagnostic\\n' >&2\n{hold}",
                quote(READY)
            ),
            Serve::Crash => format!("printf '%s\\n' {}\nsleep 0.5\nexit 3", quote(READY)),
            Serve::Silent => hold.to_owned(),
            Serve::Wrapped => format!(
                "sleep 300 &\necho $! > serve.grandchild\nprintf '%s\\n' {}\ntrap 'exit 0' TERM\nwait",
                quote(READY)
            ),
        };
        fs::write(self.root.join("remote-status"), status).unwrap();
        fs::write(self.root.join("remote-serve"), serve).unwrap();
        core_fixture::link(&self.root, "core-remote", core_fixture::Program::Remote)
    }
    /// What the scripted Remote answers to `devices --json`; absent means it gives no answer.
    fn devices(&self, json: &str) {
        fs::write(self.root.join("devices.json"), json).unwrap();
    }
    fn serve_pid(&self) -> Option<Pid> {
        let text = fs::read_to_string(self.root.join("serve.pid")).ok()?;
        Some(Pid::from_raw(text.trim().parse().ok()?))
    }
}
/// A running `tmt-colab serve` whose output goes to files, so no pipe end leaks to other tests.
struct Serving {
    child: Child,
    out: PathBuf,
    err: PathBuf,
}
impl Serving {
    fn start(pilot: &Pilot, core: PathBuf, args: &[&str]) -> Self {
        let out = pilot.root.join("serve.out");
        let err = pilot.root.join("serve.err");
        let child = pilot
            .command()
            .env("TMT_EXECUTABLE", core)
            .args(["serve", "--foreground"])
            .args(args)
            .stdout(fs::File::create(&out).unwrap())
            .stderr(fs::File::create(&err).unwrap())
            .spawn()
            .unwrap();
        Self { child, out, err }
    }
    fn wait_for(path: &PathBuf, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = fs::read_to_string(path).unwrap_or_default();
            if text.contains(needle) {
                return text;
            }
            assert!(Instant::now() < deadline, "no {needle:?} in {text:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }
    /// The first JSON line, once printed.
    fn ready(&self) -> Value {
        let text = Self::wait_for(&self.out, "\n");
        serde_json::from_str(text.lines().next().unwrap()).unwrap()
    }
    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
    /// Signals the foreground process and requires a clean, bounded exit.
    fn stop(&mut self, signal: Signal) {
        kill(Pid::from_raw(self.child.id() as i32), signal).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status:?}");
                return;
            }
            assert!(Instant::now() < deadline, "serve did not stop");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn pid_file(pilot: &Pilot, name: &str) -> Pid {
    Pid::from_raw(
        fs::read_to_string(pilot.root.join(name))
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    )
}
/// Rows are padded to the longest key, which differs by platform (the `decoder` row is macOS-only):
/// compare words, not columns.
fn squashed(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}
fn assert_gone(pid: Pid) {
    assert_eq!(kill(pid, None), Err(Errno::ESRCH), "process {pid} leaked");
}
#[test]
fn serve_attaches_to_a_running_door_and_says_what_to_do_next() {
    let pilot = Pilot::new(None);
    pilot.devices(r#"{"devices":[]}"#);
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(DOOR), Serve::Fail), &[]);
    let text = squashed(&Serving::wait_for(&serving.out, "create "));
    // Whole values in the existing LOCAL SPACE layout: state first, then the next steps.
    for wanted in [
        "LOCAL SPACE",
        "(ready)",
        "attached · http://127.0.0.1:53253",
        "no",
        "create tmt colab page create --title <title>",
        "pair for page access: tmt remote pair",
    ] {
        assert!(text.contains(wanted), "{wanted:?} missing in {text}");
    }
    // The link opens only once a browser is paired, so the pairing step comes before it.
    assert!(
        text.find("\npair ").unwrap() < text.find("\nopen ").unwrap(),
        "{text}"
    );
    serving.stop(Signal::SIGTERM);
    assert!(!pilot.root.join("serve.calls").exists());
}
#[test]
fn serve_starts_a_door_without_a_port_and_stops_it_with_ctrl_c() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(STOPPED), Serve::Hold),
        &["--json"],
    );
    let ready = serving.ready();
    assert_eq!(ready["door"], "started");
    assert_eq!(ready["url"], PAGE_LINK);
    // The decoder note lives in JSON only.
    assert!(ready["memoryLimit"].as_str().unwrap().contains("limit"));
    // Pairing could not be read: not claimed either way, and the step stays on offer.
    assert!(ready["paired"].is_null());
    assert_eq!(
        ready["next"],
        json!(["tmt remote pair", "tmt colab page create --title <title>"])
    );
    // Remote owns the port policy: Colab passes none and never retries.
    assert_eq!(
        fs::read_to_string(pilot.root.join("serve.calls")).unwrap(),
        "remote serve --json\n"
    );
    let door = pilot.serve_pid().unwrap();
    assert_eq!(kill(door, None), Ok(()));
    serving.stop(Signal::SIGINT);
    assert!(pilot.root.join("serve.term").exists());
    assert_gone(door);
}
#[test]
fn serve_without_remote_runs_local_only_and_names_the_install_step() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(&pilot, pilot.remote_core(None, Serve::Fail), &["--json"]);
    let ready = serving.ready();
    assert_eq!(ready["door"], "unavailable");
    assert!(ready["url"].is_null() && ready["origin"].is_null() && ready["paired"].is_null());
    assert_eq!(
        ready["next"],
        json!(["tmt colab page create --title <title>"])
    );
    assert_eq!(
        ready["warning"],
        "Browser access needs the Remote extension: tmt extension install remote --yes"
    );
    assert!(serving.running());
    serving.stop(Signal::SIGTERM);
}
/// The serve running from an installed release directory is told, once and only on its
/// terminal, when `current` moves to another version; it keeps serving and nothing is changed.
#[test]
fn serve_in_an_installed_release_warns_once_when_a_newer_version_becomes_current() {
    let pilot = Pilot::new(None);
    let lib = pilot.root.join("lib/tmt-colab");
    for (id, version) in [("old", "0.0.0-running"), ("new", "9.9.9-installed")] {
        let dir = lib.join("releases").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("receipt.json"),
            json!({"version": version}).to_string(),
        )
        .unwrap();
    }
    let exe = lib.join("releases/old/tmt-colab");
    tmt_test_support::write_executable(&exe, &fs::read(BINARY).unwrap(), 0o755).unwrap();
    let current = lib.join("current");
    std::os::unix::fs::symlink("releases/old", &current).unwrap();
    let base = pilot.command();
    let mut command = Command::new(&exe);
    command.env_clear();
    for (key, value) in base.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    let out = pilot.root.join("serve.out");
    let err = pilot.root.join("serve.err");
    let child = command
        .env("TMT_EXECUTABLE", pilot.remote_core(None, Serve::Fail))
        .args(["serve", "--foreground"])
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(&err).unwrap())
        .spawn()
        .unwrap();
    let mut serving = Serving { child, out, err };
    Serving::wait_for(&serving.out, "LOCAL SPACE");
    // The running release is the active one: nothing to say, and the serve does not wait for it.
    assert!(
        !fs::read_to_string(&serving.err)
            .unwrap()
            .contains("installed")
    );
    std::fs::remove_file(&current).unwrap();
    std::os::unix::fs::symlink("releases/new", &current).unwrap();
    // The warning and its hint are written separately: wait for the last of them.
    let warning = Serving::wait_for(&serving.err, "to update");
    assert!(
        warning.contains("Colab 9.9.9-installed is installed, but ")
            && warning.contains(" is still running")
            && warning.contains("Restart `tmt colab serve` to update"),
        "{warning}"
    );
    assert_eq!(warning.matches("is installed, but").count(), 1, "{warning}");
    // Still serving; no restart, no change to the installed release it was told about.
    assert!(serving.running());
    assert!(exe.is_file());
    serving.stop(Signal::SIGTERM);
}
#[test]
fn serve_with_a_door_that_will_not_start_keeps_the_local_space_without_the_install_line() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(STOPPED), Serve::Fail), &[]);
    // The warning and its hint are written separately: wait for the last of them.
    let warning = Serving::wait_for(&serving.err, "tmt remote serve shows why");
    assert!(warning.contains("did not start"), "{warning}");
    assert!(
        warning.starts_with("warning:") || warning.contains("warning:"),
        "{warning}"
    );
    assert!(!warning.contains("extension install"), "{warning}");
    assert!(warning.contains("tmt remote serve shows why"), "{warning}");
    let text = Serving::wait_for(&serving.out, "local only");
    assert!(
        text.contains("unavailable") && !text.contains("pair"),
        "{text}"
    );
    serving.stop(Signal::SIGTERM);
}
#[test]
fn serve_preserves_remote_start_errors_on_either_stream_in_human_and_json_output() {
    for stderr in [false, true] {
        for json_output in [false, true] {
            let pilot = Pilot::new(None);
            let mut serving = Serving::start(
                &pilot,
                pilot.remote_core(Some(STOPPED), Serve::Error { stderr }),
                if json_output { &["--json"] } else { &[] },
            );
            if json_output {
                let ready = serving.ready();
                assert_eq!(ready["door"], "unavailable");
                assert_eq!(ready["warning"], PORT_BUSY);
                assert!(ready["url"].is_null() && ready["origin"].is_null());
                assert_eq!(
                    ready["next"],
                    json!(["tmt colab page create --title <title>"])
                );
                assert_eq!(fs::read_to_string(&serving.err).unwrap(), "");
            } else {
                let warning = Serving::wait_for(&serving.err, "--port <n> and pair again");
                assert_eq!(warning, format!("warning: {PORT_BUSY}\n"));
                let text = Serving::wait_for(&serving.out, "local only");
                assert!(
                    text.contains("unavailable") && !text.contains("pair"),
                    "{text}"
                );
            }
            assert!(serving.running());
            assert_gone(pilot.serve_pid().unwrap());
            assert_eq!(
                fs::read_to_string(pilot.root.join("serve.calls")).unwrap(),
                "remote serve --json\n"
            );
            serving.stop(Signal::SIGTERM);
        }
    }
}
#[test]
fn a_started_door_keeps_streaming_stderr_without_blocking_or_losing_diagnostics() {
    for json_output in [false, true] {
        let pilot = Pilot::new(None);
        pilot.opener(1);
        let mut serving = Serving::start(
            &pilot,
            pilot.remote_core(Some(STOPPED), Serve::Diagnostics),
            if json_output {
                &["--json", "--open"]
            } else {
                &["--open"]
            },
        );
        if json_output {
            assert_eq!(serving.ready()["door"], "started");
        } else {
            // The failed stub opener makes Colab warn while the door still runs. Forwarding
            // diagnostics must not hold stderr's lock while waiting for more child output.
            Serving::wait_for(&serving.out, "started");
            Serving::wait_for(&serving.err, "warning:");
        }
        let diagnostics = Serving::wait_for(&serving.err, "last diagnostic");
        assert!(diagnostics.contains("first diagnostic\n"), "{diagnostics}");
        assert!(diagnostics.len() > 64 * 1024);
        assert!(serving.running());
        serving.stop(Signal::SIGTERM);
        assert_gone(pilot.serve_pid().unwrap());
    }
}
#[test]
fn a_door_that_dies_is_reported_and_the_local_space_keeps_running() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(STOPPED), Serve::Crash),
        &["--json"],
    );
    assert_eq!(serving.ready()["door"], "started");
    let warning = Serving::wait_for(&serving.err, "exited with status 3");
    assert!(warning.contains("keeps running"), "{warning}");
    assert!(serving.running());
    serving.stop(Signal::SIGTERM);
    assert_gone(pilot.serve_pid().unwrap());
}
#[test]
fn ctrl_c_while_a_door_is_still_starting_stops_it_and_leaves_nothing_behind() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(STOPPED), Serve::Silent),
        &["--json"],
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while pilot.serve_pid().is_none() {
        assert!(Instant::now() < deadline, "door never started");
        thread::sleep(Duration::from_millis(20));
    }
    let door = pilot.serve_pid().unwrap();
    serving.stop(Signal::SIGINT);
    assert_gone(door);
}
#[test]
fn stopping_serve_stops_everything_a_wrapped_door_left_behind() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(STOPPED), Serve::Wrapped),
        &["--json"],
    );
    assert_eq!(serving.ready()["door"], "started");
    let grandchild = pid_file(&pilot, "serve.grandchild");
    assert_eq!(kill(grandchild, None), Ok(()));
    serving.stop(Signal::SIGTERM);
    assert_gone(pid_file(&pilot, "serve.pid"));
    assert_gone(grandchild);
}
#[test]
fn a_paired_space_with_a_page_prints_its_link_and_no_pairing_step() {
    let pilot = Pilot::new(None);
    pilot.devices(r#"{"devices":[{"revoked":true},{"revoked":false,"name":"laptop"}]}"#);
    let created = pilot.call(&["page", "create", "--title", "First", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    let ready = serving.ready();
    assert_eq!(ready["door"], "attached");
    assert_eq!(ready["origin"], "http://127.0.0.1:53253");
    assert_eq!(ready["paired"], true);
    assert_eq!(ready["devices"], 1);
    assert_eq!(ready["pages"], 1);
    assert_eq!(
        ready["page"],
        format!("http://127.0.0.1:53253/p/{}", &page[..8])
    );
    assert_eq!(ready["next"], json!([]));
    assert!(ready["warning"].is_null());
    serving.stop(Signal::SIGTERM);
    // The human form drops the pairing row too.
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(DOOR), Serve::Fail), &[]);
    let text = Serving::wait_for(&serving.out, "yes (1 device)");
    assert!(!text.contains("pair this browser"), "{text}");
    assert!(text.contains(&format!("/p/{}", &page[..8])), "{text}");
    serving.stop(Signal::SIGTERM);
}
impl Pilot {
    /// `tmt colab stop`, returning its parsed `--json` answer.
    fn stop_json(&self) -> Value {
        let out = self.command().args(["stop", "--json"]).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn socket(&self) -> PathBuf {
        self.root.join("selected/colab/door.sock")
    }
}
impl Serving {
    /// The serving process exits cleanly by itself, without any signal from the test.
    fn exits(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status:?}");
                return;
            }
            assert!(Instant::now() < deadline, "serve did not stop");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn stop_ends_serve_and_the_door_it_started_and_is_idempotent() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(STOPPED), Serve::Wrapped),
        &["--json"],
    );
    assert_eq!(serving.ready()["door"], "started");
    let door = pid_file(&pilot, "serve.pid");
    let grandchild = pid_file(&pilot, "serve.grandchild");
    assert_eq!(
        pilot.stop_json(),
        json!({"state":"stopped","door":"started"})
    );
    // `stop` returns only once the serving process released its lock, so these are final.
    serving.exits();
    assert_gone(door);
    assert_gone(grandchild);
    assert!(!pilot.socket().exists());
    // Pairings and data are untouched; a second stop has nothing to do and still succeeds.
    assert!(pilot.root.join("selected/colab/space.db").exists());
    assert_eq!(
        pilot.stop_json(),
        json!({"state":"not-running","door":null})
    );
}
#[test]
fn stop_leaves_an_attached_door_running_and_says_so() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(DOOR), Serve::Fail), &[]);
    Serving::wait_for(&serving.out, "attached");
    let out = pilot.command().arg("stop").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Colab stopped"), "{text}");
    assert!(
        text.contains("Remote is still running; stop it with tmt remote stop"),
        "{text}"
    );
    assert!(
        text.lines().count() <= 2 && !text.contains('\u{2014}'),
        "{text}"
    );
    serving.exits();
    assert!(!pilot.root.join("serve.calls").exists());
}
#[test]
fn stop_with_nothing_running_is_a_clear_success_even_before_any_state() {
    let pilot = Pilot::new(None);
    let out = pilot.command().arg("stop").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Colab is not running"));
    assert_eq!(
        pilot.stop_json(),
        json!({"state":"not-running","door":null})
    );
    assert!(!pilot.root.join("selected/colab").exists());
}
#[test]
fn a_forwarded_browser_request_cannot_stop_serve() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    serving.ready();
    let post = |method: &str, extra: &str| {
        let mut socket = UnixStream::connect(pilot.socket()).unwrap();
        write!(
            socket,
            "{method} /.tmt/colab/local/stop HTTP/1.1\r\nHost: localhost\r\n{extra}Content-Length: 0\r\n\r\n"
        )
        .unwrap();
        let mut reply = String::new();
        socket.read_to_string(&mut reply).unwrap();
        reply
    };
    // The same request a browser path would carry once Remote forwarded a device context.
    let forwarded = post(
        "POST",
        "tmt-device-context: {\"owner\":true,\"deviceId\":\"d\",\"deviceName\":\"n\"}\r\n",
    );
    assert!(forwarded.starts_with("HTTP/1.1 403"), "{forwarded}");
    assert!(post("GET", "").starts_with("HTTP/1.1 400"));
    assert!(serving.running());
    serving.stop(Signal::SIGTERM);
}
#[test]
fn without_a_door_the_page_text_gives_the_reason_and_never_a_manual_remote_command() {
    for (core, reason) in [
        (None, "Browser access needs the Remote extension"),
        (Some(STOPPED), "browser access unavailable: see warning"),
    ] {
        let pilot = Pilot::new(None);
        pilot.call(&["page", "create", "--title", "First", "--json"]);
        let mut serving = Serving::start(&pilot, pilot.remote_core(core, Serve::Fail), &[]);
        let text = Serving::wait_for(&serving.out, "open");
        let open = text
            .lines()
            .find(|l| l.trim_start().starts_with("open"))
            .unwrap();
        assert!(open.contains("/p/") && open.contains(reason), "{open}");
        assert!(!text.contains("tmt remote serve"), "{text}");
        serving.stop(Signal::SIGTERM);
    }
}

#[test]
fn cli_publisher_label_uses_fixed_public_command_and_unknown_writes_clear_it() {
    let pilot = Pilot::new(None);
    let publisher = pilot.root.join("publisher");
    fs::write(
        &publisher,
        json!({"identity":{"name":"publishing-agent"}}).to_string(),
    )
    .unwrap();
    let created = pilot.call(&["page", "create", "--title", "Published", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    assert_eq!(
        pilot.call(&["page", "read", page, "--json"])["publisherAgent"],
        "publishing-agent"
    );
    let file = pilot.root.join("page.html");
    fs::write(&file, "<p>New content</p>").unwrap();
    for reply in [
        Some(json!({"identity":{"name":"next-agent"}})),
        None,
        Some(json!({"error":{"code":"IDENTITY_REQUIRED"}})),
        Some(json!({"identity":{"name":"é".repeat(65)}})),
    ] {
        let expected = reply
            .as_ref()
            .and_then(|value| value["identity"]["name"].as_str())
            .filter(|value| tmt_colab::decoder::valid_publisher_agent(value))
            .map(str::to_owned);
        if let Some(value) = reply {
            fs::write(&publisher, value.to_string()).unwrap();
        } else {
            fs::remove_file(&publisher).unwrap();
        }
        pilot.call(&[
            "page",
            "write",
            page,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let read = pilot.call(&["page", "read", page, "--json"]);
        assert_eq!(read["source"], "<p>New content</p>");
        assert_eq!(
            read.get("publisherAgent").and_then(Value::as_str),
            expected.as_deref()
        );
    }
    let calls = fs::read_to_string(pilot.root.join("publisher-calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        vec!["identity show --json"; 5]
    );
}

#[test]
fn serve_opens_the_space_home_only_when_told_or_allowed_and_says_so() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    let home = PAGE_LINK;
    // No terminal under test: without a flag nothing opens and the link is just printed.
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(DOOR), Serve::Fail), &[]);
    let text = Serving::wait_for(&serving.out, "create");
    assert!(squashed(&text).contains(&format!("open {home}")), "{text}");
    assert!(pilot.opened().is_empty());
    serving.stop(Signal::SIGTERM);
    // `--no-open` and `--json` always skip, even with the setting on and `--open` absent.
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--open", "--json"],
    );
    assert_eq!(serving.ready()["opened"], false);
    serving.stop(Signal::SIGTERM);
    assert!(pilot.opened().is_empty());
    // `--open` forces it past the missing terminal, and the row says what happened.
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--open"],
    );
    let text = Serving::wait_for(&serving.out, "create");
    assert!(
        text.contains(&format!("opened in your browser: {home}")),
        "{text}"
    );
    serving.stop(Signal::SIGTERM);
    // The space home is the link that lands on the space, the same one a rerun prints.
    let opened = pilot.opened();
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert_eq!(opened[0], home, "{opened:?}");
}
#[test]
fn serve_opens_the_single_page_and_the_setting_is_overridden_by_flags_only() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    let created = pilot.call(&["page", "create", "--title", "Only", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    pilot.call(&["settings", "open", "off", "--json"]);
    let core = pilot.remote_core(Some(DOOR), Serve::Fail);
    // `--no-open` wins over `--open` given later? No: the last one given wins.
    let mut serving = Serving::start(&pilot, core.clone(), &["--open", "--no-open"]);
    Serving::wait_for(&serving.out, "open");
    serving.stop(Signal::SIGTERM);
    assert!(pilot.opened().is_empty());
    let mut serving = Serving::start(&pilot, core, &["--open"]);
    Serving::wait_for(&serving.out, "opened in your browser");
    serving.stop(Signal::SIGTERM);
    let opened = pilot.opened();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].starts_with("http://127.0.0.1:53253/p/"));
    assert!(opened[0].ends_with(&page[..8]), "{opened:?}");
}
#[test]
fn a_failing_opener_warns_once_and_keeps_the_printed_link() {
    let pilot = Pilot::new(None);
    pilot.opener(1);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--open"],
    );
    let warning = Serving::wait_for(&serving.err, "Could not open the browser");
    assert_eq!(warning.matches("Could not open").count(), 1, "{warning}");
    let text = Serving::wait_for(&serving.out, "create");
    assert!(
        text.contains(PAGE_LINK) && !text.contains("opened in your browser"),
        "{text}"
    );
    serving.stop(Signal::SIGTERM);
}
#[test]
fn page_create_opener_matrix_is_independent_of_terminal_and_json() {
    // `script` gives the actual native executable a PTY, not a forced presentation setting.
    for tty in [false, true] {
        for json_output in [false, true] {
            for setting in [false, true] {
                for environment in ["local", "ci", "ssh", "forwarded", "no-opener", "no-open"] {
                    let pilot = Pilot::new(None);
                    pilot.opener(0);
                    pilot.call(&[
                        "settings",
                        "open",
                        if setting { "on" } else { "off" },
                        "--json",
                    ]);
                    let mut command = pilot.command_with_creation_door(CREATION_DOOR);
                    command.env_remove("CI").env("DISPLAY", ":fixture");
                    command.args(["page", "create", "--title", "Created"]);
                    if json_output {
                        command.arg("--json");
                    }
                    match environment {
                        "ci" => {
                            command.env("CI", "1");
                        }
                        "ssh" => {
                            command
                                .env("SSH_CONNECTION", "fixture")
                                .env_remove("DISPLAY");
                        }
                        "forwarded" => {
                            command.env("SSH_CONNECTION", "fixture");
                        }
                        "no-opener" => {
                            command.env("PATH", pilot.root.join("empty-bin"));
                        }
                        "no-open" => {
                            command.arg("--no-open");
                        }
                        _ => {}
                    }
                    let out = if tty {
                        let mut wrapped = Command::new("/usr/bin/script");
                        wrapped.env_clear().envs(
                            command
                                .get_envs()
                                .filter_map(|(name, value)| value.map(|v| (name, v))),
                        );
                        if cfg!(target_os = "macos") {
                            wrapped
                                .args(["-q", "/dev/null"])
                                .arg(command.get_program())
                                .args(command.get_args());
                        } else {
                            let line = std::iter::once(command.get_program())
                                .chain(command.get_args())
                                .map(|v| quote(v.to_str().unwrap()))
                                .collect::<Vec<_>>()
                                .join(" ");
                            wrapped.args(["-q", "-e", "-c", &line, "/dev/null"]);
                        }
                        // Keep the PTY's input open until the command exits: immediate EOF
                        // makes macOS script echo a synthetic ^D into captured JSON.
                        let mut child = wrapped
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .unwrap();
                        let input = child.stdin.take();
                        let output = child.wait_with_output().unwrap();
                        drop(input);
                        output
                    } else {
                        command.output().unwrap()
                    };
                    assert!(
                        out.status.success(),
                        "{tty}/{json_output}/{setting}/{environment}: {out:?}"
                    );
                    let expected = setting && matches!(environment, "local" | "forwarded");
                    pilot.assert_creation_acquisitions(1);
                    let calls = pilot.opened();
                    assert_eq!(
                        calls.len(),
                        usize::from(expected),
                        "{tty}/{json_output}/{setting}/{environment}"
                    );
                    if expected {
                        assert!(
                            calls[0].starts_with("http://127.0.0.1:53253/p/"),
                            "{calls:?}"
                        );
                    }
                    if json_output {
                        // Parsing the entire output rejects extra documents or human/progress text.
                        let result: Value = serde_json::from_slice(&out.stdout).unwrap();
                        assert_eq!(result["opened"], expected);
                        assert!(
                            result["shortLink"]
                                .as_str()
                                .unwrap()
                                .starts_with("http://127.0.0.1:53253/p/")
                        );
                    } else if expected {
                        assert!(
                            String::from_utf8_lossy(&out.stdout)
                                .contains("opened in your browser:")
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn page_create_without_terminal_opens_once_and_failure_or_no_door_keeps_creation() {
    for (door, exit, expected_opened) in [(true, 0, true), (true, 9, false), (false, 0, false)] {
        let pilot = Pilot::new(None);
        pilot.opener(exit);
        let mut command = if door {
            pilot.command_with_creation_door(CREATION_DOOR)
        } else {
            pilot.command()
        };
        let out = command
            .env_remove("CI")
            .env("DISPLAY", ":fixture")
            .args(["page", "create", "--title", "Kept", "--json"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let result: Value = serde_json::from_slice(&out.stdout).unwrap();
        if door {
            pilot.assert_creation_acquisitions(1);
        }
        assert_eq!(result["opened"], expected_opened);
        assert_eq!(pilot.opened().len(), usize::from(door));
        let page = result["pageId"].as_str().unwrap();
        assert_eq!(
            pilot.call(&["page", "read", page, "--json"])["title"],
            "Kept"
        );
        if expected_opened {
            assert_eq!(pilot.opened()[0], result["shortLink"].as_str().unwrap());
        }
    }
}
#[test]
fn explicit_open_hands_off_home_and_resolved_page_without_mutating_content() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    seed_page_with_title(&pilot, "Existing page");
    pilot.call(&["settings", "open", "off", "--json"]);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    serving.ready();
    let database = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&database).unwrap();
    for args in [vec!["open"], vec!["open", &PAGE[..8]]] {
        // Captured stdout is not a terminal: explicit intent still opens.
        let result = pilot.command_with_door(DOOR).args(args).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(String::from_utf8_lossy(&result.stdout).contains("opened in your browser:"));
    }
    let opened = pilot.opened();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[0], "http://127.0.0.1:53253/colab/");
    assert_eq!(
        opened[1],
        format!("http://127.0.0.1:53253/p/{}", &PAGE[..8])
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    serving.stop(Signal::SIGTERM);
}
#[test]
fn explicit_open_reports_facts_without_launching_for_json_no_open_or_unavailable_services() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    seed_page_with_title(&pilot, "Existing page");
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    serving.ready();
    let result = pilot
        .command_with_door(DOOR)
        .args(["open", &PAGE[..8], "--json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let facts: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(facts["pageId"], PAGE);
    assert_eq!(facts["running"], true);
    assert_eq!(facts["opened"], false);
    assert_eq!(
        facts["shortLink"],
        format!("http://127.0.0.1:53253/p/{}", &PAGE[..8])
    );
    for (door, args) in [
        (DOOR, vec!["open", PAGE, "--no-open"]),
        (STOPPED, vec!["open", PAGE]),
    ] {
        let result = pilot.command_with_door(door).args(args).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(!String::from_utf8_lossy(&result.stdout).contains("opened in your browser"));
    }
    serving.stop(Signal::SIGTERM);
    let result = pilot
        .command_with_door(DOOR)
        .args(["open", PAGE])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("tmt colab serve"));
    assert!(pilot.opened().is_empty());
    failure(
        &pilot,
        &["open", "ffffffff", "--json"],
        "COLAB_PAGE_NOT_FOUND",
    );
    pilot.call(&["delete", PAGE, "--yes", "--json"]);
    failure(&pilot, &["open", PAGE, "--json"], "COLAB_PAGE_DELETED");
    assert!(pilot.opened().is_empty());
}
#[test]
fn explicit_open_failure_keeps_the_link_and_warns_once() {
    let pilot = Pilot::new(None);
    pilot.opener(1);
    seed_page_with_title(&pilot, "Existing page");
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    serving.ready();
    let result = pilot
        .command_with_door(DOOR)
        .args(["open", PAGE])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr)
            .matches("Could not open the browser")
            .count(),
        1
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains(&format!("http://127.0.0.1:53253/p/{}", &PAGE[..8]))
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("opened in your browser"));
    assert_eq!(pilot.opened().len(), 1);
    serving.stop(Signal::SIGTERM);
}
#[test]
fn settings_show_and_change_open_with_its_source_and_survive_a_damaged_file() {
    let pilot = Pilot::new(None);
    assert_eq!(
        pilot.call(&["settings", "--json"]),
        json!({"open":true,"source":"default"})
    );
    assert!(!pilot.root.join("selected/colab").exists());
    assert_eq!(
        pilot.call(&["settings", "open", "off", "--json"]),
        json!({"open":false,"source":"settings.json"})
    );
    let out = pilot.command().arg("settings").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("open") && text.contains("off (settings.json)"),
        "{text}"
    );
    let file = pilot.root.join("selected/colab/settings.json");
    assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"open":false}"#);
    // A damaged file reads as the defaults with a warning, never an error.
    fs::write(&file, "{").unwrap();
    let out = pilot.command().arg("settings").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("on (default)"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("settings.json could not be read"));
    assert_eq!(
        pilot.call(&["settings", "open", "on", "--json"]),
        json!({"open":true,"source":"settings.json"})
    );
}
#[test]
fn expiry_human_lines_are_relative_dim_and_keep_exact_json_and_local_state() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let pilot = Pilot::new(None);
    pilot.devices(r#"{"devices":[]}"#);
    let created = pilot.call(&["page", "create", "--title", "Relative expiry", "--json"]);
    let id = created["pageId"].as_str().unwrap();
    let day = 86_400_000u64;
    // Only fixture SQL sets the observation time. Production still uses admitted content updates.
    let set_updated = |ms: Option<u64>| {
        rusqlite::Connection::open(pilot.root.join("selected/colab/space.db"))
            .unwrap()
            .execute(
                "UPDATE pages SET last_update_at_ms=?1",
                [ms.map(|value| i64::try_from(value).unwrap())],
            )
            .unwrap();
    };
    let now = || {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    };
    for (updated, expiry, last_edit, warning) in [
        (Some(now() - 120_000), "in 29 days", Some("2 min ago"), None),
        (
            Some(now() - 23 * day - 3_600_000),
            "◷ in 6 days",
            Some("23 days ago"),
            Some("expires-soon"),
        ),
        (
            Some(now() - 32 * day - 3_600_000),
            "expired 2 days ago",
            Some("32 days ago"),
            Some("expired"),
        ),
        (
            None,
            "starts after the next edit",
            None,
            Some("expiry-unavailable"),
        ),
    ] {
        set_updated(updated);
        let before = pilot.call(&["show", id, "--json"]);
        assert_eq!(before["page"]["lastUpdateAtMs"], json!(updated));
        assert_eq!(
            before["page"]["expiresAtMs"],
            json!(updated.map(|ms| ms + 30 * day))
        );
        assert_eq!(
            before["page"]["warnings"],
            json!(warning.into_iter().collect::<Vec<_>>())
        );
        for columns in ["40", "120"] {
            for color in [false, true] {
                let human = |args: &[&str]| {
                    let mut cmd = pilot.command();
                    cmd.env("COLUMNS", columns);
                    if color {
                        cmd.env("CLICOLOR_FORCE", "1");
                    }
                    let out = cmd.args(args).output().unwrap();
                    assert!(out.status.success(), "{out:?}");
                    out
                };
                let list = human(&["ls"]);
                let list_text = String::from_utf8(list.stdout).unwrap();
                let list_expiry = expiry.replace("in ", "expires in ");
                let line = if color {
                    format!("    \x1b[2m{list_expiry}\x1b[0m\n")
                } else {
                    format!("    {list_expiry}\n")
                };
                assert!(list_text.contains(&line), "{list_text:?}");
                let link = format!("/p/{}", &id[..8]);
                assert!(
                    list_text.contains(&link),
                    "short link survives narrow output: {list_text}"
                );
                assert!(list_text.find(&link).unwrap() < list_text.find(&list_expiry).unwrap());
                for args in [["show", id], ["retention", id]] {
                    let out = human(&args);
                    let text = String::from_utf8(out.stdout).unwrap();
                    assert!(text.contains(expiry), "{args:?}: {text}");
                    if let Some(last_edit) = last_edit {
                        assert!(text.contains(last_edit), "{text}");
                    } else {
                        assert!(!text.contains("last edit"), "legacy time is not invented");
                    }
                    assert!(text.contains("Expiry never deletes your local copy."));
                    assert!(!text.contains(" UTC") && !text.contains("advisory"));
                    if matches!(warning, Some("expires-soon" | "expired")) {
                        let stderr = String::from_utf8(out.stderr).unwrap();
                        assert!(
                            stderr.contains(expiry)
                                && stderr.contains("Expiry never deletes your local copy"),
                            "{stderr:?}"
                        );
                    } else {
                        assert!(out.stderr.is_empty());
                    }
                }
            }
        }
        assert_eq!(
            pilot.call(&["show", id, "--json"]),
            before,
            "human reads cannot refresh timestamps or policy"
        );
        let list = pilot.call(&["ls", "--json"]);
        for field in ["retentionDays", "lastUpdateAtMs", "expiresAtMs", "warnings"] {
            assert_eq!(list["pages"][0][field], before["page"][field]);
        }
    }
    set_updated(Some(now() - 120_000));
    pilot.call(&["retention", id, "9007199254740991", "--json"]);
    let out = pilot.command().args(["show", id]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("beyond the supported range"));
    assert!(text.contains("retention") && text.contains("out of range"));
    assert!(!text.contains("9007199254740991"));
    let before = pilot.call(&["retention", id, "--json"]);
    assert_eq!(before["page"]["retentionDays"], 9_007_199_254_740_991u64);
    assert!(before["page"]["expiresAtMs"].is_null());
    assert_eq!(before["page"]["warnings"], json!(["expiry-out-of-range"]));
    pilot.call(&["retention", id, "forever", "--json"]);
    for args in [vec!["ls"], vec!["show", id], vec!["retention", id]] {
        let out = pilot.command().args(&args).output().unwrap();
        assert!(out.status.success());
        assert!(
            String::from_utf8(out.stdout)
                .unwrap()
                .contains("kept forever")
        );
        assert!(out.stderr.is_empty());
    }
}
#[test]
fn page_commands_print_the_link_and_the_pairing_step_or_the_reason_there_is_none() {
    let pilot = Pilot::new(None);
    pilot.devices(r#"{"devices":[]}"#);
    let created = pilot.call(&["page", "create", "--title", "Notes", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    let path = format!("/p/{}", &page[..8]);
    let full = format!("http://127.0.0.1:53253/p/{}", &page[..8]);
    let human = |door: Option<&str>, args: &[&str]| {
        let mut cmd = pilot.command();
        if let Some(status) = door {
            cmd.env(
                "TMT_EXECUTABLE",
                pilot.remote_core(Some(status), Serve::Fail),
            );
        }
        let out = cmd.args(args).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    for args in [["show", page], ["ls", ""]] {
        let args: Vec<&str> = args.into_iter().filter(|a| !a.is_empty()).collect();
        // With a door and nobody paired: the whole link, then the pairing step.
        let text = human(Some(DOOR), &args);
        assert!(text.contains(&full), "{args:?}: {text}");
        assert!(
            text.contains("pair for page access: tmt remote pair"),
            "{text}"
        );
        // Without a door: the path and the reason, never a manual `tmt remote serve`.
        let text = human(None, &args);
        assert!(
            text.contains(&path) && text.contains("Browser access needs the Remote"),
            "{text}"
        );
        assert!(
            !text.contains("tmt remote serve") && !text.contains("pair this browser"),
            "{text}"
        );
    }
    // The listing's footer is set off from the rows at the list indent, not tucked under the last page.
    let listing = human(Some(DOOR), &["ls"]);
    let row = listing
        .lines()
        .find(|line| line.contains(&page[..8]))
        .unwrap();
    assert!(row.trim_start().starts_with("Notes"), "{listing}");
    assert!(
        row.find("Notes").unwrap() < row.find(&page[..8]).unwrap(),
        "{row}"
    );
    assert!(
        row.find(&page[..8]).unwrap() < row.find("private").unwrap(),
        "{row}"
    );
    assert!(
        !listing.contains(page),
        "human ls uses eight-character IDs: {listing}"
    );
    assert!(
        listing.contains(
            "\n\n  pair for page access: tmt remote pair\n  Expiry never deletes your local copy."
        ),
        "{listing}"
    );
    let shown = human(Some(DOOR), &["show", page]);
    assert!(
        shown.contains(page),
        "show retains the full page ID: {shown}"
    );
    assert!(!shown.contains("not-available"), "{shown}");
    assert!(
        shown.contains("audience") && shown.contains("history"),
        "{shown}"
    );
    assert!(
        shown.find("link").unwrap() < shown.find("page ").unwrap(),
        "{shown}"
    );
    // JSON: `link` null without a door; `paired` and `next` say what to do.
    let json_of = |door: Option<&str>, args: &[&str]| {
        let mut cmd = pilot.command();
        if let Some(status) = door {
            cmd.env(
                "TMT_EXECUTABLE",
                pilot.remote_core(Some(status), Serve::Fail),
            );
        }
        let out = cmd.args(args).arg("--json").output().unwrap();
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let shown = json_of(Some(DOOR), &["show", page]);
    let short_path = format!("/p/{}", &page[..8]);
    let short_link = format!("http://127.0.0.1:53253{short_path}");
    assert_eq!(shown["link"], short_link);
    assert_eq!(shown["shortLink"], full);
    assert_eq!(shown["path"], short_path);
    assert_eq!(shown["paired"], false);
    assert_eq!(shown["next"], json!(["tmt remote pair"]));
    assert!(json_of(None, &["show", page])["link"].is_null());
    let listed = json_of(Some(DOOR), &["ls"]);
    assert_eq!(listed["pages"][0]["link"], short_link);
    assert_eq!(listed["pages"][0]["shortLink"], full);
    assert_eq!(listed["paired"], false);
}
#[test]
fn human_ls_names_an_admitted_empty_title_without_changing_the_json_title() {
    let pilot = Pilot::new(None);
    // Empty document titles are admitted; CLI creation deliberately requires a title.
    seed_page_with_title(&pilot, "");
    for columns in ["40", "120"] {
        let out = pilot
            .command()
            .env("COLUMNS", columns)
            .args(["ls"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let listing = String::from_utf8(out.stdout).unwrap();
        let row = listing
            .lines()
            .find(|line| line.contains(&PAGE[..8]))
            .unwrap();
        assert!(row.trim_start().starts_with("Untitled"), "{listing}");
        if columns == "120" {
            assert!(row.trim_start().starts_with("Untitled page"), "{listing}");
        }
        assert!(!listing.contains(PAGE), "{listing}");
    }
    let listed = pilot.call(&["ls", "--json"]);
    assert_eq!(listed["pages"][0]["pageId"], PAGE);
    assert_eq!(listed["pages"][0]["title"], "");
}
#[test]
fn human_ls_uses_the_link_prefix_even_when_a_collision_is_archived_or_deleted() {
    use tmt_colab::{keyring::Layout, store::Store};
    let pilot = Pilot::new(None);
    seed_page_with_title(&pilot, "Original page");
    let other = "10000000-1000-4000-8000-000000000002";
    let layout = Layout::open(&pilot.root.join("selected")).unwrap();
    let store = Store::open(&layout).unwrap();
    // The creation projection reserves this sibling ID even before it has content.
    store.create_page(other).unwrap();
    store.close().unwrap();
    for action in [None, Some("archive"), Some("delete")] {
        if let Some(action) = action {
            let mut args = vec![action, other, "--json"];
            if action == "delete" {
                args.push("--yes");
            }
            pilot.call(&args);
        }
        let out = pilot
            .command_with_door(DOOR)
            .env("COLUMNS", "120")
            .args(["ls"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let listing = String::from_utf8(out.stdout).unwrap();
        let row = listing
            .lines()
            .find(|line| line.contains("Original page"))
            .unwrap();
        assert!(row.contains("10000000-0"), "{listing}");
        assert!(
            listing.contains("http://127.0.0.1:53253/p/10000000-0"),
            "{listing}"
        );
        if action.is_none() {
            assert!(
                listing
                    .lines()
                    .any(|line| line.contains("10000000-1") && !line.contains("http")),
                "{listing}"
            );
        } else {
            // Prefixes are chosen from the full catalog, before ls hides archived/deleted rows.
            assert!(!listing.contains("10000000-1"), "{listing}");
        }
        let json = pilot.call(&["ls", "--json"]);
        assert_eq!(json["pages"][0]["pageId"], PAGE);
    }
}
/// Exercise each CLI adapter and sharing selection without opening stdin, seed files or an export.
fn prefix_commands<'a>(prefix: &'a str, file: &'a str, destination: &'a str) -> Vec<Vec<&'a str>> {
    vec![
        vec!["open", prefix, "--json"],
        vec!["show", prefix, "--json"],
        vec!["threads", prefix, "--json"],
        vec![
            "threads",
            "resolve",
            prefix,
            "40000000-0000-4000-8000-000000000001",
            "--json",
        ],
        vec![
            "threads",
            "reopen",
            prefix,
            "40000000-0000-4000-8000-000000000001",
            "--json",
        ],
        vec!["page", "read", prefix, "--json"],
        vec!["page", "write", prefix, "--file", file, "--json"],
        vec!["export", prefix, "--dir", destination, "--json"],
        vec!["retention", prefix, "42", "--json"],
        vec!["archive", prefix, "--json"],
        vec!["delete", prefix, "--yes", "--json"],
        vec!["share", "mode", prefix, "public", "--yes", "--json"],
        vec!["share", "history", prefix, "current", "--json"],
        vec!["share", "link", "ls", prefix, "--json"],
        vec![
            "share",
            "link",
            "add",
            prefix,
            "--seed-file",
            file,
            "--yes",
            "--json",
        ],
        vec![
            "share",
            "link",
            "reset",
            prefix,
            LINK,
            "--seed-file",
            file,
            "--yes",
            "--json",
        ],
        vec!["share", "link", "remove", prefix, LINK, "--json"],
        vec![
            "share",
            "member",
            "add",
            prefix,
            MEMBER,
            "viewer",
            "--sign-key",
            "invalid",
            "--enc-key",
            "invalid",
            "--yes",
            "--json",
        ],
        vec!["share", "member", "remove", prefix, MEMBER, "--json"],
        vec![
            "share", "member", "role", prefix, MEMBER, "editor", "--yes", "--json",
        ],
    ]
}
#[test]
fn page_prefix_refusals_name_candidates_and_leave_every_adapter_without_effects() {
    use tmt_colab::{keyring::Layout, store::Store};
    let pilot = Pilot::new(None);
    seed_page_with_title(&pilot, "Original title");
    let other = "10000000-1000-4000-8000-000000000002";
    let layout = Layout::open(&pilot.root.join("selected")).unwrap();
    let store = Store::open(&layout).unwrap();
    store.create_page(other).unwrap();
    store.close().unwrap();
    let database = layout.directory.join("space.db");
    let source = pilot.root.join("missing-input");
    let destination = pilot.root.join("not-exported");
    let source = source.to_str().unwrap();
    let dest = destination.to_str().unwrap();
    for action in [None, Some("archive"), Some("delete")] {
        if let Some(action) = action {
            let mut args = vec![action, other, "--json"];
            if action == "delete" {
                args.push("--yes");
            }
            pilot.call(&args);
        }
        let before = fs::read(&database).unwrap();
        for args in prefix_commands(&PAGE[..8], source, dest) {
            let result = failure(&pilot, &args, "COLAB_PAGE_AMBIGUOUS");
            assert_eq!(result["candidates"][0]["pageId"], PAGE);
            assert_eq!(result["candidates"][0]["shortId"], "10000000-0");
            assert_eq!(result["candidates"][0]["title"], "Original title");
            assert_eq!(result["candidates"][1]["pageId"], other);
            assert_eq!(result["candidates"][1]["shortId"], "10000000-1");
            assert_eq!(result["candidates"][1]["deleted"], action == Some("delete"));
            let message = result["error"]["message"].as_str().unwrap();
            assert!(
                message.starts_with("Page prefix 10000000 matches 2 pages: "),
                "{result}"
            );
            assert!(message.contains("10000000-0 (Original title)"), "{result}");
            if action == Some("delete") {
                assert_eq!(result["candidates"][1]["title"], "Deleted page");
            }
        }
        assert_eq!(fs::read(&database).unwrap(), before);
        assert!(!destination.exists());
        // A printed prefix remains resolvable even with a hidden sibling.
        assert_eq!(
            pilot.call(&["show", "10000000-0", "--json"])["page"]["pageId"],
            PAGE
        );
    }
    let before = fs::read(&database).unwrap();
    for (prefix, code) in [
        ("10000000-1", "COLAB_PAGE_DELETED"),
        (other, "COLAB_PAGE_DELETED"),
        ("ffffffff", "COLAB_PAGE_NOT_FOUND"),
        ("1000000", "COLAB_INPUT_INVALID"),
        ("10000000x", "COLAB_INPUT_INVALID"),
        ("100000000", "COLAB_INPUT_INVALID"),
        ("10000000-00Z", "COLAB_INPUT_INVALID"),
        ("../10000000", "COLAB_INPUT_INVALID"),
        (
            "00000000-0000-0000-0000-000000000000",
            "COLAB_INPUT_INVALID",
        ),
        (
            "10000000-0000-4000-8000-0000000000010",
            "COLAB_INPUT_INVALID",
        ),
    ] {
        for args in prefix_commands(prefix, source, dest) {
            let result = failure(&pilot, &args, code);
            if code == "COLAB_PAGE_DELETED" {
                assert_eq!(result["pageId"], other);
            }
        }
    }
    assert_eq!(fs::read(&database).unwrap(), before);
    assert!(!destination.exists());
}
#[test]
fn page_prefix_reads_writes_exports_and_confirmations_use_the_resolved_full_id() {
    let pilot = Pilot::new(None);
    seed_page_with_title(&pilot, "Original title");
    let prefix = &PAGE[..8];
    assert_eq!(
        pilot.call(&["show", prefix, "--json"])["page"]["pageId"],
        PAGE
    );
    assert_eq!(
        pilot.call(&["retention", prefix, "--json"])["page"]["pageId"],
        PAGE
    );
    assert_eq!(
        pilot.call(&["share", "link", "ls", prefix, "--json"])["pageId"],
        PAGE
    );
    let threads = pilot.call(&["threads", prefix, "--json"]);
    assert_eq!(threads["pageId"], PAGE);
    assert_eq!(threads["threads"], json!([]));
    let read = pilot.call(&["page", "read", prefix, "--json"]);
    assert_eq!(read["pageId"], PAGE);
    let source = pilot.root.join("edit.html");
    fs::write(&source, "<h1>Edited source</h1>").unwrap();
    let written = pilot.call(&[
        "page",
        "write",
        prefix,
        "--file",
        source.to_str().unwrap(),
        "--expected-revision",
        read["revision"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(written["pageId"], PAGE);
    assert_eq!(
        pilot.call(&["page", "read", prefix, "--json"])["source"],
        "<h1>Edited source</h1>"
    );
    let export = pilot.call(&[
        "export",
        prefix,
        "--dir",
        pilot.root.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(export["pageId"], PAGE);
    let directory = PathBuf::from(export["directory"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(directory.join("page.html")).unwrap(),
        "<h1>Edited source</h1>"
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["pageId"], PAGE);
    let database = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&database).unwrap();
    for (args, consequence, action) in [
        (
            vec!["delete", prefix],
            format!("Deleting page {PAGE} is permanent;"),
            "delete",
        ),
        (
            vec!["share", "mode", prefix, "public"],
            format!("Making page {PAGE} public lets anyone with the link read it,"),
            "make it public",
        ),
        (
            vec!["share", "link", "add", prefix],
            format!("Adding a read-only link to page {PAGE} lets anyone who has it read the page,"),
            "add the link",
        ),
    ] {
        let mut json_args = args.clone();
        json_args.push("--json");
        let result = confirmation_required(&pilot, &json_args, &consequence, action);
        assert_eq!(result["pageId"], PAGE);
        assert!(result["error"]["message"].as_str().unwrap().contains(PAGE));
        let human = pilot.command().args(args).output().unwrap();
        assert!(!human.status.success());
        assert!(String::from_utf8_lossy(&human.stderr).contains(PAGE));
    }
    assert_eq!(fs::read(&database).unwrap(), before);
    let mode = pilot.call(&["share", "mode", prefix, "link", "--yes", "--json"]);
    assert_eq!(mode["pageId"], PAGE);
    assert_eq!(
        pilot.call(&["show", PAGE, "--json"])["page"]["sharing"],
        "link"
    );
    let link = pilot.call(&["share", "link", "add", prefix, "--yes", "--json"]);
    assert_eq!(link["pageId"], PAGE);
    assert!(link["readerPath"].as_str().unwrap().contains(PAGE));
    let human = pilot.command().args(["archive", prefix]).output().unwrap();
    assert!(human.status.success(), "{human:?}");
    assert!(String::from_utf8_lossy(&human.stdout).contains(PAGE));
    assert_eq!(
        pilot.call(&["show", PAGE, "--json"])["page"]["archived"],
        true
    );
}
#[test]
fn page_prefix_help_is_shared_by_every_page_operand() {
    let pilot = Pilot::new(None);
    for args in prefix_commands(&PAGE[..8], "input", "destination") {
        let end = args.iter().position(|arg| *arg == &PAGE[..8]).unwrap();
        let help = pilot
            .command()
            .args(&args[..end])
            .arg("--help")
            .output()
            .unwrap();
        assert!(help.status.success(), "{help:?}");
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(
            help.contains("unique lowercase UUID prefix") && help.contains("at least 8 characters"),
            "{help}"
        );
    }
}
#[test]
fn unreadable_settings_never_fail_a_committed_page_create_or_a_ready_serve() {
    let pilot = Pilot::new(None);
    pilot.opener(0);
    // A directory where the file belongs: unreadable, and not something this command may fix.
    let colab = pilot.root.join("selected/colab");
    fs::create_dir_all(&colab).unwrap();
    fs::set_permissions(&colab, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(colab.join("settings.json")).unwrap();
    let out = pilot
        .command()
        .env(
            "TMT_EXECUTABLE",
            pilot.creation_core(Some(CREATION_DOOR), None),
        )
        .args(["page", "create", "--title", "Kept", "--open"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("PAGE CREATED") && text.contains("Kept"),
        "{text}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("settings.json could not be read"),
        "{stderr}"
    );
    // The page exists exactly once: a failure after the commit would have invited a duplicate.
    assert_eq!(
        pilot.call(&["ls", "--json"])["pages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    pilot.assert_creation_acquisitions(1);
    // Serve is just as unaffected, and `--open` still works with the defaults.
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--open"],
    );
    let warning = Serving::wait_for(&serving.err, "settings.json could not be read");
    assert!(warning.contains("warning:"), "{warning}");
    Serving::wait_for(&serving.out, "opened in your browser");
    assert!(serving.running());
    serving.stop(Signal::SIGTERM);
}
#[test]
fn reading_settings_waits_for_a_setter_instead_of_seeing_a_half_written_file() {
    let pilot = Pilot::new(None);
    pilot.call(&["settings", "open", "off", "--json"]);
    // Hold the setter's lock as a running setter would; a reader must wait for it.
    let held = fs::File::open(pilot.root.join("selected/colab/settings.lock")).unwrap();
    let lock = nix::fcntl::Flock::lock(held, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let mut reader = pilot
        .command()
        .args(["settings", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(400));
    assert!(
        reader.try_wait().unwrap().is_none(),
        "the reader did not wait for the lock"
    );
    drop(lock);
    let out = reader.wait_with_output().unwrap();
    assert!(out.status.success());
    let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(answer, json!({"open":false,"source":"settings.json"}));
}
#[test]
fn setting_waits_for_a_reader_that_holds_the_lock() {
    let pilot = Pilot::new(None);
    pilot.call(&["settings", "open", "off", "--json"]);
    let held = fs::File::open(pilot.root.join("selected/colab/settings.lock")).unwrap();
    let lock = nix::fcntl::Flock::lock(held, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let mut setter = pilot
        .command()
        .args(["settings", "open", "on", "--json"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(400));
    assert!(
        setter.try_wait().unwrap().is_none(),
        "the setter did not wait for the lock"
    );
    drop(lock);
    let out = setter.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        json!({"open":true,"source":"settings.json"})
    );
}
const OUTDATED_ENVELOPE: &str = r#"!{"error":{"code":"REMOTE_SERVE_OUTDATED","message":"The running serve is older; stop it by hand."}}"#;
const OTHER_ENVELOPE: &str =
    r#"!{"error":{"code":"REMOTE_CORE_UNAVAILABLE","message":"Core did not answer."}}"#;
#[test]
fn an_error_envelope_from_remote_is_shown_as_it_is_and_never_starts_a_second_door() {
    for (envelope, shown) in [
        (
            OUTDATED_ENVELOPE,
            "The running Remote serve is older than this Colab. Stop it with Ctrl-C in its terminal, then run tmt colab serve.",
        ),
        (OTHER_ENVELOPE, "Core did not answer."),
    ] {
        let pilot = Pilot::new(None);
        // `Serve::Hold` would answer with a door: reaching it would turn the refusal into a start.
        let mut serving = Serving::start(
            &pilot,
            pilot.remote_core(Some(envelope), Serve::Hold),
            &["--json"],
        );
        let ready = serving.ready();
        assert_eq!(ready["door"], "unavailable", "{ready}");
        assert_eq!(ready["warning"], shown);
        assert!(
            !ready["warning"]
                .as_str()
                .unwrap()
                .contains("extension install")
        );
        assert!(serving.running());
        serving.stop(Signal::SIGTERM);
        assert!(
            !pilot.root.join("serve.calls").exists(),
            "serve was started"
        );
        assert!(!pilot.root.join("serve.pid").exists());
    }
}
#[test]
fn page_commands_say_why_there_is_no_link_when_remote_answers_with_an_error() {
    let pilot = Pilot::new(None);
    let created = pilot.call(&["page", "create", "--title", "Notes", "--json"]);
    let page = created["pageId"].as_str().unwrap();
    let human = |envelope: &str, args: &[&str]| {
        let out = pilot
            .command()
            .env(
                "TMT_EXECUTABLE",
                pilot.remote_core(Some(envelope), Serve::Fail),
            )
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    for args in [vec!["show", page], vec!["ls"]] {
        let text = human(OUTDATED_ENVELOPE, &args);
        assert!(text.contains("older than this Colab"), "{args:?}: {text}");
        assert!(
            text.contains("Stop it with Ctrl-C in its terminal"),
            "{text}"
        );
        assert!(
            !text.contains("extension install") && !text.contains("tmt remote serve"),
            "{text}"
        );
        let text = human(OTHER_ENVELOPE, &args);
        assert!(text.contains("Core did not answer."), "{args:?}: {text}");
    }
    pilot.opener(0);
    let mut acquisitions = 0;
    for envelope in [OUTDATED_ENVELOPE, OTHER_ENVELOPE] {
        let core = pilot.creation_core(Some(envelope), None);
        let out = pilot
            .command()
            .env("TMT_EXECUTABLE", &core)
            .args(["page", "create", "--title", "Refused link", "--open"])
            .output()
            .unwrap();
        acquisitions += 1;
        pilot.assert_creation_acquisitions(acquisitions);
        assert!(out.status.success(), "{out:?}");
        assert!(out.stderr.is_empty());
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("PAGE CREATED"), "{text}");
        assert!(text.contains("/p/"), "{text}");
        assert!(
            text.contains("Remote link unavailable from the creation snapshot"),
            "{text}"
        );
        for forbidden in [
            "older than",
            "outdated",
            "Stop",
            "stop",
            "restart",
            "install",
            "upgrade",
            "pair",
            "tmt remote serve",
            "tmt colab serve",
        ] {
            assert!(!text.contains(forbidden), "{text}");
        }
        let out = pilot
            .command()
            .env("TMT_EXECUTABLE", &core)
            .args([
                "page",
                "create",
                "--title",
                "Refused JSON link",
                "--open",
                "--json",
            ])
            .output()
            .unwrap();
        acquisitions += 1;
        pilot.assert_creation_acquisitions(acquisitions);
        assert!(out.status.success(), "{out:?}");
        assert!(out.stderr.is_empty());
        let created: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(created["link"].is_null());
        assert!(created["shortLink"].is_null());
        assert!(created["paired"].is_null());
        assert_eq!(created["next"], json!([]));
        let read = pilot.call(&[
            "page",
            "read",
            created["pageId"].as_str().unwrap(),
            "--json",
        ]);
        assert_eq!(read["source"], "");
        assert!(read.get("creationRecipient").is_none());
    }
    pilot.assert_creation_acquisitions(4);
    assert!(
        pilot
            .creation_calls()
            .iter()
            .all(|call| call == "remote status --machine --json")
    );
    assert!(pilot.opened().is_empty());
    assert!(!pilot.root.join("serve.calls").exists());
    assert!(!pilot.root.join("serve.pid").exists());
    let pages = pilot.call(&["ls", "--json"]);
    assert_eq!(pages["pages"].as_array().unwrap().len(), 5);
    for page in pages["pages"].as_array().unwrap() {
        let read = pilot.call(&["page", "read", page["pageId"].as_str().unwrap(), "--json"]);
        assert_eq!(read["source"], "");
        assert!(read.get("creationRecipient").is_none());
    }
}
#[test]
fn serve_says_the_outdated_instruction_once_in_the_warning_and_points_at_it_in_the_row() {
    let pilot = Pilot::new(None);
    pilot.call(&["page", "create", "--title", "Kept", "--json"]);
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(OUTDATED_ENVELOPE), Serve::Fail),
        &[],
    );
    let warning = Serving::wait_for(&serving.err, "older than this Colab");
    let text = Serving::wait_for(&serving.out, "open");
    assert!(
        text.contains("(Remote serve is outdated; see warning)"),
        "{text}"
    );
    assert!(!text.contains("older than this Colab"), "{text}");
    assert!(
        warning.contains("Stop it with Ctrl-C in its terminal"),
        "{warning}"
    );
    serving.stop(Signal::SIGTERM);
}

#[test]
fn skill_is_exact_embedded_bytes_after_relocation_without_core_or_state() {
    let pilot = Pilot::new(None);
    let relocated = pilot.root.join("relocated-colab");
    let binary = fs::read(BINARY).unwrap();
    let mode = fs::metadata(BINARY).unwrap().permissions().mode() & 0o777;
    tmt_test_support::write_executable(&relocated, &binary, mode).unwrap();
    assert_eq!(fs::read(&relocated).unwrap(), binary);
    assert_eq!(
        fs::metadata(&relocated).unwrap().permissions().mode() & 0o777,
        mode
    );
    fs::remove_file(pilot.root.join("core")).unwrap();
    let invoke = |args: &[&str]| {
        Command::new(&relocated)
            .env_clear()
            .env("HOME", &pilot.root)
            .env("XDG_CONFIG_HOME", pilot.root.join("config"))
            .env("XDG_DATA_HOME", pilot.root.join("data"))
            .env("XDG_STATE_HOME", pilot.root.join("state"))
            .env("XDG_CACHE_HOME", pilot.root.join("cache"))
            .env("TMT_EXECUTABLE", pilot.root.join("missing-core"))
            .env("PATH", "")
            .current_dir(&pilot.root)
            .args(args)
            .output()
            .unwrap()
    };
    let output = invoke(&["skill"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        include_bytes!("../../../skills/tmt-colab/SKILL.md")
    );
    let skill = std::str::from_utf8(&output.stdout).unwrap();
    let page_look = skill.split_once("## Page look\n").unwrap().1;
    let (page_look, _) = page_look.split_once("## HTML that renders\n").unwrap();
    assert!(page_look.contains("Colab already shows the brand, page title and actions."));
    assert!(page_look.contains("<style>"));
    assert!(page_look.contains("@media (prefers-color-scheme: dark)"));
    assert!(output.stdout.len() <= 12_000);
    assert!(skill.lines().count() <= 500);
    for args in [["skill", "--help"], ["help", "skill"]] {
        let help = invoke(&args);
        assert!(help.status.success(), "{help:?}");
        assert!(
            String::from_utf8(help.stdout)
                .unwrap()
                .contains("tmt colab skill")
        );
        assert!(help.stderr.is_empty());
    }
    let unsupported = invoke(&["skill", "--json"]);
    assert!(!unsupported.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&unsupported.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    // Only the copied executable exists: pure guidance neither discovers core nor initializes state.
    assert_eq!(fs::read_dir(&pilot.root).unwrap().count(), 1);
}

// --- Background serve (#2383): the launcher starts a detached worker and returns once it is ready.
/// Stops a background serve a test started, even when an assertion failed first.
struct Detached<'a>(&'a Pilot);
impl Drop for Detached<'_> {
    fn drop(&mut self) {
        let _ = self.0.command().arg("stop").output();
    }
}
/// One launcher invocation: `serve` with the arguments given, its output captured.
fn launcher(pilot: &Pilot, core: &PathBuf, args: &[&str]) -> Command {
    let mut command = pilot.command();
    command
        .env("TMT_EXECUTABLE", core)
        .arg("serve")
        .args(args)
        .stdin(Stdio::null());
    command
}
fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}
fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}
#[test]
fn background_serve_returns_when_ready_and_stop_ends_what_it_started() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    let output = launcher(&pilot, &core, &["--no-open"]).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = squashed(&stdout_of(&output));
    for wanted in [
        "Colab is running in the background",
        "LOCAL SPACE",
        "(ready)",
        "started · http://127.0.0.1:53253",
        "tmt colab stop",
        "tmt colab serve --foreground",
    ] {
        assert!(text.contains(wanted), "{wanted:?} missing from {text}");
    }
    assert!(!text.contains("Ctrl-C"), "{text}");
    // The door the serve started runs, owned by the detached worker, after the launcher exited.
    let door = pilot.serve_pid().unwrap();
    assert_eq!(kill(door, None), Ok(()));
    let stopped = pilot.command().arg("stop").output().unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(
        stdout_of(&stopped).contains("Colab stopped, and the Remote door it started"),
        "{stopped:?}"
    );
    assert_gone(door);
    let again = pilot.command().args(["stop", "--json"]).output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&again.stdout).unwrap()["state"],
        "not-running"
    );
}
#[test]
fn background_serve_outlives_the_terminal_that_started_it() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    // The launcher leads its own process group, like a shell job; the worker must not stay in it.
    let mut command = launcher(&pilot, &core, &["--no-open"]);
    let mut launcher_process = command
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert!(launcher_process.wait().unwrap().success());
    // A closing terminal hangs up the whole group.
    let _ = killpg(Pid::from_raw(launcher_process.id() as i32), Signal::SIGHUP);
    thread::sleep(Duration::from_millis(300));
    let door = pilot.serve_pid().unwrap();
    assert_eq!(kill(door, None), Ok(()), "the hang-up reached the door");
    let stopped = pilot.command().arg("stop").output().unwrap();
    assert!(
        stdout_of(&stopped).contains("Colab stopped, and the Remote door it started"),
        "the serve did not survive the terminal: {stopped:?}"
    );
    assert_gone(door);
}
#[test]
fn background_json_prints_one_typed_result_and_bare_json_stays_foreground() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    let output = launcher(&pilot, &core, &["--background", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{}", stderr_of(&output));
    let lines: Vec<_> = output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "{}", stdout_of(&output));
    let result: Value = serde_json::from_slice(lines[0]).unwrap();
    assert_eq!(result["background"], true);
    assert_eq!(result["state"], "mounted");
    assert_eq!(result["door"], "started");
    assert_eq!(result["opened"], false);
    assert!(result["shortLink"].is_null() || result["shortLink"].is_string());
    let stopped = pilot.command().args(["stop", "--json"]).output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap()["state"],
        "stopped"
    );
    // Bare --json is the foreground serve: it stays attached until signalled, with no `background` key.
    let mut serving = Serving::start(
        &pilot,
        pilot.remote_core(Some(DOOR), Serve::Fail),
        &["--json"],
    );
    assert!(serving.ready().get("background").is_none());
    assert!(serving.running());
    serving.stop(Signal::SIGTERM);
}
#[test]
fn a_second_start_reports_the_running_serve_and_starts_nothing() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    let first = launcher(&pilot, &core, &["--no-open"]).output().unwrap();
    assert!(first.status.success(), "{first:?}");
    let door = pilot.serve_pid().unwrap();
    // A person who reruns it gets the running page, not an error, and nothing starts. The door now
    // answers that it runs, as the one the first serve started would.
    let running = pilot.remote_core(Some(DOOR), Serve::Fail);
    let human = launcher(&pilot, &running, &["--no-open"]).output().unwrap();
    assert!(human.status.success(), "{human:?}");
    let text = squashed(&stdout_of(&human));
    assert!(text.contains("Colab is already running"), "{text}");
    assert!(text.contains("http://127.0.0.1:53253"), "{text}");
    assert!(text.contains("tmt colab stop"), "{text}");
    assert!(!text.contains("background"), "{text}");
    assert!(human.stderr.is_empty(), "{}", stderr_of(&human));
    // The same running Colab shows the same link on first start and on rerun.
    let open_row = |text: &str| {
        text.lines()
            .find(|line| line.starts_with("open "))
            .unwrap_or_else(|| panic!("no open row in {text}"))
            .to_owned()
    };
    assert_eq!(open_row(&squashed(&stdout_of(&first))), open_row(&text));
    let opener = pilot.root.join("opened");
    pilot.opener(0);
    let opens = launcher(&pilot, &running, &["--open"]).output().unwrap();
    assert!(opens.status.success(), "{opens:?}");
    assert!(
        fs::read_to_string(&opener)
            .unwrap_or_default()
            .contains("http://127.0.0.1:53253"),
        "the rerun did not open the page under the usual settings"
    );
    let json = launcher(&pilot, &running, &["--background", "--json"])
        .output()
        .unwrap();
    assert!(!json.status.success());
    let error: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_ALREADY_SERVING");
    // The first serve is untouched.
    assert_eq!(kill(door, None), Ok(()));
    assert_eq!(pilot.serve_pid().unwrap(), door);
}
#[test]
fn concurrent_starts_leave_exactly_one_serve() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    let starts: Vec<_> = (0..2)
        .map(|_| {
            let mut command = launcher(&pilot, &core, &["--no-open", "--background", "--json"]);
            thread::spawn(move || command.output().unwrap())
        })
        .collect();
    let outputs: Vec<_> = starts.into_iter().map(|s| s.join().unwrap()).collect();
    let started = outputs.iter().filter(|o| o.status.success()).count();
    assert_eq!(started, 1, "{outputs:?}");
    let refused = outputs.iter().find(|o| !o.status.success()).unwrap();
    let error: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_ALREADY_SERVING");
    let door = pilot.serve_pid().unwrap();
    let stopped = pilot.command().arg("stop").output().unwrap();
    assert!(stdout_of(&stopped).contains("Colab stopped"), "{stopped:?}");
    assert_gone(door);
}
#[test]
fn a_failed_background_start_states_the_cause_and_the_next_step() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    let colab = pilot.root.join("selected").join("colab");
    fs::create_dir_all(&colab).unwrap();
    fs::set_permissions(&colab, fs::Permissions::from_mode(0o755)).unwrap();
    let core = pilot.remote_core(Some(STOPPED), Serve::Hold);
    let human = launcher(&pilot, &core, &["--no-open"]).output().unwrap();
    assert!(!human.status.success());
    let text = stderr_of(&human);
    assert!(text.contains("0700 directory"), "{text}");
    assert!(text.contains("tmt colab serve --foreground"), "{text}");
    assert!(human.stdout.is_empty(), "{}", stdout_of(&human));
    let json = launcher(&pilot, &core, &["--background", "--json"])
        .output()
        .unwrap();
    let error: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(error["error"]["code"], "COLAB_STATE_UNSAFE");
    assert!(
        pilot.serve_pid().is_none(),
        "no door started for a refused state"
    );
}
#[test]
fn cancelling_a_launcher_before_the_handoff_leaves_nothing_running() {
    let pilot = Pilot::new(None);
    let _stop = Detached(&pilot);
    // The door never prints its descriptor, so the worker is still starting when the launcher is
    // told to stop.
    let core = pilot.remote_core(Some(STOPPED), Serve::Silent);
    let out = pilot.root.join("launcher.out");
    let err = pilot.root.join("launcher.err");
    let mut child = launcher(&pilot, &core, &["--no-open"])
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(&err).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while pilot.serve_pid().is_none() {
        assert!(Instant::now() < deadline, "the door never started");
        thread::sleep(Duration::from_millis(20));
    }
    let door = pilot.serve_pid().unwrap();
    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the launcher did not finish");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    let text = fs::read_to_string(&err).unwrap();
    assert!(text.contains("cancelled"), "{text}");
    assert!(fs::read_to_string(&out).unwrap().is_empty());
    // The launcher returns only after the worker confirmed its cleanup.
    assert_gone(door);
    let stopped = pilot.command().args(["stop", "--json"]).output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&stopped.stdout).unwrap()["state"],
        "not-running"
    );
}
#[test]
fn foreground_serve_says_how_to_run_in_the_background_next_time() {
    let pilot = Pilot::new(None);
    let mut serving = Serving::start(&pilot, pilot.remote_core(Some(DOOR), Serve::Fail), &[]);
    let text = squashed(&Serving::wait_for(&serving.out, "next time"));
    assert!(text.contains("Ctrl-C"), "{text}");
    assert!(text.contains("tmt colab serve"), "{text}");
    assert!(!text.contains("running in the background"), "{text}");
    serving.stop(Signal::SIGTERM);
}
