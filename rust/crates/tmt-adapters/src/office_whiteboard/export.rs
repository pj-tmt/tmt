//! Publish a requested PNG path without replacing files or exposing partial content.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn export_snapshot_image(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    if destination.file_name().is_none()
        || bytes.is_empty()
        || bytes.len() > tmt_office_model::codec::office_whiteboard::image::SNAPSHOT_PNG_LIMIT
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid snapshot export path or bytes.",
        ));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = parent.join(format!(".tmt-snapshot-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let stage = Stage(path);
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // Linking within the destination directory is atomic and refuses existing
    // names, including symlinks. Unlike rename, it never clobbers another file.
    fs::hard_link(&stage.0, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn export_is_private_no_clobber_and_removes_staging_after_failure() {
        let dir = TestDirectory::new();
        let target = dir.path.join("snapshot.png");
        // This test isolates publication, not PNG admission (owned by the codec).
        export_snapshot_image(&target, b"stored bytes").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"stored bytes");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            export_snapshot_image(&target, b"replacement")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"stored bytes");
        let link = dir.path.join("symlink.png");
        symlink(&target, &link).unwrap();
        assert_eq!(
            export_snapshot_image(&link, b"replacement")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"stored bytes");
        assert!(export_snapshot_image(&dir.path.join("missing/file.png"), b"content").is_err());
        let entries: Vec<_> = fs::read_dir(&dir.path)
            .unwrap()
            .map(|item| item.unwrap().file_name())
            .collect();
        assert_eq!(
            entries.len(),
            2,
            "only destination and user-created symlink remain"
        );
    }
}
