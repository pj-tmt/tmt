//! One flock primitive, with separate write-wait and singleton policies.
use crate::{Holder, StateError};
use nix::{
    errno::Errno,
    fcntl::{Flock, FlockArg, OFlag},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const WRITE_WAIT: Duration = Duration::from_secs(5);

pub(crate) fn try_file(file: File) -> Result<Flock<File>, (File, Errno)> {
    Flock::lock(file, FlockArg::LockExclusiveNonblock)
}
fn private_file(path: &Path) -> Result<File, StateError> {
    let parent = path.parent().ok_or_else(|| StateError::Invalid {
        path: path.into(),
        reason: "lock has no parent",
    })?;
    super::Layout::admit(parent.into(), &[]).map_err(|e| StateError::admission(parent, e))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(|e| StateError::io(path, "open lock", e))?;
    super::validate_file(&file).map_err(|e| StateError::admission(path, e))?;
    Ok(file)
}
fn holder(file: &mut File) -> Option<Holder> {
    if file.metadata().ok()?.len() > 128 {
        return None;
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = String::new();
    file.take(128).read_to_string(&mut bytes).ok()?;
    let mut lines = bytes.lines();
    if lines.next()? != "tmt-state-guard-v1" {
        return None;
    }
    let pid = lines.next()?.parse().ok()?;
    let acquired_at_ms = lines.next()?.parse().ok()?;
    if lines.next().is_some() || pid == 0 || acquired_at_ms == 0 {
        return None;
    }
    Some(Holder {
        pid,
        acquired_at_ms,
    })
}

pub(crate) fn acquire(path: &Path, wait: bool, retry: &str) -> Result<Flock<File>, StateError> {
    let deadline = Instant::now() + WRITE_WAIT;
    let mut file = private_file(path)?;
    loop {
        match try_file(file) {
            Ok(guard) => return Ok(guard),
            Err((returned, error)) => {
                file = returned;
                if error != Errno::EWOULDBLOCK {
                    return Err(StateError::io(
                        path,
                        "lock state",
                        io::Error::from_raw_os_error(error as i32),
                    ));
                }
                if !wait {
                    return Err(StateError::GuardBusy {
                        path: path.into(),
                        holder: holder(&mut file),
                        retry: retry.into(),
                    });
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(StateError::Timeout {
                        path: path.into(),
                        operation: "acquire write lock",
                        retry: retry.into(),
                        may_not_be_saved: false,
                        source: io::Error::from_raw_os_error(error as i32).into(),
                    });
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }
    }
}
pub(crate) fn record(guard: &mut Flock<File>, path: &Path) -> Result<(), StateError> {
    let acquired_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StateError::Invalid {
            path: path.into(),
            reason: "clock precedes Unix epoch",
        })?
        .as_millis();
    guard
        .set_len(0)
        .map_err(|e| StateError::io(path, "record guard", e))?;
    guard
        .seek(SeekFrom::Start(0))
        .map_err(|e| StateError::io(path, "record guard", e))?;
    writeln!(
        guard,
        "tmt-state-guard-v1\n{}\n{acquired_at_ms}",
        std::process::id()
    )
    .and_then(|()| guard.sync_all())
    .map_err(|e| StateError::io(path, "record guard", e))
}
