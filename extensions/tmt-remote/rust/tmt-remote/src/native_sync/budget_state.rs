//! One project-local writer for conservative attempted-read costs. The serving
//! owner shares this value across consumers; no request gets a fresh quota pool.
use crate::{
    firestore_budget::{self, Decision, Model, NativeUsage, PacificDay},
    limits,
    state::Layout,
    store::uuid_v4,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBudgetError {
    Unavailable,
    Invalid,
    Busy,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedUsage {
    version: u8,
    day: i64,
    reads: u64,
}
pub struct NativeBudgetFile {
    directory: tmt_extension_state::Layout,
    path: PathBuf,
    project: String,
    _writer: File,
    usage: NativeUsage,
    saved: Option<SavedUsage>,
    dirty: bool,
    last_flush_ms: u64,
}
fn invalid() -> NativeBudgetError {
    NativeBudgetError::Invalid
}
fn unavailable() -> NativeBudgetError {
    NativeBudgetError::Unavailable
}
fn admitted(file: &File) -> Result<(), NativeBudgetError> {
    let m = file.metadata().map_err(|_| invalid())?;
    if !m.is_file()
        || m.uid() != nix::unistd::Uid::effective().as_raw()
        || m.mode() & 0o777 != 0o600
    {
        return Err(invalid());
    }
    Ok(())
}
impl NativeBudgetFile {
    pub fn open(
        layout: &Layout,
        project: &str,
        now_ms: u64,
        now_monotonic_ms: u64,
    ) -> Result<Self, NativeBudgetError> {
        if !crate::deploy_run::project_ok(project) {
            return Err(invalid());
        }
        let directory = tmt_extension_state::Layout::open(&layout.directory, "native-budget", &[])
            .map_err(|_| invalid())?;
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(directory.directory.join(format!("{project}.lock")))
            .map_err(|_| invalid())?;
        admitted(&writer)?;
        writer.try_lock().map_err(|_| NativeBudgetError::Busy)?;
        let path = directory.directory.join(format!("{project}.json"));
        let saved = match OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => {
                admitted(&file)?;
                if file.metadata().map_err(|_| invalid())?.len() > limits::NATIVE_BUDGET_BYTES {
                    return Err(invalid());
                }
                let mut bytes = Vec::new();
                file.take(limits::NATIVE_BUDGET_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| invalid())?;
                if bytes.len() as u64 > limits::NATIVE_BUDGET_BYTES {
                    return Err(invalid());
                }
                let saved: SavedUsage = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if saved.version != 1 {
                    return Err(invalid());
                }
                Some(saved)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(invalid()),
        };
        let usage = saved.as_ref().map_or_else(
            || NativeUsage::new(now_ms),
            |saved| NativeUsage {
                day: PacificDay(saved.day),
                reads: saved.reads,
            },
        );
        Ok(Self {
            directory,
            path,
            project: project.into(),
            _writer: writer,
            usage,
            saved,
            dirty: false,
            last_flush_ms: now_monotonic_ms,
        })
    }
    pub fn usage(&self) -> NativeUsage {
        self.usage
    }
    pub fn project(&self) -> &str {
        &self.project
    }
    pub fn authorize(
        &mut self,
        model: &Model,
        maximum_reads: u64,
        now_ms: u64,
        now_monotonic_ms: u64,
    ) -> Result<Decision, NativeBudgetError> {
        let before = self.usage;
        let decision =
            firestore_budget::charge_native(model, &mut self.usage, maximum_reads, now_ms)
                .map_err(|_| invalid())?;
        self.dirty |= self.usage != before;
        if now_monotonic_ms.saturating_sub(self.last_flush_ms) >= limits::NATIVE_BUDGET_FLUSH_MS {
            self.checkpoint(now_monotonic_ms)?;
        }
        Ok(decision)
    }
    /// A serving-owned minute tick persists costs even during backoff or idle.
    /// This file owner creates no thread or timer and flushes only dirty state.
    pub fn checkpoint(&mut self, now_monotonic_ms: u64) -> Result<(), NativeBudgetError> {
        self.flush()?;
        self.last_flush_ms = now_monotonic_ms;
        Ok(())
    }
    pub(super) fn flush(&mut self) -> Result<(), NativeBudgetError> {
        if !self.dirty {
            return Ok(());
        }
        // Re-admit the destination before replacement; foreign/unsafe data is
        // never reset simply because our in-memory counter is authoritative.
        match OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&self.path)
        {
            Ok(file) => {
                admitted(&file)?;
                let mut bytes = Vec::new();
                file.take(limits::NATIVE_BUDGET_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| invalid())?;
                if bytes.len() as u64 > limits::NATIVE_BUDGET_BYTES {
                    return Err(invalid());
                }
                let current: SavedUsage = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if self.saved.as_ref() != Some(&current) {
                    return Err(invalid());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && self.saved.is_none() => {}
            Err(_) => return Err(invalid()),
        }
        let bytes = serde_json::to_vec(&SavedUsage {
            version: 1,
            day: self.usage.day.0,
            reads: self.usage.reads,
        })
        .map_err(|_| invalid())?;
        let mut staged = UsageStage {
            path: self.directory.directory.join(format!(
                ".{}-{}",
                self.project,
                uuid_v4().map_err(|_| unavailable())?
            )),
            published: false,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&staged.path)
            .map_err(|_| unavailable())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| unavailable())?;
        fs::rename(&staged.path, &self.path).map_err(|_| unavailable())?;
        staged.published = true;
        self.directory.sync().map_err(|_| unavailable())?;
        self.saved = Some(SavedUsage {
            version: 1,
            day: self.usage.day.0,
            reads: self.usage.reads,
        });
        self.dirty = false;
        Ok(())
    }
    /// Normal stop flushes before the single writer releases its lock. Drop
    /// alone is owner loss: the documented one-flush-window undercount applies.
    pub fn finish(mut self) -> Result<(), NativeBudgetError> {
        self.flush()
    }
}
struct UsageStage {
    path: PathBuf,
    published: bool,
}
impl Drop for UsageStage {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}
