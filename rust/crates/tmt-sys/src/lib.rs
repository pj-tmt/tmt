//! Core-owned, audited system-call boundary. No application policy lives here.

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
