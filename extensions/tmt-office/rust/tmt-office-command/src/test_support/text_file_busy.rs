//! Bounded retry for the one kernel refusal a fixture that writes and runs an
//! executable cannot avoid.
//!
//! `cargo test` runs tests as threads of one process. A thread that forks while
//! another thread holds a fixture's write descriptor gives the child a copy of
//! it, which the child closes only at its own `exec`. Until then Linux answers
//! the writer's `exec` with ETXTBSY ("Text file busy"), whatever the writer did
//! to its descriptor. The window is the other thread's fork-to-exec time, so it
//! stretches on a loaded runner. Production code must never retry this: it is
//! single-threaded around install and exec. Only a fixture waits it out.

use std::{
    error::Error,
    io, thread,
    time::{Duration, Instant},
};

const RETRY_WINDOW: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_millis(10);

/// Runs `operation`, repeating it while it fails with ETXTBSY for at most
/// [`RETRY_WINDOW`]. Any other error, and the last ETXTBSY once the window has
/// passed, is returned unchanged.
///
/// `operation` must leave nothing behind when it fails, as the installer does,
/// so that every attempt starts from the same state.
pub(crate) fn retry_on_text_file_busy<T>(
    operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    retry_within(RETRY_WINDOW, RETRY_INTERVAL, operation)
}

fn retry_within<T>(
    window: Duration,
    interval: Duration,
    mut operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let deadline = Instant::now() + window;
    loop {
        match operation() {
            Err(error) if is_text_file_busy(&error) && Instant::now() < deadline => {
                thread::sleep(interval);
            }
            result => return result,
        }
    }
}

/// Process spawn failures reach fixtures wrapped (`io::Error::other` around the
/// adapter's `CommandError`, whose source is the OS error), so look through
/// the whole chain for the one kind.
fn is_text_file_busy(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::ExecutableFileBusy {
        return true;
    }
    let mut cause = error.get_ref().map(|inner| inner as &(dyn Error + 'static));
    while let Some(current) = cause {
        if current
            .downcast_ref::<io::Error>()
            .is_some_and(|inner| inner.kind() == io::ErrorKind::ExecutableFileBusy)
        {
            return true;
        }
        cause = current.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::{
        ffi::OsString,
        fmt, fs,
        os::unix::fs::PermissionsExt,
        path::Path,
        sync::atomic::{AtomicBool, Ordering},
    };
    use tmt_adapters::process::{CommandRequest, CommandRunner, UnixCommandRunner};

    fn busy() -> io::Error {
        io::Error::from(io::ErrorKind::ExecutableFileBusy)
    }

    /// The shape of the adapter's process errors: not an `io::Error` itself,
    /// with the OS error as its source.
    #[derive(Debug)]
    struct SpawnFailure(io::Error);

    impl fmt::Display for SpawnFailure {
        fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(output, "External command failed: Spawn")
        }
    }

    impl Error for SpawnFailure {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    fn write_executable(path: &Path) {
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Runs `program` through the production runner, as the release verifier does.
    fn run(program: &Path) -> io::Result<()> {
        let args: [OsString; 0] = [];
        UnixCommandRunner
            .execute(CommandRequest {
                program: program.as_os_str(),
                args: &args,
                input: &[],
                deadline: Instant::now() + Duration::from_secs(5),
                max_output_bytes: 1024,
            })
            .map(drop)
            .map_err(io::Error::other)
    }

    #[test]
    fn a_busy_executable_is_retried_until_it_can_run() {
        let mut attempts = 0;
        let result = retry_on_text_file_busy(|| {
            attempts += 1;
            if attempts < 4 {
                Err(busy())
            } else {
                Ok(attempts)
            }
        });
        assert_eq!(result.unwrap(), 4);
    }

    #[test]
    fn busy_is_recognised_through_every_wrapping_the_adapters_produce() {
        assert!(is_text_file_busy(&busy()));
        assert!(is_text_file_busy(&io::Error::other(busy())));
        assert!(is_text_file_busy(&io::Error::other(SpawnFailure(busy()))));
        #[cfg(target_os = "linux")]
        assert!(is_text_file_busy(&io::Error::other(SpawnFailure(
            io::Error::from_raw_os_error(26)
        ))));
    }

    #[test]
    fn other_failures_are_never_retried() {
        for make in [
            (|| io::Error::from(io::ErrorKind::NotFound)) as fn() -> io::Error,
            || io::Error::from(io::ErrorKind::PermissionDenied),
            || io::Error::other(SpawnFailure(io::Error::from(io::ErrorKind::NotFound))),
            || io::Error::other("Text file busy, in words only"),
        ] {
            let mut attempts = 0;
            let error = retry_on_text_file_busy::<()>(|| {
                attempts += 1;
                Err(make())
            })
            .unwrap_err();
            assert_eq!(attempts, 1, "{error}");
            assert!(!is_text_file_busy(&error), "{error}");
        }
    }

    #[test]
    fn a_process_that_cannot_start_for_another_reason_is_not_retried() {
        let directory = TestDirectory::new();
        let mut attempts = 0;
        let error = retry_on_text_file_busy(|| {
            attempts += 1;
            run(&directory.path.join("missing"))
        })
        .unwrap_err();
        assert_eq!(attempts, 1, "{error}");
    }

    #[test]
    fn retrying_is_bounded_and_the_last_busy_error_is_returned() {
        let mut attempts = 0;
        let started = Instant::now();
        let error = retry_within::<()>(Duration::from_millis(60), Duration::from_millis(5), || {
            attempts += 1;
            Err(busy())
        })
        .unwrap_err();
        assert!(attempts > 1, "a busy executable is retried");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the wait is bounded"
        );
        assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy);
    }

    /// A deterministic barrier stands in for the racing fork: an open write
    /// descriptor makes the kernel refuse the real production spawn until it
    /// is closed, which the second attempt observes.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_refused_spawn_succeeds_once_the_writer_is_gone() {
        let directory = TestDirectory::new();
        let fixture = directory.path.join("fixture");
        write_executable(&fixture);
        let mut writer = Some(fs::OpenOptions::new().append(true).open(&fixture).unwrap());
        let mut attempts = 0;
        retry_on_text_file_busy(|| {
            attempts += 1;
            let result = run(&fixture);
            writer.take();
            result
        })
        .unwrap();
        assert_eq!(attempts, 2, "the first spawn was refused, the second ran");
    }

    /// Opt-in: races a writer against forking threads to show the defect the
    /// retry removes. It asserts only that the retried arm never fails; the
    /// unretried arm's count is the evidence to quote and is probabilistic.
    ///
    /// `cargo test -p tmt-office-command text_file_busy_stress -- --ignored --nocapture`
    #[test]
    #[ignore = "opt-in stress: races fixture writes against forking threads"]
    fn text_file_busy_stress() {
        const ITERATIONS: usize = 4000;
        const FORKERS: usize = 3;
        let directory = TestDirectory::new();
        let stop = AtomicBool::new(false);
        let (mut unretried, mut retried, mut retries_used) = (0, 0, 0);
        thread::scope(|scope| {
            for _ in 0..FORKERS {
                scope.spawn(|| {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = std::process::Command::new("/bin/true").status();
                    }
                });
            }
            for iteration in 0..ITERATIONS {
                let fixture = directory.path.join(format!("unretried-{iteration}"));
                write_executable(&fixture);
                if run(&fixture).is_err() {
                    unretried += 1;
                }
                let fixture = directory.path.join(format!("retried-{iteration}"));
                write_executable(&fixture);
                let mut attempts = 0;
                let result = retry_on_text_file_busy(|| {
                    attempts += 1;
                    run(&fixture)
                });
                retries_used += attempts - 1;
                if result.is_err() {
                    retried += 1;
                }
            }
            stop.store(true, Ordering::Relaxed);
        });
        eprintln!(
            "text_file_busy_stress: {ITERATIONS} write-then-exec iterations against {FORKERS} forking threads: \
             {unretried} failed without the retry; {retried} failed with it ({retries_used} retries used)"
        );
        assert_eq!(retried, 0, "the bounded retry must absorb every ETXTBSY");
    }
}
