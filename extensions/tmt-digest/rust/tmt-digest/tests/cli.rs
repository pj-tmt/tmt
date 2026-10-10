//! Observable help-only process contract, isolated from the user's state.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-digest-help-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn run(&self, words: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tmt-digest"))
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
    assert!(help.contains("help and version only"));
    assert!(help.contains("not available yet"));
    assert!(help.contains("Examples:"));
    for words in [&[][..], &["-h"][..], &["help"][..], &["--"][..]] {
        let result = sandbox.run(words);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(result.stdout, help.as_bytes());
    }
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
    for words in [
        &["worker", "30m"][..],
        &["tick"][..],
        &["--json"][..],
        &["help", "tick"][..],
    ] {
        let result = sandbox.run(words);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("unexpected argument")
        );
    }
    sandbox.assert_untouched();
}
