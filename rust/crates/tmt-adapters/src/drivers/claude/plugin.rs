//! Embedded provider assets; publication never exposes a partially written plugin.
use crate::{bounded_file, config::ConfigPaths};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const FILES: &[(&str, &str)] = &[
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../assets/claude-plugin/tmt-picker/.claude-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../assets/claude-plugin/tmt-picker/hooks/hooks.json"),
    ),
    (
        "hooks/register.mjs",
        include_str!("../../../assets/claude-plugin/tmt-picker/hooks/register.mjs"),
    ),
];

fn private_directory(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other(
            "Provider plugin directory is not private.",
        ));
    }
    Ok(())
}

fn validate(path: &Path) -> io::Result<()> {
    for directory in [
        path.to_path_buf(),
        path.join(".claude-plugin"),
        path.join("hooks"),
    ] {
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::other("Invalid provider plugin directory."));
        }
    }
    // Engine-generated editor configuration and types are inert, never provider assets.
    for (directory, allowed) in [
        (
            path.to_path_buf(),
            &[".claude-plugin", "hooks", "tsconfig.json"][..],
        ),
        (path.join("hooks"), &["hooks.json", "register.mjs"][..]),
        (path.join(".claude-plugin"), &["plugin.json", "types"][..]),
    ] {
        for entry in fs::read_dir(directory)?.take(allowed.len() + 1) {
            let entry = entry?;
            if !allowed.iter().any(|name| entry.file_name() == *name) {
                return Err(io::Error::other("Unexpected provider plugin asset."));
            }
        }
    }
    for (generated, directory) in [
        (path.join("tsconfig.json"), false),
        (path.join(".claude-plugin/types"), true),
    ] {
        let metadata = match fs::symlink_metadata(generated) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o7022 != 0
        {
            return Err(io::Error::other("Invalid generated provider editor entry."));
        }
    }
    for (name, expected) in FILES {
        let file = path.join(name);
        let metadata = fs::symlink_metadata(&file)?;
        if !metadata.is_file()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || bounded_file::read_no_follow(&file, expected.len()).map_err(io::Error::other)?
                != expected.as_bytes()
        {
            return Err(io::Error::other("Provider plugin assets have changed."));
        }
    }
    Ok(())
}

pub(super) fn materialize(paths: &ConfigPaths) -> io::Result<PathBuf> {
    publish(paths, env!("CARGO_PKG_VERSION"))
}

fn publish(paths: &ConfigPaths, version: &str) -> io::Result<PathBuf> {
    let destination = std::path::absolute(paths.provider_plugin_directory(super::NAME, version))?;
    // The configured root is trusted; new provider-owned descendants are private.
    fs::create_dir_all(&paths.global_dir)?;
    private_directory(&paths.global_dir.join("provider-plugins"))?;
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("Missing plugin parent."))?
        .to_path_buf();
    private_directory(&parent)?;
    if fs::symlink_metadata(&destination).is_ok() {
        validate(&destination)?;
        return Ok(destination);
    }
    let staged = parent.join(format!(".stage-{}", uuid::Uuid::new_v4()));
    private_directory(&staged)?;
    let result = (|| {
        private_directory(&staged.join(".claude-plugin"))?;
        private_directory(&staged.join("hooks"))?;
        for (name, bytes) in FILES {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(staged.join(name))?;
            file.write_all(bytes.as_bytes())?;
            file.sync_all()?;
        }
        for directory in [".claude-plugin", "hooks", ""] {
            File::open(staged.join(directory))?.sync_all()?;
        }
        match fs::rename(&staged, &destination) {
            Ok(()) => File::open(&parent)?.sync_all()?,
            Err(error) => {
                // Another launcher may have published this complete version first.
                validate(&destination).map_err(|_| error)?;
            }
        }
        validate(&destination)?;
        Ok(destination)
    })();
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    result
}

#[cfg(test)]
mod tests;
