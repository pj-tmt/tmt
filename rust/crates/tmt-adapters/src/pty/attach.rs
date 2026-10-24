//! A finite interactive relay. The CLI owns terminal modes; this owner waits on
//! input, the existing supervisor connection and invocation-owned signal wakes.
//! No input thread, output replay, terminal-window resize or member signal exists.

use super::{
    Connection,
    protocol::{ClientMessage, Eviction, Exit, ServerMessage, Size},
};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use signal_hook::{
    SigId,
    consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGWINCH},
    flag, low_level,
};
use std::{
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachEnd {
    Detached,
    Exited(Exit),
    Evicted(Eviction),
    Signal(i32),
}

/// Install before acquiring raw mode; retain until after terminal restoration.
/// Signal-hook registrations are invocation-exclusive (like InteractiveChild),
/// not suitable for a long-lived embedded service. SIGKILL cannot be restored.
pub struct AttachSignals {
    reader: UnixStream,
    resized: Arc<AtomicBool>,
    stopped: Arc<AtomicUsize>,
    registrations: Vec<SigId>,
}

impl AttachSignals {
    pub fn install() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let mut guard = Self {
            reader,
            resized: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicUsize::new(0)),
            registrations: Vec::new(),
        };
        guard
            .registrations
            .push(flag::register(SIGWINCH, guard.resized.clone())?);
        guard
            .registrations
            .push(low_level::pipe::register(SIGWINCH, writer.try_clone()?)?);
        for signal in [SIGTERM, SIGHUP, SIGINT, SIGQUIT] {
            guard.registrations.push(flag::register_usize(
                signal,
                guard.stopped.clone(),
                signal as usize,
            )?);
            guard
                .registrations
                .push(low_level::pipe::register(signal, writer.try_clone()?)?);
        }
        Ok(guard)
    }

    fn drain(&mut self) -> io::Result<()> {
        let mut bytes = [0u8; 128];
        loop {
            match self.reader.read(&mut bytes) {
                Ok(0) => return Err(io::Error::other("attach signal notification closed")),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for AttachSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..).rev() {
            low_level::unregister(registration);
        }
    }
}

/// Runs until detach, exit, eviction, signal or transport failure. Raw output is
/// written without parsing or styling. Only the detach byte is consumed locally.
/// `size` reads the current local terminal and clamps it before any Resize frame.
pub fn attach(
    connection: &mut Connection,
    input: &impl AsFd,
    output: &mut impl Write,
    signals: &mut AttachSignals,
    detach_others: bool,
    mut size: impl FnMut() -> io::Result<Size>,
) -> io::Result<AttachEnd> {
    connection.send(&ClientMessage::Resize(size()?))?;
    if detach_others {
        connection.send(&ClientMessage::TakeSole)?;
    }
    loop {
        let signal = signals.stopped.load(Ordering::SeqCst);
        if signal != 0 {
            return Ok(AttachEnd::Signal(signal as i32));
        }
        if signals.resized.swap(false, Ordering::SeqCst) {
            connection.send(&ClientMessage::Resize(size()?))?;
        }
        // Fairness: a continuously writing program cannot starve Ctrl-\ or a
        // signal. If decoded frames remain, poll without blocking this round.
        let mut saturated = true;
        for _ in 0..64 {
            match connection.receive_available()? {
                Some(ServerMessage::Output(bytes)) => output.write_all(&bytes)?,
                Some(ServerMessage::Resized(_)) => {}
                Some(ServerMessage::Exited(exit)) => {
                    output.flush()?;
                    return Ok(AttachEnd::Exited(exit));
                }
                Some(ServerMessage::Evicted(reason)) => {
                    output.flush()?;
                    return Ok(AttachEnd::Evicted(reason));
                }
                Some(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected attach frame",
                    ));
                }
                None => {
                    saturated = false;
                    break;
                }
            }
        }
        output.flush()?;
        let mut ready = [
            PollFd::new(input.as_fd(), PollFlags::POLLIN),
            PollFd::new(connection.descriptor(), PollFlags::POLLIN),
            PollFd::new(signals.reader.as_fd(), PollFlags::POLLIN),
        ];
        match poll(
            &mut ready,
            if saturated {
                PollTimeout::ZERO
            } else {
                PollTimeout::NONE
            },
        ) {
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let input_ready = ready[0].revents().unwrap_or_else(PollFlags::empty);
        let signal_ready = ready[2].revents().unwrap_or_else(PollFlags::empty);
        if signal_ready.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR) {
            signals.drain()?;
            continue;
        }
        if input_ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(io::Error::other("attach input is unavailable"));
        }
        if input_ready.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            let mut bytes = [0u8; 4096];
            let count = match nix::unistd::read(input.as_fd(), &mut bytes) {
                Ok(n) => n,
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            };
            if count == 0 {
                connection.send(&ClientMessage::Detach)?;
                return Ok(AttachEnd::Detached);
            }
            if forward_input(connection, &bytes[..count])? {
                return Ok(AttachEnd::Detached);
            }
        }
    }
}

fn forward_input(connection: &mut Connection, bytes: &[u8]) -> io::Result<bool> {
    let detach = bytes.iter().position(|b| *b == 0x1c);
    let before = &bytes[..detach.unwrap_or(bytes.len())];
    if !before.is_empty() {
        connection.input(before)?;
    }
    if detach.is_some() {
        connection.send(&ClientMessage::Detach)?;
    }
    Ok(detach.is_some())
}

#[cfg(test)]
mod tests;
