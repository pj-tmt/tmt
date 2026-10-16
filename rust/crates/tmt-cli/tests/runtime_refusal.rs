//! Storage refusals retain their code and use only the diagnosed runtime's fix.
#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tmt-runtime-refusal-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let state = support::state_dir(&self.0);
        if state.exists() {
            fs::set_permissions(state, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn confirmed_storage_denial_has_a_runtime_fix_only_when_the_sandbox_marker_is_set() {
    let fixture = Fixture::new();
    // Initialize only the private directory skeleton; no database is opened.
    drop(support::command(&fixture.0, &[]));
    let state = support::state_dir(&fixture.0);
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).unwrap();
    for marker in [None, Some(""), Some("seatbelt")] {
        let mut command =
            support::command(&fixture.0, &["identity", "create", "Private", "--json"]);
        if let Some(marker) = marker {
            command.env("CODEX_SANDBOX", marker);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["error"]["code"], "STORAGE_NOT_WRITABLE");
        if let Some(marker) = marker {
            assert_eq!(document["error"]["reason"], "codex-sandbox");
            assert_eq!(document["error"]["fix"], "tmt run <name> codex");
            assert!(
                document["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("cannot name itself, send as itself, or reply")
            );
            let output = support::command(&fixture.0, &["identity", "create", "Private"])
                .env("CODEX_SANDBOX", marker)
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains("fix: Exit this Codex, then start it with:  tmt run <name> codex")
            );
            assert!(!stderr.contains("escalat"));
            assert!(!stderr.contains("writable_roots"));
        } else {
            assert!(document["error"].get("reason").is_none());
            assert!(document["error"].get("fix").is_none());
        }
        assert!(!state.join("tmt.db").exists());
    }
    // A successful command with the marker is unchanged; it is not itself an
    // error, and no provider configuration is created by classification.
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let output = support::command(&fixture.0, &["identity", "create", "Private", "--json"])
        .env("CODEX_SANDBOX", "seatbelt")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(state.join("tmt.db").exists());
    assert!(!fixture.0.join("home/.codex/config.toml").exists());
}
