//! Remote binding routes on the door. Every `/r/` request is refused until pairing lands.
use crate::{
    error::RemoteError,
    http::{Handler, Head, Reply, Request},
    limits,
    transport::{LoopbackTransport, Transport},
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

const ROUTES: [&str; 4] = ["/append", "/subscribe", "/ack", "/pair"];

pub struct Routes {
    prefix: String,
    body_limit: usize,
    transport: LoopbackTransport,
    attempts: Mutex<Attempts>,
}
struct Attempts {
    count: usize,
    reset: Instant,
}
impl Routes {
    /// `input_limit` is the core-advertised decoded input bound.
    pub fn new(input_limit: usize) -> Result<Self, RemoteError> {
        if input_limit == 0 || input_limit > limits::CORE_INPUT_BYTES {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Invalid remote input bound.",
            ));
        }
        let mut entropy = [0; 16];
        getrandom::fill(&mut entropy)
            .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain route entropy."))?;
        let prefix = format!(
            "/r/{}",
            entropy
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        Ok(Self {
            prefix,
            body_limit: 4 * (input_limit + limits::METADATA_BYTES).div_ceil(3)
                + limits::METADATA_BYTES,
            transport: LoopbackTransport::default(),
            attempts: Mutex::new(Attempts {
                count: 0,
                reset: Instant::now() + Duration::from_secs(60),
            }),
        })
    }
    /// Route prefix; not a credential.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
    fn suffix<'a>(&self, path: &'a str) -> Option<&'a str> {
        path.strip_prefix(&self.prefix)
            .filter(|suffix| ROUTES.contains(suffix))
    }
    fn attempt(&self) -> bool {
        let Ok(mut attempts) = self.attempts.lock() else {
            return false;
        };
        let now = Instant::now();
        if now >= attempts.reset {
            attempts.count = 0;
            attempts.reset = now + Duration::from_secs(60);
        }
        attempts.count += 1;
        attempts.count <= limits::ATTEMPTS_PER_MINUTE
    }
}
impl Handler for Routes {
    fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        let Some(suffix) = self.suffix(head.path) else {
            return Err(Reply::empty(404));
        };
        if head.method != "POST" || head.upgrade {
            return Err(Reply::empty(404));
        }
        // Door sessions are admitted only once pairing issues them.
        if head.cookie.is_some()
            || head.content_type != Some("application/json")
            || head.content_length.is_none()
        {
            return Err(Reply::empty(400));
        }
        if !self.attempt() {
            return Err(Reply::empty(429));
        }
        Ok(if suffix == "/pair" {
            limits::PAIR_BODY_BYTES
        } else {
            self.body_limit
        })
    }
    fn handle(&self, request: Request) -> Reply {
        let denied = match self.suffix(&request.path) {
            Some("/append") => self.transport.append(&request.body),
            Some("/subscribe") => self.transport.subscribe(&request.body),
            Some("/ack") => self.transport.ack(&request.body),
            _ => return Reply::empty(404),
        };
        // No successful admission exists in this slice. Fail closed if it changes.
        let _ = denied;
        Reply::empty(404)
    }
}
