//! Fixture-executable publication for DEVELOPMENT's ETXTBSY case 2 only.
//!
//! This is not a general test-utilities home. Every additional helper requires
//! its own two-caller justification and an architecture review. Product crates
//! may consume this unpublished crate only through reviewed dev-dependencies.
//! The separate Colab verifier example owns its embedded stand-in, not library helpers.

use std::{
    ffi::OsString,
    io,
    path::Path,
    time::{Duration, Instant},
};
use tmt_invoke::{EnvironmentPolicy, LaunchOptions, Request};

/// Publish exact fixture bytes with the caller's Unix permission mode.
///
/// Only the short-lived shell opens the executable for writing: concurrent test
/// forks cannot inherit a writable descriptor for that inode. Successful bounded
/// execution waits for the writer and its synchronous commands before returning.
/// The generous 30-second deadline bounds a hung writer while accommodating
/// process startup under full-workspace parallel load.
/// Execution failures retain the invoke owner's process-group cleanup evidence.
/// Paths and modes are argv data, never interpolated shell source. Publication
/// does not stage or retry; callers own their temporary paths and fixture bytes.
pub fn write_executable(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let args: Vec<OsString> = [
        OsString::from("-c"),
        OsString::from(r#"/bin/cat > "$1" && /bin/chmod "$2" "$1""#),
        OsString::from("writer"),
        path.as_os_str().to_owned(),
        format!("{mode:o}").into(),
    ]
    .into();
    let output = tmt_invoke::invoke(
        Request {
            program: Path::new("/bin/sh"),
            args: &args,
            input: bytes,
            deadline: Instant::now() + Duration::from_secs(30),
            max_stream_bytes: 4096,
            launch: LaunchOptions {
                environment: EnvironmentPolicy::ClearAllowlist(&[]),
                ..LaunchOptions::default()
            },
        },
        None,
    )
    .map_err(io::Error::other)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "fixture writer failed for {}: {:?}: {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
