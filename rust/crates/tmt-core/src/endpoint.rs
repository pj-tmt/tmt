//! Endpoint observations are evidence, not identity registration or permission.
//! Adapters decode wire formats; application policy decides binding/retirement.

use crate::limits::MAX_JS_SAFE_INTEGER;

pub fn valid_process_id(value: u64) -> bool {
    value > 0 && value <= MAX_JS_SAFE_INTEGER
}

/// One local process: its pid and an opaque token for when it started, so a
/// reused pid is a different process. Both come from core's own inspection
/// of processes on this machine since boot, never from a host's or a
/// driver's text. Servers, panes and runtimes are compared with it; each
/// keeps its own stored columns and lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIncarnation {
    pid: u64,
    start_identity: String,
}

/// A pid or start token that no process inspection could have produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidIncarnation;

impl std::fmt::Display for InvalidIncarnation {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("Invalid process incarnation.")
    }
}

impl std::error::Error for InvalidIncarnation {}

/// Whether two recorded start tokens of the same pid prove different
/// processes. Only two known, different tokens do: an unknown one (`None`)
/// proves neither loss nor sameness, and other evidence decides.
pub fn incarnations_differ(recorded: Option<&str>, observed: Option<&str>) -> bool {
    matches!((recorded, observed), (Some(recorded), Some(observed)) if recorded != observed)
}

impl ProcessIncarnation {
    /// The start token is opaque: 1 to 256 bytes, not blank, with no control
    /// characters.
    pub fn new(pid: u64, start_identity: &str) -> Result<Self, InvalidIncarnation> {
        if !valid_process_id(pid)
            || !(1..=256).contains(&start_identity.len())
            || start_identity.trim().is_empty()
            || start_identity.chars().any(char::is_control)
        {
            return Err(InvalidIncarnation);
        }
        Ok(Self {
            pid,
            start_identity: start_identity.into(),
        })
    }

    pub fn pid(&self) -> u64 {
        self.pid
    }

    pub fn start_identity(&self) -> &str {
        &self.start_identity
    }
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
    /// The start token of `pane_pid`'s [`ProcessIncarnation`], as core
    /// observed it; `None` when the observation failed.
    pub pane_incarnation: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_incarnation_is_a_valid_pid_and_an_opaque_start_token() {
        let process = ProcessIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap();
        assert_eq!(process.pid(), 42);
        assert_eq!(process.start_identity(), "ps-v1:Sun Sep 27 10:00:00 2026");
        let longest = "s".repeat(256);
        assert!(ProcessIncarnation::new(MAX_JS_SAFE_INTEGER, &longest).is_ok());
        for (pid, start) in [
            (0, "start"),
            (MAX_JS_SAFE_INTEGER + 1, "start"),
            (1, ""),
            (1, "   "),
            (1, "a\nb"),
            (1, &"s".repeat(257)),
        ] {
            assert_eq!(
                ProcessIncarnation::new(pid, start),
                Err(InvalidIncarnation),
                "{pid} {start:?}"
            );
        }
        // The same pid with another start is another process.
        assert_ne!(process, ProcessIncarnation::new(42, "ps-v1:later").unwrap());
    }
}
