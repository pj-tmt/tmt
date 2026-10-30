//! Core-owned, audited system-call boundary. No application policy lives here.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod terminal;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use terminal::{peer_uid, set_window_size, spawn_on_terminal};

#[cfg(target_os = "macos")]
use std::mem::{size_of, zeroed};

/// Atomically rename a path without replacing an existing destination. The
/// caller owns source selection and migration policy; no fallback is performed.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn rename_exclusive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = |value: &std::path::Path| {
        CString::new(value.as_os_str().as_bytes())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    };
    let from = path(from)?;
    let to = path(to)?;
    // SAFETY: both pointers reference live NUL-terminated C strings for the
    // synchronous call. renamex_np neither mutates nor retains those buffers.
    // RENAME_EXCL refuses any existing destination atomically.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Atomic no-replace directory rename on Linux, including musl releases.
/// Use the kernel entry rather than a libc-version-dependent wrapper.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub fn rename_exclusive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = |value: &std::path::Path| {
        CString::new(value.as_os_str().as_bytes())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
    };
    let from = path(from)?;
    let to = path(to)?;
    // SAFETY: renameat2's five arguments are the two directory descriptors,
    // live NUL-terminated path pointers and the fixed no-replace flag. The
    // synchronous syscall neither mutates nor retains the string buffers.
    // No fallback can replace the destination on an unsupported kernel.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD as libc::c_long,
            from.as_ptr(),
            libc::AT_FDCWD as libc::c_long,
            to.as_ptr(),
            libc::RENAME_NOREPLACE as libc::c_long,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Read one process's BSD information. Invalid, unavailable and short kernel
/// responses are not evidence. Callers own deadlines and fallback policy.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
    if pid <= 0 {
        return None;
    }
    let size = i32::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: proc_bsdinfo is a C-layout struct containing only integer fields
    // and integer arrays, for which all-zero bytes are valid. Zero-init also
    // initializes every field the kernel might not write. The buffer
    // has size_of::<proc_bsdinfo>() bytes; finish refuses bytes != that size
    // before the value is exposed as process evidence.
    let mut info: libc::proc_bsdinfo = unsafe { zeroed() };
    // SAFETY: info is a live, aligned, zero-initialized proc_bsdinfo buffer of
    // exactly size_of::<proc_bsdinfo>() bytes. The synchronous call receives
    // that exact size and does not retain its pointer. finish checks bytes ==
    // size_of::<proc_bsdinfo>(); errors and partial writes never escape.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    finish(info, bytes)
}

#[cfg(target_os = "macos")]
fn finish(info: libc::proc_bsdinfo, bytes: i32) -> Option<libc::proc_bsdinfo> {
    (usize::try_from(bytes).ok() == Some(size_of::<libc::proc_bsdinfo>())).then_some(info)
}

/// One process's current directory as an absolute UTF-8 path. A process that
/// is gone or unreadable, a short kernel response, and a path that is not
/// UTF-8 are not evidence: `None`, never a lossy path. Callers own deadlines,
/// incarnation checks and what the path may be used for.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn process_cwd(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    let size = i32::try_from(size_of::<libc::proc_vnodepathinfo>()).ok()?;
    // SAFETY: proc_vnodepathinfo is a C-layout struct of integers and integer
    // arrays, for which all-zero bytes are valid; zero-init also initializes
    // every byte the kernel might not write.
    let mut info: libc::proc_vnodepathinfo = unsafe { zeroed() };
    // SAFETY: info is a live, aligned, zero-initialized buffer of exactly
    // size_of::<proc_vnodepathinfo>() bytes, and the synchronous call receives
    // that exact size and does not retain the pointer. directory checks the
    // reported byte count before any byte is read as a path.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    directory(&info, bytes)
}

#[cfg(target_os = "macos")]
fn directory(info: &libc::proc_vnodepathinfo, bytes: i32) -> Option<String> {
    if usize::try_from(bytes).ok() != Some(size_of::<libc::proc_vnodepathinfo>()) {
        return None;
    }
    let path: Vec<u8> = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .map(|byte| *byte as u8)
        .collect();
    let end = path.iter().position(|byte| *byte == 0)?;
    absolute(std::str::from_utf8(&path[..end]).ok()?)
}

/// One process's current directory as an absolute UTF-8 path; see the macOS
/// variant. A directory that was removed reads as `… (deleted)` and is refused.
#[cfg(target_os = "linux")]
pub fn process_cwd(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    let path = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    let path = path.into_os_string().into_string().ok()?;
    absolute(&path).filter(|path| !path.ends_with(" (deleted)"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn absolute(path: &str) -> Option<String> {
    path.starts_with('/').then(|| path.to_owned())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn current_process_has_kernel_identity() {
        let pid = std::process::id();
        let info = bsd_info(pid as i32).expect("current process BSD info");
        assert_eq!(info.pbi_pid, pid);
        assert!(info.pbi_start_tvsec > 0);
        assert!(info.pbi_ppid > 0);
    }

    #[test]
    fn invalid_process_is_not_evidence() {
        assert!(bsd_info(0).is_none());
        assert!(bsd_info(-1).is_none());
        assert!(bsd_info(i32::MAX).is_none());
    }

    #[test]
    fn partial_oversized_and_failed_responses_are_refused() {
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        for bytes in [-1, 0, size - 1, size + 1] {
            let info = bsd_info(std::process::id() as i32).unwrap();
            assert!(finish(info, bytes).is_none(), "reported byte count {bytes}");
        }
        let info = bsd_info(std::process::id() as i32).unwrap();
        assert!(finish(info, size).is_some());
    }

    fn vnode_path(text: &[u8]) -> libc::proc_vnodepathinfo {
        // SAFETY (test): an all-integer C struct, valid when zeroed.
        #[allow(unsafe_code)]
        let mut info: libc::proc_vnodepathinfo = unsafe { zeroed() };
        for (index, byte) in text.iter().enumerate() {
            info.pvi_cdir.vip_path[index / 32][index % 32] = *byte as libc::c_char;
        }
        info
    }

    #[test]
    fn a_directory_needs_the_exact_size_a_terminator_and_utf8() {
        let size = size_of::<libc::proc_vnodepathinfo>() as i32;
        let info = vnode_path(b"/work/project");
        assert_eq!(directory(&info, size).as_deref(), Some("/work/project"));
        for bytes in [-1, 0, size - 1, size + 1] {
            assert!(
                directory(&info, bytes).is_none(),
                "reported byte count {bytes}"
            );
        }
        assert!(
            directory(&vnode_path(b"/work/\xff"), size).is_none(),
            "not UTF-8"
        );
        assert!(directory(&vnode_path(b"relative"), size).is_none());
        assert!(
            directory(&vnode_path(&[b'/'; 1024]), size).is_none(),
            "no terminator"
        );
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod rename_tests {
    use super::rename_exclusive;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };
    #[test]
    fn exclusive_rename_moves_bytes_and_never_replaces_a_destination() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-sys-rename-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let from = root.join("from");
        let to = root.join("to");
        fs::create_dir(&from).unwrap();
        fs::write(from.join("bytes"), b"source").unwrap();
        fs::create_dir(&to).unwrap();
        fs::write(to.join("bytes"), b"destination").unwrap();
        let error = rename_exclusive(&from, &to).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(to.join("bytes")).unwrap(), b"destination");
        assert_eq!(fs::read(from.join("bytes")).unwrap(), b"source");
        fs::remove_dir_all(&to).unwrap();
        rename_exclusive(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(to.join("bytes")).unwrap(), b"source");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod cwd_tests {
    use super::process_cwd;

    #[test]
    fn the_current_directory_of_this_process_and_none_for_no_process() {
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        let read = process_cwd(std::process::id() as i32).expect("own directory");
        assert_eq!(std::path::Path::new(&read).canonicalize().unwrap(), here);
        for pid in [0, -1, i32::MAX] {
            assert!(process_cwd(pid).is_none(), "{pid}");
        }
    }

    #[test]
    fn a_child_reports_its_own_directory_and_nothing_once_reaped() {
        let directory = std::env::temp_dir().canonicalize().unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "read line"])
            .current_dir(&directory)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let read = process_cwd(pid).expect("child directory");
        assert_eq!(
            std::path::Path::new(&read).canonicalize().unwrap(),
            directory
        );
        drop(child.stdin.take());
        child.wait().unwrap();
        assert!(process_cwd(pid).is_none(), "reaped");
    }
}
