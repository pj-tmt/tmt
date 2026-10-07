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
        self.run_input(args, None)
    }
    fn run_input(
        &mut self,
        args: &[&str],
        input: Option<&serde_json::Value>,
    ) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>) {
        let stdout_path = self.root.as_path().join("stdout");
        let stderr_path = self.root.as_path().join("stderr");
        let mut command = support::command(self.root.as_path(), args);
        let stdin = if let Some(input) = input {
            let path = self.root.join("input");
            fs::write(&path, serde_json::to_vec(input).unwrap()).unwrap();
            Stdio::from(fs::File::open(path).unwrap())
        } else {
            Stdio::null()
        };
        command
            .stdin(stdin)
            .stdout(fs::File::create(&stdout_path).unwrap())
            .stderr(fs::File::create(&stderr_path).unwrap());
        self.child = Some(command.spawn().unwrap());
        let status = support::wait(
            &mut self.child,
            Duration::from_secs(if input.is_some() { 30 } else { 5 }),
        )
        .wait()
        .unwrap();
        let stdout = fs::read(stdout_path).unwrap();
        let stderr = fs::read(stderr_path).unwrap();
        assert!(stdout.len() <= 64 * 1024 && stderr.len() <= 64 * 1024);
        if input.is_none() {
            assert!(!support::state_dir(self.root.as_path()).exists());
        }
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
    // Native test binaries can be copied from the build stage into a new checkout.
    let working_directory = std::env::current_dir().unwrap();
    let root = working_directory
        .ancestors()
        .find(|root| root.join("rust/Cargo.toml").is_file())
        .expect("runtime checkout source tree");
    for file in value["source_files"].as_array().unwrap() {
        let bytes = fs::read(root.join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(file["sha256"], tmt_core::content_digest::sha256(&bytes));
    }
    assert_eq!(value["source_files"].as_array().unwrap().len(), 52);
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

/// This is local protocol coverage, not authenticated Actions acquisition or
/// qualification of a release archive. The PR archive keeps the actual dev
/// executable's version; no development binary is called an alpha.
#[test]
fn native_pr_handoff_checks_compiled_schema_and_preserves_installation_on_refusal() {
    use serde_json::json;
    use tmt_adapters::native_install::{InstallRequest, inspect, install};
    use tmt_core::{
        content_digest::sha256,
        native_install::{Channel, PinAction},
    };

    let mut fixture = Fixture::new("handoff");
    let target =
        tmt_core::native_install::native_target(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap();
    let source = "a".repeat(40);
    let schema = tmt_adapters::native_install::compiled_application_schema(&source).unwrap();
    let (archive, mut manifest) = fixture.archive(target, b"old fixture executable\n", "1.2.3");
    let prefix = fixture.root.join("prefix");
    let first = install(
        InstallRequest {
            archive: &archive,
            manifest: &manifest,
            prefix: &prefix,
            target,
            channel: Channel::Stable,
            pin: PinAction::Preserve,
        },
        || Ok(()),
    )
    .unwrap();
    let old_receipt_path = first
        .active_executable
        .parent()
        .unwrap()
        .join("receipt.json");
    let old_receipt = fs::read(&old_receipt_path).unwrap();
    let installed: serde_json::Value = serde_json::from_slice(&old_receipt).unwrap();
    let old_bytes = fs::read(&first.active_executable).unwrap();
    let candidate_binary = fs::read(env!("CARGO_BIN_EXE_tmt")).unwrap();
    let (archive, candidate_manifest) =
        fixture.archive(target, &candidate_binary, env!("CARGO_PKG_VERSION"));
    manifest = candidate_manifest;
    let mut manifest_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    manifest_value["tmt_application_schema"] = schema.clone();
    let manifest_bytes = serde_json::to_vec(&manifest_value).unwrap();
    fs::write(&manifest, &manifest_bytes).unwrap();
    let archive_bytes = fs::read(&archive).unwrap();
    let catalog: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../tmt-adapters/src/native_install/fixtures/pr-rc-catalog-v2.json"
    ))
    .unwrap();
    let mut candidate = catalog["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["product"] == "cli" && candidate["target"] == target)
        .unwrap()
        .clone();
    candidate["version"] = env!("CARGO_PKG_VERSION").into();
    candidate["application_schema"] = schema.clone();
    candidate["dist_manifest"]["bytes"] = manifest_bytes.len().into();
    candidate["dist_manifest"]["sha256"] = sha256(&manifest_bytes).into();
    candidate["archive"]["bytes"] = archive_bytes.len().into();
    candidate["archive"]["sha256"] = sha256(&archive_bytes).into();
    let mut request = json!({
        "protocol": 2,
        "transfer": {
            "protocol": 1, "archive": archive, "manifest": manifest, "prefix": prefix,
            "target": target, "version": env!("CARGO_PKG_VERSION"),
            "archive_sha256": sha256(&archive_bytes), "manifest_sha256": sha256(&manifest_bytes),
            "channel": "pr234", "pin": "preserve", "expected_current": installed["release_id"], "release_id": 0
        },
        "provenance": {"kind": "github-actions", "evidence": {
            "repository": "pj-tmt/tmt", "repository_id": 7001, "pr": 234, "head_sha": source,
            "producer": catalog["producer"], "eligibility": catalog["eligibility"],
            "catalog_artifact_id": 8201, "catalog_zip_sha256": "b".repeat(64), "catalog_sha256": "c".repeat(64),
            "candidate": candidate,
            "admission": {
                "observed_at_ms": catalog["published_at_ms"].as_u64().unwrap() + 1,
                "published_at_ms": catalog["published_at_ms"], "expires_at_ms": catalog["expires_at_ms"],
                "initial_pull_sha256": "d".repeat(64), "initial_timeline_sha256": "d".repeat(64),
                "pull_sha256": "d".repeat(64), "timeline_sha256": "d".repeat(64),
                "catalog_inventory_sha256": "d".repeat(64), "producer_run_sha256": "d".repeat(64),
                "catalog_metadata_sha256": "d".repeat(64), "payload_metadata_sha256": "d".repeat(64),
                "latest_alpha": {"release_id": 42, "version": "5.0.0-alpha.92", "manifest_sha256": "e".repeat(64), "application_schema": schema},
                "local_schemas": schema["databases"], "schema_ahead_opt_in": false
            }
        }},
        "explicit_channel": true
    });
    // Only this owned database is initialized. The child observes it read-only.
    let database = support::state_dir(&fixture.root).join("tmux-team.db");
    let mut storage = tmt_adapters::storage::Storage::open(&database).unwrap();
    storage.close().unwrap();
    let args = ["__native-install", "--handoff-version", "2", "--json"];
    let original_schema_hash =
        manifest_value["tmt_application_schema"]["source_files"][0]["sha256"].clone();
    manifest_value["tmt_application_schema"]["source_files"][0]["sha256"] = "f".repeat(64).into();
    let tampered = serde_json::to_vec(&manifest_value).unwrap();
    fs::write(&manifest, &tampered).unwrap();
    request["transfer"]["manifest_sha256"] = sha256(&tampered).into();
    request["provenance"]["evidence"]["candidate"]["dist_manifest"]["sha256"] =
        sha256(&tampered).into();
    request["provenance"]["evidence"]["candidate"]["application_schema"] =
        manifest_value["tmt_application_schema"].clone();
    let (status, output, _) = fixture.run_input(&args, Some(&request));
    assert!(!status.success());
    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("compiled binary"),
        "{response}"
    );
    assert_eq!(
        inspect(&prefix.join("bin/tmt")).unwrap().active_executable,
        first.active_executable
    );
    assert_eq!(fs::read(&old_receipt_path).unwrap(), old_receipt);
    assert_eq!(fs::read(&first.active_executable).unwrap(), old_bytes);
    manifest_value["tmt_application_schema"]["source_files"][0]["sha256"] = original_schema_hash;
    fs::write(&manifest, &manifest_bytes).unwrap();
    request["transfer"]["manifest_sha256"] = sha256(&manifest_bytes).into();
    request["provenance"]["evidence"]["candidate"]["dist_manifest"]["sha256"] =
        sha256(&manifest_bytes).into();
    request["provenance"]["evidence"]["candidate"]["application_schema"] =
        manifest_value["tmt_application_schema"].clone();

    // Advance the real ledger after captured admission, without timing or sleeps.
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("INSERT INTO _migrations(version,name,applied_at) VALUES (50,'future fixture migration','fixture')", []).unwrap();
    let (status, output, _) = fixture.run_input(&args, Some(&request));
    assert!(!status.success());
    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        response["error"],
        tmt_core::native_install::SchemaError::DataDowngrade.to_string(),
        "{response}"
    );
    assert_eq!(
        inspect(&prefix.join("bin/tmt")).unwrap().active_executable,
        first.active_executable
    );
    assert_eq!(fs::read(&old_receipt_path).unwrap(), old_receipt);
    assert_eq!(fs::read(&first.active_executable).unwrap(), old_bytes);
    connection
        .execute("DELETE FROM _migrations WHERE version = 50", [])
        .unwrap();
    drop(connection);
    let (status, output, stderr) = fixture.run_input(&args, Some(&request));
    assert!(
        status.success(),
        "{} {}",
        String::from_utf8_lossy(&output),
        String::from_utf8_lossy(&stderr)
    );
    assert!(stderr.is_empty());
    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["protocol"], 2);
    assert!(response["error"].is_null());
    let active = inspect(&prefix.join("bin/tmt")).unwrap();
    assert_eq!(active.state.channel, Channel::parse("pr234").unwrap());
    assert_eq!(active.state.version.to_string(), env!("CARGO_PKG_VERSION"));
    assert_eq!(
        fs::read(&active.active_executable).unwrap(),
        candidate_binary
    );
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(
            active
                .active_executable
                .parent()
                .unwrap()
                .join("receipt.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        receipt["source"]["evidence"]["producer"]["run_id"],
        catalog["producer"]["run_id"]
    );
    assert_eq!(
        receipt["source"]["evidence"]["eligibility"],
        catalog["eligibility"]
    );
    assert_eq!(
        tmt_adapters::storage::Storage::application_schema(&database).unwrap(),
        49
    );
}

impl Fixture {
    fn archive(
        &mut self,
        target: &str,
        executable: &[u8],
        version: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        let directory = self.root.join(version);
        fs::create_dir(&directory).unwrap();
        let name = format!("tmt-cli-{target}.tar.gz");
        let root = name.trim_end_matches(".tar.gz");
        let contents = directory.join(root);
        fs::create_dir(&contents).unwrap();
        fs::write(contents.join("tmt"), executable).unwrap();
        fs::set_permissions(contents.join("tmt"), fs::Permissions::from_mode(0o755)).unwrap();
        for name in ["LICENSE", "NATIVE-INSTALL.md", "THIRD-PARTY-NOTICES.txt"] {
            fs::write(
                contents.join(name),
                b"Local protocol fixture, not a released archive.\n",
            )
            .unwrap();
        }
        let archive = directory.join(&name);
        let mut command = std::process::Command::new("/usr/bin/tar");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("COPYFILE_DISABLE", "1")
            .current_dir(&directory)
            .arg("-czf")
            .arg(&archive)
            .args([
                format!("{root}/tmt"),
                format!("{root}/LICENSE"),
                format!("{root}/NATIVE-INSTALL.md"),
                format!("{root}/THIRD-PARTY-NOTICES.txt"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(directory.join("tar-stderr")).unwrap());
        self.child = Some(command.spawn().unwrap());
        assert!(
            support::wait(&mut self.child, Duration::from_secs(30))
                .wait()
                .unwrap()
                .success()
        );
        let manifest = directory.join("dist-manifest.json");
        let assets = [
            "tmt",
            "LICENSE",
            "NATIVE-INSTALL.md",
            "THIRD-PARTY-NOTICES.txt",
        ]
        .map(|path| json!({"path": path}));
        fs::write(&manifest, serde_json::to_vec(&json!({
            "artifacts": {name.clone(): {"kind": "executable-zip", "name": name,
                "target_triples": [target], "checksums": {"sha256": tmt_core::content_digest::sha256(&fs::read(&archive).unwrap())},
                "assets": assets}},
            "releases": [{"app_name": "tmt-cli", "app_version": version, "artifacts": [name]}]
        })).unwrap()).unwrap();
        (archive, manifest)
    }
}
