//! Deterministic, independent source/metadata controls for declaration admission.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    root: PathBuf,
    map: Value,
    metadata: Value,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-never-shipped-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut fixture = Self {
            map: json!({"components":{"product": {
                "package":"product", "skills":true,
                "neverShippedPaths":[{"root":"extensions/product/rust/product/tests", "reason":"Integration tests",
                    "testOnlyReferences":[{"file":"extensions/product/rust/product/src/readers/test_support.rs", "reason":"Test module"}]}]
            }}}),
            metadata: json!({"packages":[{"name":"product", "manifest_path":root.join("extensions/product/rust/product/Cargo.toml"),
                "targets":[{"kind":["bin"],"src_path":root.join("extensions/product/rust/product/src/main.rs")}],
                "metadata":{"dist":{"include":["../../skills"]}}}]}),
            root,
        };
        fixture.write("dist-workspace.toml", "[dist]\ninclude = [\"LICENSE\"]\n");
        fixture.write(
            "extensions/product/rust/product/Cargo.toml",
            "[package]\nname = \"product\"\n",
        );
        fixture.write(
            "extensions/product/rust/product/src/main.rs",
            "const SKILL: &str = include_str!(\"../../../skills/product/SKILL.md\");\n",
        );
        fixture.write(
            "extensions/product/skills/product/SKILL.md",
            "Product skill",
        );
        fixture.write(
            "extensions/product/skills/product/references/guide.md",
            "Packaged reference",
        );
        fixture.write(
            "extensions/product/rust/product/tests/fixture.rs",
            "include_bytes!(concat!(env!(\"TEST_ONLY\"), \"/fixture\"));\n",
        );
        fixture.write(
            "extensions/product/rust/product/tests/support/mod.rs",
            "pub fn fixture() {}\n",
        );
        fixture.write(
            "extensions/product/rust/product/src/readers.rs",
            "#[cfg(test)]\npub(crate) mod test_support;\n",
        );
        fixture.write(
            "extensions/product/rust/product/src/readers/test_support.rs",
            "#[path = \"../../tests/support/mod.rs\"] mod helper;\n",
        );
        fixture
    }
    fn write(&mut self, file: &str, source: &str) {
        let path = self.root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    fn check(&self) -> GuardResult<()> {
        check_map(&self.root, &self.metadata, &self.map)
    }
    fn root(&mut self, name: &str) {
        self.map["components"]["product"]["neverShippedPaths"][0] =
            json!({"root":name, "reason":"Negative control"});
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove guard fixture");
    }
}

#[test]
fn source_guard_ignores_declared_sources_strings_and_comments_but_inspects_nested_macro_tokens() {
    let mut fixture = Fixture::new();
    fixture.write(
        "extensions/product/rust/product/src/strings.rs",
        r##"
        const FAKE: &str = r#"include!(env!("IGNORED")); #[path = "missing"]"#;
        // include_str!("missing");
        /* include_bytes!(env!("IGNORED")); */
        fn example() { println!("include!(env!(\"IGNORED\"))"); }
    "##,
    );
    fixture.check().unwrap();
    fixture.write(
        "extensions/product/rust/product/src/strings.rs",
        "fn bad() { wrapper!({ include_bytes!(\"../tests/support/mod.rs\") }); }",
    );
    assert!(
        fixture
            .check()
            .unwrap_err()
            .contains("references declared root")
    );
    fixture.write(
        "extensions/product/rust/product/src/strings.rs",
        "fn restored() {}",
    );
    fixture.check().unwrap();
}

#[test]
fn source_guard_refuses_production_path_and_cfg_removal_independently() {
    let mut fixture = Fixture::new();
    fixture.write(
        "extensions/product/rust/product/src/production.rs",
        "#[path = \"../tests/support/mod.rs\"] mod helper;",
    );
    assert!(
        fixture
            .check()
            .unwrap_err()
            .contains("references declared root")
    );
    fixture.write(
        "extensions/product/rust/product/src/production.rs",
        "pub fn restored() {}",
    );
    fixture.check().unwrap();
    fixture.write(
        "extensions/product/rust/product/src/readers.rs",
        "pub(crate) mod test_support;\n",
    );
    assert!(
        fixture
            .check()
            .unwrap_err()
            .contains("immediate #[cfg(test)]")
    );
    fixture.write(
        "extensions/product/rust/product/src/readers.rs",
        "const FAKE: &str = r#\"\n#[cfg(test)]\npub(crate) mod test_support;\n\"#;\npub(crate) mod test_support;\n",
    );
    assert!(fixture.check().unwrap_err().contains("actual module"));
    fixture.write(
        "extensions/product/rust/product/src/readers.rs",
        "#[cfg(test)]\npub(crate) mod test_support;\n",
    );
    fixture.check().unwrap();
    fixture.write(
        "extensions/product/rust/product/src/production.rs",
        "include!(\"readers/test_support.rs\");",
    );
    assert!(fixture.check().unwrap_err().contains("alternate entry"));
}

#[test]
fn source_guard_refuses_skill_tree_ancestors_descendants_and_production_inputs() {
    for root in [
        "extensions/product/skills/product",
        "extensions/product/skills/product/references",
        "extensions/product/skills",
        "extensions/product/rust/product/src",
        "extensions/product/rust/product/Cargo.toml",
    ] {
        let mut fixture = Fixture::new();
        fixture.root(root);
        assert!(
            fixture.check().unwrap_err().contains("shipped input"),
            "{root}"
        );
    }
    let mut fixture = Fixture::new();
    fixture.metadata["packages"][0]["metadata"]["dist"]["include"] = json!(["tests"]);
    assert!(fixture.check().unwrap_err().contains("shipped input"));
    fixture.metadata["packages"][0]["metadata"]["dist"]["include"] = json!(["../../skills"]);
    fixture.check().unwrap();
    fixture.write(
        "extensions/product/rust/product/Cargo.toml",
        "[package]\nname=\"product\"\ninclude=[\"tests\"]\n",
    );
    assert!(fixture.check().unwrap_err().contains("shipped input"));
}

#[test]
fn source_guard_refuses_missing_and_dynamic_references_without_silently_dropping_proof() {
    let mut fixture = Fixture::new();
    for source in [
        "include!(env!(\"UNPROVED\"));",
        "include_str!(\"missing.txt\");",
        "#[cfg_attr(test, path=\"missing.rs\")] mod helper;",
        "wrapper! { #[cfg_attr(test, path=\"missing.rs\")] mod helper; }",
    ] {
        fixture.write("extensions/product/rust/product/src/production.rs", source);
        assert!(fixture.check().is_err(), "{source}");
    }
    fixture.write(
        "extensions/product/rust/product/src/production.rs",
        "pub fn restored() {}",
    );
    fixture.check().unwrap();
}

// Canonical pipeline controls use the actual reviewed source and mutate one input at a time.
#[test]
fn canonical_generated_pipeline_rejects_drift_and_app_declarations() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let mut fixture = Fixture::new();
    let crate_dir = "extensions/product/rust/product";
    let app = "extensions/tmt-colab/typescript/app";
    fixture.write(&format!("{app}/package.json"), "{}");
    let build = fs::read_to_string(repository.join("extensions/tmt-colab/rust/tmt-colab/build.rs"))
        .unwrap();
    let assets =
        fs::read_to_string(repository.join("extensions/tmt-colab/rust/tmt-colab/build/assets.rs"))
            .unwrap();
    let hosting =
        fs::read_to_string(repository.join("extensions/tmt-colab/rust/tmt-colab/build/hosting.rs"))
            .unwrap();
    fixture.write(&format!("{crate_dir}/build/hosting.rs"), &hosting);
    fixture.write(
        &format!("{crate_dir}/src/browser_policy.rs"),
        "pub const POLICY: &str = \"policy\";\n",
    );
    fixture.write(
        &format!("{crate_dir}/src/hosting.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/colab_hosting.rs\"));\n",
    );
    fixture.write(&format!("{crate_dir}/build.rs"), &build);
    fixture.write(&format!("{crate_dir}/build/assets.rs"), &assets);
    fixture.write(
        &format!("{crate_dir}/src/app_inventory.rs"),
        "pub fn validate() {}\n",
    );
    fixture.write(
        &format!("{crate_dir}/src/assets.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/colab_assets.rs\"));\n",
    );
    fixture.write("scripts/build-native-artifact.sh", &format!("unset TMT_COLAB_HOSTING_DIR\nTMT_COLAB_APP_DIR=\"$repo/{app}/dist\"\nexport TMT_COLAB_APP_DIR\ncorepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build 1>&2\n"));
    fixture.metadata["packages"][0]["targets"].as_array_mut().unwrap().push(json!({"kind":["custom-build"],"src_path":fixture.root.join(format!("{crate_dir}/build.rs"))}));
    fixture.map["components"]["product"]["generatedInputs"] = json!([{
        "includeSite":format!("{crate_dir}/src/assets.rs"), "expression":"concat!(env!(\"OUT_DIR\"), \"/colab_assets.rs\")",
        "generator":format!("{crate_dir}/build/assets.rs"), "generatorBlob":"8a934c2344d9d0be29bee61d4a054213c2c8b109", "buildScript":format!("{crate_dir}/build.rs"),
        "variable":"TMT_COLAB_APP_DIR", "inputDirectory":format!("{app}/dist"), "packageRoot":app,
        "releaseScript":"scripts/build-native-artifact.sh", "reason":"Canonical pipeline"
    }, {
        "includeSite":format!("{crate_dir}/src/hosting.rs"), "expression":"concat!(env!(\"OUT_DIR\"), \"/colab_hosting.rs\")",
        "generator":format!("{crate_dir}/build/hosting.rs"), "generatorBlob":"0ecd28b7efdda910276cfbd8ae576011d6f9e1ce", "buildScript":format!("{crate_dir}/build.rs"),
        "variable":"TMT_COLAB_HOSTING_DIR", "inputDirectory":format!("{app}/dist-hosted"), "packageRoot":app,
        "releaseScript":"scripts/build-native-artifact.sh", "reason":"Explicit hosted input excluded from native releases"
    }]);
    fixture.check().unwrap();
    fixture.write(
        &format!("{crate_dir}/build/assets.rs"),
        &format!("{assets}\nfn another_generator() {{}}\n"),
    );
    assert!(
        fixture
            .check()
            .unwrap_err()
            .contains("asset generator drift")
    );
    fixture.write(&format!("{crate_dir}/build/assets.rs"), &assets);
    fixture.check().unwrap();

    fixture.write(
        &format!("{crate_dir}/src/extra.rs"),
        "include_bytes!(env!(\"UNPROVED\"));",
    );
    assert!(fixture.check().unwrap_err().contains("unproved dynamic"));
    fixture.write(&format!("{crate_dir}/src/extra.rs"), "pub fn restored() {}");
    fixture.check().unwrap();
    fixture.write(
        &format!("{crate_dir}/build.rs"),
        &build.replace("TMT_COLAB_APP_DIR", "OTHER_APP"),
    );
    assert!(fixture.check().unwrap_err().contains("forwarding drift"));
    fixture.write(
        &format!("{crate_dir}/build.rs"),
        &format!("{build}\nfn another_generator() {{}}\n"),
    );
    assert!(fixture.check().unwrap_err().contains("forwarding drift"));
    fixture.write(&format!("{crate_dir}/build.rs"), &build);
    let release =
        fs::read_to_string(fixture.root.join("scripts/build-native-artifact.sh")).unwrap();
    for changed in [
        release.replace("unset TMT_COLAB_HOSTING_DIR\n", ""),
        format!(
            "{}unset TMT_COLAB_HOSTING_DIR\n",
            release.replace("unset TMT_COLAB_HOSTING_DIR\n", "")
        ),
    ] {
        fixture.write("scripts/build-native-artifact.sh", &changed);
        assert!(
            fixture
                .check()
                .unwrap_err()
                .contains("hosted release exclusion drift")
        );
    }
    fixture.write("scripts/build-native-artifact.sh", &release);
    fixture.write(
        &format!("{crate_dir}/build/hosting.rs"),
        &format!("{hosting}\nfn undeclared_generator() {{}}\n"),
    );
    assert!(
        fixture
            .check()
            .unwrap_err()
            .contains("asset generator drift")
    );
    fixture.write(&format!("{crate_dir}/build/hosting.rs"), &hosting);
    fixture.write(
        "scripts/build-native-artifact.sh",
        &release.replace("/dist\"", "/other\""),
    );
    assert!(fixture.check().unwrap_err().contains("directory pin drift"));
    fixture.write(
        "scripts/build-native-artifact.sh",
        &release.replace("corepack pnpm", "# corepack pnpm"),
    );
    assert!(fixture.check().unwrap_err().contains("directory pin drift"));
    fixture.write("scripts/build-native-artifact.sh", &release);
    fixture.check().unwrap();
    for root in [
        "extensions/tmt-remote/rust/tmt-remote/assets".to_owned(),
        format!("{app}/dist-hosted"),
        app.to_owned(),
        format!("{app}/dist"),
        format!("{app}/dist/assets"),
    ] {
        fs::create_dir_all(fixture.root.join(&root)).unwrap();
        fixture.root(&root);
        assert!(fixture.check().unwrap_err().contains("shipped input"));
    }
}
