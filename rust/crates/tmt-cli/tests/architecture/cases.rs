//! Adversarial and positive examples for the native architecture guards.
//!
//! These examples intentionally exercise the public test-only collector and
//! policy APIs.  The filesystem fixtures are private to this module so the
//! production adapter test directory cannot become part of the guard's API.

use super::{policy, source};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn syntax(package: &str, file: &str, text: &str) -> source::Source {
    source::Source {
        package: package.into(),
        file: file.into(),
        syntax: syn::parse_file(text)
            .unwrap_or_else(|error| panic!("fixture {package}/{file} must parse: {error}")),
    }
}

fn assert_exact(sources: &[source::Source], expected: &[&str]) {
    let actual = policy::source_violations(sources);
    let expected = expected
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn core_rejects_grouped_renamed_reexport_and_qualified_io_references() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "storage.rs",
            "use std::{fs::File, io::Read};\n",
        )],
        &[
            "tmt-core/storage.rs: non-pure core reference std::fs::File",
            "tmt-core/storage.rs: non-pure core reference std::io::Read",
        ],
    );
    assert_exact(
        &[syntax(
            "tmt-core",
            "storage.rs",
            "use std::fs::File as Handle;\n",
        )],
        &["tmt-core/storage.rs: non-pure core reference std::fs::File"],
    );
    assert_exact(
        &[syntax(
            "tmt-core",
            "storage.rs",
            "pub use std::fs::File as Handle;\n",
        )],
        &["tmt-core/storage.rs: non-pure core reference std::fs::File"],
    );
    assert_exact(
        &[syntax(
            "tmt-core",
            "storage.rs",
            "pub fn read() { let _ = std::fs::read_to_string(\"state\"); }\n",
        )],
        &["tmt-core/storage.rs: non-pure core reference std::fs::read_to_string"],
    );
    assert_exact(
        &[syntax(
            "tmt-core",
            "nested.rs",
            "mod nested { use std::fs::File; }\n",
        )],
        &["tmt-core/nested.rs: non-pure core reference std::fs::File"],
    );
    assert_exact(
        &[syntax(
            "tmt-core",
            "output.rs",
            "pub fn report() { println!(\"done\"); std::process::exit(1); }\n",
        )],
        &[
            "tmt-core/output.rs: non-pure core reference println",
            "tmt-core/output.rs: non-pure core reference std::process::exit",
        ],
    );
    assert_exact(
        &[syntax("tmt-core", "legacy.rs", "extern crate std;\n")],
        &["tmt-core/legacy.rs: non-pure core reference std"],
    );
}

#[test]
fn core_allows_grouped_renamed_reexport_and_qualified_pure_std_references() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "text.rs",
            r#"
                use std::{borrow::Cow, collections::BTreeMap, result::Result as StdResult};
                pub use std::vec::Vec as List;
                pub fn names() -> StdResult<List<Cow<'static, str>>, BTreeMap<String, String>> {
                    let _ = std::option::Option::<String>::None;
                    let _ = std::result::Result::<(), ()>::Ok(());
                    // std::fs::File in a comment and a string are not syntax references.
                    let _text = "std::fs::File";
                    Ok(Vec::new())
                }
                mod nested {
                    use std::collections::BTreeSet as Set;
                    pub fn nested_names() -> Set<String> { Set::new() }
                }
            "#,
        )],
        &[],
    );
}

#[test]
fn adapters_allow_legitimate_grouped_reexports_and_dtos() {
    assert_exact(
        &[syntax(
            "tmt-adapters",
            "dto.rs",
            r#"
                use tmt_core::{identity::Identity as CoreIdentity, names::Name as CoreName};
                pub use tmt_core::Result as CoreResult;
                pub struct AdapterDto { pub identity: CoreIdentity, pub name: CoreName }
                pub fn load() -> CoreResult<AdapterDto> { todo!() }
            "#,
        )],
        &[],
    );
}

#[test]
fn parsing_modules_reject_effects_and_handler_clap() {
    assert_exact(
        &[syntax(
            "tmt-cli",
            "grammar/completion.rs",
            "use clap_complete::Shell;",
        )],
        &[],
    );
    assert_exact(
        &[syntax(
            "tmt-cli",
            "grammar/completion.rs",
            "use tmt_adapters::storage::Store;",
        )],
        &[
            "tmt-cli/grammar/completion.rs: parsing depends on effects via tmt_adapters::storage::Store",
        ],
    );
    assert_exact(
        &[syntax(
            "tmt-cli",
            "completion.rs",
            "use clap_complete::Shell;",
        )],
        &["tmt-cli/completion.rs: CLI parsing belongs to grammar/parser, not clap_complete::Shell"],
    );
    for (file, text, expected) in [
        (
            "grammar.rs",
            "use tmt_adapters::process::CommandRunner;\n",
            "tmt-cli/grammar.rs: parsing depends on effects via tmt_adapters::process::CommandRunner",
        ),
        (
            "parser.rs",
            "use crate::identity_command::execute;\n",
            "tmt-cli/parser.rs: parsing depends on effects via crate::identity_command::execute",
        ),
        (
            "diagnostics.rs",
            "use super::config_command::execute;\n",
            "tmt-cli/diagnostics.rs: parsing depends on effects via super::config_command::execute",
        ),
        (
            "invocation.rs",
            "use tmt_adapters::storage::Store;\n",
            "tmt-cli/invocation.rs: parsing depends on effects via tmt_adapters::storage::Store",
        ),
    ] {
        assert_exact(&[syntax("tmt-cli", file, text)], &[expected]);
    }

    assert_exact(
        &[syntax(
            "tmt-cli",
            "identity_command.rs",
            "use clap::Parser;\n",
        )],
        &["tmt-cli/identity_command.rs: CLI parsing belongs to grammar/parser, not clap::Parser"],
    );
}

#[test]
fn owner_policy_derives_duplicate_types_aliases_traits_and_functions() {
    let first = syntax(
        "tmt-core",
        "identity.rs",
        r#"
            pub struct Identity;
            pub enum State { Ready }
            pub union Storage { value: u64 }
            pub type Name = String;
            pub trait Marker {}
            pub trait Alias = Send;
            pub fn identity_name() {}
        "#,
    );
    let second = syntax(
        "tmt-core",
        "identity_copy.rs",
        r#"
            pub struct Identity;
            pub enum State { Ready }
            pub union Storage { value: u64 }
            pub type Name = String;
            pub trait Marker {}
            pub trait Alias = Send;
            pub fn identity_name() {}
        "#,
    );
    assert_exact(
        &[first, second],
        &[
            "tmt-core/identity_copy.rs: duplicate owner Alias (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner Identity (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner Marker (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner Name (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner State (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner Storage (already tmt-core/identity.rs)",
            "tmt-core/identity_copy.rs: duplicate owner identity_name (already tmt-core/identity.rs)",
        ],
    );
}

#[test]
fn owner_policy_rejects_nested_and_foreign_declarations() {
    assert_exact(
        &[
            syntax("tmt-core", "identity.rs", "pub struct Identity;\n"),
            syntax(
                "tmt-core",
                "other.rs",
                "mod nested { pub struct Identity; }\n",
            ),
        ],
        &["tmt-core/other.rs: duplicate owner Identity (already tmt-core/identity.rs)"],
    );
    assert_exact(
        &[
            syntax("tmt-core", "identity.rs", "pub struct Identity;\n"),
            syntax("tmt-adapters", "dto.rs", "pub type Identity = String;\n"),
        ],
        &["tmt-adapters/dto.rs: Identity is owned by tmt-core/identity.rs"],
    );
}

#[test]
fn broad_string_failure_conversion_is_rejected() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "config.rs",
            r#"
                pub struct Failure;
                impl From<String> for Failure {
                    fn from(_value: String) -> Self { Failure }
                }
            "#,
        )],
        &[
            "tmt-core/config.rs: map String errors explicitly; do not implement From<String> for shared Failure",
        ],
    );
}

#[test]
fn typed_local_error_mapping_is_allowed() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "config.rs",
            r#"
                pub struct Failure;
                pub struct ConfigError;
                impl From<ConfigError> for Failure {
                    fn from(_error: ConfigError) -> Self { Failure }
                }
                fn invalid_setting(_message: String) -> Failure { Failure }
            "#,
        )],
        &[],
    );
}

#[test]
fn comments_text_and_test_only_items_do_not_trigger_policy() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "test_helpers.rs",
            r#"
                // std::fs::File and println! are comments, not references.
                #[cfg(test)]
                fn test_io() { let _ = std::fs::read_to_string("state"); println!("x"); }
                #[cfg(all(test, unix))]
                fn unix_test_io() { let _ = std::fs::read_to_string("state"); }
                pub fn pure() { let _ = std::collections::BTreeMap::<String, String>::new(); }
                const TEXT: &str = "std::fs::read_to_string";
            "#,
        )],
        &[],
    );
}

#[test]
fn associated_items_respect_cfg_without_hiding_production_bodies() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "methods.rs",
            r#"
        pub struct Subject;
        pub fn inspect() {}
        impl Subject {
            pub fn inspect() {}
            #[cfg(test)]
            fn test_io() { std::fs::read("fixture"); }
        }
        pub trait Reader {
            #[cfg(all(test, unix))]
            fn test_io() { std::fs::read("fixture"); }
        }
    "#,
        )],
        &[],
    );
    for text in [
        "impl Subject { #[cfg(any(test, unix))] fn read() { std::fs::read(\"fixture\"); } }",
        "trait Reader { #[cfg(not(test))] fn read() { std::fs::read(\"fixture\"); } }",
    ] {
        assert_exact(
            &[syntax("tmt-core", "methods.rs", text)],
            &["tmt-core/methods.rs: non-pure core reference std::fs::read"],
        );
    }
}

#[test]
fn public_inline_declarations_seed_ownership_without_reserving_methods() {
    assert_exact(
        &[
            syntax(
                "tmt-core",
                "inline.rs",
                "pub mod domain { pub struct Record; pub fn inspect() {} }",
            ),
            syntax(
                "tmt-adapters",
                "copy.rs",
                "pub struct Record; pub fn inspect() {}",
            ),
        ],
        &[
            "tmt-adapters/copy.rs: Record is owned by tmt-core/inline.rs",
            "tmt-adapters/copy.rs: inspect is owned by tmt-core/inline.rs",
        ],
    );
}

fn dependency(name: &str, kind: &str, target: Option<&str>, rename: Option<&str>) -> Value {
    json!({
        "name": name,
        "kind": kind,
        "target": target,
        "rename": rename,
    })
}

fn package(name: &str, dependencies: Vec<Value>) -> Value {
    json!({ "name": name, "dependencies": dependencies })
}

#[test]
fn dependency_policy_handles_normal_build_target_renamed_and_dev_entries() {
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![
                dependency("uuid", "normal", None, None),
                dependency("sha2", "normal", None, None),
                dependency("semver", "normal", None, None),
                dependency("serde_json", "dev", None, None)
            ],
        )),
        Vec::<String>::new(),
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("serde_json", "build", None, None)],
        )),
        vec![
            "tmt-core: unreviewed production dependency serde_json (kind=\"build\", target=null, rename=null)"
        ],
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("serde_json", "normal", Some("cfg(unix)"), None)],
        )),
        vec![
            "tmt-core: unreviewed production dependency serde_json (kind=\"normal\", target=\"cfg(unix)\", rename=null)"
        ],
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("serde_json", "normal", None, Some("json"))],
        )),
        vec![
            "tmt-core: unreviewed production dependency serde_json (kind=\"normal\", target=null, rename=\"json\")"
        ],
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("uuid", "normal", None, Some("ids"))],
        )),
        vec![
            "tmt-core: unreviewed production dependency uuid (kind=\"normal\", target=null, rename=\"ids\")"
        ],
    );
}

#[test]
fn dependency_policy_rejects_unknown_workspace_packages() {
    assert_eq!(
        policy::dependency_violations(&package("unknownpackage", Vec::new())),
        vec!["unreviewed workspace package unknownpackage"],
    );
}

#[test]
fn companion_uses_only_reviewed_local_service_dependencies() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-office",
            vec![
                dependency("tmt-core", "normal", None, None),
                dependency("tmt-adapters", "normal", None, None),
                dependency("base64", "normal", None, None),
                dependency("getrandom", "normal", None, None),
                dependency("httparse", "normal", None, None),
                dependency("serde", "normal", None, None),
                dependency("serde_json", "normal", None, None),
                dependency("uuid", "normal", None, None),
            ]
        ))
        .is_empty()
    );
    for name in ["tmt-cli", "rusqlite", "ureq", "url"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-office",
                vec![dependency(name, "normal", None, None)]
            )),
            vec![format!(
                "tmt-office: unreviewed production dependency {name} (kind=\"normal\", target=null, rename=null)"
            )]
        );
    }
}

#[test]
fn core_may_parse_link_values_but_cannot_fetch_them() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("url", "normal", None, None)]
        ))
        .is_empty()
    );
    assert!(
        !policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("ureq", "normal", None, None)]
        ))
        .is_empty()
    );
}

#[test]
fn receipt_dependencies_stay_at_their_reviewed_layer() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-adapters",
            vec![dependency("base64", "normal", None, None)]
        ))
        .is_empty()
    );
    for (owner, name) in [
        ("tmt-core", "base64"),
        ("tmt-cli", "sha2"),
        ("tmt-cli", "base64"),
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency(name, "normal", None, None)]
            ))
            .len(),
            1
        );
    }
}

#[test]
fn display_width_dependency_stays_in_cli_presentation() {
    for (owner, expected) in [("tmt-cli", 0), ("tmt-core", 1), ("tmt-adapters", 1)] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("unicode-width", "normal", None, None)]
            ))
            .len(),
            expected
        );
    }
}

#[test]
fn png_runtime_dependency_stays_in_the_image_adapter() {
    for (owner, expected) in [
        ("tmt-adapters", 0),
        ("tmt-core", 1),
        ("tmt-cli", 1),
        ("tmt-office", 1),
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("png", "normal", None, None)],
            ))
            .len(),
            expected,
            "{owner}",
        );
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-office",
            vec![dependency("png", "dev", None, None)],
        ))
        .is_empty()
    );
}

struct FixtureDirectory {
    path: PathBuf,
}

impl FixtureDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-native-architecture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).expect("create unique architecture fixture directory");
        Self { path }
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create architecture fixture parent");
        }
        fs::write(path, text).expect("write architecture fixture source");
    }

    fn root(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if std::thread::panicking() {
                eprintln!(
                    "Could not remove architecture fixture {}: {error}",
                    self.path.display()
                );
            } else {
                panic!(
                    "Could not remove architecture fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

#[test]
fn collector_reaches_inline_nested_and_external_modules() {
    let fixture = FixtureDirectory::new();
    fixture.write(
        "lib.rs",
        "pub mod inline { pub mod nested { pub fn leaf() {} } }\nmod outer;\n",
    );
    fixture.write("outer.rs", "pub mod inner;\n");
    fixture.write("outer/inner/mod.rs", "pub fn external_leaf() {}\n");

    let sources = source::collect("fixture", &fixture.root().join("lib.rs"))
        .expect("collect fixture modules");
    let files = sources
        .iter()
        .map(|source| source.file.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, vec!["lib.rs", "outer.rs", "outer/inner/mod.rs"]);
    assert!(sources.iter().all(|source| source.package == "fixture"));
}

#[test]
fn collector_applies_associated_item_cfg_to_nested_modules() {
    let fixture = FixtureDirectory::new();
    fixture.write(
        "lib.rs",
        r#"
        struct Subject;
        impl Subject {
            #[cfg(test)]
            fn test_only() { mod missing; }
        }
        trait Reader {
            #[cfg(all(test, unix))]
            fn test_only() { mod missing; }
        }
        mod inline { mod external; }
    "#,
    );
    fixture.write("inline/external.rs", "pub fn found() {}");
    let sources = source::collect("fixture", &fixture.root().join("lib.rs")).unwrap();
    assert_eq!(
        sources.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
        vec!["inline/external.rs", "lib.rs"]
    );
    fixture.write(
        "lib.rs",
        "impl Subject { #[cfg(any(test, unix))] fn maybe() { mod missing; } }",
    );
    let error = source::collect("fixture", &fixture.root().join("lib.rs"))
        .err()
        .unwrap();
    assert!(
        error.contains("module missing needs exactly one source file"),
        "{error}"
    );
}

#[test]
fn collector_skips_provably_test_only_all_branch_but_inspects_unknown_cfg_branches() {
    let fixture = FixtureDirectory::new();
    fixture.write(
        "lib.rs",
        "#[cfg(all(test, unix))] mod skipped;\n#[cfg(any(test, unix))] mod inspected;\n#[cfg(not(test))] mod production;\n",
    );
    fixture.write("inspected.rs", "pub fn inspected() {}\n");
    fixture.write("production.rs", "pub fn production() {}\n");

    let sources = source::collect("fixture", &fixture.root().join("lib.rs"))
        .expect("collect cfg fixture modules");
    let files = sources
        .iter()
        .map(|source| source.file.as_str())
        .collect::<Vec<_>>();
    assert_eq!(files, vec!["inspected.rs", "lib.rs", "production.rs"]);

    let any_missing = FixtureDirectory::new();
    any_missing.write("lib.rs", "#[cfg(any(test, unix))] mod inspected;\n");
    let error = source::collect("fixture", &any_missing.root().join("lib.rs"))
        .err()
        .expect("unknown any(test, unix) branch must be inspected");
    assert!(
        error.contains("module inspected needs exactly one source file"),
        "{error}"
    );

    let not_missing = FixtureDirectory::new();
    not_missing.write("lib.rs", "#[cfg(not(test))] mod production;\n");
    let error = source::collect("fixture", &not_missing.root().join("lib.rs"))
        .err()
        .expect("not(test) branch must be inspected");
    assert!(
        error.contains("module production needs exactly one source file"),
        "{error}"
    );
}

#[test]
fn collector_fails_closed_for_missing_ambiguous_invalid_and_remapped_modules() {
    let missing = FixtureDirectory::new();
    missing.write("lib.rs", "mod missing;\n");
    let error = source::collect("fixture", &missing.root().join("lib.rs"))
        .err()
        .expect("missing module must fail closed");
    assert!(
        error.contains("module missing needs exactly one source file"),
        "{error}"
    );

    let ambiguous = FixtureDirectory::new();
    ambiguous.write("lib.rs", "mod ambiguous;\n");
    ambiguous.write("ambiguous.rs", "pub fn one() {}\n");
    ambiguous.write("ambiguous/mod.rs", "pub fn two() {}\n");
    let error = source::collect("fixture", &ambiguous.root().join("lib.rs"))
        .err()
        .expect("ambiguous module must fail closed");
    assert!(
        error.contains("module ambiguous needs exactly one source file"),
        "{error}"
    );

    let invalid = FixtureDirectory::new();
    invalid.write("lib.rs", "pub fn malformed( {\n");
    let error = source::collect("fixture", &invalid.root().join("lib.rs"))
        .err()
        .expect("invalid source must fail closed");
    assert!(error.starts_with("lib.rs:"), "{error}");

    let remapped = FixtureDirectory::new();
    remapped.write("lib.rs", "#[path = \"renamed.rs\"] mod original;\n");
    remapped.write("renamed.rs", "pub fn original() {}\n");
    let error = source::collect("fixture", &remapped.root().join("lib.rs"))
        .err()
        .expect("path-remapped module must fail closed");
    assert_eq!(error, "lib.rs: unsupported module remapping on original");

    let included = FixtureDirectory::new();
    included.write("lib.rs", "include!(\"generated.rs\");\n");
    let error = source::collect("fixture", &included.root().join("lib.rs"))
        .err()
        .expect("include! must fail closed");
    assert_eq!(
        error,
        "lib.rs: source include! requires explicit collector support"
    );

    let generated_assets = FixtureDirectory::new();
    generated_assets.write(
        "local_assets.rs",
        "include!(concat!(env!(\"OUT_DIR\"), \"/office_assets.rs\"));\n",
    );
    assert!(
        source::collect(
            "tmt-office",
            &generated_assets.root().join("local_assets.rs")
        )
        .is_ok(),
        "the explicit generated Office asset collector must remain supported"
    );

    let verbatim = FixtureDirectory::new();
    verbatim.write("lib.rs", "pub fn declaration_only();\n");
    let error = source::collect("fixture", &verbatim.root().join("lib.rs"))
        .err()
        .expect("verbatim items must fail closed");
    assert_eq!(error, "lib.rs: unsupported verbatim Rust item");
}
