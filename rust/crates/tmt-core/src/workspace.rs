//! Remembered workspace structure is recovery input, never live authority.

use std::collections::HashSet;

use crate::{
    binding::{BindingEntry, session::SessionPreferences},
    endpoint::{BindingMarker, ProcessIncarnation, ServerEvidence},
};

pub const VERSION: u64 = 1;
pub const MAX_PANES: usize = 1024;
pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub captured_at_ms: u64,
    pub server: WorkspaceServer,
    pub sessions: Vec<WorkspaceSession>,
    pub windows: Vec<WorkspaceWindow>,
    pub panes: Vec<WorkspacePane>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceServer {
    pub socket: String,
    pub process: ProcessIncarnation,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSession {
    pub id: String,
    pub name: String,
    pub windows: Vec<WindowLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLink {
    pub index: u64,
    pub window: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceWindow {
    pub id: String,
    pub name: String,
    pub layout: String,
    pub visible_layout: String,
    pub width: u64,
    pub height: u64,
    pub active_pane: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePane {
    pub id: String,
    pub window: String,
    pub index: u64,
    pub left: u64,
    pub top: u64,
    pub width: u64,
    pub height: u64,
    pub cwd: String,
    pub identity: Option<WorkspaceIdentity>,
    pub command: Option<ExternalCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub id: String,
    pub name: String,
    pub lifetime: String,
    pub binding: String,
    pub harness: Option<String>,
    pub session: Option<String>,
    pub mode: Option<String>,
    pub channel: Option<bool>,
}

/// Literal extension argv, with the exact exec-surviving native owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCommand {
    pub argv: Vec<String>,
    pub owner: ProcessIncarnation,
}

pub struct StoredIdentity {
    pub entry: BindingEntry,
    pub preferences: SessionPreferences,
}

/// An annotation requires agreement with every durable marker coordinate.
/// Missing or disagreeing evidence leaves an ordinary pane; it never retires rows.
pub fn identity_for_pane(
    records: &[StoredIdentity],
    server: &ServerEvidence,
    pane: &str,
    pid: u64,
    incarnation: Option<&str>,
    marker: Option<&BindingMarker>,
) -> Option<WorkspaceIdentity> {
    let marker = marker?;
    let record = records.iter().find(|record| {
        record.entry.binding.as_ref().is_some_and(|binding| {
            &binding.server == server
                && binding.pane_id == pane
                && binding.pane_pid == pid
                && binding
                    .pane_incarnation
                    .as_deref()
                    .is_some_and(|expected| incarnation == Some(expected))
                && binding.id == marker.binding_id
                && binding.identity_id == marker.identity_id
                && marker.server_id == server.server_id
                && marker.pane_pid == pid
                && record.entry.identity.name == marker.name
                && record.entry.identity.canonical_name == marker.canonical_name
        })
    })?;
    let remembered = record.preferences.remembered.as_ref();
    Some(WorkspaceIdentity {
        id: record.entry.identity.id.clone(),
        name: record.entry.identity.name.clone(),
        lifetime: record.entry.identity.lifetime.as_str().to_owned(),
        binding: marker.binding_id.clone(),
        harness: remembered
            .map(|value| value.harness.as_str().to_owned())
            .or_else(|| {
                record
                    .preferences
                    .preferred_harness
                    .as_ref()
                    .map(|value| value.as_str().to_owned())
            }),
        session: remembered.map(|value| value.provider_session.as_str().to_owned()),
        mode: remembered.map(|value| value.mode.as_str().to_owned()),
        channel: record.preferences.channel,
    })
}

/// No interval policy in event capture. Command refresh is a separate owner.
pub fn hook_capture_allowed(remaining_ms: u64, capture_ms: u64, reserve_ms: u64) -> bool {
    remaining_ms > capture_ms.saturating_add(reserve_ms)
}

impl WorkspaceSnapshot {
    /// Check recovery structure without asserting any live identity authority.
    pub fn is_consistent(&self) -> bool {
        let snapshot = self;
        let session_ids: HashSet<_> = snapshot.sessions.iter().map(|value| &value.id).collect();
        let window_ids: HashSet<_> = snapshot.windows.iter().map(|value| &value.id).collect();
        let pane_ids: HashSet<_> = snapshot.panes.iter().map(|value| &value.id).collect();
        if snapshot.server.socket.is_empty()
            || snapshot
                .server
                .id
                .as_ref()
                .is_some_and(|id| !crate::endpoint::valid_server_id(id))
            || session_ids.len() != snapshot.sessions.len()
            || window_ids.len() != snapshot.windows.len()
            || pane_ids.len() != snapshot.panes.len()
            || snapshot.panes.is_empty()
            || snapshot.sessions.is_empty()
            || snapshot.sessions.iter().any(|session| {
                session.windows.is_empty()
                    || session.windows.iter().filter(|link| link.active).count() != 1
                    || session
                        .windows
                        .iter()
                        .any(|link| !window_ids.contains(&link.window))
                    || session
                        .windows
                        .iter()
                        .map(|link| link.index)
                        .collect::<HashSet<_>>()
                        .len()
                        != session.windows.len()
            })
            || snapshot.windows.iter().any(|window| {
                window.layout.is_empty()
                    || window.width == 0
                    || window.height == 0
                    || !snapshot
                        .panes
                        .iter()
                        .any(|pane| pane.id == window.active_pane && pane.window == window.id)
            })
            || snapshot.panes.iter().any(|pane| {
                !window_ids.contains(&pane.window)
                    || pane.cwd.is_empty()
                    || pane.width == 0
                    || pane.height == 0
            })
        {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        binding::{Binding, session::BindingSessionState},
        host::HostKind,
        identity::{Identity as Stored, Lifetime},
    };

    #[test]
    fn annotation_requires_marker_binding_server_and_native_pane_agreement() {
        let server = ServerEvidence {
            host: HostKind::Tmux,
            server_id: "3c94c4aa-33e1-4b37-978c-5e763f3d078c".into(),
            socket_path: "/socket".into(),
            server_pid: 10,
            server_start_time: "start".into(),
        };
        let identity = Stored {
            id: "identity".into(),
            name: "seat".into(),
            canonical_name: "seat".into(),
            lifetime: Lifetime::Saved,
            created_at: "2026-10-08".into(),
            updated_at: "2026-10-08".into(),
        };
        let binding = Binding {
            id: "binding".into(),
            identity_id: identity.id.clone(),
            server: server.clone(),
            pane_id: "%1".into(),
            pane_pid: 20,
            pane_incarnation: Some("pane-start".into()),
            session: BindingSessionState::default(),
        };
        let marker = binding.marker(&identity);
        let mut records = [StoredIdentity {
            entry: BindingEntry {
                identity,
                binding: Some(binding),
            },
            preferences: SessionPreferences::default(),
        }];
        assert!(
            identity_for_pane(
                &records,
                &server,
                "%1",
                20,
                Some("pane-start"),
                Some(&marker)
            )
            .is_some()
        );
        assert!(
            identity_for_pane(
                &records,
                &server,
                "%1",
                20,
                Some("reused-pid"),
                Some(&marker)
            )
            .is_none()
        );
        assert!(identity_for_pane(&records, &server, "%1", 20, None, Some(&marker)).is_none());
        assert!(identity_for_pane(&records, &server, "%1", 20, Some("pane-start"), None).is_none());
        let mut changed = marker.clone();
        changed.binding_id = "replacement".into();
        assert!(
            identity_for_pane(
                &records,
                &server,
                "%1",
                20,
                Some("pane-start"),
                Some(&changed)
            )
            .is_none()
        );
        let mut other = server.clone();
        other.server_start_time = "replacement".into();
        assert!(
            identity_for_pane(
                &records,
                &other,
                "%1",
                20,
                Some("pane-start"),
                Some(&marker)
            )
            .is_none()
        );
        records[0].entry.binding.as_mut().unwrap().pane_incarnation = None;
        assert!(
            identity_for_pane(
                &records,
                &server,
                "%1",
                20,
                Some("pane-start"),
                Some(&marker)
            )
            .is_none()
        );
    }

    #[test]
    fn hook_capture_requires_strict_headroom_including_reserve() {
        assert!(!hook_capture_allowed(1400, 1200, 200));
        assert!(hook_capture_allowed(1401, 1200, 200));
        assert!(!hook_capture_allowed(0, 1200, 200));
    }
}
