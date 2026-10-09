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
    let Some(pid) = i32::try_from(pid).ok().filter(|pid| *pid > 0) else {
        return false;
    };
    let Ok(group) = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid))) else {
        return false;
    };
    if nix::unistd::tcgetpgrp(&file).is_ok_and(|foreground| foreground == group) {
        return true;
    }
    // Linux refuses TIOCGPGRP on another session's slave tty. Observe only
    // the selected process's native tty/group fields; never acquire the tty.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        let Ok(bytes) = crate::bounded_file::read_no_follow(
            std::path::Path::new(&format!("/proc/{pid}/stat")),
            8192,
        ) else {
            return false;
        };
        let major = nix::libc::major(metadata.rdev());
        let minor = nix::libc::minor(metadata.rdev());
        let device = ((major & 0xfff) << 8) | (minor & 0xff) | ((minor & 0xfff00) << 12);
        proc_foreground(pid, &bytes, device)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        let Some(info) = tmt_sys::bsd_info(pid) else {
            return false;
        };
        u32::try_from(pid).ok() == Some(info.pbi_pid)
            && info.pbi_pgid > 0
            && info.pbi_pgid == info.e_tpgid
            && info.e_tdev != u32::MAX
            && u64::from(info.e_tdev) == metadata.rdev()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    false
}

#[cfg(any(target_os = "linux", test))]
fn proc_foreground(pid: i32, bytes: &[u8], device: u32) -> bool {
    let Some(open) = bytes.iter().position(|byte| *byte == b'(') else {
        return false;
    };
    let Some(close) = bytes.iter().rposition(|byte| *byte == b')') else {
        return false;
    };
    if open >= close
        || std::str::from_utf8(&bytes[..open])
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok())
            != Some(pid)
    {
        return false;
    }
    let Ok(suffix) = std::str::from_utf8(&bytes[close + 1..]) else {
        return false;
    };
    let fields: Vec<_> = suffix.split_ascii_whitespace().take(6).collect();
    if fields.len() != 6
        || fields[0].len() != 1
        || !matches!(fields[0].as_bytes()[0], b'R' | b'S' | b'I' | b'D')
    {
        return false;
    }
    let group = fields[2].parse::<i32>().ok().filter(|group| *group > 0);
    let foreground = fields[5].parse::<i32>().ok().filter(|group| *group > 0);
    let tty = fields[4]
        .parse::<i32>()
        .ok()
        .map(|value| u32::from_ne_bytes(value.to_ne_bytes()));
    group.is_some() && group == foreground && device != 0 && tty == Some(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_process_foreground_requires_exact_pid_tty_and_positive_matching_groups() {
        let foreground = b"20 (extension (name)) S 10 20 20 34817 20 0 0";
        assert!(proc_foreground(20, foreground, 34817));
        assert!(!proc_foreground(21, foreground, 34817));
        assert!(!proc_foreground(20, foreground, 34818));
        assert!(!proc_foreground(
            20,
            b"20 (name) S 10 21 20 34817 20",
            34817
        ));
        assert!(!proc_foreground(20, b"20 (name) S 10 20 20 0 -1", 0));
        assert!(!proc_foreground(
            20,
            b"20 (name) T 10 20 20 34817 20",
            34817
        ));
        assert!(!proc_foreground(20, b"20 (name) S 10 20", 34817));
    }
}
