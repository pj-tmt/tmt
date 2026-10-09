//! Deadline-owned pipe handoff with explicit evidence of partial publication.
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::write,
};
use std::{os::fd::AsFd, time::Instant};

#[derive(Debug)]
pub struct HandoffFailure {
    pub written: usize,
}

pub fn write_before(fd: &impl AsFd, bytes: &[u8], deadline: Instant) -> Result<(), HandoffFailure> {
    let mut written = 0;
    let result = (|| -> Result<(), Errno> {
        let original = OFlag::from_bits_retain(fcntl(fd, FcntlArg::F_GETFL)?);
        struct Restore<'a, F: AsFd>(&'a F, OFlag);
        impl<F: AsFd> Drop for Restore<'_, F> {
            fn drop(&mut self) {
                let _ = fcntl(self.0, FcntlArg::F_SETFL(self.1));
            }
        }
        fcntl(fd, FcntlArg::F_SETFL(original | OFlag::O_NONBLOCK))?;
        let _restore = Restore(fd, original);
        while written < bytes.len() {
            if Instant::now() >= deadline {
                return Err(Errno::ETIMEDOUT);
            }
            match write(fd, &bytes[written..]) {
                Ok(0) => return Err(Errno::EIO),
                Ok(count) => written += count,
                Err(Errno::EINTR) => {}
                Err(Errno::EAGAIN) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    let timeout = PollTimeout::try_from(left).unwrap_or(PollTimeout::MAX);
                    poll(&mut [PollFd::new(fd.as_fd(), PollFlags::POLLOUT)], timeout)?;
                }
                Err(error) => return Err(error),
            }
        }
        // Raw descriptor writes have no userspace buffer left to flush.
        Ok(())
    })();
    result.map_err(|_| HandoffFailure { written })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::pipe;
    use std::{io::Read, time::Duration};
    #[test]
    fn complete_pipe_output_is_exact_and_original_flags_are_restored() {
        let (read, write) = pipe().unwrap();
        let flags = fcntl(&write, FcntlArg::F_GETFL).unwrap();
        write_before(
            &write,
            b"{\"decision\":\"block\"}",
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        // Darwin adds its read-only FWASWRITTEN bit after a write. Compare
        // the public status flags; all settable flags must be restored.
        assert_eq!(
            OFlag::from_bits_truncate(fcntl(&write, FcntlArg::F_GETFL).unwrap()),
            OFlag::from_bits_truncate(flags)
        );
        drop(write);
        let mut text = String::new();
        std::fs::File::from(read).read_to_string(&mut text).unwrap();
        assert_eq!(text, "{\"decision\":\"block\"}");
    }
    #[test]
    fn a_blocked_reader_bounds_partial_output_and_zero_budget_publishes_nothing() {
        let (read, write) = pipe().unwrap();
        let flags = fcntl(&write, FcntlArg::F_GETFL).unwrap();
        assert_eq!(
            write_before(&write, b"nothing", Instant::now())
                .unwrap_err()
                .written,
            0
        );
        let large = vec![b'x'; 4 * 1024 * 1024];
        let failure =
            write_before(&write, &large, Instant::now() + Duration::from_millis(20)).unwrap_err();
        assert!(failure.written > 0 && failure.written < large.len());
        // Darwin adds its read-only FWASWRITTEN bit after a write. Compare
        // the public status flags; all settable flags must be restored.
        assert_eq!(
            OFlag::from_bits_truncate(fcntl(&write, FcntlArg::F_GETFL).unwrap()),
            OFlag::from_bits_truncate(flags)
        );
        drop(write);
        let mut actual = Vec::new();
        std::fs::File::from(read).read_to_end(&mut actual).unwrap();
        assert_eq!(actual.len(), failure.written);
        assert!(actual.iter().all(|byte| *byte == b'x'));
    }
}
