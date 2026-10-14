//! Default-off proof support must never enter a product dependency or build.
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};

fn enables(features: &Value, start: &str, seen: &mut BTreeSet<String>) -> bool {
    if start == "test-emulator" {
        return true;
    }
    if !seen.insert(start.into()) {
        return false;
    }
    features[start].as_array().is_some_and(|values| {
        values.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|name| enables(features, name, seen))
        })
    })
}
fn dependency_enables(package: &Value, remote_features: &Value) -> bool {
    for dependency in package["dependencies"].as_array().unwrap() {
        if dependency["name"] != "tmt-remote" || dependency["kind"] == "dev" {
            continue;
        }
        if dependency["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| {
                enables(
                    remote_features,
                    feature.as_str().unwrap(),
                    &mut BTreeSet::new(),
                )
            })
        {
            return true;
        }
        let alias = dependency["rename"].as_str().unwrap_or("tmt-remote");
        for values in package["features"].as_object().unwrap().values() {
            for value in values.as_array().unwrap() {
                if let Some((name, feature)) = value.as_str().unwrap().split_once('/')
                    && name.trim_end_matches('?') == alias
                    && enables(remote_features, feature, &mut BTreeSet::new())
                {
                    return true;
                }
            }
        }
    }
    false
}
fn command_enables(source: &str) -> bool {
    // Conservative for generated Rust argv as well as shell/YAML command text.
    // Quotes/brackets do not hide a feature or an all-features flag.
    let words: Vec<_> = source
        .split(|c: char| !c.is_ascii_alphanumeric() && !"-_=/,.".contains(c))
        .filter(|s| !s.is_empty())
        .collect();
    words.contains(&"--all-features")
        || (words
            .iter()
            .any(|word| *word == "--features" || word.starts_with("--features="))
            && words.iter().any(|word| {
                word.split([',', '=', '/'])
                    .any(|part| part == "test-emulator")
            }))
}
fn check_commands(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            check_commands(&path);
        } else if matches!(
            path.extension().and_then(|v| v.to_str()),
            Some("rs" | "yml" | "yaml" | "sh" | "mjs")
        ) {
            assert!(
                !command_enables(&fs::read_to_string(&path).unwrap()),
                "test-emulator may ship through {}",
                path.display()
            );
        }
    }
}

#[test]
fn test_emulator_is_never_enabled_by_default_dependency_or_release_commands() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("rust/Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    let remote = packages.iter().find(|p| p["name"] == "tmt-remote").unwrap();
    assert!(
        remote["features"]["test-emulator"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(!enables(
        &remote["features"],
        "default",
        &mut BTreeSet::new()
    ));
    for package in packages {
        assert!(
            !dependency_enables(package, &remote["features"]),
            "non-dev feature edge from {}",
            package["name"]
        );
    }
    check_commands(&root.join(".github"));
    check_commands(&root.join("rust/crates/tmt-release-tool"));
}

#[test]
fn shipping_guard_detects_direct_transitive_and_command_flag_perturbations() {
    let features = serde_json::json!({"default":["alias"], "alias":["test-emulator"], "safe":[], "test-emulator":[]});
    assert!(enables(&features, "default", &mut BTreeSet::new()));
    assert!(!enables(&features, "safe", &mut BTreeSet::new()));
    for command in [
        "cargo build --all-features",
        "cargo build --features test-emulator",
        "cargo build --features=safe,test-emulator",
        "cargo build --features 'safe tmt-remote/test-emulator'",
        "command.args([\"--features\", \"test-emulator\"])",
    ] {
        assert!(command_enables(command), "{command}");
    }
    assert!(!command_enables(
        "cargo install cargo-about --features cli --locked"
    ));
    assert!(!command_enables("cargo build --locked -p tmt-remote"));
}

#[test]
fn shipping_guard_checks_normal_build_alias_and_dev_dependency_kinds() {
    let remote = serde_json::json!({"safe":[], "alias":["test-emulator"], "test-emulator":[]});
    let mut consumer = serde_json::json!({"name":"consumer", "features":{}, "dependencies":[{
        "name":"tmt-remote", "kind":null, "rename":"remote", "features":["safe"]
    }]});
    assert!(!dependency_enables(&consumer, &remote));
    consumer["dependencies"][0]["features"] = serde_json::json!(["test-emulator"]);
    assert!(dependency_enables(&consumer, &remote));
    consumer["dependencies"][0]["kind"] = serde_json::json!("build");
    consumer["dependencies"][0]["features"] = serde_json::json!(["alias"]);
    assert!(dependency_enables(&consumer, &remote));
    consumer["dependencies"][0]["kind"] = serde_json::json!("dev");
    assert!(!dependency_enables(&consumer, &remote));
    consumer["dependencies"][0]["kind"] = Value::Null;
    consumer["dependencies"][0]["features"] = serde_json::json!(["safe"]);
    consumer["features"] = serde_json::json!({"proof":["remote?/alias"]});
    assert!(dependency_enables(&consumer, &remote));
    consumer["features"] = serde_json::json!({"proof":["remote/safe"]});
    assert!(!dependency_enables(&consumer, &remote));
}

#[test]
fn three_production_sites_share_the_project_bound_http_constructor() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["sync_cli.rs", "serve/native_factory.rs"] {
        let source = fs::read_to_string(src.join(file)).unwrap();
        assert!(
            !source.contains("UreqHttp::new"),
            "unrouted production HTTP in {file}"
        );
        assert!(
            source.contains("ProductionHttp::new"),
            "missing shared constructor in {file}"
        );
    }
}
