use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=TMT_OFFICE_SPA_DIR");
    if env::var_os("CARGO_FEATURE_LOCAL_SERVICE").is_none() {
        return;
    }
    let directory = env::var_os("TMT_OFFICE_SPA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!("local-service requires: pnpm office:build:local, then set TMT_OFFICE_SPA_DIR")
        });
    let index = directory.join("index.html");
    let assets = directory.join("assets");
    for path in [&directory, &assets] {
        let metadata = fs::symlink_metadata(path).unwrap_or_else(|error| {
            panic!(
                "inspect local Office SPA directory {}: {error}",
                path.display()
            )
        });
        assert!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Office SPA directories must be real directories"
        );
    }
    let mut files = vec![("/index.html".to_owned(), index)];
    let entries = fs::read_dir(&assets).unwrap_or_else(|error| {
        panic!(
            "read local Office SPA assets at {}: {error}",
            assets.display()
        )
    });
    for entry in entries {
        let entry = entry.expect("read local Office SPA asset entry");
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).expect("inspect local Office SPA asset");
        assert!(
            metadata.is_file(),
            "Office SPA assets must be regular files"
        );
        let name = entry
            .file_name()
            .into_string()
            .expect("Office SPA asset names are UTF-8");
        assert!(
            !name.starts_with('.')
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
            "Office SPA asset name is unsafe"
        );
        assert!(
            name.ends_with(".js") || name.ends_with(".css") || name.ends_with(".png"),
            "Office SPA supports only generated JS, CSS and PNG assets"
        );
        files.push((format!("/assets/{name}"), path));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        // The renderer uses lazy chunks; the independent 8 MiB byte cap remains.
        (2..=32).contains(&files.len()),
        "Office SPA asset count is out of bounds"
    );
    let mut total = 0_u64;
    let mut source = String::from("pub const ASSETS: &[(&str, &str, &[u8])] = &[\n");
    for (route, path) in files {
        let metadata = fs::symlink_metadata(&path).expect("inspect local Office SPA file");
        assert!(
            metadata.is_file() && metadata.len() > 0,
            "Office SPA files must be nonempty"
        );
        total = total
            .checked_add(metadata.len())
            .expect("Office SPA size overflow");
        let content_type = if route.ends_with(".html") {
            "text/html; charset=utf-8"
        } else if route.ends_with(".js") {
            "text/javascript; charset=utf-8"
        } else if route.ends_with(".png") {
            "image/png"
        } else {
            "text/css; charset=utf-8"
        };
        source.push_str(&format!(
            "    ({route:?}, {content_type:?}, include_bytes!({path:?})),\n",
            path = path.canonicalize().expect("canonicalize Office SPA file")
        ));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    assert!(
        total <= 8 * 1024 * 1024,
        "Office SPA exceeds its 8 MiB bound"
    );
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"))
            .join("office_assets.rs"),
        source,
    )
    .expect("write embedded Office asset table");
}
