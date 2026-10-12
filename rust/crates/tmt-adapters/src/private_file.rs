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
    // A refused create grants no cleanup ownership over an existing stage.
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staging)?;
    let result = (|| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::os::unix::fs::PermissionsExt;

    fn staging(path: &Path) -> std::path::PathBuf {
        path.parent().unwrap().join(format!(
            ".{}.{}.tmp",
            path.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ))
    }

    #[test]
    fn a_rejected_publisher_preserves_an_existing_stage_and_destination() {
        let directory = TestDirectory::new();
        let destination = directory.path.join("settings.json");
        let stage = staging(&destination);
        fs::write(&destination, b"committed settings").unwrap();
        fs::write(&stage, b"another publisher's bytes").unwrap();

        let error = replace(&destination, b"rejected update").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"committed settings");
        assert_eq!(fs::read(&stage).unwrap(), b"another publisher's bytes");
    }

    #[test]
    fn a_failed_rename_removes_only_the_stage_this_publisher_created() {
        let directory = TestDirectory::new();
        let destination = directory.path.join("settings.json");
        fs::create_dir(&destination).unwrap();
        let kept = destination.join("user-file");
        fs::write(&kept, b"preserve").unwrap();

        assert!(replace(&destination, b"update").is_err());
        assert_eq!(fs::read(&kept).unwrap(), b"preserve");
        assert!(!staging(&destination).exists());
    }

    #[test]
    fn successful_publication_has_exact_private_bytes_and_no_stage() {
        let directory = TestDirectory::new();
        let destination = directory.path.join("nested/settings.json");
        for bytes in [b"first".as_slice(), b"replacement\n\0"] {
            replace(&destination, bytes).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), bytes);
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(!staging(&destination).exists());
        }
    }
}
