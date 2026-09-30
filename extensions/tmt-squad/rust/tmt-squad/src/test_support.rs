//! Fixtures shared by Squad's unit tests.

use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

/// Writes `script` to `path` as an executable a test can run at once.
///
/// `cargo test` runs tests as threads of one process. Had this process opened
/// the file for writing, another test thread's `fork` would copy that
/// descriptor into its child, which keeps it until its own `exec`, and until
/// then the kernel refuses to run the file with ETXTBSY ("Text file busy").
/// A short-lived `sh` writes the file instead, so no descriptor for it ever
/// exists in the test process and the first `exec` cannot be refused.
pub(crate) fn write_executable(path: &Path, script: &str) {
    let mut writer = Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("start sh to write the executable");
    writer
        .stdin
        .take()
        .expect("sh stdin")
        .write_all(script.as_bytes())
        .expect("send the script to sh");
    let status = writer.wait().expect("wait for sh");
    assert!(status.success(), "sh could not write {}", path.display());
}
