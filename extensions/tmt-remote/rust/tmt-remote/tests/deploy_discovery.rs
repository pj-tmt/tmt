//! Disposable fake public declaration commands, never Colab production artifacts.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::AtomicBool,
};
use tmt_remote::{
    deploy_discovery::{self, DeclarationSource, DiscoveryRefusal, InstalledDeclarations},
    deploy_tools::{self, ToolDiscoveryError},
};
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[path = "support/executable_fixture.rs"]
mod executable_fixture;
use deploy_fixture::Root;
fn envelope() -> Value {
    let read = |path: &str| {
        fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/rules")
                .join(path),
        )
        .unwrap()
    };
    let declaration = read("colab.json");
    let digest = Sha256::digest(declaration.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    json!({"version":1,"extension":"colab","backend":"firestore","declaration":declaration,"declarationDigest":digest,"artifact":read("colab.rules")})
}
struct DeclaredFixture(Option<Value>, Vec<String>);
impl DeclarationSource for DeclaredFixture {
    fn declaration(&mut self, name: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        self.1.push(name.into());
        Ok(self.0.as_ref().map(|v| serde_json::to_vec(v).unwrap()))
    }
}
#[test]
fn supplied_exact_bytes_compose_once_and_absence_never_uses_fixture_bytes() {
    let mut source = DeclaredFixture(Some(envelope()), Vec::new());
    let one = deploy_discovery::discover(&mut source, &["colab"]).unwrap();
    assert_eq!(source.1, ["colab"]);
    assert_eq!(one.extensions.view().extensions[0].name, "colab");
    assert!(one.artifacts.rules.contains("match /x/colab"));
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
    let mut source = DeclaredFixture(None, Vec::new());
    let absent = deploy_discovery::discover(&mut source, &["colab"]).unwrap();
    assert!(absent.extensions.view().extensions.is_empty());
    assert_eq!(
        absent.extensions.view().unavailable[0].reason,
        "no-declaration"
    );
    assert!(!absent.artifacts.rules.contains("match /x/colab"));
    assert_eq!(source.1, ["colab"]);
    assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
}
#[test]
fn wrong_identity_unknown_field_digest_and_artifact_drift_refuse() {
    for field in [
        "extension",
        "backend",
        "declarationDigest",
        "artifact",
        "unknown",
    ] {
        let mut reply = envelope();
        reply[field] = json!("changed");
        assert!(
            deploy_discovery::discover(&mut DeclaredFixture(Some(reply), Vec::new()), &["colab"])
                .is_err(),
            "{field}"
        );
    }
    let mut source = DeclaredFixture(Some(envelope()), Vec::new());
    assert!(deploy_discovery::discover(&mut source, &["colab", "colab"]).is_err());
    assert!(source.1.is_empty());
    let empty = deploy_discovery::discover(&mut source, &[]).unwrap();
    assert!(source.1.is_empty());
    assert!(empty.extensions.view().extensions.is_empty());
}
fn executable(path: &std::path::Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn fake_public_dispatch_observes_fixed_argv_neutral_cwd_and_cleans_up() {
    let root = Root::new();
    assert!(!root.remote().exists());
    let stop = AtomicBool::new(false);
    let output = serde_json::to_string(&envelope()).unwrap();
    let program = root.0.join("tmt");
    executable_fixture::write_executable(&program, &format!("[ \"$PWD\" = / ] || exit 4\n[ \"$1 $2 $3\" = 'colab deploy-declaration --json' ] || exit 5\nprintf '%s' '{}'\n", output.replace('\'', "'\\''"))).unwrap();
    let mut source = InstalledDeclarations::at(program, &stop).unwrap();
    assert_eq!(
        deploy_discovery::discover(&mut source, &["colab"])
            .unwrap()
            .extensions
            .view()
            .extensions
            .len(),
        1
    );
    let program = root.0.join("old-tmt");
    executable_fixture::write_executable(
        &program,
        "printf '%s' '{\"error\":{\"code\":\"USAGE_ERROR\"}}'; exit 2",
    )
    .unwrap();
    let mut source = InstalledDeclarations::at(program, &stop).unwrap();
    assert!(source.declaration("colab").unwrap().is_none());
    let program = root.0.join("broken-tmt");
    executable_fixture::write_executable(
        &program,
        "printf '%s' 'TOKEN_CANARY'; printf '%s' 'TOKEN_CANARY' >&2; exit 2",
    )
    .unwrap();
    let mut source = InstalledDeclarations::at(program, &stop).unwrap();
    assert_eq!(
        source.declaration("colab"),
        Err(DiscoveryRefusal::Unavailable)
    );
}

fn install(root: &Root, shebang: &str) -> (PathBuf, PathBuf) {
    let bin = root.0.join("bin");
    let package = root.0.join("node_modules/firebase-tools");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(package.join("lib/bin")).unwrap();
    fs::write(
        package.join("package.json"),
        b"{\"name\":\"firebase-tools\",\"version\":\"15.29.0\"}",
    )
    .unwrap();
    executable(
        &package.join("lib/bin/firebase.js"),
        &format!("{shebang}\nthrow new Error('must never execute');\n"),
    );
    executable(&bin.join("node"), "#!/bin/sh\nexit 77\n");
    symlink(package.join("lib/bin/firebase.js"), bin.join("firebase")).unwrap();
    (bin, package)
}
#[test]
fn installed_firebase_realpath_selects_package_and_launcher_node_without_execution() {
    let root = Root::new();
    let (bin, package) = install(&root, "#!/usr/bin/env node");
    let search = std::env::join_paths([PathBuf::from("."), bin.clone()]).unwrap();
    let tools = deploy_tools::discover(&search).unwrap();
    assert_eq!(tools.package, fs::canonicalize(&package).unwrap());
    assert_eq!(tools.node, fs::canonicalize(bin.join("node")).unwrap());
    assert_eq!(
        deploy_tools::discover_from(bin.as_os_str(), &root.0),
        Err(ToolDiscoveryError::Unsupported)
    );
    let node = bin.join("node");
    executable(
        &package.join("lib/bin/firebase.js"),
        &format!("#!{}\n", node.display()),
    );
    assert_eq!(deploy_tools::discover(bin.as_os_str()).unwrap(), tools);
    assert_eq!(
        deploy_tools::discover(std::ffi::OsStr::new(".")),
        Err(ToolDiscoveryError::FirebaseMissing)
    );
}
#[test]
fn unsupported_layout_version_launcher_or_missing_node_fails_closed() {
    let root = Root::new();
    let (bin, package) = install(&root, "#!/usr/bin/env node");
    fs::remove_file(bin.join("node")).unwrap();
    assert_eq!(
        deploy_tools::discover(bin.as_os_str()),
        Err(ToolDiscoveryError::NodeMissing)
    );
    executable(&bin.join("node"), "#!/bin/sh\nexit 77\n");
    fs::write(
        package.join("package.json"),
        b"{\"name\":\"firebase-tools\",\"version\":\"0.0.0\"}",
    )
    .unwrap();
    assert_eq!(
        deploy_tools::discover(bin.as_os_str()),
        Err(ToolDiscoveryError::Unsupported)
    );
    fs::write(
        package.join("package.json"),
        b"{\"name\":\"firebase-tools\",\"version\":\"15.29.0\"}",
    )
    .unwrap();
    executable(
        &package.join("lib/bin/firebase.js"),
        "#!/usr/bin/env -S node\n",
    );
    assert_eq!(
        deploy_tools::discover(bin.as_os_str()),
        Err(ToolDiscoveryError::Unsupported)
    );
    fs::remove_file(bin.join("firebase")).unwrap();
    executable(&bin.join("firebase"), "#!/bin/sh\nexit 99\n");
    assert_eq!(
        deploy_tools::discover(bin.as_os_str()),
        Err(ToolDiscoveryError::Unsupported)
    );
}
