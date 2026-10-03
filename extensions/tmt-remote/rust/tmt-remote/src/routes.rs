//! Remote binding routes on the door: `/pair` enrollment and the `session.open`
//! control on `/append`, plus signed subscribe/ack.
//! Application adoption remains closed.
use crate::{
    canonical,
    error::RemoteError,
    http::{Handler, Head, Reply, Request},
    limits,
    pairing::{Pairing, Submission},
    session::DoorSessions,
    transport::{LoopbackTransport, Transport},
};
use serde_json::json;
use std::{
    net::TcpStream,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const ROUTES: [&str; 4] = ["/append", "/subscribe", "/ack", "/pair"];

pub struct Routes {
    prefix: String,
    /// Present while serve can pair; without it `/pair` refuses like any route.
    pairing: Option<Arc<Pairing>>,
    /// Present while serve can open door sessions.
    sessions: Option<Arc<DoorSessions>>,
    body_limit: usize,
    input_limit: usize,
    transport: LoopbackTransport,
    attempts: Mutex<Attempts>,
}
struct Attempts {
    count: usize,
    reset: Instant,
}
impl Routes {
    /// `input_limit` is the core-advertised decoded input bound; `prefix` is the
    /// machine's stable `/r/<32 lowercase hex>` route prefix.
    pub fn new(input_limit: usize, prefix: String) -> Result<Self, RemoteError> {
        let prefix_valid = prefix.strip_prefix("/r/").is_some_and(|hex| {
            hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if input_limit == 0 || input_limit > limits::CORE_INPUT_BYTES || !prefix_valid {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Invalid remote input bound or route prefix.",
            ));
        }
        Ok(Self {
            prefix,
            pairing: None,
            sessions: None,
            body_limit: 4 * input_limit.div_ceil(3) + limits::METADATA_BYTES,
            input_limit,
            transport: LoopbackTransport::default(),
            attempts: Mutex::new(Attempts {
                count: 0,
                reset: Instant::now() + Duration::from_secs(60),
            }),
        })
    }
    pub fn with_pairing(mut self, pairing: Arc<Pairing>) -> Self {
        self.pairing = Some(pairing);
        self
    }
    pub fn with_sessions(mut self, sessions: Arc<DoorSessions>) -> Self {
        self.transport = LoopbackTransport::new(Arc::clone(&sessions), self.input_limit);
        self.sessions = Some(sessions);
        self
    }
    /// Route prefix; not a credential.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
    pub fn shutdown(&self) {
        if let Some(sessions) = &self.sessions {
            sessions.shutdown();
        }
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
    fn shutdown(&self) {
        Routes::shutdown(self);
    }
    fn admit(&self, head: &Head<'_>) -> Result<usize, Reply> {
        let Some(suffix) = self.suffix(head.path) else {
            return Err(Reply::empty(404));
        };
        if head.method != "POST" || head.upgrade {
            return Err(Reply::empty(404));
        }
        // The door cookie is scoped to the mount space, so a cookie here is never a
        // session and a cookie alone can never reach an operation or pairing.
        if head.cookie.is_some()
            || head.content_type != Some("application/json")
            || head.content_length.is_none()
        {
            return Err(Reply::empty(400));
        }
        if suffix == "/pair" && !self.attempt() {
            return Err(Reply::empty(429));
        }
        Ok(if suffix == "/pair" {
            limits::PAIR_BODY_BYTES
        } else {
            self.body_limit
        })
    }
    fn handle(&self, request: Request, _: &mut TcpStream) -> Option<Reply> {
        let denied = match self.suffix(&request.path) {
            Some("/append") => {
                let opened = self
                    .sessions
                    .as_ref()
                    .and_then(|sessions| sessions.open(request.origin.as_deref(), &request.body));
                if let Some(opened) = opened {
                    let mut reply = Reply::empty(200);
                    reply.body = opened.response;
                    if let Some(cookie) = opened.cookie {
                        reply.headers.push(("set-cookie".into(), cookie));
                    }
                    return Some(reply);
                }
                self.transport
                    .append(request.origin.as_deref(), &request.body)
            }
            Some("/subscribe") => self
                .transport
                .subscribe(request.origin.as_deref(), &request.body),
            Some("/ack") => self.transport.ack(request.origin.as_deref(), &request.body),
            Some("/pair") => {
                let Some(pairing) = &self.pairing else {
                    return Some(Reply::empty(404));
                };
                return Some(
                    match pairing.submit(request.origin.as_deref(), &request.body) {
                        Submission::Receipt { receipt, proof } => {
                            let mut reply = Reply::empty(200);
                            reply.body = json!({
                                "receipt": canonical::base64url(&receipt),
                                "serverProof": canonical::base64url(&proof),
                            })
                            .to_string()
                            .into_bytes();
                            reply
                        }
                        Submission::Pending => {
                            let mut reply = Reply::empty(202);
                            reply.body = br#"{"state":"pending"}"#.to_vec();
                            reply
                        }
                        // Pre-auth refusals are generic and reveal nothing.
                        Submission::Refused => Reply::empty(404),
                    },
                );
            }
            _ => return Some(Reply::empty(404)),
        };
        Some(match denied {
            Ok(body) => {
                let mut reply = Reply::empty(200);
                reply.body = body;
                reply
            }
            Err(_) => Reply::empty(if self.attempt() { 404 } else { 429 }),
        })
    }
}
