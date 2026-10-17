//! Singleton ownership; never wait or infer death from a metadata record.
use crate::{StateError, locking};
use nix::fcntl::Flock;
use std::{fs::File, path::Path};

pub struct Guard {
    _guard: Flock<File>,
}
impl Guard {
    pub fn acquire(path: &Path, retry: &str) -> Result<Self, StateError> {
        if !path.is_absolute() || retry.is_empty() {
            return Err(StateError::Invalid {
                path: path.into(),
                reason: "guard requires an absolute path and exact retry",
            });
        }
        let mut guard = locking::acquire(path, false, retry)?;
        locking::record(&mut guard, path)?;
        Ok(Self { _guard: guard })
    }
}
