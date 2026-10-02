//! Core-owned, audited system-call boundary. No application policy lives here.
#![cfg(target_os = "macos")]

use std::mem::{size_of, zeroed};

/// Read one process's BSD information. Invalid, unavailable and short kernel
/// responses are not evidence. Callers own deadlines and fallback policy.
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

fn finish(info: libc::proc_bsdinfo, bytes: i32) -> Option<libc::proc_bsdinfo> {
    (usize::try_from(bytes).ok() == Some(size_of::<libc::proc_bsdinfo>())).then_some(info)
}

#[cfg(test)]
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
