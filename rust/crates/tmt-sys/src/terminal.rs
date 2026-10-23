//! Pseudo-terminal and socket-peer primitives for the built-in host. Each
//! function does one system call sequence; session policy lives in the caller.

use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
    process::{Child, Command, Stdio},
};

/// Starts `command` with `slave` as its standard input, output and error, in a
/// new session whose controlling terminal is `slave`. The command is consumed so
/// this process holds no copy of `slave` afterwards; the master then sees the
/// program's end as a hangup.
#[allow(unsafe_code)]
pub fn spawn_on_terminal(mut command: Command, slave: OwnedFd) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;
    let output = slave.try_clone()?;
    let error = slave.try_clone()?;
    command
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(error));
    // SAFETY: the closure runs between fork and exec and calls only setsid and
    // ioctl, both async-signal-safe; it allocates nothing and touches no lock.
    // Descriptor 0 is already the slave when it runs.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

/// Sets a terminal's window size; its foreground processes receive SIGWINCH.
#[allow(unsafe_code)]
pub fn set_window_size(terminal: BorrowedFd<'_>, cols: u16, rows: u16) -> io::Result<()> {
    let size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads one `winsize` that outlives the call, on a
    // descriptor the borrow keeps open.
    let result = unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCSWINSZ as _, &size) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The user id of the process on the other end of a connected Unix socket.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn peer_uid(socket: BorrowedFd<'_>) -> io::Result<u32> {
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: both out-pointers are live locals for the duration of the call.
    let result = unsafe { libc::getpeereid(socket.as_raw_fd(), &mut uid, &mut gid) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(uid)
    }
}

/// The user id of the process on the other end of a connected Unix socket.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn peer_uid(socket: BorrowedFd<'_>) -> io::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the buffer is a live `ucred` and `length` is its exact size.
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &mut length,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(credentials.uid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::{fd::AsFd, unix::net::UnixStream};

    #[test]
    fn the_peer_of_a_socket_pair_is_this_user() {
        let (left, right) = UnixStream::pair().unwrap();
        // SAFETY: getuid has no preconditions.
        #[allow(unsafe_code)]
        let me = unsafe { libc::getuid() };
        assert_eq!(peer_uid(left.as_fd()).unwrap(), me);
        assert_eq!(peer_uid(right.as_fd()).unwrap(), me);
    }

    #[test]
    fn a_descriptor_that_is_not_a_socket_has_no_peer() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(peer_uid(file.as_fd()).is_err());
    }

    #[test]
    fn a_window_size_needs_a_terminal() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(set_window_size(file.as_fd(), 80, 24).is_err());
    }
}
