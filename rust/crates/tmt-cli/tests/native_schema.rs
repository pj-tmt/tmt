//! Compiled schema export and installer probes must not initialize local state.
#![cfg(unix)]
mod support;
use std::{
    fs,
    process::{Child, Stdio},
    time::Duration,
};

struct Fixture {
    root: std::path::PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("tmt-native-schema-{}-{name}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn run(&mut self, args: &[&str]) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>) {
        let stdout_path = self.root.as_path().join("stdout");
        let stderr_path = self.root.as_path().join("stderr");
        let mut command = support::command(self.root.as_path(), args);
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout_path).unwrap())
            .stderr(fs::File::create(&stderr_path).unwrap());
        self.child = Some(command.spawn().unwrap());
        let status = support::wait(&mut self.child, Duration::from_secs(5))
            .wait()
            .unwrap();
        let stdout = fs::read(stdout_path).unwrap();
        let stderr = fs::read(stderr_path).unwrap();
        assert!(stdout.len() <= 64 * 1024 && stderr.len() <= 64 * 1024);
        assert!(!support::state_dir(self.root.as_path()).exists());
        assert!(!self.root.as_path().join("home/.codex").exists());
        (status, stdout, stderr)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn native_export_has_exact_compiled_source_closure_without_storage_or_credentials() {
    let mut fixture = Fixture::new("export");
    let source = "a".repeat(40);
    let (status, stdout, stderr) =
        fixture.run(&["__native-schema", "--source-sha", &source, "--json"]);
    assert!(status.success());
    assert!(stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(
        value,
        tmt_adapters::native_install::compiled_application_schema(&source).unwrap()
    );
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["product"], "cli");
    assert_eq!(value["databases"][0]["domain"], "tmt-core-db");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    for file in value["source_files"].as_array().unwrap() {
        let bytes = fs::read(root.join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(file["sha256"], tmt_core::content_digest::sha256(&bytes));
    }
    assert_eq!(value["source_files"].as_array().unwrap().len(), 51);
    // A caller can supply a descriptive SHA; it cannot change compiled authority.
    let (status, other, stderr) =
        fixture.run(&["__native-schema", "--source-sha", &"b".repeat(40), "--json"]);
    assert!(status.success());
    assert!(stderr.is_empty());
    let other: serde_json::Value = serde_json::from_slice(&other).unwrap();
    assert_ne!(other["source_sha"], value["source_sha"]);
    assert_eq!(other["source_files"], value["source_files"]);
    assert_eq!(other["databases"], value["databases"]);
    let (status, _, _) = fixture.run(&["__native-schema", "--source-sha", "main", "--json"]);
    assert!(!status.success());
}

#[test]
fn both_installer_probes_are_disjoint_and_unknown_versions_refuse_before_input() {
    let mut fixture = Fixture::new("probes");
    for version in ["1", "2"] {
        let (status, stdout, stderr) = fixture.run(&[
            "__native-install",
            "--handoff-version",
            version,
            "--probe",
            "--json",
        ]);
        assert!(status.success());
        assert!(stderr.is_empty());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&stdout).unwrap(),
            serde_json::json!({"protocol":version.parse::<u32>().unwrap()})
        );
    }
    let (status, _, _) = fixture.run(&[
        "__native-install",
        "--handoff-version",
        "3",
        "--probe",
        "--json",
    ]);
    assert!(!status.success());
}
