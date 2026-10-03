//! Build-only filesystem admission and snapshots for the embedded asset table.
use crate::app_inventory;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub fn generate(directory: Option<&Path>, output: &Path) -> io::Result<Vec<PathBuf>> {
    let mut inputs = Vec::new();
    let mut files = Vec::new();
    if let Some(directory) = directory {
        if !directory.is_absolute() {
            return Err(io::Error::other("TMT_COLAB_APP_DIR must be absolute"));
        }
        let assets = directory.join("assets");
        for path in [directory, &assets] {
            if !fs::symlink_metadata(path)?.is_dir() {
                return Err(io::Error::other("App directories must be real directories"));
            }
            inputs.push(path.to_path_buf());
        }
        let mut paths = Vec::new();
        for (prefix, parent) in [("/", directory), ("/assets/", assets.as_path())] {
            for entry in fs::read_dir(parent)? {
                let entry = entry?;
                let name = entry.file_name();
                if prefix == "/" && name == "assets" {
                    continue;
                }
                let name = name
                    .to_str()
                    .ok_or_else(|| io::Error::other("App filename must be UTF-8"))?;
                if paths.len() >= app_inventory::APP_FILES {
                    return Err(io::Error::other("App has too many assets"));
                }
                let route = format!("{prefix}{name}");
                if app_inventory::content_type(&route).is_none() {
                    return Err(io::Error::other("Unsupported or unsafe app output"));
                }
                paths.push((route, entry.path()));
            }
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        let mut total = 0usize;
        for (route, path) in paths {
            let metadata = fs::symlink_metadata(&path)?;
            let remaining = app_inventory::APP_BYTES - total;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > remaining as u64 {
                return Err(io::Error::other(
                    "App assets must be nonempty bounded regular files",
                ));
            }
            let mut bytes = Vec::new();
            fs::File::open(&path)?
                .take(remaining as u64 + 1)
                .read_to_end(&mut bytes)?;
            total += bytes.len();
            if total > app_inventory::APP_BYTES {
                return Err(io::Error::other("App exceeds its byte budget"));
            }
            inputs.push(path);
            files.push((route, bytes));
        }
        app_inventory::validate(
            &files
                .iter()
                .map(|(route, bytes)| (route.as_str(), bytes.as_slice()))
                .collect::<Vec<_>>(),
        )
        .map_err(io::Error::other)?;
        if !files
            .iter()
            .any(|(route, _)| route == "/THIRD-PARTY-NOTICES.txt")
        {
            return Err(io::Error::other("App dependency notices are missing"));
        }
    }
    let mut source = String::from("pub const ASSETS: &[(&str, &[u8])] = &[\n");
    for (index, (route, bytes)) in files.iter().enumerate() {
        let snapshot = output.join(format!("colab-asset-{index}"));
        fs::write(&snapshot, bytes)?;
        source.push_str(&format!("    ({route:?}, include_bytes!({snapshot:?})),\n"));
    }
    source.push_str("];\n");
    fs::write(output.join("colab_assets.rs"), source)?;
    Ok(inputs)
}
