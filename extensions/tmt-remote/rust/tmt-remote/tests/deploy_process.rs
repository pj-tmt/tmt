//! Shipped argv through disposable public-dispatch fixtures; no account/provider access.
use std::{fs, process::Command};
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[path = "support/executable_fixture.rs"]
mod executable_fixture;
use deploy_fixture::Root;
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-remote");
// Optional delivery export; ordinary test assertions always run and no production input uses it.
fn export_human(name: &str, output: &std::process::Output) {
    if let Some(directory) = std::env::var_os("TMT_REMOTE_GOLDENS") {
        let directory = std::path::PathBuf::from(directory);
        assert!(directory.is_absolute());
        fs::create_dir_all(&directory).unwrap();
        let mut bytes = output.stdout.clone();
        bytes.extend_from_slice(&output.stderr);
        fs::write(directory.join(format!("{name}.txt")), bytes).unwrap();
    }
}
fn base(root: &Root) -> Command {
    let mut command = Command::new(BINARY);
    command
        .env_clear()
        .env("HOME", &root.0)
        .env("PATH", "/usr/bin:/bin")
        .env("TMT_EXECUTABLE", root.0.join("tmt"))
        .env("XDG_CONFIG_HOME", root.0.join("config"))
        .env("XDG_DATA_HOME", root.0.join("data"))
        .current_dir(&root.0);
    command
}
const ARGS: [&str; 8] = [
    "deploy",
    "firestore",
    "--project",
    "demo-remote-1",
    "--region",
    "asia-east1",
    "--sign-in",
    "anonymous",
];
#[test]
fn help_and_invalid_authorization_do_not_invoke_core_or_provider() {
    let root = Root::new();
    for words in [
        vec!["deploy", "firestore", "--help"],
        [
            ARGS.as_slice(),
            &[
                "--replace-rules",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--json",
            ],
        ]
        .concat(),
        [
            ARGS.as_slice(),
            &[
                "--authorize",
                "aaaaaaaaaaaa",
                "--replace-rules",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--json",
            ],
        ]
        .concat(),
        [ARGS.as_slice(), &["--authorize", "abc", "--json"]].concat(),
    ] {
        let out = base(&root).args(&words).output().unwrap();
        if words.contains(&"--help") {
            assert!(out.status.success());
            assert!(
                String::from_utf8(out.stdout)
                    .unwrap()
                    .contains("signing in alone never deploys")
            );
        } else if words.contains(&"--replace-rules") {
            assert!(!out.status.success());
            assert!(out.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("unexpected argument '--replace-rules'")
            );
        } else {
            assert!(!out.status.success());
            let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["error"]["code"], "USAGE_ERROR");
        }
    }
    assert!(!root.remote().exists());
}
#[test]
fn old_installed_colab_is_truthfully_unavailable_without_tool_or_storage_calls() {
    let root = Root::new();
    let layout = root.layout();
    let mut record =
        tmt_remote::deploy_record::DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    record
        .persist(&tmt_remote::deploy_run::DeployRecord::new(
            deploy_fixture::ID,
        ))
        .unwrap();
    drop(record);
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    executable_fixture::write_executable(&root.0.join("tmt"),&format!("[ \"$PWD\" = / ] || exit 4\nprintf '%s\\n' \"$*\" >> '{}'\n[ \"$1 $2 $3\" = 'colab deploy-declaration --json' ] || exit 5\nprintf '%s' '{{\"error\":{{\"code\":\"COLAB_INPUT_INVALID\"}}}}'; exit 1",root.0.join("calls").display())).unwrap();
    let out = base(&root).args(ARGS).arg("--json").output().unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert!(out.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["available"], false);
    assert_eq!(value["reason"], "no-declaration");
    assert_eq!(value["extensions"]["unavailable"][0]["name"], "colab");
    assert_eq!(
        fs::read_to_string(root.0.join("calls")).unwrap(),
        "colab deploy-declaration --json\n"
    );
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
    let human = base(&root).args(ARGS).output().unwrap();
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8(human.stdout).unwrap(),
        "Firestore sharing is unavailable: no enabled extension uses Firestore.\nNothing changed in your Firebase project.\n"
    );
}

fn fixture(path: &str) -> Vec<u8> {
    fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path),
    )
    .unwrap()
}
fn available(root: &Root) {
    use sha2::{Digest, Sha256};
    let declaration = String::from_utf8(fixture("rules/colab.json")).unwrap();
    let digest: String = Sha256::digest(declaration.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let reply = serde_json::json!({"version":1,"extension":"colab","backend":"firestore","declaration":declaration,"artifact":String::from_utf8(fixture("rules/colab.rules")).unwrap(),"declarationDigest":digest});
    fs::write(
        root.0.join("reply.json"),
        serde_json::to_vec(&reply).unwrap(),
    )
    .unwrap();
    executable_fixture::write_executable(&root.0.join("tmt"), &format!(
        "case \"$*\" in
'colab deploy-declaration --json') [ \"$PWD\" = / ] || exit 4; /bin/cat '{}';;
api) input=$(/bin/cat); case \"$input\" in *storage.root*) printf '%s' '{{\"dataRoot\":\"{}\"}}';; *) exit 5;; esac;;
*) exit 6;;
esac", root.0.join("reply.json").display(), root.0.display()
    )).unwrap();
}
fn installed(root: &Root, mode: &str) -> std::path::PathBuf {
    use std::os::unix::fs::symlink;
    let package = root.0.join("firebase-tools");
    fs::create_dir_all(package.join("lib/bin")).unwrap();
    for file in [
        "package.json",
        "lib/auth.js",
        "lib/logger.js",
        "lib/api.js",
        "lib/apiv2.js",
        "lib/configstore.js",
    ] {
        fs::write(
            package.join(file),
            fixture(&format!("deploy_firestore/firebase-tools/{file}")),
        )
        .unwrap();
    }
    fs::write(package.join("state.json"), serde_json::to_vec(&serde_json::json!({"account":"owner@example.test","project":"demo-remote-1","location":"asia-east1","mode":mode,"canary":"TOKEN_CANARY_DO_NOT_DISCLOSE","source":""})).unwrap()).unwrap();
    // The installed launcher is inspected, never invoked; its approved shebang selects Node.
    fs::write(
        package.join("lib/bin/firebase.js"),
        "#!/usr/bin/env node\nthrow new Error('must not launch firebase');\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        package.join("lib/bin/firebase.js"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let bin = root.0.join("bin");
    fs::create_dir(&bin).unwrap();
    symlink(package.join("lib/bin/firebase.js"), bin.join("firebase")).unwrap();
    let node = tmt_invoke::find_executable(
        std::ffi::OsStr::new("node"),
        &std::env::var_os("PATH").unwrap(),
    )
    .expect("installed Node for process fixtures");
    symlink(node, bin.join("node")).unwrap();
    package
}
fn invoke(root: &Root, extra: &[&str]) -> std::process::Output {
    base(root)
        .env(
            "PATH",
            std::env::join_paths([root.0.join("bin"), "/usr/bin".into(), "/bin".into()]).unwrap(),
        )
        .args(ARGS)
        .args(extra)
        .output()
        .unwrap()
}
fn json_output(output: &std::process::Output) -> serde_json::Value {
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty(), "{:?}", output);
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn available_binary_plan_is_read_only_and_authorization_completes_the_saved_original() {
    let root = Root::new();
    available(&root);
    let package = installed(&root, "process");
    let mut writer =
        tmt_remote::deploy_record::DeployRecordStore::open(&root.layout(), "demo-remote-1")
            .unwrap();
    writer
        .persist(&tmt_remote::deploy_run::DeployRecord::new(
            deploy_fixture::ID,
        ))
        .unwrap();
    drop(writer);
    let preview = json_output(&invoke(&root, &["--json"]));
    assert_eq!(preview["authorized"], false);
    let draft = tmt_remote::deploy_record::read(&root.layout())
        .unwrap()
        .unwrap();
    assert_eq!(
        draft.deployment_id,
        preview["record"]["deploymentId"].as_str().unwrap()
    );
    assert!(preview["record"]["run"].is_null());
    assert!(preview["record"]["binding"].is_null());
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let draft_document: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(draft_document["version"], 1);
    assert!(draft_document.get("target").is_none());
    let calls = fs::read_to_string(package.join("calls.jsonl")).unwrap();
    assert!(
        calls.lines().all(
            |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == "GET"
        )
    );
    let human = invoke(&root, &[]);
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    let human = String::from_utf8(human.stdout).unwrap();
    let digest = preview["planDigest"].as_str().unwrap();
    assert!(human.contains(&format!("Plan digest: {}", &digest[..12])));
    assert!(human.contains(&format!(
        "To deploy this plan, run the same command with --authorize {}",
        &digest[..12]
    )));
    export_human("plan", &invoke(&root, &[]));
    assert!(human.contains("object 256 KiB"));
    assert!(human.contains("TTL: not set up"));
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
    let done = json_output(&invoke(&root, &["--authorize", &digest[..12], "--json"]));
    assert_eq!(
        done["record"]["deploymentId"],
        preview["record"]["deploymentId"]
    );
    assert_eq!(done["record"]["run"]["state"], "complete");
    assert!(done["record"]["binding"].is_object());
    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["record"], done["record"]);
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let calls = fs::read_to_string(package.join("calls.jsonl")).unwrap();
    // Even with no installed Firebase tool on PATH, a retained target conflict is
    // the refusal: no login, inventory or provider process is reached.
    {
        let (project, region) = ("demo-remote-1", "us-central1");
        let mut words = ARGS.to_vec();
        words[3] = project;
        words[5] = region;
        let out = base(&root).args(words).arg("--json").output().unwrap();
        assert!(!out.status.success());
        assert!(out.stderr.is_empty());
        let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(reply["error"]["code"], "REMOTE_DEPLOY_PROJECT_CONFLICT");
        let message = reply["error"]["message"].as_str().unwrap();
        assert_eq!(
            message,
            "Project demo-remote-1 uses region asia-east1, not us-central1.: Run tmt remote deploy firestore --project demo-remote-1 --region asia-east1 --sign-in anonymous"
        );
        assert_eq!(
            fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(package.join("calls.jsonl")).unwrap(),
            calls
        );
    }
    let mut another = ARGS.to_vec();
    another[3] = "demo-remote-2";
    let out = base(&root).args(another).arg("--json").output().unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply["error"]["code"], "REMOTE_DEPLOY_TOOL_MISSING");
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(package.join("calls.jsonl")).unwrap(),
        calls
    );
    let methods: Vec<_> = fs::read_to_string(package.join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        methods.iter().filter(|m| *m != "GET").count(),
        2,
        "only Rules create and release switch mutate"
    );
}
#[test]
fn available_binary_reports_missing_tool_and_missing_login_without_creating_a_plan_identity() {
    for (mode, code, message) in [
        (
            "no-tool",
            "REMOTE_DEPLOY_TOOL_MISSING",
            "Firebase CLI is not installed. Install it with: npm install -g firebase-tools@15.29.0",
        ),
        (
            "missing",
            "REMOTE_DEPLOY_SETUP_UNAVAILABLE",
            "Firebase is not signed in. Run firebase login with your own account, then try again.",
        ),
    ] {
        let root = Root::new();
        available(&root);
        if mode != "no-tool" {
            installed(&root, mode);
        }
        let out = invoke(&root, &["--json"]);
        assert!(!out.status.success());
        let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(reply["error"]["code"], code);
        assert_eq!(reply["error"]["message"], message);
        assert!(out.stderr.is_empty());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("TOKEN_CANARY"));
        // The target preflight takes the local writer lock before tool/login setup;
        // no valid plan exists yet, so neither identity nor target is published.
        assert!(
            !root
                .remote()
                .join("deployments/demo-remote-1/deploy.json")
                .exists()
        );
        assert!(
            tmt_remote::deploy_record::read(&root.layout())
                .unwrap()
                .is_none()
        );
        assert!(!root.0.join("firebase-tools/calls.jsonl").exists());
    }
}

fn declare_hosting(root: &Root, html: &[u8]) {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    available(root);
    let mut reply: Value =
        serde_json::from_slice(&fs::read(root.0.join("reply.json")).unwrap()).unwrap();
    let mut declaration: Value =
        serde_json::from_str(reply["declaration"].as_str().unwrap()).unwrap();
    let hash = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let manifest=tmt_remote::hosting::HostingManifest::parse(&json!({"version":1,"files":[{"path":"/index.html","sha256":hash(html),"length":html.len(),"contentType":"text/html"}]})).unwrap();
    declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
    let bytes = serde_json::to_string(&declaration).unwrap();
    reply["declarationDigest"] = json!(hash(bytes.as_bytes()));
    reply["declaration"] = json!(bytes);
    fs::write(
        root.0.join("reply.json"),
        serde_json::to_vec(&reply).unwrap(),
    )
    .unwrap();
    fs::write(root.0.join("bundle.json"),serde_json::to_vec(&json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(html)}]})).unwrap()).unwrap();
    executable_fixture::write_executable(&root.0.join("tmt"),&format!("case \"$*\" in
'colab deploy-declaration --json') [ \"$PWD\" = / ] || exit 4; /bin/cat '{}';;
'colab hosting-bundle --json') [ \"$PWD\" = / ] || exit 4; /bin/cat '{}';;
api) input=$(/bin/cat); case \"$input\" in *storage.root*) printf '%s' '{{\"dataRoot\":\"{}\"}}';; *) exit 5;; esac;;
*) exit 6;; esac",root.0.join("reply.json").display(),root.0.join("bundle.json").display(),root.0.display())).unwrap();
}
fn hosted_fixture(root: &Root, html: &[u8]) -> std::path::PathBuf {
    use serde_json::{Value, json};
    declare_hosting(root, html);
    let package = installed(root, "process");
    let mut state: Value =
        serde_json::from_slice(&fs::read(package.join("state.json")).unwrap()).unwrap();
    state["hosting"] = json!({"apps":[],"site":null,"version":null,"files":[],"live":null});
    fs::write(
        package.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    package
}

#[test]
fn completed_hosting_is_own_on_identical_and_changed_bundle_replans() {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    let root = Root::new();
    let package = hosted_fixture(&root, b"hello");
    let planned = json_output(&invoke(&root, &["--json"]));
    let complete = json_output(&invoke(
        &root,
        &[
            "--authorize",
            planned["planDigest"].as_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(complete["record"]["run"]["state"], "complete");
    let provider = fs::read(package.join("state.json")).unwrap();
    let same = json_output(&invoke(&root, &["--json"]));
    assert_eq!(
        same["plan"]["hosting"]["replaces"], "none",
        "an unmodified real-helper label must identify our identical bundle"
    );
    assert!(same["plan"]["hosting"]["replacedFingerprint"].is_null());

    declare_hosting(&root, b"changed");
    let changed = json_output(&invoke(&root, &["--json"]));
    assert_eq!(changed["plan"]["hosting"]["replaces"], "own");
    assert_eq!(
        fs::read(package.join("state.json")).unwrap(),
        provider,
        "planning never mutates the provider"
    );
    declare_hosting(&root, b"hello");

    let original: Value = serde_json::from_slice(&provider).unwrap();
    // This is the old helper's sorted JSON oracle, not the new identity owner.
    let mut legacy = json!({"files": original["hosting"]["files"], "config": original["hosting"]["version"]["config"]});
    legacy.sort_all_objects();
    let legacy: String = Sha256::digest(serde_json::to_vec(&legacy).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_ne!(
        &legacy[..32],
        original["hosting"]["version"]["labels"]["tmt-content"]
            .as_str()
            .unwrap()
    );
    for old_label in [false, true] {
        let mut state = original.clone();
        for version in ["version", "live"] {
            let labels = if version == "version" {
                &mut state["hosting"]["version"]["labels"]
            } else {
                &mut state["hosting"]["live"]["version"]["labels"]
            };
            if old_label {
                labels["tmt-content"] = json!(&legacy[..32]);
            } else {
                *labels = json!({});
            }
        }
        let bytes = serde_json::to_vec(&state).unwrap();
        fs::write(package.join("state.json"), &bytes).unwrap();
        let foreign = json_output(&invoke(&root, &["--json"]));
        assert_eq!(
            foreign["plan"]["hosting"]["replaces"], "foreign",
            "unlabelled and old-helper-labelled releases stay foreign"
        );
        assert_eq!(fs::read(package.join("state.json")).unwrap(), bytes);
    }
}

#[test]
fn declared_hosting_runs_through_the_binary_and_publishes_only_after_joint_readback() {
    use serde_json::{Value, json};
    let root = Root::new();
    let package = hosted_fixture(&root, b"hello");
    let preview = json_output(&invoke(&root, &["--json"]));
    let digest = preview["planDigest"].as_str().unwrap();
    let calls = fs::read_to_string(package.join("calls.jsonl")).unwrap();
    assert!(
        calls
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).unwrap()["method"] == "GET")
    );
    // Inventory has no app yet; a non-ASCII config discovered at final joint
    // read-back must remain unconfirmed, never Complete without a derivable link.
    let auth_path = package.join("lib/auth.js");
    let auth = fs::read_to_string(&auth_path).unwrap();
    assert!(auth.contains("apiKey: 'public-api-key'"));
    fs::write(
        &auth_path,
        auth.replace(
            "apiKey: 'public-api-key'",
            "apiKey: h.live ? 'public-é' : 'public-api-key'",
        ),
    )
    .unwrap();
    let unconfirmed_output = invoke(&root, &["--authorize", &digest[..12], "--json"]);
    fs::write(&auth_path, &auth).unwrap();
    let unconfirmed = json_output(&unconfirmed_output);
    assert_eq!(unconfirmed["record"]["run"]["state"], "partial");
    let unfinished = base(&root).args(["status", "--layers"]).output().unwrap();
    let unfinished_text = String::from_utf8_lossy(&unfinished.stdout);
    assert!(unfinished_text.contains("warning: Firestore setup is not finished"));
    assert!(!unfinished_text.contains("Firestore is not configured"));
    export_human("status-unfinished", &unfinished);
    let provider_state: Value =
        serde_json::from_slice(&fs::read(package.join("state.json")).unwrap()).unwrap();
    assert!(!provider_state["hosting"]["live"].is_null());
    assert!(
        unconfirmed["record"]["run"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["id"] == "verify" && step["state"] == "unknown")
    );
    assert!(unconfirmed.get("remoteLink").is_none());
    assert!(
        tmt_remote::deploy_record::read(&root.layout())
            .unwrap()
            .unwrap()
            .verified_publication()
            .is_none()
    );
    let complete = json_output(&invoke(&root, &["--authorize", &digest[..12], "--json"]));
    assert_eq!(complete["planDigest"], preview["planDigest"]);
    assert_eq!(complete["record"]["run"]["state"], "complete");
    let publication = &complete["record"]["run"]["hosting"]["publication"];
    assert_eq!(publication["entryUrl"], "https://demo-remote-1.web.app");
    assert_eq!(publication["siteAppId"], publication["appId"]);
    assert_eq!(
        publication["publicConfig"],
        json!({"apiKey":"public-api-key","authDomain":"demo-remote-1.firebaseapp.com","projectId":"demo-remote-1","appId":"1:123:web:mine"})
    );
    let saved = tmt_remote::deploy_record::read(&root.layout())
        .unwrap()
        .unwrap();
    assert!(saved.usable_binding().is_some());
    assert!(saved.verified_publication().is_some());
    let document: Value = serde_json::from_slice(
        &fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(document["version"], 3);
    let calls: Vec<Value> = fs::read_to_string(package.join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["method"] == "POST" && c["url"].as_str().unwrap().ends_with("/webApps"))
            .count(),
        1
    );
    let rules = calls
        .iter()
        .position(|c| c["method"] == "POST" && c["url"].as_str().unwrap().ends_with("/releases"))
        .unwrap();
    let stage = calls
        .iter()
        .position(|c| {
            c["method"] == "PATCH" && c["url"].as_str().unwrap().ends_with("?updateMask=status")
        })
        .unwrap();
    let live = calls
        .iter()
        .position(|c| {
            c["method"] == "POST"
                && c["url"]
                    .as_str()
                    .unwrap()
                    .contains("/releases?versionName=")
        })
        .unwrap();
    assert!(stage < rules && rules < live);
    // Re-print derives one snapshot while the writer is held, without saving
    // anything or invoking declarations, Node or the provider.
    export_human(
        "status-complete",
        &base(&root).args(["status", "--layers"]).output().unwrap(),
    );
    export_human(
        "link",
        &base(&root)
            .args(["link", "--remote", "demo-remote-1"])
            .output()
            .unwrap(),
    );
    let derived_link = complete["remoteLink"].as_str().unwrap();
    tmt_remote::remote_link::validate(derived_link).unwrap();
    assert!(document.get("remoteLink").is_none());
    let bytes_before =
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let provider_before = fs::read(package.join("calls.jsonl")).unwrap();
    let layout = root.layout();
    let writer =
        tmt_remote::deploy_record::DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    let first = base(&root).args(["link", "--json"]).output().unwrap();
    let second = base(&root).args(["link", "--json"]).output().unwrap();
    assert!(first.status.success(), "{:?}", first);
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(
        serde_json::from_slice::<Value>(&first.stdout).unwrap(),
        json!({"version":1,"remoteLink":derived_link})
    );
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        bytes_before
    );
    assert_eq!(
        fs::read(package.join("calls.jsonl")).unwrap(),
        provider_before
    );
    drop(writer);
    // Version 3 stays unchanged; there is no link metadata or reader conversion.
    assert!(
        tmt_remote::deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .verified_publication()
            .is_some()
    );
    assert_eq!(
        tmt_remote::deploy_record::read_link(&layout).unwrap(),
        derived_link
    );
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        bytes_before
    );
    // A partial publication cannot yield a link; the reader changes nothing.
    let mut partial = document.clone();
    partial["record"]["run"]["state"] = json!("partial");
    for step in partial["record"]["run"]["steps"].as_array_mut().unwrap() {
        if step["id"] == "hosting:release" {
            step["state"] = json!("unknown");
        }
    }
    let partial_bytes = serde_json::to_vec(&partial).unwrap();
    fs::write(
        root.remote().join("deployments/demo-remote-1/deploy.json"),
        &partial_bytes,
    )
    .unwrap();
    let refused = base(&root).args(["link", "--json"]).output().unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&refused.stdout).unwrap()["error"]["code"],
        "REMOTE_LINK_UNAVAILABLE"
    );
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        partial_bytes
    );
    fs::write(
        root.remote().join("deployments/demo-remote-1/deploy.json"),
        &bytes_before,
    )
    .unwrap();
    // Checkpoint data is validated on read, never converted or silently repaired.
    for corrupt in [
        "extra-envelope-key",
        "foreign-public-config",
        "foreign-site-association",
    ] {
        let mut damaged = document.clone();
        if corrupt == "extra-envelope-key" {
            damaged["record"]["run"]["hosting"]["envelope"]["extra"] = json!(true);
        } else if corrupt == "foreign-site-association" {
            damaged["record"]["run"]["hosting"]["publication"]["siteAppId"] = json!("another-app");
        } else {
            damaged["record"]["run"]["hosting"]["publication"]["publicConfig"]["projectId"] =
                json!("another-project");
        }
        let bytes = serde_json::to_vec(&damaged).unwrap();
        fs::write(
            root.remote().join("deployments/demo-remote-1/deploy.json"),
            &bytes,
        )
        .unwrap();
        assert_eq!(
            tmt_remote::deploy_record::read(&root.layout())
                .unwrap_err()
                .code,
            "REMOTE_STATE_UNSAFE"
        );
        assert_eq!(
            fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn project_selection_is_local_opt_in_and_catalog_changes_never_rewrite_history() {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };

    let root = Root::new();
    available(&root);
    let layout = root.layout();
    let empty = base(&root).args(["ls"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&empty.stdout),
        "No remotes yet; add one with tmt remote add <name> <link>\n"
    );
    export_human("empty-ls", &empty);
    export_human(
        "status-unconfigured",
        &base(&root).args(["status", "--layers"]).output().unwrap(),
    );
    for (project, region, id) in [
        ("demo-remote-1", "asia-east1", deploy_fixture::ID),
        (
            "demo-remote-2",
            "us-central1",
            "ad2fb77d-b5e8-4a32-bf35-413f2220d909",
        ),
    ] {
        // Seed selector inputs without a writer descriptor that sibling test forks can inherit.
        // Writer admission and sequential-project persistence are covered by deploy_record.rs.
        let directory = layout.directory.join("deployments").join(project);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 2,
            "record": tmt_remote::deploy_run::DeployRecord::new(id),
            "target": { "project": project, "region": region },
        }))
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("deploy.json"))
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let unregistered = base(&root).args(["ls"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&unregistered.stdout),
        "  REMOTE         TARGET         REGION       DEFAULT\n  demo-remote-1  demo-remote-1  asia-east1\n  demo-remote-2  demo-remote-2  us-central1\n"
    );
    export_human("owned-unregistered-ls", &unregistered);
    let selected = base(&root)
        .args(["status", "--layers", "--remote", "demo-remote-2", "--json"])
        .output()
        .unwrap();
    let value = json_output(&selected);
    assert_eq!(
        value["firestoreDeployment"],
        serde_json::json!({"project":"demo-remote-2", "region":"us-central1", "deploymentId":"ad2fb77d-b5e8-4a32-bf35-413f2220d909"})
    );
    assert_eq!(value["firestoreLayers"], serde_json::json!([]));
    let required = base(&root)
        .args(["status", "--layers", "--json"])
        .output()
        .unwrap();
    assert!(!required.status.success());
    let required: serde_json::Value = serde_json::from_slice(&required.stdout).unwrap();
    assert_eq!(required["error"]["code"], "REMOTE_SELECTOR_REQUIRED");
    let ordinary = json_output(&base(&root).args(["status", "--json"]).output().unwrap());
    assert_eq!(
        ordinary,
        serde_json::json!({"running":false, "lastPort":null,"signIn":{"state":"none","expiresAtMs":null}})
    );
    let set = base(&root)
        .args(["settings", "default-remote", "demo-remote-2", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_output(&set)["defaultRemote"], "demo-remote-2");
    let default = json_output(
        &base(&root)
            .args(["status", "--layers", "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(default, value);
    assert_eq!(
        json_output(
            &base(&root)
                .args(["status", "--layers", "--project", "demo-remote-2", "--json"])
                .output()
                .unwrap()
        ),
        value
    );
    let conflict = base(&root)
        .args([
            "status",
            "--layers",
            "--project",
            "demo-remote-1",
            "--remote",
            "demo-remote-2",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&conflict.stdout).unwrap()["error"]["code"],
        "REMOTE_SELECTOR_INVALID"
    );
    let conflict_human = base(&root)
        .args([
            "status",
            "--layers",
            "--project",
            "demo-remote-1",
            "--remote",
            "demo-remote-2",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&conflict_human.stderr),
        "error: The --project and --remote options name different projects: demo-remote-1 and demo-remote-2\nhint: Run the same command with only one of --project or --remote\n"
    );
    export_human("invalid-selectors", &conflict_human);

    assert_eq!(
        json_output(
            &base(&root)
                .args([
                    "status",
                    "--layers",
                    "--project",
                    "demo-remote-2",
                    "--remote",
                    "demo-remote-2",
                    "--json"
                ])
                .output()
                .unwrap()
        ),
        value
    );
    let selected_human = base(&root).args(["status", "--layers"]).output().unwrap();
    let selected_text = String::from_utf8_lossy(&selected_human.stdout);
    assert!(selected_text.contains("Recorded Firestore setup: demo-remote-2 (us-central1)\nwarning: Firestore setup has not started"));
    assert!(!selected_text.contains("Firestore is not configured"));
    export_human("two-project-default-layers", &selected_human);
    let add = base(&root)
        .args(["add", "other", "https://remote.example/r/abc/", "--json"])
        .output()
        .unwrap();
    assert!(add.status.success(), "{:?}", add);
    let owned_command = base(&root)
        .args(["devices", "--remote", "other", "--json"])
        .output()
        .unwrap();
    let refused: serde_json::Value = serde_json::from_slice(&owned_command.stdout).unwrap();
    assert_eq!(refused["error"]["code"], "REMOTE_SELECTOR_LOCAL_REQUIRED");
    let remove = base(&root)
        .args(["rm", "other", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_output(&remove)["deployHistoryChanged"], false);
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
    export_human(
        "status-layers",
        &base(&root)
            .args(["status", "--layers", "--remote", "demo-remote-2"])
            .output()
            .unwrap(),
    );
    export_human(
        "default-remote",
        &base(&root).args(["settings"]).output().unwrap(),
    );
    let added = base(&root)
        .args(["add", "other", "https://remote.example/r/abc/"])
        .output()
        .unwrap();
    assert!(added.status.success());
    export_human("remotes-add", &added);
    export_human(
        "local-required",
        &base(&root)
            .args(["devices", "--remote", "other"])
            .output()
            .unwrap(),
    );
    tmt_remote::settings::set_default_remote(&root.0, "local").unwrap();
    let required_human = base(&root).args(["status", "--layers"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&required_human.stderr),
        "error: Several projects are deployed from this machine: demo-remote-1, demo-remote-2\nhint: Run the same command with --project demo-remote-1\nhint: Run tmt remote settings default-remote demo-remote-1\n"
    );
    export_human("selection-required", &required_human);
    export_human(
        "default-local",
        &base(&root).args(["settings"]).output().unwrap(),
    );
    let mut descriptor: serde_json::Value =
        serde_json::from_slice(&fixture("remote_link/descriptor-v1.json")).unwrap();
    descriptor.sort_all_objects();
    use base64::Engine;
    let link = format!(
        "https://demo-remote-1.web.app/pair#{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&descriptor).unwrap())
    );
    let tenant = base(&root).args(["add", "tenant", &link]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&tenant.stdout),
        "Added remote 'tenant' (demo-remote-1, asia-east1)\n"
    );
    export_human("remotes-add-firebase", &tenant);
    tmt_remote::settings::set_default_remote(&root.0, "tenant").unwrap();
    export_human("remotes-ls", &base(&root).args(["ls"]).output().unwrap());
    export_human(
        "not-owned",
        &base(&root)
            .args(["link", "--remote", "tenant"])
            .output()
            .unwrap(),
    );
    export_human(
        "kind-unsupported",
        &base(&root)
            .args(["devices", "--remote", "tenant"])
            .output()
            .unwrap(),
    );
    export_human(
        "remotes-rm",
        &base(&root).args(["rm", "tenant"]).output().unwrap(),
    );
    export_human(
        "stale-default",
        &base(&root).args(["status", "--layers"]).output().unwrap(),
    );
    export_human(
        "not-found",
        &base(&root)
            .args(["link", "--remote", "removed"])
            .output()
            .unwrap(),
    );
    fs::write(root.remote().join("deploy.json"), b"{}").unwrap();
    export_human(
        "old-single-path",
        &base(&root).args(["link"]).output().unwrap(),
    );
    fs::remove_file(root.remote().join("deploy.json")).unwrap();
    assert!(!root.remote().join("deploy.json").exists());
}

#[test]
fn invalid_remote_argument_is_distinct_from_an_unreadable_catalog() {
    let root = Root::new();
    available(&root);
    let layout = root.layout();
    tmt_remote::remote_catalog::add(&layout, "kept", "https://remote.example/r/root/").unwrap();
    let path = root.remote().join("remotes.json");
    let before = fs::read(&path).unwrap();
    for invalid in ["https://demo-remote-1.web.app/pair#truncated", "not-a-link"] {
        let reply = base(&root)
            .args(["add", "new", invalid, "--json"])
            .output()
            .unwrap();
        assert!(!reply.status.success());
        assert!(reply.stderr.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&reply.stdout).unwrap();
        assert_eq!(value["error"]["code"], "REMOTE_SELECTOR_INVALID");
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let invalid = base(&root)
        .args(["add", "new", "not-a-link"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        "error: The remote name or link is invalid\nhint: Run tmt remote add <name> <remote-link>\n"
    );
    export_human("remotes-add-invalid", &invalid);
    fs::write(&path, b"damaged catalog").unwrap();
    let unreadable = base(&root)
        .args(["add", "valid", "https://remote.example/r/root/", "--json"])
        .output()
        .unwrap();
    assert!(!unreadable.status.success());
    let value: serde_json::Value = serde_json::from_slice(&unreadable.stdout).unwrap();
    assert_eq!(value["error"]["code"], "REMOTE_SELECTOR_UNAVAILABLE");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("The remotes could not be read")
    );
    assert_eq!(fs::read(&path).unwrap(), b"damaged catalog");
}

#[test]
fn preflight_http_failure_names_the_operation_and_status_without_provider_text_or_effects() {
    let root = Root::new();
    available(&root);
    let package = installed(&root, "normal");
    let state_path = package.join("state.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["readFailure"] = serde_json::json!({"path":"/collectionGroups/-/fields?", "status":400});
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    for json in [true, false] {
        let output = invoke(&root, if json { &["--json"] } else { &[] });
        assert!(!output.status.success());
        if json {
            assert!(output.stderr.is_empty());
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"]["code"], "REMOTE_DEPLOY_PROVIDER_REJECTED");
            assert_eq!(
                value["error"]["provider"],
                serde_json::json!({"operation":"fields","status":400})
            );
            assert_eq!(
                value["error"]["message"],
                "Firebase could not read Firestore field configurations (HTTP 400). Nothing changed in your Firebase project."
            );
        } else {
            assert!(output.stdout.is_empty());
            let text = String::from_utf8(output.stderr.clone()).unwrap();
            assert!(text.contains("Firestore field configurations (HTTP 400)"));
            assert!(!text.contains("wait"));
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains("TOKEN_CANARY"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("TOKEN_CANARY"));
    }
    let calls = fs::read_to_string(package.join("calls.jsonl")).unwrap();
    assert!(
        calls.lines().all(
            |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == "GET"
        )
    );
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("/collectionGroups/-/fields?"))
            .count(),
        2,
        "one attempt per explicit command, no retry"
    );
}
