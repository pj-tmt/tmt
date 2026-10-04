#[path = "architecture/board_widgets.rs"]
mod board_widgets;
#[path = "architecture/cases.rs"]
mod cases;
#[path = "architecture/colors.rs"]
mod colors;
#[path = "architecture/driver_names.rs"]
mod driver_names;
#[path = "architecture/extension_host.rs"]
mod extension_host;
#[path = "architecture/host_names.rs"]
mod host_names;
#[path = "architecture/interaction.rs"]
mod interaction;
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
#[path = "architecture/unsafe_boundary.rs"]
mod unsafe_boundary;

use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::process::{CommandRequest, CommandRunner, UnixCommandRunner};

// Reviewed package boundaries and manifest owners; implementation files are not fixed.
const WORKSPACE_MANIFESTS: &[(&str, &str)] = &[
    (
        "tmt-release-tool",
        "rust/crates/tmt-release-tool/Cargo.toml",
    ),
    (
        "tmt-test-support",
        "rust/crates/tmt-test-support/Cargo.toml",
    ),
    (
        "tmt-colab",
        "extensions/tmt-colab/rust/tmt-colab/Cargo.toml",
    ),
    ("tmt-core", "rust/crates/tmt-core/Cargo.toml"),
    ("tmt-sys", "rust/crates/tmt-sys/Cargo.toml"),
    ("tmt-adapters", "rust/crates/tmt-adapters/Cargo.toml"),
    ("tmt-cli", "rust/crates/tmt-cli/Cargo.toml"),
    ("tmt-cli-style", "rust/crates/tmt-cli-style/Cargo.toml"),
    ("tmt-invoke", "rust/crates/tmt-invoke/Cargo.toml"),
    (
        "tmt-extension-state",
        "rust/crates/tmt-extension-state/Cargo.toml",
    ),
    ("tmt-tui", "rust/crates/tmt-tui/Cargo.toml"),
    (
        "tmt-command-output",
        "rust/crates/tmt-command-output/Cargo.toml",
    ),
    (
        "tmt-driver-protocol",
        "rust/crates/tmt-driver-protocol/Cargo.toml",
    ),
    (
        "tmt-driver-herdr",
        "rust/crates/tmt-driver-herdr/Cargo.toml",
    ),
    (
        "tmt-host-grammar",
        "rust/crates/tmt-host-grammar/Cargo.toml",
    ),
    (
        "tmt-office",
        "extensions/tmt-office/rust/tmt-office/Cargo.toml",
    ),
    (
        "tmt-office-command",
        "extensions/tmt-office/rust/tmt-office-command/Cargo.toml",
    ),
    (
        "tmt-office-model",
        "extensions/tmt-office/rust/tmt-office-model/Cargo.toml",
    ),
    (
        "tmt-office-pairing",
        "extensions/tmt-office/rust/tmt-office-pairing/Cargo.toml",
    ),
    (
        "tmt-office-service",
        "extensions/tmt-office/rust/tmt-office-service/Cargo.toml",
    ),
    (
        "tmt-office-storage",
        "extensions/tmt-office/rust/tmt-office-storage/Cargo.toml",
    ),
    (
        "tmt-squad",
        "extensions/tmt-squad/rust/tmt-squad/Cargo.toml",
    ),
    (
        "tmt-colab-model",
        "extensions/tmt-colab/rust/tmt-colab-model/Cargo.toml",
    ),
    (
        "tmt-remote",
        "extensions/tmt-remote/rust/tmt-remote/Cargo.toml",
    ),
];

fn manifest_location_violation(package: &serde_json::Value, repository: &Path) -> Option<String> {
    let name = package["name"].as_str().expect("Cargo package name");
    let (_, relative) = WORKSPACE_MANIFESTS
        .iter()
        .find(|(reviewed, _)| *reviewed == name)
        .expect("workspace package names were reviewed");
    let expected = repository.join(relative);
    let actual = Path::new(
        package["manifest_path"]
            .as_str()
            .expect("Cargo manifest path"),
    );
    (actual != expected).then(|| {
        format!(
            "{name} manifest is {}, expected {}. Restore its documented owner directory or \
             review the architecture ownership docs and change the ({name:?}, {relative:?}) entry in \
             WORKSPACE_MANIFESTS in rust/crates/tmt-cli/tests/architecture.rs.",
            actual.display(),
            expected.display(),
        )
    })
}

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
        WORKSPACE_MANIFESTS
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>(),
        "Review native package boundaries when changing workspace members"
    );
    let repository = Path::new(
        metadata["workspace_root"]
            .as_str()
            .expect("Cargo workspace root"),
    )
    .parent()
    .expect("the Rust workspace has a repository parent");
    for package in metadata["packages"].as_array().expect("Cargo packages") {
        violations.extend(manifest_location_violation(package, repository));
        violations.extend(policy::dependency_violations(package));
        violations.extend(policy::test_support_package_violations(package));
        violations.extend(policy::release_tool_package_violations(package));
        violations.extend(unsafe_boundary::check(package, repository));
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
    let built_in_hosts = tmt_core::host::HostKind::ALL;
    violations.extend(host_names::violations(
        &sources,
        &built_in_hosts
            .iter()
            .map(|host| host.as_str())
            .collect::<Vec<_>>(),
    ));
    let extensions =
        extension_host::sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extensions"));
    assert!(
        extensions
            .iter()
            .any(|(file, _)| file.ends_with("local_service/core/in_process.rs")),
        "the extension guard reads test sources"
    );
    violations.extend(extension_host::violations(&extensions));
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
    violations.extend(interaction::violations(&sources, interaction::MIGRATING));
    // The extensions outside the workspace draw too; they obey the same rule.
    let mut drawn = sources;
    for (package, root) in [
        ("tmt-squad", "tmt-squad/rust/tmt-squad/src/main.rs"),
        ("tmt-remote", "tmt-remote/rust/tmt-remote/src/lib.rs"),
        ("tmt-remote", "tmt-remote/rust/tmt-remote/src/main.rs"),
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extensions")
            .join(root);
        drawn.extend(source::collect(package, &root).expect("collect extension source"));
    }
    assert!(
        drawn
            .iter()
            .any(|s| s.package == "tmt-squad" && s.file.ends_with("view.rs")),
        "the color guard reads the Squad board"
    );
    violations.extend(colors::violations(&drawn));
    violations.extend(board_widgets::violations(&drawn, board_widgets::EXCEPTIONS));
    assert!(
        violations.is_empty(),
        "Native architecture violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn core_manifest_in_its_documented_owner_is_accepted() {
    let package = serde_json::json!({
        "name": "tmt-core",
        "manifest_path": "/repository/rust/crates/tmt-core/Cargo.toml",
    });
    assert_eq!(
        manifest_location_violation(&package, Path::new("/repository")),
        None
    );
}

#[test]
fn relocated_core_manifest_names_the_reviewed_entry_to_change() {
    let package = serde_json::json!({
        "name": "tmt-core",
        "manifest_path": "/repository/extensions/tmt-core/Cargo.toml",
    });
    assert_eq!(
        manifest_location_violation(&package, Path::new("/repository")),
        Some(
            concat!(
                "tmt-core manifest is /repository/extensions/tmt-core/Cargo.toml, ",
                "expected /repository/rust/crates/tmt-core/Cargo.toml. ",
                "Restore its documented owner directory or review the architecture ownership docs and change the ",
                "(\"tmt-core\", \"rust/crates/tmt-core/Cargo.toml\") entry in ",
                "WORKSPACE_MANIFESTS in rust/crates/tmt-cli/tests/architecture.rs.",
            )
            .into()
        ),
    );
}
