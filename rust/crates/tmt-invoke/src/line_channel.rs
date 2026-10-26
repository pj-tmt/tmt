//! Ordered newline frames over one owned child. Protocol admission belongs to callers.

use crate::{FailureKind, InvokeError, LaunchOptions, Output, Phase, Stream, process};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    os::fd::AsFd,
    path::Path,
    time::Instant,
};

pub struct LineChannelOptions<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub deadline: Instant,
    /// Payload bound excluding the newline; stderr has the same cumulative bound.
    pub max_frame_bytes: usize,
    pub launch: LaunchOptions<'a>,
}

/// No idle thread or timer. Each exchange writes one frame and reads one frame.
/// A failed started exchange closes the channel; it never resubmits a request.
pub struct LineChannel {
    child: Option<process::Child>,
    stdin: Option<File>,
    stdout: Option<File>,
    stderr: Option<File>,
    stderr_bytes: Vec<u8>,
    limit: usize,
}

fn io_failure(cause: io::Error) -> InvokeError {
    process::failure(FailureKind::Io(Phase::Communicate), Some(cause))
}

fn nonblocking(file: &File) -> io::Result<()> {
    let flags = fcntl(file, FcntlArg::F_GETFL).map_err(io::Error::from)?;
    fcntl(
        file,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map_err(io::Error::from)?;
    Ok(())
}

impl LineChannel {
    pub fn start(
        options: LineChannelOptions<'_>,
        stopped: impl Fn() -> bool,
    ) -> Result<Self, InvokeError> {
        process::budget(options.deadline, stopped())?;
        let mut child = process::start(options.program, options.args, &options.launch, None)?;
        let pipes = (|| {
            let take = |pipe: Option<File>| {
                pipe.ok_or_else(|| io::Error::other("child did not expose its pipe"))
            };
            let stdin = take(child.job.stdin.take())?;
            let stdout = take(child.job.stdout.take())?;
            let stderr = take(child.job.stderr.take())?;
            for pipe in [&stdin, &stdout, &stderr] {
                nonblocking(pipe)?;
            }
            Ok::<_, io::Error>((stdin, stdout, stderr))
        })();
        let (stdin, stdout, stderr) = match pipes {
            Ok(pipes) => pipes,
            Err(cause) => {
                let mut error = process::failure(FailureKind::Io(Phase::OpenPipes), Some(cause));
                error.cleanup = child.cleanup();
                child.finished = true;
                return Err(error);
            }
        };
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
            stderr: Some(stderr),
            stderr_bytes: Vec::new(),
            limit: options.max_frame_bytes,
        })
    }

    /// Rejects invalid local frames before touching the channel. After I/O starts,
    /// cancellation or failure can leave effects uncertain and always ends it.
    pub fn exchange(
        &mut self,
        payload: &[u8],
        deadline: Instant,
        stopped: impl Fn() -> bool,
    ) -> Result<Vec<u8>, InvokeError> {
        process::budget(deadline, stopped())?;
        if payload.len() > self.limit || payload.contains(&b'\n') {
            return Err(io_failure(io::Error::new(
                io::ErrorKind::InvalidInput,
                "one bounded newline-free frame is required",
            )));
        }
        if self.child.is_none() {
            return Err(io_failure(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "channel is closed",
            )));
        }
        let result = self.communicate(payload, deadline, &stopped);
        if let Err(mut error) = result {
            self.stdin.take();
            self.stdout.take();
            self.stderr.take();
            let mut child = self.child.take().expect("exchange owns a child");
            error.cleanup = child.cleanup();
            child.finished = true;
            return Err(error);
        }
        result
    }

    fn communicate(
        &mut self,
        payload: &[u8],
        deadline: Instant,
        stopped: &impl Fn() -> bool,
    ) -> Result<Vec<u8>, InvokeError> {
        let mut input = Vec::with_capacity(payload.len() + 1);
        input.extend_from_slice(payload);
        input.push(b'\n');
        let mut written = 0;
        let mut output = Vec::new();
        loop {
            let left = process::budget(deadline, stopped())?.min(process::PULSE);
            let (readable, error_ready, writable) = {
                let stdout = self.stdout.as_ref().expect("open output");
                let mut fds = vec![PollFd::new(stdout.as_fd(), PollFlags::POLLIN)];
                let error_index = self.stderr.as_ref().map(|stderr| {
                    let index = fds.len();
                    fds.push(PollFd::new(stderr.as_fd(), PollFlags::POLLIN));
                    index
                });
                let input_index = (written < input.len()).then(|| {
                    let index = fds.len();
                    fds.push(PollFd::new(
                        self.stdin.as_ref().expect("open input").as_fd(),
                        PollFlags::POLLOUT,
                    ));
                    index
                });
                let timeout = PollTimeout::try_from(left).expect("pulse fits poll timeout");
                match poll(&mut fds, timeout) {
                    Ok(_) => {}
                    Err(Errno::EINTR) => continue,
                    Err(cause) => return Err(io_failure(cause.into())),
                }
                let ready =
                    |index: usize| fds[index].revents().is_some_and(|flags| !flags.is_empty());
                (
                    ready(0),
                    error_index.is_some_and(ready),
                    input_index.is_some_and(ready),
                )
            };
            if error_ready {
                let mut bytes = [0; 8192];
                match self.stderr.as_mut().expect("open stderr").read(&mut bytes) {
                    Ok(0) => {
                        self.stderr.take();
                    }
                    Ok(read) => {
                        if read > self.limit.saturating_sub(self.stderr_bytes.len()) {
                            return Err(process::failure(
                                FailureKind::OutputLimit(Stream::Stderr),
                                None,
                            ));
                        }
                        self.stderr_bytes.extend_from_slice(&bytes[..read]);
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(io_failure(error)),
                }
            }
            if writable {
                match self
                    .stdin
                    .as_mut()
                    .expect("open input")
                    .write(&input[written..])
                {
                    Ok(0) => return Err(io_failure(io::ErrorKind::WriteZero.into())),
                    Ok(count) => written += count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(io_failure(error)),
                }
            }
            if readable {
                let mut bytes = [0; 8192];
                match self.stdout.as_mut().expect("open output").read(&mut bytes) {
                    Ok(0) => return Err(io_failure(io::ErrorKind::UnexpectedEof.into())),
                    Ok(count) => {
                        let frame = &bytes[..count];
                        let newline = frame.iter().position(|byte| *byte == b'\n');
                        let payload = newline.map_or(frame, |end| &frame[..end]);
                        if payload.len() > self.limit.saturating_sub(output.len()) {
                            return Err(process::failure(
                                FailureKind::OutputLimit(Stream::Stdout),
                                None,
                            ));
                        }
                        output.extend_from_slice(payload);
                        if let Some(end) = newline {
                            if end + 1 != count || written != input.len() {
                                return Err(io_failure(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "unexpected output outside the response frame",
                                )));
                            }
                            return Ok(output);
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => return Err(io_failure(error)),
                }
            }
        }
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr_bytes
    }

    /// Close input and drain/reap within the supplied budget. Nonzero exits are
    /// data; a started cleanup failure retains the ordinary invocation evidence.
    pub fn close(
        mut self,
        deadline: Instant,
        stopped: impl Fn() -> bool,
    ) -> Result<Output, InvokeError> {
        self.close_inner(deadline, stopped)
    }

    fn close_inner(
        &mut self,
        deadline: Instant,
        stopped: impl Fn() -> bool,
    ) -> Result<Output, InvokeError> {
        self.stdin.take();
        let Some(mut child) = self.child.take() else {
            return Err(io_failure(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "channel is closed",
            )));
        };
        child.job.stdout = self.stdout.take();
        child.job.stderr = self.stderr.take();
        let result = process::observe_with_stop(
            &mut child.job,
            deadline,
            self.limit.saturating_sub(self.stderr_bytes.len()),
            stopped,
        );
        let result = match result {
            Ok(mut output) => {
                self.stderr_bytes.append(&mut output.stderr);
                output.stderr = std::mem::take(&mut self.stderr_bytes);
                Ok(output)
            }
            Err(mut error) => {
                error.cleanup = child.cleanup();
                Err(error)
            }
        };
        child.finished = true;
        result
    }
}

impl Drop for LineChannel {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.close_inner(Instant::now() + process::CLEANUP, || false);
        }
    }
}

#[cfg(test)]
#[path = "line_channel_tests.rs"]
mod tests;
