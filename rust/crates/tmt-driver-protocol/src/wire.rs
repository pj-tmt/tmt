//! The JSON a driver reads and prints. Members are camelCase; both sides
//! ignore members they don't know, so a later protocol-1 addition is
//! optional by construction. A new required member needs protocol 2.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a driver answers to `capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Every protocol version the driver speaks.
    pub protocols: Vec<u32>,
    /// `host` for a terminal-host driver.
    pub kind: String,
    /// The driver's name: `[a-z][a-z0-9-]{0,31}`, also the stored host token.
    pub name: String,
    pub version: String,
    /// The operations it implements besides `capabilities`; any other
    /// operation answers `unsupported`.
    pub ops: Vec<String>,
    pub pane_id: PaneIdSyntax,
    /// How a user names one of its panes, as a template where `{n}` stands
    /// for 1–9 digits (`w{n}:p{n}`); `None` when panes have no public name.
    #[serde(default)]
    pub target: Option<String>,
    /// The environment variables `caller` reads; core passes only these.
    #[serde(default)]
    pub caller_env: Vec<String>,
}

/// A pane ID is `prefix` followed by 1–64 of `[0-9a-z]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneIdSyntax {
    pub prefix: String,
}

/// Every request carries how long the driver has left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request<T> {
    pub deadline_ms: u64,
    #[serde(flatten)]
    pub body: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerRequest {
    /// Only the variables the driver declared, and only those set.
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerResponse {
    pub pane: Option<CallerPane>,
}

/// The pane the environment names. Core counts it only when `shell_pid` is
/// an ancestor of the caller, which it checks itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallerPane {
    pub id: String,
    pub socket: String,
    pub shell_pid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRequest {
    /// The socket to ask about; `None` asks the driver's default server.
    pub socket: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerResponse {
    /// `None` when no server runs there.
    pub server: Option<ServerIncarnation>,
}

/// One running server, named by its socket and its process's incarnation;
/// core gives each incarnation its own UUID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerIncarnation {
    pub socket: String,
    pub pid: u64,
    pub start_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveTargetRequest {
    pub socket: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveTargetResponse {
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRequest {
    pub socket: String,
    /// Only these panes; `None` for every pane.
    pub panes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub panes: Vec<Pane>,
}

/// One pane as the host reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    pub id: String,
    pub target: Option<String>,
    pub cwd: Option<String>,
    /// The foreground command's name.
    pub command: String,
    /// The pane's shell process.
    pub pane_pid: u64,
    pub suggested_name: Option<String>,
    /// The marker `publish` stored, returned as stored.
    pub marker: Option<Marker>,
}

/// What core stores on a bound pane so any process can see the binding.
/// The driver keeps it and returns it; it never interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub name: String,
    pub canonical_name: String,
    pub identity_id: String,
    pub binding_id: String,
    pub server_id: String,
    pub pane_pid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub server: ServerIncarnation,
    pub panes: Vec<String>,
}

/// `dead` means the incarnation is gone; core believes it only when its own
/// check of the recorded server process agrees, and treats it as `unknown`
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ProbeResponse {
    Live { panes: Vec<Pane> },
    Dead,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    pub socket: String,
    pub pane_id: String,
    /// Refuse with `not_found` unless the pane still runs this shell.
    pub pane_pid: u64,
    pub marker: Marker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearRequest {
    pub socket: String,
    pub pane_id: String,
    /// Clear only a marker with this binding.
    pub binding_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearResponse {
    pub cleared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub socket: String,
    pub pane_id: String,
    pub lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureResponse {
    pub text: String,
}

/// Paste `text` literally, then press Enter when asked. Core has already
/// applied its delivery policy; the driver adds and interprets nothing, and
/// never retries. Which answers mean "not sent" is the contract's delivery
/// outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequest {
    pub socket: String,
    pub pane_id: String,
    pub text: String,
    pub enter: bool,
}

/// Hands `text` to the agent the host recognizes in the pane, which submits
/// it; core has applied its delivery policy and decides any fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub socket: String,
    pub pane_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusRequest {
    pub socket: String,
    pub pane_id: String,
}

/// What a runtime driver answers to `capabilities`: what core applies itself
/// on an agent's hook path, without starting the driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilities {
    pub protocols: Vec<u32>,
    /// `runtime` for a runtime driver.
    pub kind: String,
    /// The driver's name, also the harness ID stored with a session.
    pub name: String,
    pub version: String,
    /// `locations`, and optionally `resume` and `usage`.
    pub ops: Vec<String>,
    /// Bare command names; a pane command whose last path component is one of
    /// them is this agent, and `tmt run` starts the first.
    pub executables: Vec<String>,
    /// The environment variables `locations` reads; core passes only these.
    #[serde(default)]
    pub env: Vec<String>,
    /// The variable holding the caller's provider session ID, which core
    /// reads itself.
    #[serde(default)]
    pub session_env: Option<String>,
    #[serde(default)]
    pub hooks: Option<Hooks>,
}

/// Where a provider hook's payload holds each value, and what each event
/// means; core decodes hooks from this without starting the driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hooks {
    /// `sessionHooksJson`: the settings layout of the built-in Claude and
    /// Codex hooks.
    pub format: String,
    pub fields: HookFields,
    pub events: Vec<HookEventDeclaration>,
}

/// JSON Pointers (RFC 6901) into a hook's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookFields {
    pub event: String,
    pub session: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub transcript: Option<String>,
    #[serde(default)]
    pub turn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEventDeclaration {
    /// The string at `fields.event` that selects this event.
    pub name: String,
    pub effect: HookEffect,
    /// For `start` and `end`: the pointer whose string `values` maps.
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub values: BTreeMap<String, Transition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookEffect {
    /// A session began or changed.
    Start,
    /// A session ended or changed.
    End,
    /// A turn began.
    Working,
    /// A turn ended.
    Idle,
}

/// How a session changed, as a `start` or `end` event reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transition {
    Started,
    Resumed,
    Cleared,
    Compacted,
    Forked,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationsRequest {
    pub home: String,
    /// Only the variables the driver declared, and only those set.
    pub env: BTreeMap<String, String>,
}

/// Absolute paths; core decides how it uses each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationsResponse {
    /// The agent counts as installed when one of these exists.
    pub config_dirs: Vec<String>,
    /// Where `tmt setup` installs TMT's skill.
    pub skills: String,
    /// Where `tmt setup` writes hooks; required when hooks are declared.
    pub hook_settings: Option<String>,
    /// The only directory whose files core passes to `usage`; required when
    /// `usage` is declared.
    pub transcript_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub session: String,
    /// Only a model a hook reported for this session.
    pub model: Option<String>,
}

/// Core starts `argv` directly, never through a shell, and only when its
/// first element is a declared executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeResponse {
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRequest {
    pub session: String,
    /// A path core checked lies inside `transcriptRoot`.
    pub transcript: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageResponse {
    /// `None` when the transcript shows no usage.
    pub context_tokens: Option<u64>,
}

/// A driver prints exactly one of `{"ok": …}` or `{"error": …}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Response<T> {
    Ok(T),
    Error(DriverError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The operation isn't implemented; core falls back as for a host
    /// without it (the inbox for `input`).
    Unsupported,
    /// The request didn't parse or break the contract.
    BadRequest,
    /// The host server can't be reached.
    Unavailable,
    /// The pane or server is gone, or no longer the one asked about.
    NotFound,
    Failed,
    /// `prompt` only: the host recognizes no agent in the pane.
    NoAgent,
    /// `prompt` only: the agent waits on its user.
    Blocked,
    /// `prompt` only: the agent can't take a prompt now.
    NotReady,
}

impl ErrorCode {
    /// The codes valid only as `prompt` answers.
    pub const fn prompt_only(self) -> bool {
        matches!(self, Self::NoAgent | Self::Blocked | Self::NotReady)
    }
}

impl DriverError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_envelope_and_probe_states_have_their_documented_shape() {
        let ok: Response<ClearResponse> = Response::Ok(ClearResponse { cleared: true });
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"ok": {"cleared": true}})
        );
        let error: Response<Empty> =
            Response::Error(DriverError::new(ErrorCode::NotFound, "pane closed"));
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"error": {"code": "not_found", "message": "pane closed"}})
        );
        assert_eq!(
            serde_json::to_value(ProbeResponse::Dead).unwrap(),
            json!({"state": "dead"})
        );
        let request = Request {
            deadline_ms: 250,
            body: ClearRequest {
                socket: "/s".into(),
                pane_id: "term_1".into(),
                binding_id: "b".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"deadlineMs": 250, "socket": "/s", "paneId": "term_1", "bindingId": "b"})
        );
    }

    #[test]
    fn unknown_members_are_ignored_both_ways() {
        let request: Request<ServerRequest> =
            serde_json::from_value(json!({"deadlineMs": 1, "socket": null, "later": 1})).unwrap();
        assert_eq!(request.body.socket, None);
        let response: Response<ClearResponse> =
            serde_json::from_value(json!({"ok": {"cleared": false, "later": true}})).unwrap();
        assert_eq!(response, Response::Ok(ClearResponse { cleared: false }));
    }
}
