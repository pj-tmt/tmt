#[path = "architecture/cases.rs"]
mod cases;
#[path = "architecture/driver_names.rs"]
mod driver_names;
#[path = "architecture/host_names.rs"]
mod host_names;
#[path = "architecture/output.rs"]
mod output;
#[path = "architecture/output_allowlist.rs"]
mod output_allowlist;
#[path = "architecture/output_cases.rs"]
mod output_cases;
#[path = "architecture/policy.rs"]
mod policy;
#[path = "architecture/skill_lists.rs"]
mod skill_lists;
#[path = "architecture/source.rs"]
mod source;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::process::{CommandRequest, CommandRunner, UnixCommandRunner};

#[test]
fn workspace_obeys_native_architecture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let cargo = std::env::var_os("CARGO").expect("cargo test provides its Cargo executable");
    let args: Vec<OsString> = [
        "metadata",
        "--offline",
        "--locked",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
    ]
    .into_iter()
    .map(Into::into)
    .chain([manifest.into_os_string()])
    .collect();
    let output = UnixCommandRunner
        .execute(CommandRequest {
            program: &cargo,
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(10),
            max_output_bytes: 4 * 1024 * 1024,
        })
        .expect("bounded offline Cargo metadata");
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Cargo metadata JSON");
    let mut violations = Vec::new();
    let mut sources = Vec::new();
    let packages: BTreeSet<_> = metadata["packages"]
        .as_array()
        .expect("Cargo packages")
        .iter()
        .map(|p| p["name"].as_str().expect("Cargo package name"))
        .collect();
    assert_eq!(
        packages,
        BTreeSet::from([
            "tmt-core",
            "tmt-adapters",
            "tmt-cli",
            "tmt-cli-style",
            "tmt-command-output",
            "tmt-office",
            "tmt-office-command",
            "tmt-office-model",
            "tmt-office-pairing",
            "tmt-office-service",
            "tmt-office-storage",
            "tmt-squad"
        ]),
        "Review native package boundaries when changing workspace members"
    );
    for package in metadata["packages"].as_array().expect("Cargo packages") {
        violations.extend(policy::dependency_violations(package));
        for target in package["targets"].as_array().expect("Cargo targets") {
            let kind = target["kind"].as_array().expect("Cargo target kinds");
            if kind.iter().any(|k| k == "lib" || k == "bin") {
                sources.extend(
                    source::collect(
                        package["name"].as_str().unwrap(),
                        Path::new(target["src_path"].as_str().unwrap()),
                    )
                    .expect("collect production source"),
                );
            }
        }
    }
    assert!(
        sources
            .iter()
            .any(|s| s.package == "tmt-core" && s.file == "identity.rs")
    );
    for (package, file) in [
        ("tmt-core", "names.rs"),
        ("tmt-cli", "invocation.rs"),
        ("tmt-command-output", "lib.rs"),
    ] {
        assert!(
            sources
                .iter()
                .any(|s| s.package == package && s.file == file),
            "Missing SSOT owner {package}/{file}"
        );
    }
    assert!(
        sources
            .iter()
            .any(|s| s.package == "tmt-cli" && s.file == "identity_command.rs")
    );
    violations.extend(policy::source_violations(&sources));
    violations.extend(driver_names::violations(
        &sources,
        &tmt_core::driver::ALL.map(|driver| driver.name),
    ));
    violations.extend(host_names::violations(
        &sources,
        &tmt_core::host::HostKind::ALL.map(|host| host.as_str()),
    ));
    violations.extend(skill_lists::violations(
        &sources,
        &tmt_core::skill_catalog::BUNDLED
            .iter()
            .map(|skill| skill.name)
            .collect::<Vec<_>>(),
    ));
    violations.extend(output::violations(
        &sources,
        output_allowlist::EXACT_BODIES,
        output_allowlist::MIGRATING,
    ));
    assert!(
        violations.is_empty(),
        "Native architecture violations:\n{}",
        violations.join("\n")
    );
}
