//! The worker side: take the private endpoint, report readiness once and learn whether the
//! launcher accepted the handoff.
use crate::{
    Handshake, StartupError,
    frame::{ACCEPT, ACCEPTED, FAILED, PULSE, read_exact_until, write_frame},
};
use serde_json::{Value, json};
use std::{
    io::Write,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

/// Only one monitor and one private endpoint during startup. Accept is read before EOF; once
/// read, neither a later EOF nor a lost acknowledgment cancels it.
pub struct Handoff {
    stream: Option<UnixStream>,
    monitor: Option<JoinHandle<bool>>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    handshake: Handshake,
}

/// Become the detached worker: a new session, the private pair taken from inherited stdin (which
/// is then closed), and the handoff monitor running. Call it first in the worker process.
pub fn adopt(handshake: &Handshake, stop: &Arc<AtomicBool>) -> Result<Handoff, StartupError> {
    let unconfirmed = || handshake.unconfirmed.clone();
    // Stdio transfers the private pair safely through exec, without raw-FD adoption. No
    // process-group change occurs in the parent; the exact worker drops its terminal at once.
    nix::unistd::setsid().map_err(|_| unconfirmed())?;
    let input = std::io::stdin();
    nix::fcntl::fcntl(
        &input,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )
    .map_err(|_| unconfirmed())?;
    let owned = input
        .as_fd()
        .try_clone_to_owned()
        .map_err(|_| unconfirmed())?;
    let stream = UnixStream::from(owned);
    if stream.peer_addr().is_err() {
        return Err(unconfirmed());
    }
    nix::unistd::close(0).map_err(|_| unconfirmed())?;
    Handoff::new(handshake, stream, Arc::clone(stop))
}

impl Handoff {
    /// The monitor sets `stop` when the launcher cancels, disappears or runs out of time before
    /// accepting, which unwinds the worker through its ordinary shutdown.
    pub fn new(
        handshake: &Handshake,
        stream: UnixStream,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, StartupError> {
        let unconfirmed = || handshake.unconfirmed.clone();
        nix::fcntl::fcntl(
            &stream,
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .map_err(|_| unconfirmed())?;
        stream
            .set_write_timeout(Some(handshake.timing.startup))
            .map_err(|_| unconfirmed())?;
        let mut reader = stream.try_clone().map_err(|_| unconfirmed())?;
        let monitor_stop = Arc::clone(&stop);
        let deadline = Instant::now() + handshake.timing.startup;
        let monitor = thread::Builder::new()
            .name("serve-startup".into())
            .spawn(move || {
                let mut command = [0];
                let accepted = read_exact_until(&mut reader, &mut command, deadline, &monitor_stop)
                    .is_ok()
                    && command[0] == ACCEPT;
                if !accepted {
                    monitor_stop.store(true, Ordering::SeqCst);
                }
                accepted
            })
            .map_err(|_| unconfirmed())?;
        Ok(Self {
            stream: Some(stream),
            monitor: Some(monitor),
            stop,
            deadline,
            handshake: handshake.clone(),
        })
    }

    /// Send `Ready` and wait for the launcher's decision. `Ok` means the handoff is accepted and
    /// the serve may start serving. An error means it must unwind, and the product then calls
    /// [`Handoff::fail`] so the launcher learns that cleanup is confirmed.
    pub fn ready(&mut self, value: &Value) -> Result<(), StartupError> {
        let unconfirmed = || self.handshake.unconfirmed.clone();
        let cancelled = || self.handshake.cancelled.clone();
        if self.stop.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(unconfirmed)?;
        let stream = self.stream.as_mut().expect("startup endpoint");
        stream
            .set_write_timeout(Some(remaining))
            .map_err(|_| unconfirmed())?;
        write_frame(
            stream,
            crate::frame::READY,
            value,
            self.handshake.timing.record_bytes,
        )
        .map_err(|_| unconfirmed())?;
        let accepted = self
            .monitor
            .take()
            .expect("one startup monitor")
            .join()
            .unwrap_or(false);
        if !accepted {
            return Err(cancelled());
        }
        // The command may have been accepted while a real shutdown signal raced. Preserve that
        // signal. A lost Accepted write never revokes the handoff.
        let mut stream = self.stream.take().expect("startup endpoint");
        let _ = stream.set_write_timeout(Some(PULSE));
        let _ = stream.write_all(&[ACCEPTED]);
        Ok(())
    }

    /// Stop the monitor if the handoff never reached its decision.
    pub fn finish(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            if let Some(stream) = &self.stream {
                let _ = stream.shutdown(std::net::Shutdown::Read);
            }
            let _ = monitor.join();
        }
    }

    /// Report a failure before the handoff was accepted, if the endpoint is still open. The
    /// message is sent only when plain and bounded. `cleanup_confirmed` is the product's claim
    /// that nothing it started still runs.
    pub fn fail(&mut self, error: &StartupError, cleanup_confirmed: bool) {
        self.finish();
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let message = if error.message.len() <= 1024 && !error.message.chars().any(char::is_control)
        {
            error.message.clone()
        } else {
            self.handshake.unconfirmed.message.clone()
        };
        let _ = write_frame(
            stream,
            FAILED,
            &json!({"code":error.code,"message":message,"hint":error.hint,"cleanupConfirmed":cleanup_confirmed}),
            self.handshake.timing.record_bytes,
        );
    }
}
impl Drop for Handoff {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Timing, frame::CANCEL};
    use std::time::Duration;

    fn handshake() -> Handshake {
        Handshake {
            unconfirmed: StartupError::new("TEST_UNCONFIRMED", "Not confirmed."),
            cancelled: StartupError::new("TEST_CANCELLED", "Cancelled."),
            timing: Timing {
                startup: Duration::from_secs(10),
                cleanup_wait: Duration::from_secs(10),
                record_bytes: 4096,
            },
        }
    }

    #[test]
    fn buffered_accept_wins_over_later_eof_but_never_erases_shutdown() {
        let (mut parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(&handshake(), worker, Arc::clone(&stop)).unwrap();
        parent.write_all(&[ACCEPT]).unwrap();
        parent.shutdown(std::net::Shutdown::Write).unwrap();
        handoff.ready(&json!({"fixture":"ready"})).unwrap();
        assert!(!stop.load(Ordering::SeqCst));
        assert!(handoff.monitor.is_none() && handoff.stream.is_none());

        let (mut parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(&handshake(), worker, Arc::clone(&stop)).unwrap();
        parent.write_all(&[ACCEPT]).unwrap();
        assert!(handoff.monitor.take().unwrap().join().unwrap());
        stop.store(true, Ordering::SeqCst);
        handoff.finish();
        assert!(stop.load(Ordering::SeqCst));
    }

    #[test]
    fn eof_cancel_and_failure_wake_and_join_the_only_monitor() {
        for command in [None, Some(CANCEL), Some(99)] {
            let (mut parent, worker) = UnixStream::pair().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let mut handoff = Handoff::new(&handshake(), worker, Arc::clone(&stop)).unwrap();
            if let Some(command) = command {
                parent.write_all(&[command]).unwrap();
            }
            drop(parent);
            assert!(!handoff.monitor.take().unwrap().join().unwrap());
            assert!(stop.load(Ordering::SeqCst));
        }
        let (_parent, worker) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut handoff = Handoff::new(&handshake(), worker, stop).unwrap();
        handoff.finish();
        assert!(handoff.monitor.is_none());
    }
}
