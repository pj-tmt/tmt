//! Exact-path, bounded metadata header only; no transcript record traversal.
use std::{io::Read, path::Path, time::Instant};

pub(crate) fn read(root: &Path, path: &Path, deadline: Instant) -> Option<Vec<u8>> {
    let mut file = super::transcript::open(root, path)?;
    let mut header = Vec::new();
    // Read only through the header newline, without buffering the next record.
    for _ in 0..64 * 1024 {
        if Instant::now() >= deadline {
            return None;
        }
        let mut byte = [0];
        if file.read(&mut byte).ok()? != 1 {
            return None;
        }
        if byte[0] == b'\n' {
            return Some(header);
        }
        header.push(byte[0]);
    }
    None
}
