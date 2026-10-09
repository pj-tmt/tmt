//! Resolve the installed Firebase launcher, never a cwd Node module or helper file.
use crate::{deploy_firestore::FIREBASE_TOOLS_VERSIONS, limits, wire};
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDiscoveryError {
    FirebaseMissing,
    NodeMissing,
    Unsupported,
}
/// Absolute executable and package paths passed to the embedded compatibility gate.
#[derive(Debug, PartialEq, Eq)]
pub struct FirebaseTools {
    pub node: PathBuf,
    pub package: PathBuf,
}

/// PATH entries must be absolute: a relative entry must not promote a project-local tool.
pub fn discover(search: &OsStr) -> Result<FirebaseTools, ToolDiscoveryError> {
    let cwd = std::env::current_dir().map_err(|_| ToolDiscoveryError::Unsupported)?;
    discover_from(search, &cwd)
}
/// Explicit cwd input lets fixtures prove project-local PATH entries cannot win.
pub fn discover_from(search: &OsStr, cwd: &Path) -> Result<FirebaseTools, ToolDiscoveryError> {
    let paths: Vec<_> = std::env::split_paths(search)
        .filter(|p| p.is_absolute())
        .collect();
    let search = std::env::join_paths(paths).map_err(|_| ToolDiscoveryError::Unsupported)?;
    let firebase = tmt_invoke::find_executable(OsStr::new("firebase"), &search)
        .ok_or(ToolDiscoveryError::FirebaseMissing)?;
    let firebase = fs::canonicalize(firebase).map_err(|_| ToolDiscoveryError::Unsupported)?;
    let cwd = fs::canonicalize(cwd).map_err(|_| ToolDiscoveryError::Unsupported)?;
    if cwd
        .ancestors()
        .any(|directory| firebase.starts_with(directory.join("node_modules")))
    {
        return Err(ToolDiscoveryError::Unsupported);
    }
    let package = firebase
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or(ToolDiscoveryError::Unsupported)?
        .to_path_buf();
    if firebase != package.join("lib/bin/firebase.js") {
        return Err(ToolDiscoveryError::Unsupported);
    }
    let bytes = bounded(
        &package.join("package.json"),
        limits::DEPLOY_TOOL_METADATA_BYTES,
    )?;
    let metadata = wire::strict_json(&bytes).ok_or(ToolDiscoveryError::Unsupported)?;
    if metadata["name"] != "firebase-tools"
        || !metadata["version"]
            .as_str()
            .is_some_and(|v| FIREBASE_TOOLS_VERSIONS.contains(&v))
    {
        return Err(ToolDiscoveryError::Unsupported);
    }
    let bytes = bounded(&firebase, limits::DEPLOY_PROVIDER_BYTES)?;
    let line = bytes
        .split(|b| *b == b'\n')
        .next()
        .ok_or(ToolDiscoveryError::Unsupported)?;
    let line = std::str::from_utf8(line)
        .map_err(|_| ToolDiscoveryError::Unsupported)?
        .trim_end_matches('\r');
    let node = if line == "#!/usr/bin/env node" {
        tmt_invoke::find_executable(OsStr::new("node"), &search)
            .ok_or(ToolDiscoveryError::NodeMissing)?
    } else {
        let path = PathBuf::from(
            line.strip_prefix("#!")
                .ok_or(ToolDiscoveryError::Unsupported)?,
        );
        if !path.is_absolute()
            || path.file_name() != Some(OsStr::new("node"))
            || !tmt_invoke::is_executable(&path)
        {
            return Err(ToolDiscoveryError::Unsupported);
        }
        path
    };
    let node = fs::canonicalize(node).map_err(|_| ToolDiscoveryError::NodeMissing)?;
    if cwd
        .ancestors()
        .any(|directory| node.starts_with(directory.join("node_modules")))
    {
        return Err(ToolDiscoveryError::Unsupported);
    }
    Ok(FirebaseTools { node, package })
}
fn bounded(path: &Path, cap: usize) -> Result<Vec<u8>, ToolDiscoveryError> {
    let file = fs::File::open(path).map_err(|_| ToolDiscoveryError::Unsupported)?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(ToolDiscoveryError::Unsupported);
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ToolDiscoveryError::Unsupported)?;
    if bytes.len() > cap {
        return Err(ToolDiscoveryError::Unsupported);
    }
    Ok(bytes)
}
