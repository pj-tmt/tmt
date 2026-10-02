//! One connection and one delivery write. The caller must establish endpoint
//! ownership before connecting; an arbitrary endpoint is never authoritative.

use super::queue::{QueueOutcome, QueueRequest, RESPONSE_LIMIT};
use serde_json::{Value, json};
use std::{
    net::{SocketAddr, SocketAddrV4, TcpStream},
    time::Instant,
};
use tungstenite::{
    Message, WebSocket,
    client::{IntoClientRequest, client_with_config},
    protocol::WebSocketConfig,
};

mod deadline;
use deadline::DeadlineStream;

const FRAME_LIMIT: usize = 256 * 1024;
const EVENT_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    InvalidEndpoint,
    Unreachable,
    NotReady,
    UnsupportedVersion,
    Uncertain,
}

/// Capability material is deliberately neither Debug nor serializable here.
/// Enrollment records own its private persistence and incarnation validation.
pub struct Endpoint {
    address: SocketAddrV4,
    token: String,
}

impl Endpoint {
    pub fn new(address: SocketAddrV4, token: String) -> Result<Self, TransportError> {
        if !address.ip().is_loopback()
            || address.port() == 0
            || token.is_empty()
            || token.len() > 512
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(TransportError::InvalidEndpoint);
        }
        Ok(Self { address, token })
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    pub fn url(&self) -> String {
        format!("ws://{}", self.address)
    }
}

/// Consuming queue() prevents reuse of this initialized transport for a resend.
/// Reconnection/retry policy is intentionally absent.
pub struct Client {
    socket: WebSocket<DeadlineStream>,
    deadline: Instant,
}

impl Client {
    pub fn connect(endpoint: &Endpoint, deadline: Instant) -> Result<Self, TransportError> {
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(TransportError::Unreachable)?;
        let stream = TcpStream::connect_timeout(&SocketAddr::V4(endpoint.address), timeout)
            .map_err(|_| TransportError::Unreachable)?;
        let stream = DeadlineStream::new(stream, deadline);
        let mut request = endpoint
            .url()
            .into_client_request()
            .map_err(|_| TransportError::InvalidEndpoint)?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", endpoint.token)
                .parse()
                .map_err(|_| TransportError::InvalidEndpoint)?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(RESPONSE_LIMIT))
            .max_frame_size(Some(FRAME_LIMIT));
        let (socket, _) = client_with_config(request, stream, Some(config))
            .map_err(|_| TransportError::NotReady)?;
        let mut client = Self { socket, deadline };
        let id = uuid::Uuid::new_v4().to_string();
        let initialized = client
            .call(
                &json!({"id":id, "method":"initialize", "params":{
                    "clientInfo":{"name":"tmt", "version":env!("CARGO_PKG_VERSION")},
                    "capabilities":{"experimentalApi":true}
                }}),
                &id,
            )
            .map_err(|_| TransportError::NotReady)?;
        if initialized.get("error").is_some() {
            return Err(TransportError::NotReady);
        }
        let user_agent = initialized
            .pointer("/result/userAgent")
            .and_then(Value::as_str)
            .ok_or(TransportError::NotReady)?;
        if !supported_version(user_agent) {
            return Err(TransportError::UnsupportedVersion);
        }
        client
            .write(&json!({"method":"initialized"}))
            .map_err(|_| TransportError::NotReady)?;
        Ok(client)
    }

    pub fn queue(mut self, request: &QueueRequest) -> QueueOutcome {
        // Any failure from the attempted delivery write onward is uncertain.
        // JSON-RPC rejection classification belongs to the bounded receipt.
        match self.call(&request.frame(), request.id()) {
            Ok(value) => {
                request.receipt(&serde_json::to_vec(&value).expect("JSON value serializes"))
            }
            Err(_) => QueueOutcome::Uncertain,
        }
    }

    /// Used only by the enrollment owner for thread creation/inspection on its
    /// newly spawned endpoint. It is not an automatic delivery retry surface.
    pub fn call(&mut self, request: &Value, id: &str) -> Result<Value, TransportError> {
        self.write(request)?;
        for _ in 0..EVENT_LIMIT {
            self.budget()?;
            match self.socket.read().map_err(|_| TransportError::Uncertain)? {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(text.as_str())
                        .map_err(|_| TransportError::Uncertain)?;
                    if value.get("id").and_then(Value::as_str) == Some(id)
                        && value.get("method").is_none()
                    {
                        return Ok(value);
                    }
                    // Notifications may interleave. Requests, mismatched IDs
                    // and malformed envelopes are not silently skipped.
                    if value.get("id").is_some()
                        || value.get("method").and_then(Value::as_str).is_none()
                        || value.get("result").is_some()
                        || value.get("error").is_some()
                    {
                        return Err(TransportError::Uncertain);
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => return Err(TransportError::Uncertain),
            }
        }
        Err(TransportError::Uncertain)
    }

    fn write(&mut self, value: &Value) -> Result<(), TransportError> {
        self.budget()?;
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|_| TransportError::Uncertain)
    }

    fn budget(&self) -> Result<(), TransportError> {
        if Instant::now() >= self.deadline {
            Err(TransportError::Uncertain)
        } else {
            Ok(())
        }
    }
}

fn supported_version(user_agent: &str) -> bool {
    // The owned server's initialize userAgent begins ORIGINATOR/BUILD_VERSION.
    // The trailing client-supplied version is not the provider version.
    let Some((originator, version)) = user_agent
        .split_ascii_whitespace()
        .next()
        .and_then(|p| p.split_once('/'))
    else {
        return false;
    };
    !originator.is_empty() && matches!(version, "0.159.2" | "0.159.3" | "0.160.0")
}

#[cfg(test)]
mod tests;
