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
fn private_hook_binding_lookup_and_locator_have_one_owner_each() {
    assert_exact(
        &[syntax(
            "tmt-cli",
            "provider_hook_command.rs",
            "fn verified_caller() { Storage::context_by_binding(); }",
        )],
        &[],
    );
    for code in [
        "fn other() { Storage::context_by_binding(); }",
        "use storage::context_by_binding as context; fn other() { context(); }",
        "fn verified_caller() { fn nested() { Storage::context_by_binding(); } }",
        "fn verified_caller() { impl Other { fn nested() { Storage::context_by_binding(); } } }",
    ] {
        let failures =
            policy::source_violations(&[syntax("tmt-cli", "provider_hook_command.rs", code)]);
        assert!(
            failures
                .iter()
                .any(|v| v.contains("belongs only to verified_caller"))
        );
    }
    let valid = "impl LifecycleObservation for ChannelObservation { fn verified_binding(&self) -> Option<&str> { Some(\"binding\") } }";
    assert_exact(
        &[syntax(
            "tmt-adapters",
            "drivers/codex/channel_hooks.rs",
            valid,
        )],
        &[],
    );
    for file in ["drivers/claude.rs", "extra.rs"] {
        let failures = policy::source_violations(&[syntax("tmt-adapters", file, valid)]);
        assert!(
            failures
                .iter()
                .any(|v| v.contains("belongs only to Codex ChannelObservation"))
        );
    }
}

#[test]
fn the_herdr_driver_is_referenced_only_by_its_bin() {
    let call = "fn main() { tmt_driver_herdr::serve_call(); }";
    assert_exact(&[syntax("tmt-driver-herdr", "main.rs", call)], &[]);
    for (package, file, code) in [
        ("tmt-cli", "tmt-driver-herdr.rs", call),
        ("tmt-cli", "main.rs", call),
        (
            "tmt-cli",
            "driver_command.rs",
            "use tmt_driver_herdr::HerdrDriver;",
        ),
        ("tmt-adapters", "host.rs", call),
        ("tmt-driver-herdr", "lib.rs", call),
        (
            "tmt-driver-herdr",
            "main.rs",
            "use tmt_driver_herdr::HerdrDriver;",
        ),
    ] {
        let failures = policy::source_violations(&[syntax(package, file, code)]);
        assert!(
            failures
                .iter()
                .any(|v| v.contains("tmt_driver_herdr belongs only to the tmt-driver-herdr bin")),
            "{package}/{file}: {failures:?}"
        );
    }
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
fn core_allows_duration_budget_values_but_rejects_clocks_and_broad_time_imports() {
    assert_exact(
        &[syntax(
            "tmt-core",
            "budget.rs",
            r#"
            use std::time::Duration as Budget;
            pub fn budget() -> Budget { std::time::Duration::from_secs(6) }
        "#,
        )],
        &[],
    );
    for (reference, statement) in [
        ("std::time::Instant", "use std::time::Instant;"),
        ("std::time::SystemTime", "use std::time::SystemTime;"),
        ("std::time", "use std::time;"),
        ("std::time::*", "use std::time::*;"),
        (
            "std::time::Instant::now",
            "fn clock() { let _ = std::time::Instant::now(); }",
        ),
        (
            "std::time::SystemTime::now",
            "fn clock() { let _ = std::time::SystemTime::now(); }",
        ),
    ] {
        assert_exact(
            &[syntax("tmt-core", "budget.rs", statement)],
            &[&format!(
                "tmt-core/budget.rs: non-pure core reference {reference}"
            )],
        );
    }
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
fn core_keeps_one_write_once_host_registry_and_no_other_global_state() {
    assert_exact(
        &[syntax("tmt-core", "host.rs", "use std::sync::OnceLock;")],
        &[],
    );
    for (file, text, reference) in [
        ("host.rs", "use std::sync::Mutex;", "std::sync::Mutex"),
        (
            "names.rs",
            "use std::sync::OnceLock;",
            "std::sync::OnceLock",
        ),
    ] {
        assert_exact(
            &[syntax("tmt-core", file, text)],
            &[&format!(
                "tmt-core/{file}: non-pure core reference {reference}"
            )],
        );
    }
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
                dependency("semver", "normal", None, None)
            ],
        )),
        Vec::<String>::new(),
    );
    // The removed, unused core dev dependency must not retain permission.
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("serde_json", "dev", None, None)]
        ))
        .len(),
        1
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
fn dev_dependencies_require_an_exact_reviewed_edge() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-cli",
            vec![dependency("rusqlite", "dev", Some("cfg(unix)"), None)]
        ))
        .is_empty()
    );
    for target in [None, Some("cfg(windows)")] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-cli",
                vec![dependency("rusqlite", "dev", target, None)]
            )),
            vec![format!(
                "tmt-cli: unreviewed dev dependency rusqlite (target={}, rename=null); remove any rename and, after review, add (\"tmt-cli\", \"rusqlite\", {target:?}), // <review reason> to DEV_DEPENDENCIES in rust/crates/tmt-cli/tests/architecture/policy.rs",
                json!(target)
            )]
        );
    }
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-adapters",
            vec![dependency(
                "nix",
                "dev",
                Some("cfg(unix)"),
                Some("unix_helpers")
            )]
        )),
        vec![
            "tmt-adapters: unreviewed dev dependency nix (target=\"cfg(unix)\", rename=\"unix_helpers\"); remove rename from the tmt-adapters Cargo.toml; DEV_DEPENDENCIES in rust/crates/tmt-cli/tests/architecture/policy.rs already reviews (\"tmt-adapters\", \"nix\", Some(\"cfg(unix)\")),"
        ]
    );
    // A production permission is not a dev permission, even in a leaf crate.
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("uuid", "dev", None, None)]
        ))
        .len(),
        1
    );
}

#[test]
fn every_workspace_crate_reviews_new_dev_dependencies() {
    for owner in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-cli-style",
        "tmt-command-output",
        "tmt-host-grammar",
        "tmt-driver-protocol",
        "tmt-driver-herdr",
        "tmt-invoke",
        "tmt-test-support",
        "tmt-remote",
        "tmt-ops",
        "tmt-office",
        "tmt-office-command",
        "tmt-office-model",
        "tmt-office-pairing",
        "tmt-office-service",
        "tmt-office-storage",
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("tempfile", "dev", None, None)]
            ))
            .len(),
            1,
            "{owner} must not silently accept a new dev edge"
        );
    }
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-core",
            vec![dependency("tempfile", "dev", None, None)]
        )),
        vec![
            "tmt-core: unreviewed dev dependency tempfile (target=null, rename=null); remove any rename and, after review, add (\"tmt-core\", \"tempfile\", None), // <review reason> to DEV_DEPENDENCIES in rust/crates/tmt-cli/tests/architecture/policy.rs"
        ]
    );
}

#[test]
fn release_toml_tool_dependencies_are_private_production_edges() {
    for name in ["serde_json", "toml_edit"] {
        assert!(
            policy::dependency_violations(&package(
                "tmt-release-tool",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
        for (kind, target, rename) in [
            ("build", None, None),
            ("dev", None, None),
            ("normal", None, Some("release_tool")),
        ] {
            assert_eq!(
                policy::dependency_violations(&package(
                    "tmt-release-tool",
                    vec![dependency(name, kind, target, rename)]
                ))
                .len(),
                1
            );
        }
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-test-support",
                vec![dependency(name, "dev", None, None)]
            ))
            .len(),
            usize::from(name == "toml_edit")
        );
    }
    for owner in [
        "tmt-cli",
        "tmt-core",
        "tmt-adapters",
        "tmt-ops",
        "tmt-office",
        "tmt-remote",
        "tmt-colab",
        "tmt-test-support",
        "tmt-release-tool",
    ] {
        for kind in ["normal", "build", "dev"] {
            for (target, rename) in [(None, None), (Some("cfg(unix)"), Some("release_tool"))] {
                assert_eq!(
                    policy::dependency_violations(&package(
                        owner,
                        vec![dependency("tmt-release-tool", kind, target, rename)]
                    ))
                    .len(),
                    1,
                    "{owner}/{kind}"
                );
            }
        }
    }
    for kind in ["normal", "build", "dev"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-release-tool",
                vec![dependency("tmt-cli", kind, None, None)]
            ))
            .len(),
            1
        );
    }
}

#[test]
fn release_tool_cannot_be_published_or_distributed() {
    let valid =
        json!({"name":"tmt-release-tool", "publish":[], "metadata":{"dist":{"dist":false}}});
    assert!(policy::release_tool_package_violations(&valid).is_empty());
    for publish in [Value::Null, json!(["crates-io"])] {
        let mut invalid = valid.clone();
        invalid["publish"] = publish;
        assert_eq!(
            policy::release_tool_package_violations(&invalid),
            ["tmt-release-tool: publish must be false"]
        );
    }
    for dist in [Value::Null, json!(true)] {
        let mut invalid = valid.clone();
        invalid["metadata"]["dist"]["dist"] = dist;
        assert_eq!(
            policy::release_tool_package_violations(&invalid),
            ["tmt-release-tool: dist must be false"]
        );
    }
}

#[test]
fn fixture_publication_has_exactly_six_dev_consumers_and_no_product_edges() {
    for owner in [
        "tmt-adapters",
        "tmt-cli",
        "tmt-ops",
        "tmt-office",
        "tmt-colab",
        "tmt-office-command",
    ] {
        assert!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("tmt-test-support", "dev", None, None)]
            ))
            .is_empty(),
            "{owner}"
        );
        for (kind, target, rename) in [
            ("normal", None, None),
            ("build", None, None),
            ("dev", Some("cfg(unix)"), None),
            ("dev", None, Some("fixture_helpers")),
        ] {
            assert_eq!(
                policy::dependency_violations(&package(
                    owner,
                    vec![dependency("tmt-test-support", kind, target, rename)]
                ))
                .len(),
                1,
                "{owner}: {kind} {target:?} {rename:?}"
            );
        }
        assert!(
            !policy::source_violations(&[syntax(
                owner,
                "lib.rs",
                "use tmt_test_support::write_executable;"
            )])
            .is_empty()
        );
        assert!(
            policy::source_violations(&[syntax(
                owner,
                "lib.rs",
                "#[cfg(test)] mod tests { use tmt_test_support::write_executable; }"
            )])
            .is_empty()
        );
    }
    for owner in [
        "tmt-core",
        "tmt-remote",
        "tmt-invoke",
        "tmt-tui",
        "tmt-office-model",
        "tmt-office-storage",
        "tmt-office-pairing",
        "tmt-office-service",
        "tmt-colab-model",
        "tmt-command-output",
        "tmt-cli-style",
        "tmt-driver-protocol",
        "tmt-driver-herdr",
        "tmt-host-grammar",
        "tmt-sys",
        "tmt-test-support",
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![dependency("tmt-test-support", "dev", None, None)]
            ))
            .len(),
            1,
            "{owner}"
        );
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-test-support",
            vec![dependency("tmt-invoke", "normal", None, None)]
        ))
        .is_empty()
    );
    for dependency_name in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-office",
        "tmt-ops",
        "tmt-colab",
        "tmt-remote",
    ] {
        for kind in ["normal", "build", "dev"] {
            assert_eq!(
                policy::dependency_violations(&package(
                    "tmt-test-support",
                    vec![dependency(dependency_name, kind, None, None)]
                ))
                .len(),
                1
            );
        }
        assert!(
            !policy::source_violations(&[syntax(
                "tmt-test-support",
                "lib.rs",
                &format!("use {}::Value;", dependency_name.replace('-', "_"))
            )])
            .is_empty()
        );
    }
    for kind in ["dev", "build"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-test-support",
                vec![dependency("tmt-invoke", kind, None, None)]
            ))
            .len(),
            1
        );
    }
}

#[test]
fn fixture_publication_cannot_be_published_or_distributed() {
    let valid =
        json!({"name": "tmt-test-support", "publish": [], "metadata": {"dist": {"dist": false}}});
    assert!(policy::test_support_package_violations(&valid).is_empty());
    for publish in [Value::Null, json!(["crates-io"])] {
        let mut invalid = valid.clone();
        invalid["publish"] = publish;
        assert_eq!(
            policy::test_support_package_violations(&invalid),
            ["tmt-test-support: publish must be false"]
        );
    }
    for dist in [Value::Null, json!(true)] {
        let mut invalid = valid.clone();
        invalid["metadata"]["dist"]["dist"] = dist;
        assert_eq!(
            policy::test_support_package_violations(&invalid),
            ["tmt-test-support: dist must be false"]
        );
    }
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
fn display_width_dependency_is_limited_to_presentation_owners() {
    for (owner, expected) in [
        ("tmt-cli-style", 0),
        ("tmt-command-output", 1),
        ("tmt-cli", 1),
        ("tmt-core", 1),
        ("tmt-adapters", 0),
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

    let colab_assets = FixtureDirectory::new();
    colab_assets.write(
        "assets.rs",
        "include!(concat!(env!(\"OUT_DIR\"), \"/colab_assets.rs\"));\n",
    );
    assert!(source::collect("tmt-colab", &colab_assets.root().join("assets.rs")).is_ok());
    assert!(
        source::collect("fixture", &colab_assets.root().join("assets.rs")).is_err(),
        "the generated asset allowance must remain package-specific"
    );
    colab_assets.write(
        "lib.rs",
        "include!(concat!(env!(\"OUT_DIR\"), \"/colab_assets.rs\"));\n",
    );
    assert!(
        source::collect("tmt-colab", &colab_assets.root().join("lib.rs")).is_err(),
        "the generated asset allowance must remain module-specific"
    );

    let verbatim = FixtureDirectory::new();
    verbatim.write("lib.rs", "pub fn declaration_only();\n");
    let error = source::collect("fixture", &verbatim.root().join("lib.rs"))
        .err()
        .expect("verbatim items must fail closed");
    assert_eq!(error, "lib.rs: unsupported verbatim Rust item");
}

#[test]
fn the_host_grammar_is_the_only_crate_core_and_the_protocol_share() {
    let violations = |owner: &str, dependencies: &[&str]| {
        policy::dependency_violations(&package(
            owner,
            dependencies
                .iter()
                .map(|name| dependency(name, "normal", None, None))
                .collect(),
        ))
        .len()
    };
    assert_eq!(
        violations(
            "tmt-driver-protocol",
            &["serde", "serde_json", "tmt-host-grammar"]
        ),
        0
    );
    assert_eq!(violations("tmt-core", &["tmt-host-grammar"]), 0);
    // Core never takes the wire crate or serde to reach the grammar.
    for crate_name in ["tmt-driver-protocol", "serde", "serde_json"] {
        assert_eq!(violations("tmt-core", &[crate_name]), 1, "{crate_name}");
    }
    for crate_name in ["tmt-core", "tmt-adapters", "tmt-cli-style", "uuid"] {
        assert_eq!(
            violations("tmt-driver-protocol", &[crate_name]),
            1,
            "{crate_name}"
        );
    }
    // The grammar depends on nothing at all.
    for crate_name in ["serde", "tmt-core", "tmt-driver-protocol"] {
        assert_eq!(
            violations("tmt-host-grammar", &[crate_name]),
            1,
            "{crate_name}"
        );
    }
}

#[test]
fn the_herdr_driver_reaches_tmt_only_through_the_protocol() {
    let violations = |dependencies: &[&str]| {
        policy::dependency_violations(&package(
            "tmt-driver-herdr",
            dependencies
                .iter()
                .map(|name| dependency(name, "normal", None, None))
                .collect(),
        ))
        .len()
    };
    assert_eq!(
        violations(&["semver", "serde_json", "tmt-driver-protocol", "tmt-invoke"]),
        0
    );
    for crate_name in ["tmt-core", "tmt-adapters", "tmt-host-grammar", "serde"] {
        assert_eq!(violations(&[crate_name]), 1, "{crate_name}");
    }
}

#[test]
fn squad_and_core_are_independent_in_both_directions() {
    // Squad may use its reviewed third-party crates and the leaf style crate,
    // never a workspace crate with TMT behavior.
    assert!(
        policy::dependency_violations(&package(
            "tmt-ops",
            [
                "tmt-cli-style",
                "clap",
                "serde_json",
                "toml_edit",
                "subprocess",
                "sha2",
                "nix"
            ]
            .into_iter()
            .map(|name| dependency(name, "normal", Some("cfg(unix)"), None))
            .collect(),
        ))
        .is_empty()
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-ops",
            vec![dependency("rusqlite", "normal", None, None)]
        ))
        .len(),
        1,
        "Squad must not add a direct core-storage connection"
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
                "tmt-ops",
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
                vec![dependency("tmt-ops", "normal", None, None)]
            ))
            .len(),
            1,
            "{owner} -> squad"
        );
    }
    // Source references are checked too, independently of Cargo metadata.
    assert_exact(
        &[syntax(
            "tmt-ops",
            "main.rs",
            "use crate::core::Core; use serde_json::Value; use toml_edit::DocumentMut; use tmt_cli_style::Terminal;",
        )],
        &[],
    );
    assert_exact(
        &[syntax(
            "tmt-ops",
            "status.rs",
            "fn f() { let _ = tmt_core::room::resolve_room; }",
        )],
        &[
            "tmt-ops/status.rs: ops reaches TMT only through public commands, not tmt_core::room::resolve_room",
        ],
    );
    assert_exact(
        &[syntax("tmt-cli", "extra.rs", "use tmt_ops::status;")],
        &["tmt-cli/extra.rs: no package may depend on the ops extension: tmt_ops::status"],
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

#[test]
fn remote_crypto_pins_do_not_open_other_dependency_boundaries() {
    for name in ["ed25519-dalek", "hmac", "sha2"] {
        assert!(
            policy::dependency_violations(&package(
                "tmt-remote",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-remote",
                vec![dependency(name, "normal", None, Some("alias"))]
            ))
            .len(),
            1
        );
    }
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-remote",
            vec![dependency("curve25519-dalek", "normal", None, None)]
        ))
        .len(),
        1
    );
}

#[test]
fn remote_keeps_public_command_isolation() {
    assert_exact(
        &[syntax(
            "tmt-remote",
            "main.rs",
            "use tmt_remote::core::CoreClient; use tmt_cli_style::command;",
        )],
        &[],
    );
    for name in ["tmt-core", "tmt-adapters", "tmt-office-model"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-remote",
                vec![dependency(name, "normal", None, None)]
            ))
            .len(),
            1
        );
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-remote",
            vec![dependency("tmt-cli-style", "normal", None, None)]
        ))
        .is_empty()
    );
    assert_eq!(
        policy::source_violations(&[syntax(
            "tmt-remote",
            "core.rs",
            "use tmt_core::room::RoomRepository;"
        )])
        .len(),
        1
    );
    assert_eq!(
        policy::source_violations(&[syntax(
            "tmt-cli",
            "extra.rs",
            "use tmt_remote::core::CoreClient;"
        )])
        .len(),
        1
    );
}

#[test]
fn squad_may_use_the_neutral_invoke_leaf_but_not_core_process_adapters() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-ops",
            vec![dependency("tmt-invoke", "normal", None, None)]
        ))
        .is_empty()
    );
    assert_exact(
        &[syntax("tmt-ops", "runner.rs", "use tmt_invoke::invoke;")],
        &[],
    );
    assert!(
        !policy::source_violations(&[syntax(
            "tmt-ops",
            "runner.rs",
            "use tmt_adapters::process::UnixCommandRunner;"
        )])
        .is_empty()
    );
    assert!(
        !policy::dependency_violations(&package(
            "tmt-ops",
            vec![dependency("tmt-invoke", "normal", None, Some("runner"))]
        ))
        .is_empty()
    );
}

#[test]
fn extension_state_is_a_leaf_with_only_the_two_reviewed_executable_consumers() {
    for consumer in ["tmt-remote", "tmt-colab"] {
        assert!(
            policy::dependency_violations(&package(
                consumer,
                vec![dependency("tmt-extension-state", "normal", None, None)]
            ))
            .is_empty()
        );
        assert_exact(
            &[syntax(
                consumer,
                "state.rs",
                "use tmt_extension_state::Layout;",
            )],
            &[],
        );
        assert!(
            !policy::dependency_violations(&package(
                consumer,
                vec![dependency(
                    "tmt-extension-state",
                    "normal",
                    None,
                    Some("state")
                )]
            ))
            .is_empty()
        );
    }
    for kind in ["normal", "dev", "build"] {
        for target in [None, Some("cfg(unix)")] {
            assert!(
                policy::dependency_violations(&package(
                    "tmt-extension-state",
                    vec![dependency("nix", kind, target, None)]
                ))
                .is_empty()
            );
            for dependency_name in [
                "tmt-core",
                "tmt-adapters",
                "tmt-remote",
                "tmt-colab",
                "tmt-cli-style",
                "getrandom",
                "ed25519-dalek",
            ] {
                assert!(
                    !policy::dependency_violations(&package(
                        "tmt-extension-state",
                        vec![dependency(dependency_name, kind, target, None)]
                    ))
                    .is_empty()
                );
            }
            assert!(
                !policy::dependency_violations(&package(
                    "tmt-extension-state",
                    vec![dependency("nix", kind, target, Some("system"))]
                ))
                .is_empty()
            );
            for consumer in [
                "tmt-core",
                "tmt-adapters",
                "tmt-cli",
                "tmt-colab-model",
                "tmt-ops",
            ] {
                assert!(
                    !policy::dependency_violations(&package(
                        consumer,
                        vec![dependency("tmt-extension-state", kind, target, None)]
                    ))
                    .is_empty()
                );
            }
        }
    }
    for consumer in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-colab-model",
        "tmt-ops",
    ] {
        assert!(
            !policy::source_violations(&[syntax(
                consumer,
                "state.rs",
                "use tmt_extension_state::Layout;"
            )])
            .is_empty()
        );
    }
    for source in [
        "use tmt_core::identity::Identity;",
        "pub use tmt_adapters::process;",
        "use tmt_remote::state::Layout;",
        "use tmt_colab::keyring::Layout;",
    ] {
        assert!(
            !policy::source_violations(&[syntax("tmt-extension-state", "lib.rs", source)])
                .is_empty()
        );
    }
    assert_exact(
        &[syntax(
            "tmt-extension-state",
            "state.rs",
            "use tmt_extension_state::Layout;",
        )],
        &[],
    );
}

/// Every workspace package, so a consumer cannot be forgotten or misspelled: a
/// misspelled name would be refused as an unreviewed package, not for the leaf.
const WORKSPACE_PACKAGES: [&str; 24] = [
    "tmt-core",
    "tmt-adapters",
    "tmt-cli",
    "tmt-cli-style",
    "tmt-command-output",
    "tmt-host-grammar",
    "tmt-driver-protocol",
    "tmt-driver-herdr",
    "tmt-invoke",
    "tmt-extension-state",
    "tmt-tui",
    "tmt-test-support",
    "tmt-release-tool",
    "tmt-sys",
    "tmt-colab-model",
    "tmt-office",
    "tmt-office-model",
    "tmt-office-command",
    "tmt-office-storage",
    "tmt-office-pairing",
    "tmt-office-service",
    "tmt-ops",
    "tmt-remote",
    "tmt-colab",
];

/// The refusal names `dependency`, so an unrelated failure cannot satisfy a negative.
fn refuses(
    owner: &str,
    dependency_name: &str,
    kind: &str,
    target: Option<&str>,
    rename: Option<&str>,
) -> bool {
    policy::dependency_violations(&package(
        owner,
        vec![dependency(dependency_name, kind, target, rename)],
    ))
    .iter()
    .any(|violation| violation.contains(dependency_name))
}

#[test]
fn extension_objects_is_a_strict_leaf_consumed_by_remote_service_and_colab_channel() {
    for kind in ["normal", "dev", "build"] {
        for target in [None, Some("cfg(unix)")] {
            for allowed in ["base64", "serde", "serde_json"] {
                assert!(
                    policy::dependency_violations(&package(
                        "tmt-extension-objects",
                        vec![dependency(allowed, kind, target, None)]
                    ))
                    .is_empty(),
                    "{allowed} {kind} {target:?}"
                );
                assert!(
                    refuses(
                        "tmt-extension-objects",
                        allowed,
                        kind,
                        target,
                        Some("alias")
                    ),
                    "renamed {allowed} {kind} {target:?}"
                );
            }
            // Includes crates the generic dev ledger permits elsewhere: the leaf is
            // strict for every dependency kind.
            for forbidden in [
                "tmt-core",
                "tmt-adapters",
                "tmt-cli",
                "tmt-colab-model",
                "tmt-office-model",
                "tmt-driver-protocol",
                "tmt-driver-herdr",
                "tmt-host-grammar",
                "tmt-extension-state",
                "tmt-invoke",
                "tmt-test-support",
                "tmt-remote",
                "tmt-colab",
                "sha2",
                "getrandom",
                "ed25519-dalek",
                "tempfile",
            ] {
                assert!(
                    refuses("tmt-extension-objects", forbidden, kind, target, None),
                    "leaf must not use {forbidden} ({kind}, {target:?})"
                );
            }
            // The carrier's two OS-facing dependencies are reviewed for the Unix target
            // of a normal dependency alone, and never renamed.
            for carrier in ["nix", "httparse"] {
                let reviewed = kind == "normal" && target == Some("cfg(unix)");
                assert_eq!(
                    refuses("tmt-extension-objects", carrier, kind, target, None),
                    !reviewed,
                    "{carrier} {kind} {target:?}"
                );
                assert!(
                    refuses(
                        "tmt-extension-objects",
                        carrier,
                        kind,
                        target,
                        Some("alias")
                    ),
                    "renamed {carrier} {kind} {target:?}"
                );
            }
            // The two consumers have only plain normal edges; no rename or gated/dev/build edge.
            for consumer in WORKSPACE_PACKAGES {
                for rename in [None, Some("objects")] {
                    let reviewed = ["tmt-remote", "tmt-colab"].contains(&consumer)
                        && kind == "normal"
                        && target.is_none()
                        && rename.is_none();
                    assert_eq!(
                        refuses(consumer, "tmt-extension-objects", kind, target, rename),
                        !reviewed,
                        "{consumer} -> leaf ({kind}, {target:?}, {rename:?})"
                    );
                }
            }
        }
    }
    for consumer in WORKSPACE_PACKAGES {
        assert!(
            !policy::source_violations(&[syntax(
                consumer,
                "objects.rs",
                "use tmt_extension_objects::Counter;"
            )])
            .is_empty(),
            "{consumer}"
        );
    }
    // Remote's object service may name the leaf; no other Remote file, no other package.
    for file in ["object_service.rs", "object_service/bus.rs"] {
        assert_exact(
            &[syntax(
                "tmt-remote",
                file,
                "use tmt_extension_objects::Counter;",
            )],
            &[],
        );
    }
    for file in ["object_channel.rs", "object_channel/client.rs"] {
        assert_exact(
            &[syntax(
                "tmt-colab",
                file,
                "use tmt_extension_objects::Counter;",
            )],
            &[],
        );
    }
    for file in ["socket.rs", "attachments.rs", "object_channels.rs"] {
        assert!(
            !policy::source_violations(&[syntax(
                "tmt-colab",
                file,
                "use tmt_extension_objects::Counter;"
            )])
            .is_empty(),
            "{file}"
        );
    }
    for file in [
        "mount.rs",
        "serve.rs",
        "objects/local.rs",
        "object_services.rs",
    ] {
        assert!(
            !policy::source_violations(&[syntax(
                "tmt-remote",
                file,
                "use tmt_extension_objects::Counter;"
            )])
            .is_empty(),
            "{file}"
        );
    }
    for consumer in WORKSPACE_PACKAGES.iter().filter(|p| **p != "tmt-remote") {
        assert!(
            !policy::source_violations(&[syntax(
                consumer,
                "object_service.rs",
                "use tmt_extension_objects::Counter;"
            )])
            .is_empty(),
            "{consumer}"
        );
    }
    for source in [
        "use tmt_core::identity::Identity;",
        "pub use tmt_adapters::process;",
        "use tmt_remote::state::Layout;",
        "use tmt_colab::keyring::Layout;",
        "use tmt_extension_state::Layout;",
    ] {
        assert!(
            !policy::source_violations(&[syntax("tmt-extension-objects", "lib.rs", source)])
                .is_empty(),
            "{source}"
        );
    }
    assert_exact(
        &[syntax(
            "tmt-extension-objects",
            "ids.rs",
            "use tmt_extension_objects::Counter;",
        )],
        &[],
    );
    // Only the carrier may name the OS-facing crates; the protocol modules may not.
    for file in ["carrier.rs", "carrier/bounded.rs", "carrier/handshake.rs"] {
        for source in ["use nix::poll::poll;", "use httparse::Request;"] {
            assert_exact(&[syntax("tmt-extension-objects", file, source)], &[]);
        }
    }
    for file in [
        "lib.rs",
        "codec.rs",
        "ids.rs",
        "frame.rs",
        "frame/read.rs",
        "carriers.rs",
    ] {
        for (source, root) in [
            ("use nix::poll::poll;", "nix"),
            ("use httparse::Request;", "httparse"),
        ] {
            assert!(
                policy::source_violations(&[syntax("tmt-extension-objects", file, source)])
                    .iter()
                    .any(|violation| violation.contains(root)),
                "{file} {root}"
            );
        }
    }
}

#[test]
fn invoke_is_a_leaf_even_in_tests_builds_and_target_dependencies() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-invoke",
            vec![
                dependency("subprocess", "normal", None, None),
                dependency("nix", "normal", None, None)
            ]
        ))
        .is_empty()
    );
    for kind in ["normal", "dev", "build"] {
        for target in [None, Some("cfg(unix)")] {
            for name in ["tmt-core", "tmt-adapters", "tmt-cli-style", "serde_json"] {
                assert_eq!(
                    policy::dependency_violations(&package(
                        "tmt-invoke",
                        vec![dependency(name, kind, target, None)]
                    ))
                    .len(),
                    1
                );
            }
            assert_eq!(
                policy::dependency_violations(&package(
                    "tmt-invoke",
                    vec![dependency("nix", kind, target, Some("alias"))]
                ))
                .len(),
                1
            );
        }
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-remote",
            vec![dependency("tmt-invoke", "normal", None, None)]
        ))
        .is_empty()
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-remote",
            vec![dependency("tmt-invoke", "normal", None, Some("alias"))]
        ))
        .len(),
        1
    );
    assert_exact(
        &[syntax("tmt-remote", "core.rs", "use tmt_invoke::invoke;")],
        &[],
    );
    for source in [
        "use tmt_core::identity::Identity;",
        "pub use tmt_adapters::process;",
    ] {
        assert_eq!(
            policy::source_violations(&[syntax("tmt-invoke", "lib.rs", source)]).len(),
            1
        );
    }
    assert_exact(
        &[syntax("tmt-invoke", "lib.rs", "use tmt_invoke::Request;")],
        &[],
    );
}

#[test]
fn tui_admission_is_an_internal_presentation_leaf() {
    for owner in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-cli-style",
        "tmt-ops",
        "tmt-remote",
    ] {
        assert!(
            !policy::dependency_violations(&package(
                owner,
                vec![dependency("taffy", "normal", None, None)]
            ))
            .is_empty()
        );
    }

    for name in [
        "roxmltree",
        "serde_json",
        "tmt-cli-style",
        "taffy",
        "ratatui",
        "unicode-width",
        "unicode-segmentation",
    ] {
        assert!(
            policy::dependency_violations(&package(
                "tmt-tui",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
    }
    for kind in ["normal", "dev", "build"] {
        for name in ["tmt-core", "tmt-adapters", "tmt-cli", "tmt-ops"] {
            assert_eq!(
                policy::dependency_violations(&package(
                    "tmt-tui",
                    vec![dependency(name, kind, Some("cfg(unix)"), None)]
                ))
                .len(),
                1
            );
        }
    }
    for owner in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-cli-style",
        "tmt-remote",
    ] {
        assert_eq!(
            policy::dependency_violations(&package(
                owner,
                vec![
                    dependency("tmt-tui", "normal", None, None),
                    dependency("tmt-tui", "dev", None, None),
                ]
            ))
            .len(),
            2
        );
        assert!(
            !policy::source_violations(&[syntax(owner, "lib.rs", "use tmt_tui::parse;")])
                .is_empty()
        );
    }
    assert!(
        policy::dependency_violations(&package(
            "tmt-ops",
            vec![dependency("tmt-tui", "normal", None, None)]
        ))
        .is_empty()
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-ops",
            vec![dependency("tmt-tui", "dev", None, None)]
        ))
        .len(),
        1,
        "the production row owner must not regress to a dev-only edge"
    );
    assert_exact(
        &[syntax("tmt-ops", "markup.rs", "use tmt_tui::binding;")],
        &[],
    );
    for owner in ["tmt-core", "tmt-adapters", "tmt-cli", "tmt-ops"] {
        assert!(
            !policy::source_violations(&[syntax(
                "tmt-tui",
                "lib.rs",
                &format!("use {}::value;", owner.replace('-', "_"))
            )])
            .is_empty()
        );
    }
    assert_exact(
        &[syntax("tmt-tui", "lib.rs", "use tmt_cli_style::Role;")],
        &[],
    );
    assert_eq!(
        policy::source_violations(&[syntax("tmt-tui", "lib.rs", "use tmt_core::identity;")]).len(),
        1
    );
    assert_eq!(
        policy::dependency_violations(&package(
            "tmt-tui",
            vec![dependency("roxmltree", "normal", None, Some("alias"))]
        ))
        .len(),
        1
    );
}

#[test]
fn colab_persistence_keeps_core_remote_and_office_isolated() {
    for name in ["ed25519-dalek", "getrandom", "nix", "rusqlite", "sha2"] {
        assert!(
            policy::dependency_violations(&package(
                "tmt-colab",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
    }
    for name in [
        "tmt-core",
        "tmt-adapters",
        "tmt-remote",
        "tmt-office-storage",
    ] {
        assert!(
            !policy::dependency_violations(&package(
                "tmt-colab",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
    }
    assert_exact(
        &[syntax(
            "tmt-colab",
            "lib.rs",
            "use tmt_colab::store::Store;",
        )],
        &[],
    );
    for code in [
        "use tmt_adapters::storage::Storage;",
        "use tmt_remote::core::CoreClient;",
    ] {
        assert!(!policy::source_violations(&[syntax("tmt-colab", "lib.rs", code)]).is_empty());
    }
}

#[test]
fn colab_model_has_only_fixed_crypto_and_no_runtime_authority() {
    for name in [
        "hpke",
        "x25519-dalek",
        "aes-gcm",
        "base64",
        "ed25519-dalek",
        "getrandom",
        "hmac",
        "serde",
        "serde_json",
        "sha2",
    ] {
        assert!(
            policy::dependency_violations(&package(
                "tmt-colab-model",
                vec![dependency(name, "normal", None, None)]
            ))
            .is_empty()
        );
    }
    for name in ["tmt-core", "tmt-adapters", "tmt-remote", "rusqlite"] {
        assert_eq!(
            policy::dependency_violations(&package(
                "tmt-colab-model",
                vec![dependency(name, "normal", None, None)]
            ))
            .len(),
            1
        );
    }
    for path in [
        "std::fs::read",
        "std::io::stdin",
        "std::env::var",
        "std::thread::spawn",
        "std::net::TcpStream",
        "std::process::Command",
        "tmt_core::Identity",
    ] {
        assert_eq!(
            policy::source_violations(&[syntax(
                "tmt-colab-model",
                "lib.rs",
                &format!("use {path};")
            )])
            .len(),
            1
        );
    }
}

#[test]
fn colab_yrs_imports_are_confined_to_the_decoder_child() {
    assert_exact(
        &[syntax("tmt-colab", "decoder/child.rs", "use yrs::Update;")],
        &[],
    );
    for file in ["decoder.rs", "http.rs", "store.rs"] {
        let violations = policy::source_violations(&[syntax(
            "tmt-colab",
            file,
            "use yrs::Update as ForeignUpdate;",
        )]);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("foreign Yjs decoding belongs only in the decoder child"))
        );
    }
}

#[test]
fn colab_model_consumers_are_confined_to_colab_package() {
    assert!(
        policy::dependency_violations(&package(
            "tmt-colab",
            vec![dependency("tmt-colab-model", "normal", None, None)]
        ))
        .is_empty()
    );
    for file in ["keyring.rs", "http.rs", "core.rs", "store.rs"] {
        assert_exact(
            &[syntax(
                "tmt-colab",
                file,
                "use tmt_colab_model::crypto::space_id;",
            )],
            &[],
        );
    }
    for consumer in [
        "tmt-core",
        "tmt-adapters",
        "tmt-cli",
        "tmt-remote",
        "tmt-ops",
        "tmt-office",
    ] {
        assert!(
            !policy::dependency_violations(&package(
                consumer,
                vec![dependency("tmt-colab-model", "normal", None, None)]
            ))
            .is_empty()
        );
        assert!(
            policy::source_violations(&[syntax(
                consumer,
                "lib.rs",
                "use tmt_colab_model::crypto::space_id;"
            )])
            .iter()
            .any(|v| v.contains("unreviewed colab model consumer"))
        );
    }
}

#[test]
fn browser_ui_is_forbidden_across_core_dependency_kinds_paths_and_renames() {
    for owner in ["tmt-core", "tmt-cli", "tmt-adapters", "tmt-cli-style"] {
        for kind in [Value::Null, json!("normal"), json!("build"), json!("dev")] {
            for target in [Value::Null, json!("cfg(unix)")] {
                for rename in [Value::Null, json!("hidden_presentation")] {
                    for dependency in [
                        json!({"name":"browser-ui", "kind":kind, "target":target, "rename":rename}),
                        json!({"name":"neutral-name", "path":"/repository/design/browser-ui", "kind":kind, "target":target, "rename":rename}),
                    ] {
                        let mut value = package(owner, vec![dependency]);
                        value["manifest_path"] =
                            json!(format!("/repository/rust/crates/{owner}/Cargo.toml"));
                        assert!(
                            policy::dependency_violations(&value)
                                .iter()
                                .any(|v| v.contains("must neither depend on nor embed"))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn core_browser_embedding_guard_inspects_includes_paths_and_nested_macro_tokens() {
    let fixture = FixtureDirectory::new();
    let root = fixture.root();
    fs::create_dir_all(root.join("rust/crates/tmt-cli/src")).unwrap();
    fs::create_dir_all(root.join("design/browser-ui/generated")).unwrap();
    fs::write(
        root.join("design/browser-ui/generated/static.css"),
        "checked bytes",
    )
    .unwrap();
    let metadata = json!({"packages":[{"name":"tmt-cli","manifest_path":root.join("rust/crates/tmt-cli/Cargo.toml")}]});
    let source = root.join("rust/crates/tmt-cli/src/main.rs");
    for code in [
        "const CSS: &str = include_str!(\"../../../../design/browser-ui/generated/static.css\");",
        "wrapper!({ inner!({ include_bytes!(\"../../../../design/browser-ui/generated/static.css\"); }); });",
        "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../design/browser-ui/generated/static.css\"));",
        "#[path = \"../../../../design/browser-ui/generated/static.css\"] mod hidden;",
    ] {
        fs::write(&source, code).unwrap();
        let failure = super::never_shipped::browser_leaf_inputs(root, &metadata).unwrap_err();
        assert!(failure.contains("must not embed browser-ui"), "{failure}");
    }
    fs::write(&source, "include_str!(env!(\"UNKNOWN_INPUT\"));").unwrap();
    assert!(
        super::never_shipped::browser_leaf_inputs(root, &metadata)
            .unwrap_err()
            .contains("unproved")
    );
    fs::write(&source, "const TOKENS: &str = include_str!(\"../../../../design/tokens/tokens.json\"); const COPY: &str = \"include_str!(browser-ui)\";").unwrap();
    assert!(super::never_shipped::browser_leaf_inputs(root, &metadata).is_ok());
    fs::create_dir_all(root.join("extensions/tmt-colab/rust/tmt-colab/src")).unwrap();
    fs::write(
        root.join("extensions/tmt-colab/rust/tmt-colab/src/main.rs"),
        "include_str!(\"../../../../../design/browser-ui/generated/static.css\");",
    )
    .unwrap();
    let product = json!({"packages":[{"name":"tmt-colab","manifest_path":root.join("extensions/tmt-colab/rust/tmt-colab/Cargo.toml")}]});
    assert!(super::never_shipped::browser_leaf_inputs(root, &product).is_ok());
}

#[test]
fn core_browser_embedding_guard_proves_local_literal_wrapper_inputs() {
    let fixture = FixtureDirectory::new();
    let root = fixture.root();
    fs::create_dir_all(root.join("rust/crates/tmt-cli/src")).unwrap();
    fs::create_dir_all(root.join("design/browser-ui/generated")).unwrap();
    fs::write(
        root.join("design/browser-ui/generated/static.css"),
        "checked bytes",
    )
    .unwrap();
    let metadata = json!({"packages":[{"name":"tmt-cli","manifest_path":root.join("rust/crates/tmt-cli/Cargo.toml")}]});
    let source = root.join("rust/crates/tmt-cli/src/main.rs");
    let wrapper = r#"macro_rules! migration { ($name:literal, $path:literal) => {
        Migration { path: concat!("storage/", $path), name: $name, sql: include_str!($path) }
    }; }"#;
    let calls = (1..=48)
        .map(|n| format!("migration!(\"migration {n}\", \"schema/{n:03}.sql\")"))
        .collect::<Vec<_>>()
        .join(",");
    fs::write(&source, format!("{wrapper}\nconst M: &[Migration] = &[{calls}];\nconst INDEX: &str = include_str!(\"schema/index.sql\");")).unwrap();
    assert!(super::never_shipped::browser_leaf_inputs(root, &metadata).is_ok());
    fs::write(&source, format!("{wrapper}\nconst M: Migration = migration!(\"bad\", \"../../../../design/browser-ui/generated/static.css\");")).unwrap();
    assert!(
        super::never_shipped::browser_leaf_inputs(root, &metadata)
            .unwrap_err()
            .contains("must not embed browser-ui")
    );
    let additional = wrapper.replace("sql: include_str!($path)", "sql: include_str!($path), extra: include_bytes!(\"../../../../design/browser-ui/generated/static.css\")");
    fs::write(
        &source,
        format!("{additional}\nconst M: Migration = migration!(\"ok\", \"schema/001.sql\");"),
    )
    .unwrap();
    assert!(
        super::never_shipped::browser_leaf_inputs(root, &metadata)
            .unwrap_err()
            .contains("must not embed browser-ui")
    );
    for code in [
        format!("{wrapper}\nconst M: Migration = migration!(\"bad\", env!(\"PATH\"));"),
        format!(
            "#[macro_export] {wrapper}\nconst M: Migration = migration!(\"ok\", \"schema/001.sql\");"
        ),
        format!(
            "{wrapper}\nuse migration as alias; const M: Migration = alias!(\"ok\", \"schema/001.sql\");"
        ),
        format!("{wrapper}\nouter! {{ migration!(\"ok\", \"schema/001.sql\") }}"),
        format!(
            "{wrapper}\nmacro_rules! expose {{ ($alias:ident) => {{ use $alias as renamed; }}; }} expose!(migration); const M: Migration = migration!(\"ok\", \"schema/001.sql\");"
        ),
        format!("const M: Migration = migration!(\"ok\", \"schema/001.sql\"); {wrapper}"),
        wrapper.replace("$path:literal", "$path:expr")
            + "const M: Migration = migration!(\"ok\", \"schema/001.sql\");",
        wrapper.replace("include_str!($path)", "include_str!($unknown)")
            + "const M: Migration = migration!(\"ok\", \"schema/001.sql\");",
        format!("{wrapper}\nconst M: Migration = other::migration!(\"ok\", \"schema/001.sql\");"),
    ] {
        fs::write(&source, &code).unwrap();
        assert!(
            super::never_shipped::browser_leaf_inputs(root, &metadata).is_err(),
            "must refuse: {code}"
        );
    }
    fs::write(
        &source,
        format!("{wrapper}\nconst M: Migration = migration!(\"ok\", \"schema/001.sql\");"),
    )
    .unwrap();
    let second = root.join("rust/crates/tmt-cli/src/other.rs");
    fs::write(
        &second,
        "const M: Migration = migration!(\"other\", \"schema/002.sql\");",
    )
    .unwrap();
    assert!(
        super::never_shipped::browser_leaf_inputs(root, &metadata)
            .unwrap_err()
            .contains("scope/alias")
    );
    fs::remove_file(second).unwrap();
    fs::write(
        root.join("rust/crates/tmt-cli/src/other.rs"),
        "fn migration() {} fn ordinary() { let migration = 1; let _ = migration; }",
    )
    .unwrap();
    assert!(super::never_shipped::browser_leaf_inputs(root, &metadata).is_ok());
    fs::remove_file(root.join("rust/crates/tmt-cli/src/other.rs")).unwrap();
    fs::write(&source, format!("{wrapper}\nconst M: Migration = migration!(\"ok\", \"schema/001.sql\"); fn projection(migration: Migration) {{ values!(migration.name); }}")).unwrap();
    assert!(super::never_shipped::browser_leaf_inputs(root, &metadata).is_ok());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("design/browser-ui"), root.join("browser-alias"))
            .unwrap();
        fs::write(&source, format!("{wrapper}\nconst M: Migration = migration!(\"alias\", \"../../../../browser-alias/generated/static.css\");")).unwrap();
        assert!(
            super::never_shipped::browser_leaf_inputs(root, &metadata)
                .unwrap_err()
                .contains("must not embed browser-ui")
        );
    }
}
