//! Shipped argv through disposable public-dispatch fixtures; no account/provider access.
use std::{fs, process::Command};
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[path = "support/executable_fixture.rs"]
mod executable_fixture;
use deploy_fixture::Root;
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-remote");
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
    let mut record = tmt_remote::deploy_record::DeployRecordStore::open(&layout).unwrap();
    record
        .persist(&tmt_remote::deploy_run::DeployRecord::new(
            deploy_fixture::ID,
        ))
        .unwrap();
    drop(record);
    let before = fs::read(root.remote().join("deploy.json")).unwrap();
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
    assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
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
    let before = fs::read(root.remote().join("deploy.json")).unwrap();
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
    assert!(human.contains("To deploy this plan, run the same command with --authorize <12-hex>."));
    assert!(human.contains("object 256 KiB"));
    assert!(human.contains("TTL: not set up"));
    assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
    let done = json_output(&invoke(&root, &["--authorize", &digest[..12], "--json"]));
    assert_eq!(
        done["record"]["deploymentId"],
        preview["record"]["deploymentId"]
    );
    assert_eq!(done["record"]["run"]["state"], "complete");
    assert!(done["record"]["binding"].is_object());
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.remote().join("deploy.json")).unwrap()).unwrap();
    assert_eq!(saved["record"], done["record"]);
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
fn available_binary_reports_missing_tool_and_missing_login_before_opening_record() {
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
        assert!(!root.remote().exists());
        assert!(!root.0.join("firebase-tools/calls.jsonl").exists());
    }
}
