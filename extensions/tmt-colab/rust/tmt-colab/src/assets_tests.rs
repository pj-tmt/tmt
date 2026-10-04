use super::*;
use std::{
    os::unix::fs::symlink,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Build(PathBuf);
impl Build {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-1253-assets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("index.html"), br#"<link href="./assets/app.css"><script type="module" src="./assets/app.js"></script>"#).unwrap();
        fs::write(
            root.join("renderer.html"),
            b"<!doctype html><title>Renderer</title>",
        )
        .unwrap();
        fs::write(
            root.join("reader.html"),
            br#"<script type="module" src="./assets/app.js"></script>"#,
        )
        .unwrap();
        fs::write(root.join("assets/app.js"), b"export {};").unwrap();
        fs::write(root.join("assets/app.css"), b"body{color:red}").unwrap();
        Self(root)
    }
}
impl Drop for Build {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn default_falls_back_but_explicit_selection_fails_for_the_same_incomplete_build() {
    let build = Build::new();
    let missing = build.0.join("missing");
    assert!(App::selected_from(None, &[], &build.0).unwrap().is_some());
    assert!(App::selected_from(Some(&missing), &[], &build.0).is_err());
    for directory in [&missing, &build.0] {
        if directory == &build.0 {
            fs::remove_file(build.0.join("assets/app.css")).unwrap();
        }
        assert!(App::selected_from(None, &[], directory).unwrap().is_none());
        assert!(
            App::selected_from(Some(directory), &[], &build.0)
                .err()
                .unwrap()
                .downcast_ref::<AssetFault>()
                .is_some()
        );
    }
    assert!(App::selected_from(Some(Path::new("relative")), &[], &build.0).is_err());
}
#[test]
fn snapshot_survives_file_replacement_and_only_exact_keys_resolve() {
    let build = Build::new();
    let original = fs::read(build.0.join("assets/app.js")).unwrap();
    let app = App::load(&build.0).unwrap();
    fs::write(build.0.join("assets/app.js"), b"replacement").unwrap();
    assert_eq!(
        app.find("/assets/app.js"),
        Some(("text/javascript; charset=utf-8", original.as_slice()))
    );
    assert_eq!(app.find("/"), app.find("/index.html"));
    for path in [
        "/assets/../index.html",
        "/assets/%2e%2e/index.html",
        "//assets/app.js",
        "/assets/app.js?q=1",
        "/other",
        "/assets/./app.js",
    ] {
        assert!(app.find(path).is_none(), "{path}");
    }
}
#[test]
fn symlinks_directories_empty_files_and_unknown_outputs_refuse() {
    for unsafe_output in [
        "root-link",
        "asset-dir-link",
        "file-link",
        "nested",
        "empty",
        "map",
        "missing-entry",
        "missing-renderer",
        "missing-reader",
        "renderer-link",
    ] {
        let build = Build::new();
        match unsafe_output {
            "root-link" => {
                let path = build.0.join("link");
                symlink(&build.0, &path).unwrap();
                assert!(App::load(&path).is_err());
                continue;
            }
            "asset-dir-link" => {
                fs::rename(build.0.join("assets"), build.0.join("saved")).unwrap();
                symlink(build.0.join("saved"), build.0.join("assets")).unwrap();
            }
            "file-link" => {
                fs::remove_file(build.0.join("assets/app.js")).unwrap();
                symlink(build.0.join("index.html"), build.0.join("assets/app.js")).unwrap();
            }
            "nested" => {
                fs::create_dir(build.0.join("assets/subdir.js")).unwrap();
            }
            "empty" => {
                fs::write(build.0.join("assets/app.js"), b"").unwrap();
            }
            "map" => {
                fs::write(build.0.join("assets/app.js.map"), b"source").unwrap();
            }
            "missing-reader" => {
                fs::remove_file(build.0.join("reader.html")).unwrap();
            }
            "missing-renderer" => {
                fs::remove_file(build.0.join("renderer.html")).unwrap();
            }
            "renderer-link" => {
                fs::remove_file(build.0.join("renderer.html")).unwrap();
                symlink(build.0.join("index.html"), build.0.join("renderer.html")).unwrap();
            }
            "missing-entry" => {
                fs::write(
                    build.0.join("index.html"),
                    br#"<script src="./assets/missing.js"></script>"#,
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(App::load(&build.0).is_err(), "{unsafe_output}");
    }
}
#[test]
fn startup_bounds_bytes_and_file_count() {
    let build = Build::new();
    let oversized = File::create(build.0.join("assets/font.woff2")).unwrap();
    oversized.set_len(limits::APP_BYTES as u64 + 1).unwrap();
    assert!(App::load(&build.0).is_err());
    fs::remove_file(build.0.join("assets/font.woff2")).unwrap();
    for index in 0..limits::APP_FILES {
        fs::write(build.0.join(format!("assets/{index}.js")), b"x").unwrap();
    }
    assert!(App::load(&build.0).is_err());
}

#[test]
fn explicit_embedded_checkout_and_hint_have_one_selection_order() {
    let build = Build::new();
    let missing = build.0.join("missing");
    let embedded: &[(&str, &[u8])] = &[
        (
            "/index.html",
            b"<script src=\"./assets/embedded.js\"></script>",
        ),
        ("/assets/embedded.js", b"embedded"),
        ("/assets/embedded.css", b"body{}"),
        ("/renderer.html", b"renderer"),
        ("/reader.html", b"reader"),
    ];
    let app = App::selected_from(None, embedded, &missing)
        .unwrap()
        .unwrap();
    assert_eq!(app.find("/assets/embedded.js").unwrap().1, b"embedded");
    assert_eq!(app.find("/renderer.html").unwrap().1, b"renderer");
    // An embedded inventory missing the required renderer fails closed.
    assert!(App::selected_from(None, &embedded[..3], &build.0).is_err());
    assert!(App::selected_from(None, &embedded[..4], &build.0).is_err());
    let override_app = App::selected_from(Some(&build.0), embedded, &missing)
        .unwrap()
        .unwrap();
    assert!(override_app.find("/assets/embedded.js").is_none());
    assert_eq!(
        override_app.find("/assets/app.js").unwrap().1,
        b"export {};"
    );
    assert!(App::selected_from(Some(&missing), embedded, &build.0).is_err());
    assert!(App::selected_from(None, &[], &build.0).unwrap().is_some());
    assert!(App::selected_from(None, &[], &missing).unwrap().is_none());
    // Corrupt embedded bytes never fall back to a valid checkout.
    assert!(App::selected_from(None, &embedded[..1], &build.0).is_err());
}

#[test]
fn embedded_and_disk_admission_validate_renderer_entries_and_duplicates() {
    let build = Build::new();
    fs::write(
        build.0.join("renderer.html"),
        br#"<script src="./assets/app.js"></script>"#,
    )
    .unwrap();
    assert!(
        App::load(&build.0)
            .unwrap()
            .find("/renderer.html")
            .is_some()
    );
    fs::write(
        build.0.join("renderer.html"),
        br#"<script src="./assets/missing.js"></script>"#,
    )
    .unwrap();
    assert!(App::load(&build.0).is_err());
    let files: &[(&str, &[u8])] = &[
        ("/index.html", b"x"),
        ("/assets/a.js", b"x"),
        ("/assets/a.css", b"x"),
        ("/renderer.html", b"renderer"),
        ("/reader.html", b"reader"),
        ("/assets/a.js", b"duplicate"),
    ];
    assert!(App::from_embedded(files).is_err());
}
