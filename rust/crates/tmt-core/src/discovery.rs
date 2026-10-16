//! Bounded, advisory discovery evidence. It never creates or reconciles identities.

use crate::{
    binding::BindingEntry,
    endpoint::{BindingMarker, ProcessIncarnation},
};

pub const MAX_CANDIDATES: usize = 256;
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
pub const SCAN_BUDGET_MS: u64 = 2_000;
pub const MAX_DISPLAY_BYTES: usize = 1024;

/// Native target and ownership fence, independent of display names and scan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryTarget {
    pub socket: String,
    pub server: ProcessIncarnation,
    pub server_id: Option<String>,
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
                host: crate::host::HostKind::Tmux,
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
            crate::host::HostKind::Tmux.as_str(),
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
    pub fence: Option<DiscoveryTarget>,
    /// Stored evidence only; fresh verification is required to call it known.
    pub identity: Option<BindingEntry>,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub duplicate: bool,
    pub text_truncated: bool,
}

impl DiscoveryCandidate {
    pub fn adoptable(&self) -> bool {
        self.fence.is_some() && self.reason.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> DiscoveryTarget {
        DiscoveryTarget {
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
}
