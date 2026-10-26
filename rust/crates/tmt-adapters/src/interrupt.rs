//! Invocation-owned SIGINT notification without a worker thread or busy polling.

use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use signal_hook::{SigId, consts::SIGINT, flag, low_level};
use std::{
    io::{self, Read},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct Interrupt {
    reader: UnixStream,
    interrupted: Arc<AtomicBool>,
    registrations: Vec<SigId>,
    emergency: Option<SigId>,
}

impl Interrupt {
    pub fn install() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let mut guard = Self {
            reader,
            interrupted: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
            emergency: None,
        };
        // Registration order is significant: a second SIGINT keeps the default
        // emergency termination behavior even during blocked synchronous work.
        guard.emergency = Some(flag::register_conditional_default(
            SIGINT,
            guard.interrupted.clone(),
        )?);
        guard
            .registrations
            .push(flag::register(SIGINT, guard.interrupted.clone())?);
        guard
            .registrations
            .push(low_level::pipe::register(SIGINT, writer)?);
        Ok(guard)
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    /// A bounded child owner must clean up before this invocation can exit.
    /// Repeated SIGINTs remain notifications and shorten that owner's wait.
    pub fn defer_emergency_exit(&mut self) {
        if let Some(registration) = self.emergency.take() {
            low_level::unregister(registration);
        }
    }

    pub(crate) fn take_notifications(&self) -> io::Result<usize> {
        let mut count = 0;
        let mut reader = &self.reader;
        loop {
            match reader.read(&mut [0; 64]) {
                Ok(0) => return Err(io::Error::other("Interrupt notification closed")),
                Ok(read) => {
                    count += read;
                    if count >= 2 {
                        return Ok(count);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(count),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn wait_until(&self, deadline: Instant) -> io::Result<()> {
        while !self.is_interrupted() {
            let Some(remaining) = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
            else {
                return Ok(());
            };
            let millis = remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128);
            let timeout = PollTimeout::try_from(millis).map_err(io::Error::other)?;
            let mut events = [PollFd::new(self.reader.as_fd(), PollFlags::POLLIN)];
            match poll(&mut events, timeout) {
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            let ready = events[0]
                .revents()
                .ok_or_else(|| io::Error::other("Invalid interrupt descriptor state"))?;
            if ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL | PollFlags::POLLHUP) {
                return Err(io::Error::other("Interrupt notification failed"));
            }
        }
        Ok(())
    }
}

impl Drop for Interrupt {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..).rev() {
            low_level::unregister(registration);
        }
        self.defer_emergency_exit();
    }
}

#[cfg(test)]
mod tests;
