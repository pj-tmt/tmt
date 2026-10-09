//! The launcher side: start the exact worker, exchange the private records and decide, within one
//! deadline, whether the serve is ready, failed with a known cleanup, or unconfirmed.
use crate::{
    Handshake, StartupError,
    frame::{ACCEPT, ACCEPTED, CANCEL, FAILED, PULSE, read_exact_until, read_frame},
};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::Write,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

/// What to start. The product builds the arguments of its own foreground worker.
pub struct Launch<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub handshake: &'a Handshake,
}

/// A typed failure the worker reported, once validated.
struct Failed {
    error: StartupError,
    cleanup_confirmed: bool,
}
impl Failed {
    fn validate(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.len() != 4 {
            return None;
        }
        let code = object.get("code")?.as_str()?;
        let message = object.get("message")?.as_str()?;
        let hint = match object.get("hint")? {
            Value::Null => None,
            hint => Some(hint.as_str()?),
        };
        let cleanup_confirmed = object.get("cleanupConfirmed")?.as_bool()?;
        let plain =
            |text: &str, limit: usize| text.len() <= limit && !text.chars().any(char::is_control);
        if code.is_empty()
            || code.len() > 64
            || !code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit())
            || !plain(message, 1024)
            || hint.is_some_and(|hint| !plain(hint, 512))
        {
            return None;
        }
        let mut error = StartupError::new(code, message);
        error.hint = hint.map(str::to_owned);
        Some(Self {
            error,
            cleanup_confirmed,
        })
    }
}

/// Start the worker and return its validated `Ready` record once the handoff is accepted and
/// acknowledged. The product prints the result and owns any failure to do so.
pub fn launch(
    spec: &Launch<'_>,
    stop: &AtomicBool,
    validate_ready: impl FnOnce(Value) -> Result<Value, StartupError>,
) -> Result<Value, StartupError> {
    let Handshake {
        unconfirmed,
        timing,
        ..
    } = spec.handshake;
    let unconfirmed = || unconfirmed.clone();
    let cancelled = || spec.handshake.cancelled.clone();
    let deadline = Instant::now() + timing.startup;
    let (mut parent, worker) = UnixStream::pair().map_err(|_| unconfirmed())?;
    parent
        .set_write_timeout(Some(timing.startup))
        .map_err(|_| unconfirmed())?;
    let mut child = Command::new(spec.program)
        .args(spec.args)
        .stdin(Stdio::from(OwnedFd::from(worker)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unconfirmed())?;

    let mut cleanup_confirmed = false;
    let result = (|| {
        let (tag, value) = read_frame(&mut parent, deadline, stop, timing.record_bytes)
            .map_err(|_| unconfirmed())?;
        if tag == FAILED {
            let failed = Failed::validate(&value).ok_or_else(unconfirmed)?;
            cleanup_confirmed = failed.cleanup_confirmed;
            return Err(failed.error);
        }
        let ready = validate_ready(value)?;
        if stop.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(unconfirmed)?;
        parent
            .set_write_timeout(Some(remaining))
            .map_err(|_| unconfirmed())?;
        // ONE byte: a successful write is the exact irreversible handoff cutoff.
        parent.write_all(&[ACCEPT]).map_err(|_| unconfirmed())?;
        Ok(ready)
    })();
    let ready = match result {
        Ok(ready) => ready,
        Err(error) => {
            let _ = parent.set_write_timeout(Some(PULSE));
            let _ = parent.write_all(&[CANCEL]);
            let _ = parent.shutdown(std::net::Shutdown::Write);
            let cleanup_deadline = Instant::now() + timing.cleanup_wait;
            if !cleanup_confirmed {
                let keep_reading = AtomicBool::new(false);
                while let Ok((tag, value)) = read_frame(
                    &mut parent,
                    cleanup_deadline,
                    &keep_reading,
                    timing.record_bytes,
                ) {
                    if tag == FAILED {
                        cleanup_confirmed =
                            Failed::validate(&value).is_some_and(|failed| failed.cleanup_confirmed);
                        break;
                    }
                }
            }
            let exited = reap(&mut child, cleanup_deadline);
            return if exited && cleanup_confirmed {
                Err(if stop.load(Ordering::SeqCst) {
                    cancelled()
                } else {
                    error
                })
            } else {
                Err(unconfirmed())
            };
        }
    };
    // From here the serve may be accepted: no cleanup, kill or automatic retry.
    let mut acknowledgment = [0];
    read_exact_until(&mut parent, &mut acknowledgment, deadline, stop)
        .map_err(|_| unconfirmed())?;
    if acknowledgment[0] != ACCEPTED {
        return Err(unconfirmed());
    }
    Ok(ready)
}

/// Observe and reap only our own child. A false result leaves its PID reserved even if the exit
/// races the signal; never signal after a successful reap. Forced termination cannot confirm
/// that separately grouped descendants ended.
fn reap(child: &mut Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Err(_) => return false,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(PULSE);
    }
    let pid = child.id() as i32;
    // Before setsid the worker is not a group leader. Signal only this reserved child as a
    // fallback; never the inherited caller's process group.
    let _ = nix::sys::signal::killpg(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
    let reap_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < reap_deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return false;
        }
        thread::sleep(PULSE);
    }
    false
}
