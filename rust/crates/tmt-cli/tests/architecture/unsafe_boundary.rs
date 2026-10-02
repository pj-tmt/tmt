//! One audited FFI leaf; Cargo's forbid lint enforces every other crate.
use serde_json::Value;
use std::{fs, path::Path};
use toml_edit::{DocumentMut, Item};

fn level(item: Option<&Item>) -> Option<&str> {
    let item = item?;
    item.as_str().or_else(|| item.get("level")?.as_str())
}

fn rust_lint<'a>(doc: &'a DocumentMut, name: &str) -> Option<&'a Item> {
    doc.get("lints")?.get("rust")?.get(name)
}

fn violations(workspace: &str, manifest: &str, package: &Value, build_file: bool) -> Vec<String> {
    let workspace: DocumentMut = workspace.parse().expect("workspace TOML");
    let manifest: DocumentMut = manifest.parse().expect("package TOML");
    let name = package["name"].as_str().expect("package name");
    let inherited = manifest
        .get("lints")
        .and_then(|v| v.get("workspace"))
        .and_then(Item::as_bool)
        == Some(true);
    let workspace_lint = workspace
        .get("workspace")
        .and_then(|v| v.get("lints"))
        .and_then(|v| v.get("rust"))
        .and_then(|v| v.get("unsafe_code"));
    let mut errors = Vec::new();
    let dependencies = package["dependencies"].as_array().expect("dependencies");
    if name != "tmt-adapters" && dependencies.iter().any(|d| d["name"] == "tmt-sys") {
        errors.push(format!("{name}: only tmt-adapters may depend on tmt-sys"));
    }
    if level(workspace_lint) != Some("forbid") {
        errors.push("workspace: unsafe_code must remain forbid".into());
    }
    if name != "tmt-sys" {
        if !inherited && level(rust_lint(&manifest, "unsafe_code")) != Some("forbid") {
            errors.push(format!(
                "{name}: must inherit unsafe_code=forbid or explicitly forbid it"
            ));
        }
        return errors;
    }
    if inherited
        || level(rust_lint(&manifest, "unsafe_code")) != Some("deny")
        || level(rust_lint(&manifest, "unsafe_op_in_unsafe_fn")) != Some("deny")
    {
        errors
            .push("tmt-sys: audited leaf must deny unsafe except at its reviewed function".into());
    }
    if dependencies.len() != 1
        || dependencies
            .iter()
            .any(|d| d["name"] != "libc" || !d["rename"].is_null() || !d["kind"].is_null())
    {
        errors.push("tmt-sys: only the normal libc dependency is permitted".into());
    }
    if build_file
        || package["targets"]
            .as_array()
            .expect("targets")
            .iter()
            .any(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|k| k == "custom-build"))
            })
    {
        errors.push("tmt-sys: build.rs and custom build scripts are forbidden".into());
    }
    errors
}

pub fn check(package: &Value, repository: &Path) -> Vec<String> {
    let manifest = Path::new(package["manifest_path"].as_str().expect("manifest path"));
    violations(
        &fs::read_to_string(repository.join("rust/Cargo.toml")).expect("workspace manifest"),
        &fs::read_to_string(manifest).expect("package manifest"),
        package,
        manifest.parent().unwrap().join("build.rs").exists(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const WORKSPACE: &str = "[workspace.lints.rust]\nunsafe_code='forbid'";
    const SYS: &str = "[lints.rust]\nunsafe_code='deny'\nunsafe_op_in_unsafe_fn='deny'";
    fn sys() -> Value {
        json!({"name":"tmt-sys", "dependencies":[{"name":"libc","kind":null,"rename":null}],"targets":[]})
    }

    #[test]
    fn every_other_crate_must_retain_forbid() {
        let package = json!({"name":"tmt-adapters", "dependencies": []});
        for text in [
            "[lints]\nworkspace=true",
            "[lints.rust]\nunsafe_code='forbid'",
            "[lints.rust]\nunsafe_code={level='forbid',priority=1}",
        ] {
            assert!(violations(WORKSPACE, text, &package, false).is_empty());
        }
        for text in [
            "",
            "[lints.rust]\nunsafe_code='allow'",
            "[lints.rust]\nunsafe_code='deny'",
        ] {
            assert!(!violations(WORKSPACE, text, &package, false).is_empty());
        }
        assert!(
            !violations(
                "[workspace.lints.rust]\nunsafe_code='deny'",
                "[lints]\nworkspace=true",
                &package,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn sys_is_the_only_exception_and_has_no_dependency_escape() {
        assert!(violations(WORKSPACE, SYS, &sys(), false).is_empty());
        for dependency in [
            json!({"name":"tmt-core"}),
            json!({"name":"libc","kind":"build"}),
            json!({"name":"libc","kind":"dev"}),
            json!({"name":"libc","rename":"native"}),
        ] {
            let mut package = sys();
            package["dependencies"] = json!([dependency]);
            assert!(!violations(WORKSPACE, SYS, &package, false).is_empty());
        }
        assert!(!violations(WORKSPACE, "[lints]\nworkspace=true", &sys(), false).is_empty());
    }

    #[test]
    fn only_adapters_can_depend_on_sys_in_any_dependency_kind_or_target() {
        let manifest = "[lints]\nworkspace=true";
        for kind in [Value::Null, json!("dev"), json!("build")] {
            for rename in [Value::Null, json!("ffi")] {
                let dependency = json!({"name":"tmt-sys", "kind":kind, "rename":rename,
                    "target":"cfg(target_os = \"macos\")"});
                let adapter = json!({"name":"tmt-adapters", "dependencies":[dependency.clone()]});
                assert!(violations(WORKSPACE, manifest, &adapter, false).is_empty());
                for name in [
                    "tmt-core",
                    "tmt-cli",
                    "tmt-squad",
                    "tmt-remote",
                    "tmt-colab",
                    "tmt-office",
                    "future-extension",
                ] {
                    let package = json!({"name":name, "dependencies":[dependency.clone()]});
                    assert_eq!(
                        violations(WORKSPACE, manifest, &package, false),
                        [format!("{name}: only tmt-adapters may depend on tmt-sys")]
                    );
                }
            }
        }
    }

    #[test]
    fn sys_rejects_default_or_renamed_build_scripts() {
        assert!(!violations(WORKSPACE, SYS, &sys(), true).is_empty());
        let mut package = sys();
        package["targets"] = json!([{"kind":["custom-build"],"src_path":"somewhere/generate.rs"}]);
        assert!(!violations(WORKSPACE, SYS, &package, false).is_empty());
    }
}
