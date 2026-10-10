//! The provider process boundary uses a disposable package, never the owner's login.
use std::{ffi::OsString, fs, path::PathBuf, sync::atomic::AtomicBool};
use tmt_remote::deploy_firestore::{DeployFirestore, DeploySetupError};
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
use deploy_fixture::Root;

fn node() -> PathBuf {
    tmt_invoke::find_executable(
        std::ffi::OsStr::new("node"),
        &std::env::var_os("PATH").unwrap(),
    )
    .expect("installed Node required for bridge fixtures")
}
#[test]
fn incompatible_package_is_refused_before_loading_auth_or_any_provider_effect() {
    let root = Root::new();
    let package = installed_stub(&root, "", "throw");
    fs::write(
        package.join("package.json"),
        br#"{"name":"firebase-tools","version":"0.0.0"}"#,
    )
    .unwrap();
    let stop = AtomicBool::new(false);
    let error = DeployFirestore::at(node(), package, &stop).err().unwrap();
    assert_eq!(error, DeploySetupError::UnsupportedTool);
    assert!(error.to_string().contains("15.29.0"));
    assert!(
        error
            .to_string()
            .contains("Install it with: npm install -g firebase-tools@15.29.0")
    );
    assert!(!root.0.join("firebase-tools/auth-loaded").exists());
    assert!(!root.0.join("firebase-tools/calls.jsonl").exists());
}
#[test]
fn fixture_environment_does_not_pass_secret_or_credential_override_variables() {
    let allowed = DeployFirestore::environment_names();
    assert_eq!(
        allowed,
        ["HOME", "PATH", "XDG_CONFIG_HOME"].map(OsString::from)
    );
}

fn installed_stub(root: &Root, source: &str, mode: &str) -> PathBuf {
    let package = root.0.join("firebase-tools");
    fs::create_dir_all(package.join("lib")).unwrap();
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/deploy_firestore/firebase-tools");
    for file in [
        "package.json",
        "lib/auth.js",
        "lib/logger.js",
        "lib/api.js",
        "lib/apiv2.js",
        "lib/configstore.js",
    ] {
        fs::copy(base.join(file), package.join(file)).unwrap();
    }
    fs::write(package.join("state.json"),serde_json::to_vec(&serde_json::json!({"account":"owner@example.test","project":"demo-remote-1", "location":"asia-east1", "canary":"TOKEN_CANARY_DO_NOT_DISCLOSE", "source":source,"mode":mode})).unwrap()).unwrap();
    package
}
#[test]
fn thrown_dependency_stdout_stderr_and_errors_never_cross_the_boundary() {
    let root = Root::new();
    let stop = AtomicBool::new(false);
    let mut adapter =
        DeployFirestore::at(node(), installed_stub(&root, "", "throw"), &stop).unwrap();
    let error = adapter.login_account().unwrap_err();
    assert_eq!(error, DeploySetupError::Unavailable);
    assert!(!format!("{error:?} {error}").contains("TOKEN_CANARY"));
}
#[test]
fn missing_login_does_not_fall_back_to_another_credential_source() {
    let root = Root::new();
    let stop = AtomicBool::new(false);
    let package = installed_stub(&root, "", "missing");
    let mut adapter = DeployFirestore::at(node(), package.clone(), &stop).unwrap();
    assert_eq!(
        adapter.login_account(),
        Err(DeploySetupError::LoginRequired)
    );
    assert!(!package.join("calls.jsonl").exists());
}
#[test]
fn existing_project_run_uses_the_real_process_boundary_and_real_record_owner() {
    use tmt_remote::{
        deploy_plan::{self, CloudBackend, Enabled, Supplied, Target},
        deploy_record::DeployRecordStore,
        deploy_run::{self, DeployInput, RunState, SignInProvider},
    };
    let root = Root::new();
    let layout = root.layout();
    let stop = AtomicBool::new(false);
    let read = |file: &str| {
        fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap()
    };
    let declaration = read("declarations/colab-firestore.json");
    let fragment = read("declarations/rules/colab-admission.rules");
    let extensions = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: &declaration,
                artifact: &fragment,
            }),
        }],
    )
    .unwrap();
    let body = read("deploy_run/rules-body.rules");
    let plan = deploy_run::prepare(
        &extensions,
        &DeployInput {
            account: "owner@example.test",
            project: "demo-remote-1",
            deployment_id: deploy_fixture::ID,
            location: "asia-east1",
            sign_in: &[SignInProvider::Anonymous],
            rules_body: &body,
            live_rules: None,
        },
    )
    .unwrap();
    let package = installed_stub(
        &root,
        std::str::from_utf8(plan.deployed_rules()).unwrap(),
        "ready",
    );
    let mut adapter = DeployFirestore::at(node(), package.clone(), &stop).unwrap();
    let auth = deploy_run::authorize(&plan, plan.digest()).unwrap();
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    let record = deploy_run::run(
        &plan,
        &auth,
        deploy_run::DeployRecord::new(deploy_fixture::ID),
        &mut adapter,
        &mut writer,
        1,
    )
    .unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert!(record.usable_binding().is_some());
    assert_eq!(writer.load_or_draft().unwrap(), record);
    assert!(root.remote().join("deploy.json").is_file());
    for line in fs::read_to_string(package.join("calls.jsonl"))
        .unwrap()
        .lines()
    {
        let call: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(call["method"], "GET");
    }
}

#[test]
fn cancellation_after_child_readiness_reaps_the_process_and_closes_its_socket() {
    use nix::{
        errno::Errno,
        poll::{PollFd, PollFlags, poll},
        sys::signal::kill,
        unistd::Pid,
    };
    use std::{
        io::{BufRead, BufReader},
        os::{fd::AsFd, unix::net::UnixListener},
        sync::atomic::Ordering,
        time::Duration,
    };
    struct StopOnDrop<'a>(&'a AtomicBool);
    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let root = Root::new();
    let stop = AtomicBool::new(false);
    let package = installed_stub(&root, "", "hang");
    struct GateFile(PathBuf);
    impl Drop for GateFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    // macOS temp_dir paths exceed sun_path; /tmp keeps this fixture socket short.
    let gate = GateFile(PathBuf::from("/tmp").join(format!(
        "tmt-dg-{}.sock",
        tmt_remote::store::uuid_v4().unwrap()
    )));
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("state.json")).unwrap()).unwrap();
    state["gate"] = serde_json::json!(gate.0);
    fs::write(
        package.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let listener = UnixListener::bind(&gate.0).unwrap();
    let mut adapter = DeployFirestore::at(node(), package.clone(), &stop).unwrap();
    std::thread::scope(|scope| {
        let run = scope.spawn(|| adapter.login_account());
        let _cleanup = StopOnDrop(&stop);
        let mut readiness = [PollFd::new(listener.as_fd(), PollFlags::POLLIN)];
        assert_eq!(
            poll(&mut readiness, 3000u16).unwrap(),
            1,
            "helper did not reach its credential readiness gate"
        );
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let pid = line.trim().parse::<i32>().unwrap();
        stop.store(true, Ordering::Relaxed);
        assert_eq!(run.join().unwrap(), Err(DeploySetupError::Unavailable));
        assert_eq!(kill(Pid::from_raw(pid), None), Err(Errno::ESRCH));
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    });
    drop(listener);
    fs::remove_file(&gate.0).unwrap();
}

#[test]
fn helper_protocol_and_provider_contract_fixtures_run_in_the_native_gate() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/deploy_firestore/helper.test.cjs");
    let args = [OsString::from("--test"), script.into_os_string()];
    let env = DeployFirestore::environment_names();
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &node(),
            args: &args,
            input: &[],
            deadline: std::time::Instant::now() + tmt_remote::limits::DEPLOY_PROVIDER_CALL,
            max_stream_bytes: tmt_remote::limits::DEPLOY_PROVIDER_BYTES,
            launch: tmt_invoke::LaunchOptions {
                environment: tmt_invoke::EnvironmentPolicy::ClearAllowlist(&env),
                ..Default::default()
            },
        },
        None,
    )
    .unwrap();
    assert!(
        output.status.success(),
        "helper contract fixture failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn plan_inventory_counts_unrelated_live_configs_before_any_provider_mutation() {
    use tmt_remote::{
        deploy_plan::{self, CloudBackend, Enabled, Supplied, Target},
        deploy_run::{DeployFault, DeployProviderError},
    };
    let root = Root::new();
    let stop = AtomicBool::new(false);
    let read = |name: &str| {
        fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/rules")
                .join(name),
        )
        .unwrap()
    };
    let declaration = read("colab.json");
    let artifact = read("colab.rules");
    let plan = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: &declaration,
                artifact: &artifact,
            }),
        }],
    )
    .unwrap();
    let package = installed_stub(&root, "", "normal");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("state.json")).unwrap()).unwrap();
    state["configs"]=serde_json::json!((0..200).map(|n|serde_json::json!({"name":format!("projects/demo-remote-1/databases/(default)/collectionGroups/unrelated/fields/field{n}")})).collect::<Vec<_>>());
    fs::write(
        package.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let mut adapter = DeployFirestore::at(node(), package.clone(), &stop).unwrap();
    assert_eq!(
        adapter.check_index_budget("demo-remote-1", &plan),
        Err(DeployProviderError::Rejected(DeployFault::QuotaExceeded))
    );
    state["configs"] = serde_json::json!([]);
    fs::write(
        package.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    adapter.check_index_budget("demo-remote-1", &plan).unwrap();
    let calls = fs::read_to_string(package.join("calls.jsonl")).unwrap();
    assert!(
        calls.lines().all(
            |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == "GET"
        )
    );
}
