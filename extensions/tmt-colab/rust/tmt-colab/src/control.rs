//! The root-local stop request: `tmt colab stop` asks the serving process to shut down through
//! the owner-only serve socket. Browsers never reach this route (the mount refuses `/.tmt/`),
//! and a request carrying a forwarded device context is denied like every other local route.
use crate::{Result, keyring::Layout};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

pub const STOP_PATH: &str = "/.tmt/colab/local/stop";
/// How a serving process holds its Remote door; the door itself is described in `colab-v1`.
pub const DOORS: [&str; 3] = ["attached", "started", "unavailable"];

/// What a serving socket needs to answer a stop request.
#[derive(Clone)]
pub struct Control {
    /// One of [`DOORS`], so the stopping command can say what happens to the door.
    pub door: &'static str,
    /// Raised by a stop request; the accept loop treats it like SIGTERM.
    pub stopping: Arc<AtomicBool>,
}
impl Control {
    pub fn new() -> Self {
        Self {
            door: "unavailable",
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopReply {
    stopping: bool,
    door: String,
}
impl StopReply {
    pub fn new(control: &Control) -> Self {
        Self {
            stopping: true,
            door: control.door.to_owned(),
        }
    }
    /// The door state the serving process reported, or `None` for an unrecognized reply.
    pub fn door(&self) -> Option<&'static str> {
        DOORS.into_iter().find(|door| *door == self.door)
    }
}

#[derive(Debug)]
pub enum StopFault {
    /// The serving process did not accept the request.
    Unavailable,
    /// It accepted the request but is still running after the bounded wait.
    Slow,
}
impl StopFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "COLAB_UNAVAILABLE",
            Self::Slow => "COLAB_OUTCOME_UNKNOWN",
        }
    }
}
impl std::fmt::Display for StopFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "The serving Colab did not accept the stop request.",
            Self::Slow => {
                "Colab was asked to stop but is still running; check again before retrying."
            }
        })
    }
}
impl std::error::Error for StopFault {}

/// Ask the serving process of this data root to stop. One bounded request, never retried.
pub fn request_stop(layout: &Layout) -> Result<StopReply> {
    let (code, body) =
        crate::ipc::exchange(layout, STOP_PATH, &[]).map_err(|_| StopFault::Unavailable)?;
    let reply: StopReply = serde_json::from_slice(&body).map_err(|_| StopFault::Unavailable)?;
    if code != 200 || !reply.stopping || reply.door().is_none() {
        return Err(StopFault::Unavailable.into());
    }
    Ok(reply)
}
