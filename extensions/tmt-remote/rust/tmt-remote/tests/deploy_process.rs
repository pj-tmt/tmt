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
                    .contains("whole-envelope")
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
        "Firestore sharing is unavailable: no enabled extension supplies a Firestore declaration.\nNothing changed in your Firebase project.\n"
    );
}
