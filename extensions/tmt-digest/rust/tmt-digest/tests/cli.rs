//! Observable settings and help process contract, isolated from the user's state.

mod support;

use std::{
    fs,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        // The history store refuses a symlinked path, and macOS's temp dir is under `/var`.
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "tmt-digest-help-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn run(&self, words: &[&str]) -> Output {
        support::digest()
            .args(words)
            .current_dir(&self.0)
            .env("HOME", &self.0)
            .env("TMT_HOME", self.0.join("state"))
            .env("TMT_EXECUTABLE", self.0.join("missing-core"))
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
    fn assert_untouched(&self) {
        assert_eq!(
            fs::read_dir(&self.0).unwrap().count(),
            0,
            "help-only execution must not create state"
        );
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn help_routes_agree_without_core_or_state() {
    let sandbox = Sandbox::new();
    let expected = sandbox.run(&["--help"]);
    assert!(expected.status.success());
    assert!(expected.stderr.is_empty());
    let help = String::from_utf8(expected.stdout).unwrap();
    assert!(help.contains("Usage: tmt digest"));
    assert!(help.contains("default to remove it"));
    assert!(help.contains("Run tmt digest tick once a minute"));
    assert!(help.contains("Examples:"));
    for words in [&[][..], &["-h"][..], &["help"][..]] {
        let result = sandbox.run(words);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(result.stdout, help.as_bytes());
    }
    sandbox.assert_untouched();
}

#[test]
fn skill_is_exact_bundled_content_without_core_or_state() {
    let sandbox = Sandbox::new();
    let result = sandbox.run(&["skill"]);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    assert_eq!(
        result.stdout,
        include_bytes!("../../../skills/tmt-digest/SKILL.md")
    );
    let help = sandbox.run(&["skill", "--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("without Core discovery")
    );
    sandbox.assert_untouched();
}

#[test]
fn version_is_source_version_and_behavior_is_refused_without_state() {
    let sandbox = Sandbox::new();
    for words in [&["--version"][..], &["-V"][..]] {
        let result = sandbox.run(words);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .contains(env!("CARGO_PKG_VERSION"))
        );
    }
    for words in [&["--"][..], &["--json"][..], &["tick", "--json"][..]] {
        let result = sandbox.run(words);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8(result.stderr).unwrap().contains("error:"));
    }
    sandbox.assert_untouched();
}

#[test]
fn invalid_duration_is_refused_before_core_and_valid_value_requires_core_without_writes() {
    let sandbox = Sandbox::new();
    for value in ["0s", "-1s", "bogus", "999999999999999999h"] {
        let result = sandbox.run(&["worker", value]);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
    }
    let result = sandbox.run(&["worker", "20s"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("CORE_UNAVAILABLE")
    );
    sandbox.assert_untouched();
}

#[test]
fn public_core_process_resolution_roundtrips_overrides_and_default() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let core = sandbox.0.join("core-fixture");
    let config = sandbox.0.join("config.json");
    let script = r#"#!/bin/sh
case "$*" in
  'identity ls --json') printf '%s\n' '{"identities":[{"id":"10000000-0000-4000-8000-000000000001","name":"worker"}]}' ;;
  'identity show --json') printf '%s\n' '{"identity":{"id":"20000000-0000-4000-8000-000000000001"}}' ;;
  'config show --json') printf '%s\n' '{"paths":{"global":"__GLOBAL__"}}' ;;
  *) exit 9 ;;
esac
"#.replace("__GLOBAL__", config.to_str().unwrap());
    fs::write(&core, script).unwrap();
    fs::set_permissions(&core, fs::Permissions::from_mode(0o700)).unwrap();
    let path = sandbox.0.join("digest.toml");
    fs::write(&path, "# keep\ndefault = '30m'\nflushCount = 10\n").unwrap();
    let run = |member: &str, value: &str| {
        support::digest()
            .args([member, value])
            .env("TMT_EXECUTABLE", &core)
            .env("NO_COLOR", "1")
            .current_dir(&sandbox.0)
            .output()
            .unwrap()
    };
    for value in ["20s", "auto", "off"] {
        let result = run("worker", value);
        assert!(result.status.success(), "{:?}", result.stderr);
        let document = fs::read_to_string(&path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        let row = &document["members"]["10000000-0000-4000-8000-000000000001"];
        assert_eq!(row["mode"].as_str(), Some(value));
        assert_eq!(
            row["setByIdentityId"].as_str(),
            Some("20000000-0000-4000-8000-000000000001")
        );
        assert!(row["setAtMs"].as_integer().unwrap() > 0);
        assert!(fs::read_to_string(&path).unwrap().contains("# keep"));
    }
    let result = run("worker", "20");
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("every 20s")
    );
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    assert_eq!(
        document["members"]["10000000-0000-4000-8000-000000000001"]["mode"].as_str(),
        Some("20s")
    );
    let result = run("10000000-0000-4000-8000-000000000001", "default");
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("default digest: every 30m")
    );
    let before = fs::read(&path).unwrap();
    assert!(
        !String::from_utf8(before.clone())
            .unwrap()
            .contains("setByIdentityId")
    );
    let result = run("unknown", "off");
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::write(&path, "flushCount = 0\n").unwrap();
    let before = fs::read(&path).unwrap();
    let result = run("worker", "off");
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn the_global_default_records_who_set_it_and_a_missing_caller_is_said_not_hidden() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let config = sandbox.0.join("config.json");
    let path = sandbox.0.join("digest.toml");
    let core = |name: &str, identity: &str| {
        let program = sandbox.0.join(name);
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  'identity show --json') {identity} ;;\n  'config show --json') printf '%s\\n' '{{\"paths\":{{\"global\":\"{}\"}}}}' ;;\n  *) exit 9 ;;\nesac\n",
            config.to_str().unwrap()
        );
        fs::write(&program, script).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        program
    };
    let named = core(
        "named-core",
        "printf '%s\\n' '{\"identity\":{\"id\":\"20000000-0000-4000-8000-000000000001\"}}'",
    );
    let anonymous = core(
        "anonymous-core",
        "printf '%s\\n' '{\"error\":{\"code\":\"IDENTITY_REQUIRED\",\"message\":\"no caller\"}}'; exit 1",
    );
    let run = |program: &std::path::Path, words: &[&str]| {
        support::digest()
            .args(words)
            .env("TMT_EXECUTABLE", program)
            .env("NO_COLOR", "1")
            .current_dir(&sandbox.0)
            .output()
            .unwrap()
    };
    fs::write(&path, "# keep\nflushCount = 7\n").unwrap();
    let result = run(&named, &["global", "auto"]);
    assert!(result.status.success(), "{:?}", result.stderr);
    let message = String::from_utf8(result.stdout).unwrap();
    assert!(
        message.contains("Saved the global digest default: auto"),
        "{message}"
    );
    assert!(!message.contains("No caller identity"), "{message}");
    let document = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    assert_eq!(document["default"].as_str(), Some("auto"));
    assert_eq!(
        document["defaultSetByIdentityId"].as_str(),
        Some("20000000-0000-4000-8000-000000000001")
    );
    assert!(document["defaultSetAtMs"].as_integer().unwrap() > 0);
    assert_eq!(document["flushCount"].as_integer(), Some(7));
    assert!(fs::read_to_string(&path).unwrap().contains("# keep"));
    // Writing without a caller says so and forgets the old setter.
    let result = run(&anonymous, &["global", "20m"]);
    assert!(result.status.success(), "{:?}", result.stderr);
    let message = String::from_utf8(result.stdout).unwrap();
    assert!(message.contains("every 20m"), "{message}");
    assert!(message.contains("No caller identity"), "{message}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("default = \"20m\""));
    assert!(!text.contains("defaultSetByIdentityId"));
    // Only a mode is a global default: a member word or a bad duration is refused first.
    let before = fs::read(&path).unwrap();
    for value in ["default", "0s", "bogus"] {
        let result = run(&named, &["global", value]);
        assert_eq!(result.status.code(), Some(2), "{value}");
    }
    assert_eq!(run(&named, &["global"]).status.code(), Some(2));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn decisions_needs_json_and_a_missing_core_is_a_json_error_without_state() {
    let sandbox = Sandbox::new();
    let help = sandbox.run(&["decisions", "--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("never message text"));
    assert!(text.contains("--since"));
    let plain = sandbox.run(&["decisions"]);
    assert_eq!(plain.status.code(), Some(2));
    assert!(plain.stdout.is_empty());
    let bad = sandbox.run(&["decisions", "--json", "--since", "1h"]);
    assert_eq!(bad.status.code(), Some(2));
    sandbox.assert_untouched();
    let result = sandbox.run(&["decisions", "--json"]);
    assert_eq!(result.status.code(), Some(1));
    let document: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(document["version"], 1);
    assert_eq!(document["error"]["code"], "CORE_UNAVAILABLE");
    sandbox.assert_untouched();
}

#[test]
fn the_report_options_are_admitted_before_core_and_only_in_their_order() {
    let sandbox = Sandbox::new();
    for words in [
        &["decisions", "--json", "--recompute", "20m"][..],
        &["decisions", "--report", "--json", "--recompute", "auto"][..],
        &["decisions", "--report", "--json", "--recompute", "off"][..],
        &["decisions", "--report", "--json", "--recompute", "0s"][..],
        &["decisions", "--report", "--json", "--flush-count", "5"][..],
        &[
            "decisions",
            "--report",
            "--json",
            "--recompute",
            "20m",
            "--flush-count",
            "0",
        ][..],
        &["decisions", "--report"][..],
    ] {
        let result = sandbox.run(words);
        assert_eq!(result.status.code(), Some(2), "{words:?}");
        assert!(result.stdout.is_empty(), "{words:?}");
    }
    sandbox.assert_untouched();
    // An admitted report reaches Core, which this sandbox lacks.
    let result = sandbox.run(&[
        "decisions",
        "--report",
        "--json",
        "--recompute",
        "20m",
        "--flush-count",
        "5",
    ]);
    assert_eq!(result.status.code(), Some(1));
    let document: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(document["error"]["code"], "CORE_UNAVAILABLE");
    sandbox.assert_untouched();
}
