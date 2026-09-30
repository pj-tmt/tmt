//! Settings files TMT replaces whole, readable only by their owner. A crash
//! leaves the previous file or the new one, never a mix.

use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// Writes `bytes` to `path` with mode 0600 through a staged file renamed
/// into place, creating the directory first.
pub fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No settings directory.",
        ));
    };
    fs::create_dir_all(directory)?;
    let staging = directory.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&staging)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&staging, path)?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}
