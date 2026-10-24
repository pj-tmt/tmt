//! Bounded, advisory discovery evidence. It never creates or reconciles identities.

use crate::{
    binding::BindingEntry,
    endpoint::{BindingMarker, ProcessIncarnation},
    host::HostKind,
};

pub const MAX_CANDIDATES: usize = 256;
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
pub const SCAN_BUDGET_MS: u64 = 2_000;
pub const MAX_DISPLAY_BYTES: usize = 1024;

/// Native target and ownership fence, independent of display names and scan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryTarget {
    /// The first fenced field: a pane of one host never matches another's.
    pub host: HostKind,
    pub socket: String,
    pub server: ProcessIncarnation,
    pub server_id: Option<String>,
    /// The host's `ServerEvidence.server_start_time`: tmux's own start time, or
    /// core's start token for an external host's server process.
    pub server_start_time: String,
    pub pane: String,
    pub process: ProcessIncarnation,
    pub marker: Option<BindingMarker>,
    /// Advisory executable evidence; this is not readiness or routing authority.
    pub runtime: Option<String>,
}

impl DiscoveryTarget {
    /// Minimal native/marker evidence for the existing binding evaluator.
    pub fn endpoint_snapshot(&self) -> Option<crate::endpoint::EndpointSnapshot> {
        Some(crate::endpoint::EndpointSnapshot {
            server: crate::endpoint::ServerEvidence {
                host: self.host,
                server_id: self.server_id.clone()?,
                socket_path: self.socket.clone(),
                server_pid: self.server.pid(),
                server_start_time: self.server_start_time.clone(),
            },
            panes: vec![crate::endpoint::PaneObservation {
                agent_name: None,
                id: self.pane.clone(),
                target: None,
                cwd: None,
                command: String::new(),
                pane_pid: self.process.pid(),
                pane_incarnation: Some(self.process.start_identity().into()),
                suggested_name: self.runtime.clone(),
                marker: self.marker.clone(),
                location: None,
            }],
        })
    }

    pub fn digest(&self) -> String {
        let mut bytes = Vec::new();
        let mut field = |value: &str| {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        };
        for value in [
            self.host.as_str(),
            &self.socket,
            &self.server.pid().to_string(),
            self.server.start_identity(),
            &self.server_start_time,
            &self.pane,
            &self.process.pid().to_string(),
            self.process.start_identity(),
            self.runtime.as_deref().unwrap_or(""),
        ] {
            field(value);
        }
        // Initializing the server UUID must not invalidate other unmarked panes
        // from the same scan. Native server identity fences replacement; marker
        // UUIDs fence existing pane ownership, including across cosmetic rename.
        if let Some(marker) = &self.marker {
            for value in [
                &marker.identity_id,
                &marker.binding_id,
                &marker.server_id,
                &marker.pane_pid.to_string(),
            ] {
                field(value);
            }
        }
        crate::content_digest::framed_sha256(b"tmt-discovery-v1\0", &bytes)
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryCandidate {
    pub pane: String,
    pub session: Option<String>,
    pub window: Option<String>,
    pub target: Option<String>,
    pub title: String,
    pub command: String,
    pub cwd: String,
    /// The host's untrusted agent name: display only, never evidence and
    /// never copied into an identity.
    pub agent_name: Option<String>,
    pub fence: Option<DiscoveryTarget>,
    /// Stored evidence only; fresh verification is required to call it known.
    pub identity: Option<BindingEntry>,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub duplicate: bool,
    pub text_truncated: bool,
}

/// `value` cut to [`MAX_DISPLAY_BYTES`] on a character boundary, and whether
/// it was cut.
pub fn display_text(value: &str) -> (String, bool) {
    let mut end = value.len().min(MAX_DISPLAY_BYTES);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].into(), end != value.len())
}

impl DiscoveryCandidate {
    pub fn adoptable(&self) -> bool {
        self.fence.is_some() && self.reason.is_none()
    }
}

/// What a host answered for its source, beside its rows. Operational failures
/// are errors of the adapter that observed them, not a source state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverySource {
    Ok,
    /// The host's definite answer that no server runs.
    Offline,
    /// The host cannot list panes; the reason is a stable token.
    Unsupported(&'static str),
}

impl DiscoverySource {
    /// The JSON `status` and `reason` of the source.
    pub fn status(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Ok => ("ok", None),
            Self::Offline => ("offline", None),
            Self::Unsupported(reason) => ("unsupported", Some(reason)),
        }
    }
}

/// One host's bounded scan of one server.
#[derive(Debug, Clone)]
pub struct DiscoveryObservation {
    pub host: HostKind,
    pub source: DiscoverySource,
    pub socket: String,
    pub candidates: Vec<DiscoveryCandidate>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> DiscoveryTarget {
        DiscoveryTarget {
            host: HostKind::Tmux,
            socket: "/tmp/a".into(),
            server: ProcessIncarnation::new(1, "server-start").unwrap(),
            server_id: None,
            server_start_time: "1".into(),
            pane: "%1".into(),
            process: ProcessIncarnation::new(2, "pane-start").unwrap(),
            marker: None,
            runtime: Some("codex".into()),
        }
    }
    #[test]
    fn discovery_digest_fences_native_incarnations_and_ownership_not_names() {
        let mut original = target();
        original.marker = Some(BindingMarker {
            name: "first".into(),
            canonical_name: "first".into(),
            identity_id: "id-a".into(),
            binding_id: "binding-a".into(),
            server_id: "server-a".into(),
            pane_pid: 2,
        });
        let digest = original.digest();
        assert!(crate::content_digest::is_sha256(&digest));
        let mut renamed = original.clone();
        renamed.marker.as_mut().unwrap().name = "new".into();
        renamed.marker.as_mut().unwrap().canonical_name = "new".into();
        assert_eq!(renamed.digest(), digest);
        let mut reused = original.clone();
        reused.process = ProcessIncarnation::new(2, "reused-pid").unwrap();
        assert_ne!(reused.digest(), digest);
        let mut replaced = original.clone();
        replaced.marker.as_mut().unwrap().identity_id = "id-b".into();
        assert_ne!(replaced.digest(), digest);
        let mut other_owner = original.clone();
        other_owner.marker.as_mut().unwrap().server_id = "server-b".into();
        assert_ne!(other_owner.digest(), digest);
        let mut restart = original.clone();
        restart.server = ProcessIncarnation::new(1, "server-restart").unwrap();
        assert_ne!(restart.digest(), digest);
        let mut other = original.clone();
        other.socket = "/tmp/b".into();
        assert_ne!(other.digest(), digest);
        assert_ne!(target().digest(), digest);
    }

    #[test]
    fn discovery_digest_survives_server_uuid_initialization_not_native_replacement() {
        let unmarked = target();
        let mut initialized = unmarked.clone();
        initialized.server_id = Some("server-a".into());
        assert_eq!(initialized.digest(), unmarked.digest());
        initialized.server = ProcessIncarnation::new(1, "server-replaced").unwrap();
        assert_ne!(initialized.digest(), unmarked.digest());
    }

    fn external() -> DiscoveryTarget {
        DiscoveryTarget {
            host: HostKind::parse("herdr").unwrap(),
            socket: "/tmp/herdr.sock".into(),
            server: ProcessIncarnation::new(1, "server-start").unwrap(),
            server_id: None,
            server_start_time: "server-start".into(),
            pane: "term_7".into(),
            process: ProcessIncarnation::new(2, "pane-start").unwrap(),
            marker: None,
            runtime: Some("codex".into()),
        }
    }

    #[test]
    fn discovery_digest_and_snapshot_take_the_fenced_host() {
        let tmux = target();
        let mut other = tmux.clone();
        other.host = HostKind::parse("herdr").unwrap();
        assert_ne!(other.digest(), tmux.digest());
        let mut known = other.clone();
        known.server_id = Some("server-a".into());
        let snapshot = known.endpoint_snapshot().unwrap();
        assert_eq!(snapshot.server.host, other.host);
        assert_eq!(snapshot.server.server_start_time, "1");
        assert_eq!(snapshot.panes[0].id, "%1");
    }

    #[test]
    fn external_discovery_digest_refuses_reopened_panes_and_restarts_not_renames() {
        let mut owned = external();
        owned.marker = Some(BindingMarker {
            name: "first".into(),
            canonical_name: "first".into(),
            identity_id: "id-a".into(),
            binding_id: "binding-a".into(),
            server_id: "server-a".into(),
            pane_pid: 2,
        });
        let digest = owned.digest();
        // The same `term_N` and a reused shell PID are a new pane.
        let mut reopened = owned.clone();
        reopened.process = ProcessIncarnation::new(2, "pane-reopened").unwrap();
        assert_ne!(reopened.digest(), digest);
        let mut restarted = owned.clone();
        restarted.server = ProcessIncarnation::new(1, "server-restart").unwrap();
        restarted.server_start_time = "server-restart".into();
        assert_ne!(restarted.digest(), digest);
        let mut released = owned.clone();
        released.marker.as_mut().unwrap().binding_id = "binding-b".into();
        assert_ne!(released.digest(), digest);
        let mut renamed = owned.clone();
        renamed.marker.as_mut().unwrap().name = "second".into();
        renamed.marker.as_mut().unwrap().canonical_name = "second".into();
        assert_eq!(renamed.digest(), digest);
        let mut initialized = external();
        initialized.server_id = Some("server-a".into());
        assert_eq!(initialized.digest(), external().digest());
    }

    #[test]
    fn a_source_reports_the_hosts_answer() {
        assert_eq!(DiscoverySource::Ok.status(), ("ok", None));
        assert_eq!(DiscoverySource::Offline.status(), ("offline", None));
        assert_eq!(
            DiscoverySource::Unsupported("driver_lacks_inventory").status(),
            ("unsupported", Some("driver_lacks_inventory"))
        );
    }
}
