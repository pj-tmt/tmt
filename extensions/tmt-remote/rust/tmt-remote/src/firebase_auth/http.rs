//! One bounded ordinary-user HTTP effect. Redirects, retries and provider
//! diagnostics stay outside this adapter; typed owners classify status/body.
use super::{ATTEMPT_MS, AuthError, AuthErrorKind, Cancellation, Clock, Http, Request, Response};
use std::{io::Read, sync::Arc, time::Duration};

pub struct UreqHttp {
    agent: ureq::Agent,
    clock: Arc<dyn Clock + Send + Sync>,
}
impl UreqHttp {
    pub fn new(clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
                .max_idle_connections(0)
                .build()
                .into(),
            clock,
        }
    }
    fn remaining(&self, deadline: u64, cancel: &dyn Cancellation) -> Result<u64, AuthError> {
        if cancel.cancelled() {
            return Err(AuthError::new(AuthErrorKind::Cancelled));
        }
        let remaining = deadline.saturating_sub(self.clock.now().monotonic_ms);
        if remaining == 0 {
            return Err(AuthError::unavailable());
        }
        Ok(remaining.min(ATTEMPT_MS))
    }
}
impl Http for UreqHttp {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        let remaining = self.remaining(deadline, cancel)?;
        let mut builder = ureq::http::Request::builder()
            .method(request.method())
            .uri(request.url())
            .header("Content-Type", request.content_type());
        if let Some(bearer) = request.bearer() {
            builder = builder.header("Authorization", format!("Bearer {bearer}"));
        }
        let outgoing = builder
            .body(request.body())
            .map_err(|_| AuthError::new(AuthErrorKind::InputInvalid))?;
        let outgoing = self
            .agent
            .configure_request(outgoing)
            .timeout_global(Some(Duration::from_millis(remaining)))
            .build();
        // ureq's global timeout includes DNS, connect, send, headers and body.
        // An in-flight blocking call finishes within this attempt's deadline;
        // cancellation fences every boundary and prevents adopting a late reply.
        self.remaining(deadline, cancel)?;
        let mut incoming = self
            .agent
            .run(outgoing)
            .map_err(|_| AuthError::unavailable().uncertain())?;
        let status = incoming.status().as_u16();
        let retry_after_ms = incoming
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|seconds| seconds.saturating_mul(1000));
        self.remaining(deadline, cancel)
            .map_err(AuthError::uncertain)?;
        let mut body = Vec::new();
        let mut reader = incoming
            .body_mut()
            .as_reader()
            .take(request.response_bytes().saturating_add(1) as u64);
        let mut chunk = [0; 4096];
        loop {
            self.remaining(deadline, cancel)
                .map_err(AuthError::uncertain)?;
            let read = reader
                .read(&mut chunk)
                .map_err(|_| AuthError::unavailable().uncertain())?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
            if body.len() > request.response_bytes() {
                return Err(AuthError::new(AuthErrorKind::InvalidReply).uncertain());
            }
        }
        self.remaining(deadline, cancel)
            .map_err(AuthError::uncertain)?;
        Ok(Response {
            status,
            body,
            retry_after_ms,
        })
    }
}
