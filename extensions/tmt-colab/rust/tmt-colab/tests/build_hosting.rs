//! Hosted inventory generator, independent wire checks, and release headroom boundaries.
#[path = "../src/browser_policy.rs"]
mod browser_policy;
#[path = "../build/hosting.rs"]
mod hosting;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Build {
    root: PathBuf,
    app: PathBuf,
    sdk: PathBuf,
    output: PathBuf,
}
impl Build {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-hosting-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let app = root.join("hosted");
        let sdk = root.join("remote-v1.js");
        let output = root.join("cargo");
        fs::create_dir_all(app.join("assets")).unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(
            app.join("index.html"),
            b"<link rel=stylesheet href='/assets/app.css'><script type=module src='/sdk/remote-v1.js'></script>",
        )
        .unwrap();
        fs::write(app.join("assets/app.css"), b"body{}").unwrap();
        fs::write(
            app.join("reader.html"),
            b"<!doctype html><title>Reader fixture</title>",
        )
        .unwrap();
        fs::write(
            app.join("renderer.html"),
            b"<!doctype html><title>Renderer fixture</title>",
        )
        .unwrap();
        fs::write(
            app.join("THIRD-PARTY-NOTICES.txt"),
            b"Fixture dependency notices",
        )
        .unwrap();
        fs::write(&sdk, b"export const fixture = 'Remote SDK output';").unwrap();
        Self {
            root,
            app,
            sdk,
            output,
        }
    }
    fn generate(&self) -> std::io::Result<Vec<PathBuf>> {
        hosting::generate(Some(&self.app), &self.sdk, &self.output)
    }
}
impl Drop for Build {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn snapshot_matches_canonical_manifest_and_upstream_sdk_without_runtime_source_paths() {
    let build = Build::new();
    let inputs = build.generate().unwrap();
    assert!(inputs.contains(&build.sdk));
    assert!(inputs.contains(&build.app.join("assets")));
    let manifest_bytes = fs::read(build.output.join("colab-hosting-manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let bundle: Value =
        serde_json::from_slice(&fs::read(build.output.join("colab-hosting-bundle.json")).unwrap())
            .unwrap();
    assert_eq!(bundle.as_object().unwrap().len(), 3);
    assert_eq!(bundle["version"], 1);
    assert_eq!(bundle["manifestDigest"], digest(&manifest_bytes));
    // Canonical identity is the Remote struct field order, not Value's alphabetical key order.
    assert!(manifest_bytes.starts_with(b"{\"version\":1,\"files\":[{\"path\":"));
    let files = manifest["files"].as_array().unwrap();
    let paths: Vec<_> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        vec![
            "/THIRD-PARTY-NOTICES.txt",
            "/assets/app.css",
            "/index.html",
            "/reader.html",
            "/renderer.html",
            "/sdk/remote-v1.js"
        ]
    );
    for (metadata, wire) in files.iter().zip(bundle["files"].as_array().unwrap()) {
        assert_eq!(wire.as_object().unwrap().len(), 2);
        assert_eq!(wire["path"], metadata["path"]);
        let encoded = wire["bytesBase64"].as_str().unwrap();
        let bytes = STANDARD.decode(encoded).unwrap();
        assert_eq!(STANDARD.encode(&bytes), encoded);
        assert_eq!(metadata["length"], bytes.len());
        assert_eq!(metadata["sha256"], digest(&bytes));
        assert!(!metadata["contentType"].as_str().unwrap().contains(';'));
        if metadata["path"] == "/renderer.html" {
            assert_eq!(metadata["csp"], browser_policy::RENDERER_POLICY);
        } else {
            assert_eq!(metadata["csp"], browser_policy::POLICY);
        }
        if metadata["path"] == "/sdk/remote-v1.js" {
            assert_eq!(bytes, fs::read(&build.sdk).unwrap());
            assert_eq!(
                bytes,
                fs::read(build.output.join("colab-remote-v1.js")).unwrap()
            );
        }
    }
    fs::write(&build.sdk, b"Changed upstream output").unwrap();
    fs::write(build.app.join("index.html"), b"Changed entry").unwrap();
    assert_eq!(
        fs::read(build.output.join("colab-hosting-manifest.json")).unwrap(),
        manifest_bytes
    );
    let generated = fs::read_to_string(build.output.join("colab_hosting.rs")).unwrap();
    assert!(!generated.contains(build.app.to_str().unwrap()));
    assert!(!generated.contains(build.sdk.to_str().unwrap()));
}

#[test]
fn changed_sdk_copy_is_refused_and_input_names_cannot_select_renderer_policy() {
    let build = Build::new();
    let sdk = fs::read(&build.sdk).unwrap();
    hosting::verify_sdk_copy(&digest(&sdk), &sdk).unwrap();
    let mut changed = sdk.clone();
    changed[0] ^= 1;
    assert!(
        hosting::verify_sdk_copy(&digest(&sdk), &changed)
            .unwrap_err()
            .to_string()
            .contains("copy digest mismatch")
    );
    fs::write(
        build.app.join("renderer-copy.html"),
        b"<meta http-equiv=Content-Security-Policy content=\"connect-src *\">",
    )
    .unwrap();
    build.generate().unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(build.output.join("colab-hosting-manifest.json")).unwrap(),
    )
    .unwrap();
    let copy = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "/renderer-copy.html")
        .unwrap();
    assert_eq!(copy["csp"], browser_policy::POLICY);
    assert_ne!(copy["csp"], browser_policy::RENDERER_POLICY);
}

#[test]
fn absent_inventory_embeds_no_native_or_checkout_fallback() {
    let build = Build::new();
    assert!(
        hosting::generate(None, &build.root.join("missing-sdk"), &build.output)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(build.output.join("colab_hosting.rs")).unwrap(),
        "const BUNDLE: Option<&[u8]> = None;\nconst MANIFEST: Option<&[u8]> = None;\n"
    );
}

#[test]
fn incomplete_unsafe_or_oversized_hosted_builds_are_refused() {
    for case in [
        "entry",
        "notice",
        "empty",
        "link",
        "directory-link",
        "unknown",
        "sdk-override",
        "reserved",
        "sdk-missing",
        "sdk-link",
        "file-cap",
        "total-cap",
        "count-cap",
    ] {
        let build = Build::new();
        match case {
            "entry" => fs::remove_file(build.app.join("index.html")).unwrap(),
            "notice" => fs::remove_file(build.app.join("THIRD-PARTY-NOTICES.txt")).unwrap(),
            "empty" => fs::write(build.app.join("empty.js"), []).unwrap(),
            "link" => symlink(&build.sdk, build.app.join("link.js")).unwrap(),
            "directory-link" => {
                symlink(build.app.join("assets"), build.app.join("linked")).unwrap()
            }
            "unknown" => fs::write(build.app.join("unknown.exe"), b"fixture").unwrap(),
            "sdk-override" => {
                fs::create_dir(build.app.join("sdk")).unwrap();
                fs::write(build.app.join("sdk/remote-v1.js"), b"modified SDK").unwrap();
            }
            "reserved" => {
                fs::create_dir(build.app.join("__")).unwrap();
                fs::write(build.app.join("__/init.json"), b"{}").unwrap();
            }
            "sdk-missing" => fs::remove_file(&build.sdk).unwrap(),
            "sdk-link" => {
                fs::remove_file(&build.sdk).unwrap();
                symlink(build.app.join("index.html"), &build.sdk).unwrap();
            }
            "file-cap" => fs::File::create(build.app.join("large.js"))
                .unwrap()
                .set_len(hosting::FILE_BYTES as u64)
                .unwrap(),
            "total-cap" => {
                let seeded: u64 = [
                    build.app.join("index.html"),
                    build.app.join("renderer.html"),
                    build.app.join("reader.html"),
                    build.app.join("THIRD-PARTY-NOTICES.txt"),
                    build.app.join("assets/app.css"),
                    build.sdk.clone(),
                ]
                .iter()
                .map(|p| fs::metadata(p).unwrap().len())
                .sum();
                let mut remaining = hosting::TOTAL_BYTES as u64 - seeded;
                for n in 0..4 {
                    let length = remaining.min(hosting::FILE_BYTES as u64 - 1);
                    fs::File::create(build.app.join(format!("large-{n}.js")))
                        .unwrap()
                        .set_len(length)
                        .unwrap();
                    remaining -= length;
                }
                assert_eq!(remaining, 0);
            }
            "count-cap" => {
                for n in 0..hosting::FILES - 6 {
                    fs::write(build.app.join(format!("extra-{n}.js")), b"fixture").unwrap();
                }
            }
            _ => unreachable!(),
        }
        assert!(build.generate().is_err(), "accepted {case}");
        assert!(
            !build.output.join("colab_hosting.rs").exists(),
            "emitted inventory for {case}"
        );
    }
}
