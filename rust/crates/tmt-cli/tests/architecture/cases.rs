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
fn office_command_edges_are_confined_to_the_reserved_facade() {
    for package in ["tmt-core", "tmt-adapters", "tmt-cli"] {
        let violations = policy::source_violations(&[syntax(
            package,
            "extra.rs",
            "use tmt_office_command::execute as office;",
        )]);
        assert!(violations.iter().any(|v| v.contains("reserved facade")));
    }
    assert_exact(
        &[syntax(
            "tmt-cli",
            "office_facade.rs",
            "use tmt_office_command::execute;",
        )],
        &[],
    );
    assert_exact(
        &[syntax(
            "tmt-cli",
            "grammar.rs",
            "use crate::office_facade::grammar::grammar;",
        )],
        &[],
    );
    let violations = policy::source_violations(&[syntax(
        "tmt-cli",
        "parser.rs",
        "use crate::office_facade::execute;",
    )]);
    assert!(
        violations
            .iter()
            .any(|v| v.contains("parsing depends on effects"))
    );
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
fn office_model_direction_and_dependency_aliases_are_guarded() {
    for (owner, dependency_name, alias) in [
        ("tmt-core", "tmt-office-model", None),
        ("tmt-office-model", "tmt-adapters", None),
        ("tmt-office-model", "rusqlite", None),
        ("tmt-office-model", "tmt-core", Some("hidden")),
        ("tmt-adapters", "tmt-office-model", None),
        ("tmt-office-command", "tmt-office-model", Some("hidden")),
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency(dependency_name, "normal", None, alias)]
            ))
            .len(),
            1
        );
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-office-model",
            vec![dependency("tmt-core", "normal", None, None)]
        ))
        .is_empty()
    );
}

#[test]
fn office_consumers_cannot_expand_or_hide_behind_reexports() {
    let import = "use tmt_office_model::office_world::WorldLayout as Layout;";
    for package in [
        "tmt-office-command",
        "tmt-office-pairing",
        "tmt-office-service",
        "tmt-office-storage",
    ] {
        assert_exact(&[syntax(package, "lib.rs", import)], &[]);
    }
    // The world reply moved to the Office model; core cannot regrow it.
    assert_exact(
        &[syntax("tmt-adapters", "office_world/reply.rs", import)],
        &[
            "tmt-adapters/office_world/reply.rs: core crates cannot declare Office module office_world",
            "tmt-adapters/office_world/reply.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
    // Office repositories moved to tmt-office-storage; core cannot regrow them.
    assert_exact(
        &[syntax(
            "tmt-adapters",
            "storage/office_world/layout.rs",
            import,
        )],
        &[
            "tmt-adapters/storage/office_world/layout.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
    assert_exact(
        &[syntax(
            "tmt-office-storage",
            "office_world/layout.rs",
            import,
        )],
        &[],
    );
    assert_exact(
        &[syntax("tmt-adapters", "storage/new_office.rs", import)],
        &[
            "tmt-adapters/storage/new_office.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
    assert_exact(
        &[syntax("tmt-core", "lib.rs", import)],
        &[
            "tmt-core/lib.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
    assert_exact(
        &[syntax(
            "tmt-adapters",
            "office_world.rs",
            "pub use tmt_office_model::office_world::WorldLayout as Layout;",
        )],
        &[
            "tmt-adapters/office_world.rs: core crates cannot declare Office module office_world",
            "tmt-adapters/office_world.rs: import the Office owner directly; do not re-export tmt_office_model::office_world::WorldLayout",
            "tmt-adapters/office_world.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
    assert_exact(
        &[syntax(
            "tmt-cli",
            "new_command.rs",
            "fn run() { let _: Option<tmt_office_model::office_world::WorldLayout> = None; }",
        )],
        &[
            "tmt-cli/new_command.rs: unreviewed Office dependency tmt_office_model::office_world::WorldLayout",
        ],
    );
}

#[test]
fn office_model_allows_memory_codecs_not_runtime_effects() {
    assert_exact(
        &[syntax(
            "tmt-office-model",
            "codec/image.rs",
            "use std::{io::{Cursor, Error}, sync::OnceLock};",
        )],
        &[],
    );
    for text in [
        "use std::fs::File as Hidden;",
        "pub use std::process::Command;",
        "fn run() { std::env::var(\"HOME\"); }",
    ] {
        assert!(
            !policy::source_violations(&[syntax("tmt-office-model", "codec/image.rs", text)])
                .is_empty()
        );
    }
    assert!(
        !policy::source_violations(&[syntax(
            "tmt-office-model",
            "codec/new.rs",
            "use tmt_core::request::RequestService;"
        )])
        .is_empty()
    );
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
fn office_model_may_parse_link_values_but_cannot_fetch_them() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-office-model",
            vec![dependency("url", "normal", None, None)]
        ))
        .is_empty()
    );
    assert!(
        !policy::dependency_violations(&package(
            "tmt-office-model",
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
fn display_width_dependency_stays_in_the_shared_cli_style() {
    for (owner, expected) in [
        ("tmt-cli-style", 0),
        ("tmt-command-output", 1),
        ("tmt-cli", 1),
        ("tmt-core", 1),
        ("tmt-adapters", 1),
    ] {
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
fn the_shared_cli_style_depends_on_no_tmt_crate() {
    for crate_name in [
        "tmt-core",
        "tmt-adapters",
        "tmt-command-output",
        "tmt-office-model",
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-cli-style",
                vec![dependency(crate_name, "normal", None, None)]
            ))
            .len(),
            1,
            "{crate_name}"
        );
    }
}

#[test]
fn png_runtime_dependency_stays_in_the_office_model() {
    for (owner, expected) in [
        ("tmt-office-model", 0),
        ("tmt-adapters", 1),
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

#[test]
fn the_driver_protocol_depends_on_no_tmt_crate() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-driver-protocol",
            ["serde", "serde_json"]
                .into_iter()
                .map(|name| dependency(name, "normal", None, None))
                .collect()
        ))
        .is_empty()
    );
    for crate_name in ["tmt-core", "tmt-adapters", "tmt-cli-style", "uuid"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-driver-protocol",
                vec![dependency(crate_name, "normal", None, None)]
            ))
            .len(),
            1,
            "{crate_name}"
        );
    }
}

#[test]
fn squad_and_core_are_independent_in_both_directions() {
    // Squad may use its reviewed third-party crates and the leaf style crate,
    // never a workspace crate with TMT behavior.
    assert!(
        policy::dependency_violations(&package(
            "tmt-squad",
            [
                "tmt-cli-style",
                "clap",
                "serde_json",
                "toml_edit",
                "subprocess"
            ]
            .into_iter()
            .map(|name| dependency(name, "normal", Some("cfg(unix)"), None))
            .collect(),
        ))
        .is_empty()
    );
    for core in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-command-output",
        "tmt-office-model",
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-squad",
                vec![dependency(core, "normal", None, None)]
            ))
            .len(),
            1,
            "squad -> {core}"
        );
    }
    for owner in ["tmt-core", "tmt-adapters", "tmt-cli", "tmt-office"] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("tmt-squad", "normal", None, None)]
            ))
            .len(),
            1,
            "{owner} -> squad"
        );
    }
    // Source references are checked too, independently of Cargo metadata.
    assert_exact(
        &[syntax(
            "tmt-squad",
            "main.rs",
            "use crate::core::Core; use serde_json::Value; use toml_edit::DocumentMut; use tmt_cli_style::Terminal;",
        )],
        &[],
    );
    assert_exact(
        &[syntax(
            "tmt-squad",
            "status.rs",
            "fn f() { let _ = tmt_core::room::resolve_room; }",
        )],
        &[
            "tmt-squad/status.rs: squad reaches TMT only through public commands, not tmt_core::room::resolve_room",
        ],
    );
    assert_exact(
        &[syntax("tmt-cli", "extra.rs", "use tmt_squad::status;")],
        &["tmt-cli/extra.rs: no package may depend on the squad extension: tmt_squad::status"],
    );
}

#[test]
fn terminal_hosts_are_reached_only_through_the_host_port() {
    assert_exact(
        &[
            syntax(
                "tmt-cli",
                "talk_command.rs",
                "use tmt_adapters::tmux::Tmux;",
            ),
            syntax(
                "tmt-office",
                "in_process.rs",
                "fn list() { let _ = tmt_adapters::tmux::Tmux::default(); }",
            ),
            syntax(
                "tmt-adapters",
                "delivery.rs",
                "use crate::tmux::BindingSession;",
            ),
        ],
        &[
            "tmt-adapters/delivery.rs: reach the terminal host through tmt_adapters::host, not crate::tmux::BindingSession",
            "tmt-cli/talk_command.rs: reach the terminal host through tmt_adapters::host, not tmt_adapters::tmux::Tmux",
            "tmt-office/in_process.rs: reach the terminal host through tmt_adapters::host, not tmt_adapters::tmux::Tmux::default",
        ],
    );
    assert_exact(
        &[
            syntax(
                "tmt-cli",
                "talk_command.rs",
                "use tmt_adapters::host::Host;",
            ),
            syntax("tmt-adapters", "host.rs", "use crate::tmux::Tmux;"),
            syntax("tmt-adapters", "tmux/focus.rs", "use crate::tmux::Tmux;"),
        ],
        &[],
    );
}

#[test]
fn core_crates_cannot_add_office_modules() {
    let empty = "";
    // Only the facade remains until #355's PR B removes it.
    assert_exact(&[syntax("tmt-cli", "office_facade.rs", empty)], &[]);
    // Moved or new Office modules cannot return to a core crate.
    assert_exact(
        &[syntax("tmt-adapters", "office_companion.rs", empty)],
        &[
            "tmt-adapters/office_companion.rs: core crates cannot declare Office module office_companion",
        ],
    );
    assert_exact(
        &[syntax("tmt-adapters", "office_service.rs", empty)],
        &[
            "tmt-adapters/office_service.rs: core crates cannot declare Office module office_service",
        ],
    );
    assert_exact(
        &[syntax("tmt-adapters", "office_pairing.rs", empty)],
        &[
            "tmt-adapters/office_pairing.rs: core crates cannot declare Office module office_pairing",
        ],
    );
    assert_exact(
        &[syntax("tmt-adapters", "office_http/client.rs", empty)],
        &[
            "tmt-adapters/office_http/client.rs: core crates cannot declare Office module office_http",
        ],
    );
    assert_exact(
        &[syntax("tmt-core", "office_new.rs", empty)],
        &["tmt-core/office_new.rs: core crates cannot declare Office module office_new"],
    );
    // Extension crates own their Office modules.
    assert_exact(
        &[syntax("tmt-office-pairing", "office_pairing.rs", empty)],
        &[],
    );
    assert_exact(
        &[syntax("tmt-office-command", "office_companion.rs", empty)],
        &[],
    );
}
