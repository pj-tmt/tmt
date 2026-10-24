//! The built-in host's member supervisor (#4605). One process per member owns
//! that member's pseudo-terminal and serves it over a private Unix socket in an
//! owner-only directory; there is no daemon, and a supervisor that dies takes its
//! program down with it (the program's terminal hangs up). TMT keeps no
//! scrollback: output goes to whoever is attached and is dropped otherwise.
//! `tmt-pty-protocol` defines the frames; `Connection` is the blocking client.

mod attach;
mod client;
mod launch;
mod supervisor;
#[cfg(test)]
mod tests;

pub use attach::{AttachEnd, AttachSignals, attach};
pub use client::Connection;
pub use launch::{LaunchError, StartedMember, start_member};
pub use supervisor::{Launch, Supervisor};
pub use tmt_pty_protocol as protocol;

use nix::unistd::{dup2_stderr, dup2_stdin, dup2_stdout, setsid};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

/// macOS `sockaddr_un` holds 104 bytes; stay below it (and Linux's 108).
const SOCKET_PATH_LIMIT: usize = 100;
const ID_LIMIT: usize = 40;

/// Runs a member as the calling process: leaves its session so closing the
/// launcher's terminal does not hang the member up, starts, calls `ready` with
/// the program's process id, then points the standard streams at `/dev/null` so
/// the launcher can read to the end of its pipes, and serves until the program
/// ends.
pub fn serve_process(launch: Launch, ready: impl FnOnce(u32) -> io::Result<()>) -> io::Result<()> {
    // A process that already leads its group cannot start a session; it then
    // simply keeps the one it has.
    let _ = setsid();
    let supervisor = Supervisor::start(launch)?;
    ready(supervisor.program_pid())?;
    let null = File::options().read(true).write(true).open("/dev/null")?;
    dup2_stdin(&null)?;
    dup2_stdout(&null)?;
    dup2_stderr(&null)?;
    supervisor.run()
}

/// A member's socket: `<directory>/<id>.sock`.
pub fn socket_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.sock"))
}

/// The lock a live supervisor holds: `<directory>/<id>.lock`.
pub fn lock_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.lock"))
}

/// An id is letters, digits and `_`, at most [`ID_LIMIT`] bytes, and the socket
/// path built from it must fit a Unix socket address.
fn validate(directory: &Path, id: &str) -> io::Result<()> {
    let plain = !id.is_empty()
        && id.len() <= ID_LIMIT
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a member id is 1 to 40 letters, digits or underscores",
        ));
    }
    if socket_path(directory, id).as_os_str().len() > SOCKET_PATH_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the socket path is too long for a Unix socket; use a shorter directory",
        ));
    }
    Ok(())
}
