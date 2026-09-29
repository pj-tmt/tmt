//! Endpoint observations are evidence, not identity registration or permission.
//! Adapters decode wire formats; application policy decides binding/retirement.

use crate::limits::MAX_JS_SAFE_INTEGER;

pub fn valid_process_id(value: u64) -> bool {
    value > 0 && value <= MAX_JS_SAFE_INTEGER
}

pub fn valid_server_id(value: &str) -> bool {
    value.len() == 36
        && uuid::Uuid::parse_str(value).is_ok_and(|uuid| {
            uuid.get_version_num() == 4 && uuid.get_variant() == uuid::Variant::RFC4122
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEvidence {
    /// The host that runs this server; pane IDs follow its syntax.
    pub host: crate::host::HostKind,
    pub server_id: String,
    pub socket_path: String,
    pub server_pid: u64,
    pub server_start_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingMarker {
    pub name: String,
    pub canonical_name: String,
    pub identity_id: String,
    pub binding_id: String,
    pub server_id: String,
    pub pane_pid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneObservation {
    pub id: String,
    pub target: Option<String>,
    pub cwd: Option<String>,
    pub command: String,
    pub pane_pid: u64,
    pub suggested_name: Option<String>,
    pub marker: Option<BindingMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSnapshot {
    pub server: ServerEvidence,
    pub panes: Vec<PaneObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointProbe {
    Live(EndpointSnapshot),
    Dead,
    Unknown,
}
