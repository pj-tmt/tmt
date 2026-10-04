//! Exercise the real build generator without recursively invoking Cargo.
#[path = "../src/app_inventory.rs"]
mod app_inventory;
#[path = "../build/assets.rs"]
mod build_assets;
use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Build {
    root: PathBuf,
    output: PathBuf,
}
impl Build {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-1421-build-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("assets")).unwrap();
        // Cargo output is separate from the input tree.
        let output = root.with_extension("cargo");
        fs::create_dir(&output).unwrap();
        fs::write(
            root.join("index.html"),
            br#"<link href="./assets/a.css"><script src="./assets/a.js"></script>"#,
        )
        .unwrap();
        fs::write(
            root.join("renderer.html"),
            b"<!doctype html><title>Renderer</title>",
        )
        .unwrap();
        fs::write(
            root.join("reader.html"),
            b"<!doctype html><title>Reader</title>",
        )
        .unwrap();
        fs::write(root.join("assets/a.js"), b"console.log('embedded');").unwrap();
        fs::write(root.join("assets/a.css"), b"body{}").unwrap();
        fs::write(root.join("THIRD-PARTY-NOTICES.txt"), b"test notice").unwrap();
        Self { root, output }
    }
}
impl Drop for Build {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
        fs::remove_dir_all(&self.output).unwrap();
    }
}
#[test]
fn generated_snapshot_is_sorted_complete_and_independent_of_source_changes() {
    let build = Build::new();
    fs::write(
        build.root.join("renderer.html"),
        br#"<script src="./assets/a.js"></script>"#,
    )
    .unwrap();
    let inputs = build_assets::generate(Some(&build.root), &build.output).unwrap();
    assert!(inputs.contains(&build.root.join("renderer.html")));
    assert!(inputs.contains(&build.root.join("assets")));
    let table = fs::read_to_string(build.output.join("colab_assets.rs")).unwrap();
    assert!(table.contains("/renderer.html"));
    assert!(table.contains("/THIRD-PARTY-NOTICES.txt"));
    assert!(table.find("/assets/a.css").unwrap() < table.find("/assets/a.js").unwrap());
    assert!(!table.contains(&format!("{}/", build.root.display())));
    let snapshot = fs::read(build.output.join("colab-asset-2")).unwrap();
    assert_eq!(snapshot, b"console.log('embedded');");
    fs::write(build.root.join("assets/a.js"), b"replacement").unwrap();
    assert_eq!(
        fs::read(build.output.join("colab-asset-2")).unwrap(),
        snapshot
    );
    assert!(
        build_assets::generate(None, &build.output)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(build.output.join("colab_assets.rs")).unwrap(),
        "pub const ASSETS: &[(&str, &[u8])] = &[\n];\n"
    );
}
#[test]
fn generator_rejects_invalid_or_partial_builds_before_emitting_a_table() {
    for case in [
        "notice",
        "entry",
        "renderer",
        "missing-renderer",
        "missing-reader",
        "empty",
        "nested",
        "file-link",
        "asset-link",
        "root-link",
        "unknown",
        "bytes",
        "aggregate",
        "count",
    ] {
        let build = Build::new();
        let mut directory = build.root.clone();
        match case {
            "notice" => fs::remove_file(build.root.join("THIRD-PARTY-NOTICES.txt")).unwrap(),
            "entry" => fs::remove_file(build.root.join("assets/a.js")).unwrap(),
            "missing-renderer" => fs::remove_file(build.root.join("renderer.html")).unwrap(),
            "missing-reader" => fs::remove_file(build.root.join("reader.html")).unwrap(),
            "renderer" => fs::write(
                build.root.join("renderer.html"),
                br#"<script src="./assets/missing.js"></script>"#,
            )
            .unwrap(),
            "empty" => fs::write(build.root.join("assets/a.css"), b"").unwrap(),
            "nested" => fs::create_dir(build.root.join("assets/nested.js")).unwrap(),
            "file-link" => {
                fs::remove_file(build.root.join("assets/a.js")).unwrap();
                symlink(
                    build.root.join("index.html"),
                    build.root.join("assets/a.js"),
                )
                .unwrap();
            }
            "asset-link" => {
                fs::rename(build.root.join("assets"), build.root.join("saved")).unwrap();
                symlink(build.root.join("saved"), build.root.join("assets")).unwrap();
            }
            "root-link" => {
                directory = build.output.join("link");
                symlink(&build.root, &directory).unwrap();
            }
            "unknown" => fs::write(build.root.join("assets/a.js.map"), b"map").unwrap(),
            "bytes" => fs::File::create(build.root.join("assets/large.woff2"))
                .unwrap()
                .set_len(app_inventory::APP_BYTES as u64 + 1)
                .unwrap(),
            "aggregate" => {
                for name in ["left.woff2", "right.woff2"] {
                    fs::File::create(build.root.join("assets").join(name))
                        .unwrap()
                        .set_len((app_inventory::APP_BYTES / 2) as u64)
                        .unwrap();
                }
            }
            "count" => {
                for n in 0..app_inventory::APP_FILES {
                    fs::write(build.root.join(format!("assets/{n}.js")), b"x").unwrap();
                }
            }
            _ => unreachable!(),
        }
        assert!(
            build_assets::generate(Some(&directory), &build.output).is_err(),
            "{case}"
        );
        assert!(!build.output.join("colab_assets.rs").exists(), "{case}");
    }
    let build = Build::new();
    assert!(build_assets::generate(Some(std::path::Path::new("relative")), &build.output).is_err());
}
