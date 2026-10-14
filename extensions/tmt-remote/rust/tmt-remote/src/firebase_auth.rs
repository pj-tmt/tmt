//! Remote-owned ordinary Firebase Auth credentials. Effects are injected; this
//! owner adds no Core state, credential setting, background process or carrier.
mod credential;
#[cfg(test)]
mod emulator_tests;
mod http;
#[cfg(test)]
mod http_tests;
mod lifecycle;
mod port;
pub mod presentation;
mod rest;
mod system;

pub use credential::FileStore;
pub use http::UreqHttp;
pub use lifecycle::Client;
pub use port::AuthOwner;
pub use rest::{Http, Operation, Request, Response};
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};
pub use system::{RandomJitter, SystemClock};

use crate::limits::{
    FIREBASE_ATTEMPT_MS as ATTEMPT_MS, FIREBASE_CREDENTIAL_BYTES as CREDENTIAL_BYTES,
    FIREBASE_CREDENTIAL_LIFETIME_MS as CREDENTIAL_LIFETIME_MS,
    FIREBASE_EXPIRY_NOTICE_MS as EXPIRY_NOTICE_MS, FIREBASE_MAX_ATTEMPTS as MAX_ATTEMPTS,
    FIREBASE_REFRESH_JITTER_MS as REFRESH_JITTER_MS, FIREBASE_REFRESH_LEAD_MS as REFRESH_LEAD_MS,
    FIREBASE_RESUME_MAX_MS as RESUME_MAX_MS, FIREBASE_RESUME_MIN_MS as RESUME_MIN_MS,
    FIREBASE_SERIES_MS as SERIES_MS,
};
pub const PERMISSION_NOTICE: &str = "Firestore sign-in file permissions tightened to 0600.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthErrorKind {
    Unavailable,
    InvalidReply,
    InvalidCredential,
    ReSignInNeeded,
    UserNotFound,
    Cancelled,
    StateChanged,
    InputInvalid,
    Stopped,
}

/// Provider bodies and filesystem causes never enter a credential diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthError {
    pub kind: AuthErrorKind,
    possible_effect: bool,
}
impl AuthError {
    pub fn unavailable() -> Self {
        Self::new(AuthErrorKind::Unavailable)
    }
    pub fn state_changed() -> Self {
        Self::new(AuthErrorKind::StateChanged)
    }
    pub fn may_have_effect(&self) -> bool {
        self.possible_effect
    }
    pub fn code(&self) -> &'static str {
        if self.kind == AuthErrorKind::InputInvalid {
            "REMOTE_INPUT_INVALID"
        } else {
            "REMOTE_STATE_UNAVAILABLE"
        }
    }
    fn new(kind: AuthErrorKind) -> Self {
        Self {
            kind,
            possible_effect: false,
        }
    }
    fn uncertain(mut self) -> Self {
        self.possible_effect = true;
        self
    }
}
impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            AuthErrorKind::ReSignInNeeded | AuthErrorKind::UserNotFound => {
                presentation::SIGN_IN_ENDED
            }
            AuthErrorKind::InvalidCredential => {
                "Firestore sign-in file could not be read. Check the project JSON, directory mode 0700 and file ownership under remote/firebase-auth; no replacement identity was created."
            }
            AuthErrorKind::InputInvalid => "Invalid Firestore sign-in configuration.",
            AuthErrorKind::Cancelled | AuthErrorKind::Stopped => {
                "Firestore sign-in work was stopped."
            }
            AuthErrorKind::StateChanged => {
                "Firestore sign-in state changed; the reply was not adopted."
            }
            AuthErrorKind::InvalidReply => "Firestore did not return a valid sign-in reply.",
            AuthErrorKind::Unavailable => "Firestore sign-in could not be confirmed.",
        };
        write!(f, "{message} ({})", self.code())
    }
}
impl std::error::Error for AuthError {}

#[derive(Clone, Debug)]
pub struct Project {
    pub project_id: String,
    pub project_number: String,
    api_key: String,
}
impl Project {
    /// The checked public web app ID carries the project number needed to bind
    /// refresh replies. This accepts no owner account or deployment record.
    pub fn from_public_config(config: &serde_json::Value) -> Result<Self, AuthError> {
        let invalid = || AuthError::new(AuthErrorKind::InputInvalid);
        let app = config["appId"].as_str().ok_or_else(invalid)?;
        let parts: Vec<_> = app.split(':').collect();
        if parts.len() != 4 || parts[0] != "1" || parts[2] != "web" || parts[3].is_empty() {
            return Err(invalid());
        }
        Self::new(
            config["projectId"].as_str().ok_or_else(invalid)?,
            parts[1],
            config["apiKey"].as_str().ok_or_else(invalid)?,
        )
    }
    pub fn new(project_id: &str, project_number: &str, api_key: &str) -> Result<Self, AuthError> {
        if !crate::deploy_run::project_ok(project_id)
            || project_number.is_empty()
            || project_number.len() > 32
            || !project_number.bytes().all(|b| b.is_ascii_digit())
            || api_key.is_empty()
            || api_key.len() > 256
            || !api_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(AuthError::new(AuthErrorKind::InputInvalid));
        }
        Ok(Self {
            project_id: project_id.into(),
            project_number: project_number.into(),
            api_key: api_key.into(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthState {
    Unenrolled,
    Active,
    ReSignInNeeded,
    Revoking,
}

/// Safe status projection; no credential or ID token is serializable here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: AuthState,
    pub uid: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub expiry_notice: bool,
    pub permissions_repaired: bool,
}

/// Only the credential file codec serializes this value. Debug is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    project_id: String,
    uid: String,
    refresh_token: String,
    created_at_ms: u64,
    expires_at_ms: u64,
    last_used_at_ms: u64,
    revoking: bool,
}
impl Credential {
    pub fn uid(&self) -> &str {
        &self.uid
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    pub fn mark_revoking(&mut self) {
        self.revoking = true;
    }
}
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("project_id", &self.project_id)
            .field("uid", &self.uid)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("revoking", &self.revoking)
            .finish_non_exhaustive()
    }
}
pub struct StoreRead {
    pub credential: Option<Credential>,
    pub permissions_repaired: bool,
}
pub trait CredentialStore {
    fn load(&self) -> Result<StoreRead, AuthError>;
    /// Compare the captured generation under the writer lock before replacing.
    fn replace(&self, expected: Option<&Credential>, next: &Credential) -> Result<(), AuthError>;
    fn remove(&self, expected: &Credential) -> Result<(), AuthError>;
}

#[derive(Clone, Copy, Debug)]
pub struct Time {
    pub wall_ms: u64,
    pub monotonic_ms: u64,
}
pub trait Clock {
    fn now(&self) -> Time;
    /// Returns false when cancellation interrupts the bounded wait.
    fn wait_until(&self, deadline_ms: u64, cancel: &dyn Cancellation) -> bool;
}
pub trait Jitter {
    fn milliseconds(&self, maximum_inclusive: u64) -> u64;
}
pub trait Cancellation {
    fn cancelled(&self) -> bool;
}
impl Cancellation for AtomicBool {
    fn cancelled(&self) -> bool {
        self.load(Ordering::SeqCst)
    }
}

pub struct Token {
    uid: String,
    value: String,
    expires_monotonic_ms: u64,
    refresh_monotonic_ms: u64,
}
impl Token {
    pub fn uid(&self) -> &str {
        &self.uid
    }
    pub fn bearer(&self) -> &str {
        &self.value
    }
}
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}
