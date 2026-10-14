//! Fixed Auth endpoints and strict ordinary-user replies. No provider error body
//! survives classification; the HTTP adapter owns the absolute request deadline.
use super::{AuthError, AuthErrorKind, CREDENTIAL_BYTES, Cancellation, Credential, Project, Token};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    SignUp,
    Refresh,
    Delete,
    Read,
    Query,
    Commit,
}
pub struct Request {
    operation: Operation,
    url: String,
    body: Vec<u8>,
    content_type: &'static str,
    bearer: Option<String>,
    response_bytes: usize,
}
impl Request {
    /// Private lib-test injection; absent from release artifacts.
    #[cfg(test)]
    pub(super) fn with_test_url(&self, url: String) -> Self {
        Self {
            operation: self.operation,
            url,
            body: self.body.clone(),
            content_type: self.content_type,
            bearer: self.bearer.clone(),
            response_bytes: self.response_bytes,
        }
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn body(&self) -> &[u8] {
        &self.body
    }
    pub fn content_type(&self) -> &'static str {
        self.content_type
    }
    pub fn bearer(&self) -> Option<&str> {
        self.bearer.as_deref()
    }
    pub fn response_bytes(&self) -> usize {
        self.response_bytes
    }
    pub fn method(&self) -> &'static str {
        if self.operation == Operation::Read {
            "GET"
        } else {
            "POST"
        }
    }
    pub(crate) fn firestore(
        operation: Operation,
        url: String,
        body: Vec<u8>,
        bearer: String,
        response_bytes: usize,
    ) -> Self {
        Self {
            operation,
            url,
            body,
            content_type: "application/json",
            bearer: Some(bearer),
            response_bytes,
        }
    }
    pub(super) fn signup(project: &Project) -> Self {
        Self::json(
            project,
            Operation::SignUp,
            "signUp",
            json!({"returnSecureToken":true}),
        )
    }
    pub(super) fn delete(project: &Project, token: &Token) -> Self {
        Self::json(
            project,
            Operation::Delete,
            "delete",
            json!({"idToken":token.value}),
        )
    }
    fn json(project: &Project, operation: Operation, method: &str, body: Value) -> Self {
        Self {
            operation,
            url: format!(
                "https://identitytoolkit.googleapis.com/v1/accounts:{method}?key={}",
                project.api_key
            ),
            body: body.to_string().into_bytes(),
            content_type: "application/json",
            bearer: None,
            response_bytes: CREDENTIAL_BYTES,
        }
    }
    pub(super) fn refresh(project: &Project, credential: &Credential) -> Self {
        let encoded = credential
            .refresh_token
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>();
        Self {
            operation: Operation::Refresh,
            url: format!(
                "https://securetoken.googleapis.com/v1/token?key={}",
                project.api_key
            ),
            body: format!("grant_type=refresh_token&refresh_token={encoded}").into_bytes(),
            content_type: "application/x-www-form-urlencoded",
            bearer: None,
            response_bytes: CREDENTIAL_BYTES,
        }
    }
}
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}
#[derive(Default)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after_ms: Option<u64>,
}
impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
pub trait Http {
    fn send(
        &self,
        request: &Request,
        deadline_monotonic_ms: u64,
        cancel: &dyn Cancellation,
    ) -> Result<Response, AuthError>;
}

#[derive(Deserialize)]
struct Signup {
    #[serde(rename = "localId")]
    uid: String,
    #[serde(rename = "idToken")]
    token: String,
    #[serde(rename = "refreshToken")]
    refresh: String,
    #[serde(rename = "expiresIn")]
    expires: String,
}
#[derive(Deserialize)]
struct Refresh {
    user_id: String,
    project_id: String,
    id_token: String,
    refresh_token: String,
    expires_in: String,
}
pub(super) struct Issued {
    pub uid: String,
    pub token: String,
    pub refresh: String,
    pub lifetime_ms: u64,
}

fn body(response: &Response) -> Result<(), AuthError> {
    if response.body.len() > CREDENTIAL_BYTES {
        return Err(AuthError::new(AuthErrorKind::InvalidReply));
    }
    if response.status == 200 {
        return Ok(());
    }
    let error: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    let kind = match error["error"]["message"].as_str() {
        Some("TOKEN_EXPIRED" | "USER_DISABLED" | "INVALID_REFRESH_TOKEN") => {
            AuthErrorKind::ReSignInNeeded
        }
        Some("USER_NOT_FOUND") => AuthErrorKind::UserNotFound,
        _ if response.status == 429 || response.status >= 500 => AuthErrorKind::Unavailable,
        _ => AuthErrorKind::InvalidReply,
    };
    Err(AuthError::new(kind))
}
fn issued(
    project: &Project,
    uid: String,
    token: String,
    refresh: String,
    lifetime: String,
) -> Result<Issued, AuthError> {
    let invalid = || AuthError::new(AuthErrorKind::InvalidReply);
    let seconds = lifetime.parse::<u64>().map_err(|_| invalid())?;
    if uid.is_empty()
        || uid.len() > 128
        || uid.chars().any(char::is_control)
        || refresh.is_empty()
        || refresh.len() > 8192
        || refresh.chars().any(char::is_control)
        || !(1..=86_400).contains(&seconds)
        || token.len() > 8192
    {
        return Err(invalid());
    }
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0].is_empty()
        || parts[1].is_empty()
        || parts.iter().any(|part| {
            !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        })
    {
        return Err(invalid());
    }
    // These claims bind a reply from the trusted Auth endpoint; this is not a
    // JWT signature verifier. Production Firestore verifies the bearer token.
    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    if claims["aud"] != project.project_id
        || claims["sub"] != uid
        || claims["firebase"]["sign_in_provider"] != "anonymous"
    {
        return Err(invalid());
    }
    Ok(Issued {
        uid,
        token,
        refresh,
        lifetime_ms: seconds * 1000,
    })
}
pub(super) fn signup_reply(project: &Project, response: Response) -> Result<Issued, AuthError> {
    body(&response)?;
    let reply: Signup = serde_json::from_slice(&response.body)
        .map_err(|_| AuthError::new(AuthErrorKind::InvalidReply))?;
    issued(
        project,
        reply.uid,
        reply.token,
        reply.refresh,
        reply.expires,
    )
}
pub(super) fn refresh_reply(
    project: &Project,
    credential: &Credential,
    response: Response,
) -> Result<Issued, AuthError> {
    body(&response)?;
    let reply: Refresh = serde_json::from_slice(&response.body)
        .map_err(|_| AuthError::new(AuthErrorKind::InvalidReply))?;
    if reply.user_id != credential.uid || reply.project_id != project.project_number {
        return Err(AuthError::new(AuthErrorKind::InvalidReply));
    }
    issued(
        project,
        reply.user_id,
        reply.id_token,
        reply.refresh_token,
        reply.expires_in,
    )
}
pub(super) fn delete_reply(response: Response) -> Result<(), AuthError> {
    if response.body.len() > CREDENTIAL_BYTES {
        return Err(AuthError::new(AuthErrorKind::InvalidReply));
    }
    if response.status == 200 {
        return Ok(());
    }
    let error: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    if error["error"]["message"] == "USER_NOT_FOUND" {
        return Ok(());
    }
    body(&response)
}
