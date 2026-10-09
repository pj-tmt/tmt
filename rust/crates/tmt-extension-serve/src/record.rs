use serde_json::json;
use std::{fs::File, io::Write};

/// The one private failure record of a background serve. The caller opens the file, so the
/// path, permissions and the moment (after the serve lock is held) stay the product's. It is
/// cleared once, written at most once and never appended to; no asynchronous writer exists and
/// file I/O carries no wall-time promise.
pub struct ErrorRecord {
    file: File,
    limit: usize,
}
impl ErrorRecord {
    pub fn clear(file: File, limit: usize) -> std::io::Result<Self> {
        file.set_len(0)?;
        file.sync_all()?;
        Ok(Self { file, limit })
    }
    /// Record a fixed, sanitized `phase`, `code` and `message`. A record that would exceed the
    /// bound or fail to reach disk leaves the file as it was.
    pub fn failure(&mut self, phase: &str, code: &str, message: &str) {
        let bytes =
            serde_json::to_vec(&json!({"version":1,"phase":phase,"code":code,"message":message}))
                .expect("fixed record");
        if bytes.len() <= self.limit {
            let _ = self
                .file
                .write_all(&bytes)
                .and_then(|()| self.file.sync_all());
        }
    }
}
