mod support;
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
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
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("This colab space is private"));
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
        meta.insert(&mut tx, "title", "Encrypted π\u{1b}[31m");
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
    assert_eq!(show["page"], list["pages"][0]);
    assert_eq!(show["members"][0]["id"], MEMBER);
    assert_eq!(show["page"]["warnings"], json!(["expiry-unavailable"]));
    assert_eq!(show["page"]["lastUpdateAtMs"], Value::Null);
    assert_eq!(show["page"]["expiresAtMs"], Value::Null);
    assert_eq!(show["discussions"], "not-available");
    let human = pilot.command().args(["ls"]).output().unwrap();
    assert!(human.status.success());
    assert!(!human.stdout.contains(&0x1b));
    assert!(String::from_utf8_lossy(&human.stdout).contains("Expiry times are not available yet"));
    assert_eq!(fs::read(db).unwrap(), before);
}
#[test]
fn management_confirmation_and_input_denials_have_no_state_effects() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let db = pilot.root.join("selected/colab/space.db");
    let before = fs::read(&db).unwrap();
    for args in [
        vec!["share", "mode", PAGE, "link", "--json"],
        vec!["share", "mode", PAGE, "public", "--json"],
    ] {
        failure(&pilot, &args, "COLAB_CONFIRMATION_REQUIRED");
    }
    failure(
        &pilot,
        &["show", "not-a-page", "--json"],
        "COLAB_INPUT_INVALID",
    );
    for args in [
        vec!["retention", PAGE, "--json"],
        vec!["archive", PAGE, "--json"],
        vec!["delete", PAGE, "--yes", "--json"],
        vec!["share", "members", "list", PAGE, "--json"],
        vec!["share", "history", PAGE, "shared", "--json"],
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
        assert_eq!(route, "x/colab/read");
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
    assert!(String::from_utf8_lossy(&human.stdout).contains("x/colab/read#v=1&"));
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
                "x/colab/read#v=1&space={space}&page={PAGE}&link={LINK}&rev=4&st={}&seed={}",
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
        failure(&pilot, &reset, "COLAB_CONFIRMATION_REQUIRED");
        assert_eq!(fs::read(&db).unwrap(), committed);
        let mut confirmed = reset.to_vec();
        confirmed.push("--yes");
        let outcome = pilot.call(&confirmed);
        assert_eq!(outcome["linkId"], replacement);
        assert_eq!(
            outcome["readerPath"],
            format!(
                "x/colab/read#v=1&space={space}&page={PAGE}&link={replacement}&rev={}&st={}&seed={}",
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
fn management_read_refuses_unsafe_or_old_state_without_migration() {
    let pilot = Pilot::new(None);
    seed_page(&pilot);
    let directory = pilot.root.join("selected/colab");
    let db = directory.join("space.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    failure(&pilot, &["ls", "--json"], "COLAB_SCHEMA_UNSUPPORTED");
    assert_eq!(fs::read(&db).unwrap(), before);
    let key = directory.join("owner.key");
    fs::remove_file(&key).unwrap();
    std::os::unix::fs::symlink("missing", &key).unwrap();
    let output = pilot.command().args(["ls", "--json"]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(db).unwrap(), before);
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
        assert_eq!(
            created["path"],
            format!(
                "x/colab/#space={}&path=%2Fpages%2F{id}",
                created["spaceId"].as_str().unwrap()
            )
        );
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
    assert!(message.contains("Open x/colab/#space="));
    assert!(message.contains("under your Remote door address (the one tmt remote pair printed)."));
}
