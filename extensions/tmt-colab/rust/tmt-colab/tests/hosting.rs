//! The release command survives relocation without core, state, or app-directory fallback.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn relocated_bundle_uses_only_embedded_bytes_and_rejects_digest_arguments() {
    let root = std::env::temp_dir().join(format!("tmt-hosting-cli-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    let source = env!("CARGO_BIN_EXE_tmt-colab");
    let relocated = root.join("colab");
    tmt_test_support::write_executable(
        &relocated,
        &fs::read(source).unwrap(),
        fs::metadata(source).unwrap().permissions().mode() & 0o777,
    )
    .unwrap();
    let fake = root.join("dist");
    fs::create_dir(&fake).unwrap();
    fs::write(
        fake.join("index.html"),
        b"Runtime fallback must not be served",
    )
    .unwrap();
    let invoke = |args: &[&str]| {
        Command::new(&relocated)
            .env_clear()
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("TMT_EXECUTABLE", root.join("missing-core"))
            .env("PATH", "")
            .env("TMT_COLAB_HOSTING_DIR", &fake)
            .env("TMT_COLAB_APP_DIR", &fake)
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap()
    };
    let output = invoke(&["hosting-bundle", "--json"]);
    match tmt_colab::hosting::manifest() {
        Some(manifest) => {
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
            let body: Value = serde_json::from_slice(&output.stdout).unwrap();
            let manifest: Value = serde_json::from_slice(manifest).unwrap();
            assert_eq!(body["version"], 1);
            assert_eq!(
                body["files"].as_array().unwrap().len(),
                manifest["files"].as_array().unwrap().len()
            );
            assert_eq!(
                output.stdout.strip_suffix(b"\n").unwrap(),
                tmt_colab::hosting::bundle().unwrap()
            );
            // Remote compares the bundle with the declared manifest before any provider call. Here
            // the two commands agree exactly, from the executable's own bytes and no network: the
            // manifest is the declaration's last key, verbatim, and every file matches its entry.
            let bytes = tmt_colab::hosting::manifest().unwrap();
            let declared = invoke(&["deploy-declaration", "--json"]);
            assert!(declared.status.success(), "{declared:?}");
            let reply: Value = serde_json::from_slice(&declared.stdout).unwrap();
            let declaration = reply["declaration"].as_str().unwrap();
            let section = format!(
                ",\n  \"hosting\": {}\n}}\n",
                std::str::from_utf8(bytes).unwrap()
            );
            assert!(declaration.ends_with(&section), "manifest is the last key");
            assert_eq!(
                reply["declarationDigest"],
                sha256_hex(declaration.as_bytes())
            );
            assert_eq!(body["manifestDigest"], sha256_hex(bytes));
            let declared: Value = serde_json::from_slice(bytes).unwrap();
            let entries = declared["files"].as_array().unwrap();
            let sent = body["files"].as_array().unwrap();
            assert_eq!(entries.len(), sent.len());
            for (entry, wire) in entries.iter().zip(sent) {
                let decoded = STANDARD
                    .decode(wire["bytesBase64"].as_str().unwrap())
                    .unwrap();
                assert_eq!(wire["path"], entry["path"]);
                assert_eq!(entry["length"], decoded.len());
                assert_eq!(entry["sha256"], sha256_hex(&decoded));
            }
        }
        None => {
            assert!(!output.status.success());
            let error: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(error["error"]["code"], "COLAB_UNAVAILABLE");
            // A binary with no hosted inventory declares no hosting key.
            let declared = invoke(&["deploy-declaration", "--json"]);
            assert!(declared.status.success(), "{declared:?}");
            let reply: Value = serde_json::from_slice(&declared.stdout).unwrap();
            let declaration: Value =
                serde_json::from_str(reply["declaration"].as_str().unwrap()).unwrap();
            assert!(declaration.get("hosting").is_none());
        }
    }
    let invalid = invoke(&["hosting-bundle", "--json", "--digest", "unused"]);
    assert!(!invalid.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    for args in [["hosting-bundle", "--help"], ["help", "hosting-bundle"]] {
        let help = invoke(&args);
        assert!(help.status.success(), "{help:?}");
        assert!(
            String::from_utf8(help.stdout)
                .unwrap()
                .contains("tmt colab hosting-bundle --json")
        );
    }
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        2,
        "pure bundle command initialized state"
    );
}
