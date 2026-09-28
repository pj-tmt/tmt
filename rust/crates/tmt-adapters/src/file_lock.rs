//! Stable, regular-file advisory locks shared by independent publication owners.

use nix::fcntl::{Flock, FlockArg, OFlag};
use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

pub fn exclusive(path: &Path) -> io::Result<Flock<File>> {
    // Never unlink: replacing the inode would split concurrent lock domains.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Publication lock is not a regular file."));
    }
    Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_, error)| {
        io::Error::new(
            io::Error::from_raw_os_error(error as i32).kind(),
            format!("Cannot acquire publication lock; another installer may be running: {error}"),
        )
    })
}
