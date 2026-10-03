//! Immutable local-build app bytes. Request paths never reach the filesystem.
use crate::{Result, limits};
use nix::{
    fcntl::{OFlag, open, openat},
    sys::stat::Mode,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::Path,
};

pub const DEFAULT_DIRECTORY: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../typescript/app/dist");
pub const BUILD_HINT: &str =
    "build the app: corepack pnpm --dir typescript --filter @tmt/colab-app build";
/// srcdoc inherits this policy; its stricter renderer CSP denies network,
/// while the unconditional opaque sandbox denies app storage.
pub const POLICY: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'";

#[derive(Debug)]
pub struct AssetFault(String);
impl std::fmt::Display for AssetFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Colab app build is unavailable: {}. {BUILD_HINT}",
            self.0
        )
    }
}
impl std::error::Error for AssetFault {}

struct Asset {
    content_type: &'static str,
    bytes: Vec<u8>,
}
pub struct App {
    files: BTreeMap<String, Asset>,
}
impl App {
    /// An explicit selection must be valid. A missing/incomplete default keeps
    /// the foreground service available with owner-only build instructions.
    pub fn selected(explicit: Option<&Path>) -> Result<Option<Self>> {
        Self::selected_or_default(explicit, Path::new(DEFAULT_DIRECTORY))
    }
    fn selected_or_default(explicit: Option<&Path>, default: &Path) -> Result<Option<Self>> {
        let directory = explicit.unwrap_or(default);
        match Self::load(directory) {
            Ok(app) => Ok(Some(app)),
            Err(error) if explicit.is_some() => {
                Err(AssetFault(format!("{}: {error}", directory.display())).into())
            }
            Err(_) => Ok(None),
        }
    }
    pub fn load(directory: &Path) -> Result<Self> {
        if !directory.is_absolute() || !fs::symlink_metadata(directory)?.is_dir() {
            return Err("App directory must be an absolute, real directory.".into());
        }
        let directory = directory.canonicalize()?;
        let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK;
        let root = File::from(open(&directory, flags | OFlag::O_DIRECTORY, Mode::empty())?);
        let assets = File::from(openat(
            &root,
            "assets",
            flags | OFlag::O_DIRECTORY,
            Mode::empty(),
        )?);
        let mut app = Self {
            files: BTreeMap::new(),
        };
        let mut total = 0;
        for entry in fs::read_dir(&directory)? {
            let name = entry?.file_name();
            if name == "assets" {
                continue;
            }
            let name = name.to_str().ok_or("App filename must be UTF-8.")?;
            if !matches!(name, "index.html" | "THIRD-PARTY-NOTICES.txt") {
                return Err("Unexpected app build output.".into());
            }
            app.insert(&root, name, format!("/{name}"), &mut total)?;
        }
        // Names come from enumeration; all opens stay anchored to the admitted
        // directory handles, including if a build replaces a directory meanwhile.
        for entry in fs::read_dir(directory.join("assets"))? {
            let name = entry?.file_name();
            let name = name.to_str().ok_or("App filename must be UTF-8.")?;
            if name.starts_with('.')
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            {
                return Err("Unsafe app asset filename.".into());
            }
            app.insert(&assets, name, format!("/assets/{name}"), &mut total)?;
        }
        let (_, index) = app.find("/").ok_or("App index is missing.")?;
        let index = std::str::from_utf8(index)?;
        // Vite emits relative, quoted entry references. Check them so a partially
        // copied build cannot silently publish an index whose entry is missing.
        for reference in index.split("\"./assets/").skip(1) {
            let name = reference
                .split('"')
                .next()
                .ok_or("Invalid app entry reference.")?;
            if app.find(&format!("/assets/{name}")).is_none() {
                return Err("App entry asset is missing.".into());
            }
        }
        for kind in ["text/javascript; charset=utf-8", "text/css; charset=utf-8"] {
            if !app.files.values().any(|asset| asset.content_type == kind) {
                return Err("App build requires JavaScript and CSS.".into());
            }
        }
        Ok(app)
    }
    fn insert(
        &mut self,
        directory: &File,
        name: &str,
        route: String,
        total: &mut usize,
    ) -> Result<()> {
        if self.files.len() >= limits::APP_FILES {
            return Err("App has too many assets.".into());
        }
        let content_type = content_type(name).ok_or("Unsupported app asset type.")?;
        let file = File::from(openat(
            directory,
            name,
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK,
            Mode::empty(),
        )?);
        let metadata = file.metadata()?;
        let remaining = limits::APP_BYTES - *total;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > remaining as u64 {
            return Err("App assets must be nonempty bounded regular files.".into());
        }
        let mut bytes = Vec::new();
        file.take(remaining as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() > remaining {
            return Err("App exceeds its byte budget.".into());
        }
        *total += bytes.len();
        self.files.insert(
            route,
            Asset {
                content_type,
                bytes,
            },
        );
        Ok(())
    }
    pub fn find(&self, path: &str) -> Option<(&'static str, &[u8])> {
        let path = if path == "/" { "/index.html" } else { path };
        self.files
            .get(path)
            .map(|asset| (asset.content_type, asset.bytes.as_slice()))
    }
}
fn content_type(name: &str) -> Option<&'static str> {
    Some(match name.rsplit('.').next()? {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
