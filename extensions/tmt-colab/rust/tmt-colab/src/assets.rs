//! Immutable embedded or local-build app bytes. Requests never reach the filesystem.
use crate::{Result, app_inventory, limits};
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
/// Trusted chrome admits only build-owned scripts and styles.
pub const POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'";

/// The renderer is opaque even when opened directly rather than in an iframe.
pub const RENDERER_POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; font-src 'none'; media-src 'none'; worker-src 'none'; manifest-src 'none'; sandbox allow-scripts";

/// Public static bytes an unpaired browser may fetch: the read-only reader entry (served at
/// `/read`, which keeps the entry's relative `./assets/` references under the mount), the files it
/// loads, the opaque renderer and the guidance script. Owner app files stay owner-only. The names
/// are fixed by the `reader` and `recovery` build modes, so this allowlist is exact.
pub const READER_ROUTE: &str = "/read";
pub fn anonymous_file(path: &str) -> Option<&'static str> {
    Some(match path {
        READER_ROUTE => "/reader.html",
        "/renderer.html" => "/renderer.html",
        "/assets/reader.js" => "/assets/reader.js",
        "/assets/reader.css" => "/assets/reader.css",
        "/assets/reader-fold.js" => "/assets/reader-fold.js",
        "/assets/recovery.js" => "/assets/recovery.js",
        _ => return None,
    })
}

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

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/colab_assets.rs"));
}

struct Asset {
    content_type: &'static str,
    bytes: Vec<u8>,
}
pub struct App {
    files: BTreeMap<String, Asset>,
}
impl App {
    /// Explicit builds fail closed; embedded bytes precede local checkout output.
    pub fn selected(explicit: Option<&Path>) -> Result<Option<Self>> {
        Self::selected_from(explicit, embedded::ASSETS, Path::new(DEFAULT_DIRECTORY))
    }
    fn selected_from(
        explicit: Option<&Path>,
        embedded: &[(&str, &[u8])],
        default: &Path,
    ) -> Result<Option<Self>> {
        if let Some(directory) = explicit {
            return Self::load(directory)
                .map(Some)
                .map_err(|error| AssetFault(format!("{}: {error}", directory.display())).into());
        }
        if !embedded.is_empty() {
            return Self::from_embedded(embedded)
                .map(Some)
                .map_err(|error| AssetFault(format!("embedded inventory: {error}")).into());
        }
        Ok(Self::load(default).ok())
    }
    fn from_embedded(files: &[(&str, &[u8])]) -> Result<Self> {
        app_inventory::validate(files)?;
        Ok(Self {
            files: files
                .iter()
                .map(|&(route, bytes)| {
                    (
                        route.to_owned(),
                        Asset {
                            content_type: app_inventory::content_type(route)
                                .expect("validated route"),
                            bytes: bytes.to_vec(),
                        },
                    )
                })
                .collect(),
        })
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
            app.insert(&root, name, format!("/{name}"), &mut total)?;
        }
        // Names come from enumeration; all opens stay anchored to the admitted
        // directory handles, including if a build replaces a directory meanwhile.
        for entry in fs::read_dir(directory.join("assets"))? {
            let name = entry?.file_name();
            let name = name.to_str().ok_or("App filename must be UTF-8.")?;
            app.insert(&assets, name, format!("/assets/{name}"), &mut total)?;
        }
        app_inventory::validate(
            &app.files
                .iter()
                .map(|(route, asset)| (route.as_str(), asset.bytes.as_slice()))
                .collect::<Vec<_>>(),
        )?;
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
        let content_type =
            app_inventory::content_type(&route).ok_or("Unsupported app asset type.")?;
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
#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
