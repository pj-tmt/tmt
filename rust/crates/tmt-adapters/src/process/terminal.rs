//! Native foreground-group evidence for remembered external commands.

use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

pub fn foreground(tty: &str, pid: u64) -> bool {
    if !tty.starts_with("/dev/") || tty.contains("/../") {
        return false;
    }
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOCTTY | nix::libc::O_NONBLOCK | nix::libc::O_NOFOLLOW)
        .open(tty)
    else {
        return false;
    };
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    let Ok(group) = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid))) else {
        return false;
    };
    nix::unistd::tcgetpgrp(&file).is_ok_and(|foreground| foreground == group)
}
