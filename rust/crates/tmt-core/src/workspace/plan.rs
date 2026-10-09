//! A restore preview uses current durable preferences, without authorizing launch.

use super::{StoredIdentity, WorkspaceSnapshot};
use crate::binding::session::RememberedSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSessionAction {
    Create,
    SkipExisting,
    Unknown,
}

impl WorkspaceSessionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::SkipExisting => "skip_existing",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspacePaneAction {
    Shell,
    RelaunchCommand,
    IdentityMissing,
    NoRememberedSession,
    Resumable,
    StaleRequiresRetry,
}

impl WorkspacePaneAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::RelaunchCommand => "relaunch_command",
            Self::IdentityMissing => "identity_missing",
            Self::NoRememberedSession => "no_remembered_session",
            Self::Resumable => "resumable",
            Self::StaleRequiresRetry => "stale_requires_retry",
        }
    }
}

pub struct WorkspacePanePlan<'a> {
    pub pane: &'a str,
    pub action: WorkspacePaneAction,
    pub identity: Option<&'a StoredIdentity>,
    pub remembered: Option<&'a RememberedSession>,
}

pub struct WorkspaceRestorePlan<'a> {
    pub sessions: Vec<(&'a str, WorkspaceSessionAction)>,
    pub panes: Vec<WorkspacePanePlan<'a>>,
}

/// None means current session names could not be observed. Old names/UUIDs are
/// annotations only; an implementation applying this plan must recheck live state.
pub fn restore_plan<'a>(
    snapshot: &'a WorkspaceSnapshot,
    identities: &'a [StoredIdentity],
    current_sessions: Option<&[String]>,
) -> WorkspaceRestorePlan<'a> {
    let sessions = snapshot
        .sessions
        .iter()
        .map(|session| {
            let action = match current_sessions {
                Some(names) if names.contains(&session.name) => {
                    WorkspaceSessionAction::SkipExisting
                }
                Some(_) => WorkspaceSessionAction::Create,
                None => WorkspaceSessionAction::Unknown,
            };
            (session.id.as_str(), action)
        })
        .collect();
    let panes = snapshot
        .panes
        .iter()
        .map(|pane| {
            let identity = pane.identity.as_ref().and_then(|saved| {
                identities
                    .iter()
                    .find(|record| record.entry.identity.id == saved.id)
            });
            let remembered = identity.and_then(|record| record.preferences.remembered.as_ref());
            let action = if pane.command.is_some() {
                WorkspacePaneAction::RelaunchCommand
            } else if pane.identity.is_none() {
                WorkspacePaneAction::Shell
            } else if identity.is_none() {
                WorkspacePaneAction::IdentityMissing
            } else {
                match remembered {
                    None => WorkspacePaneAction::NoRememberedSession,
                    Some(session) if session.stale_at_ms.is_some() => {
                        WorkspacePaneAction::StaleRequiresRetry
                    }
                    Some(_) => WorkspacePaneAction::Resumable,
                }
            };
            WorkspacePanePlan {
                pane: &pane.id,
                action,
                identity,
                remembered,
            }
        })
        .collect();
    WorkspaceRestorePlan { sessions, panes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        binding::{
            BindingEntry,
            session::{HarnessId, ProviderSessionId, RuntimeMode, SessionPreferences},
        },
        endpoint::ProcessIncarnation,
        identity::{Identity, Lifetime},
        workspace::*,
    };

    fn sample() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            captured_at_ms: 1,
            server: WorkspaceServer {
                socket: "/socket".into(),
                process: ProcessIncarnation::new(10, "start").unwrap(),
                id: None,
            },
            sessions: vec![WorkspaceSession {
                id: "$1".into(),
                name: "work".into(),
                windows: vec![WindowLink {
                    index: 0,
                    window: "@1".into(),
                    active: true,
                }],
            }],
            windows: vec![WorkspaceWindow {
                id: "@1".into(),
                name: "window".into(),
                layout: "layout".into(),
                visible_layout: "layout".into(),
                width: 80,
                height: 24,
                active_pane: "%1".into(),
            }],
            panes: vec![WorkspacePane {
                id: "%1".into(),
                window: "@1".into(),
                index: 0,
                left: 0,
                top: 0,
                width: 80,
                height: 24,
                cwd: "/work".into(),
                identity: None,
                command: None,
            }],
        }
    }

    #[test]
    fn pane_states_use_exact_current_uuid_and_preferences_never_old_annotations() {
        let mut snapshot = sample();
        assert_eq!(
            restore_plan(&snapshot, &[], Some(&[])).panes[0].action,
            WorkspacePaneAction::Shell
        );
        snapshot.panes[0].identity = Some(WorkspaceIdentity {
            id: "old-uuid".into(),
            name: "old-name".into(),
            lifetime: "saved".into(),
            binding: "old-binding".into(),
            harness: Some("claude".into()),
            session: Some("old-session".into()),
            mode: Some("independent".into()),
            channel: Some(false),
        });
        let mut records = [StoredIdentity {
            entry: BindingEntry {
                identity: Identity {
                    id: "replacement-uuid".into(),
                    name: "old-name".into(),
                    canonical_name: "old-name".into(),
                    lifetime: Lifetime::Saved,
                    created_at: "date".into(),
                    updated_at: "date".into(),
                },
                binding: None,
            },
            preferences: SessionPreferences::default(),
        }];
        assert_eq!(
            restore_plan(&snapshot, &records, Some(&[])).panes[0].action,
            WorkspacePaneAction::IdentityMissing
        );
        records[0].entry.identity.id = "old-uuid".into();
        records[0].entry.identity.name = "renamed".into();
        assert_eq!(
            restore_plan(&snapshot, &records, Some(&[])).panes[0].action,
            WorkspacePaneAction::NoRememberedSession
        );
        records[0].preferences.remembered = Some(RememberedSession {
            harness: HarnessId::new("codex").unwrap(),
            mode: RuntimeMode::new("independent").unwrap(),
            provider_session: ProviderSessionId::new("new-session").unwrap(),
            state: None,
            stale_at_ms: None,
            resume_pending_at_ms: Some(2),
        });
        let plan = restore_plan(&snapshot, &records, Some(&[]));
        assert_eq!(
            plan.panes[0].action,
            WorkspacePaneAction::Resumable,
            "pending is an informational crash mark"
        );
        assert_eq!(
            plan.panes[0].identity.unwrap().entry.identity.name,
            "renamed"
        );
        assert_eq!(
            plan.panes[0].remembered.unwrap().provider_session.as_str(),
            "new-session"
        );
        records[0]
            .preferences
            .remembered
            .as_mut()
            .unwrap()
            .stale_at_ms = Some(3);
        assert_eq!(
            restore_plan(&snapshot, &records, Some(&[])).panes[0].action,
            WorkspacePaneAction::StaleRequiresRetry
        );
        snapshot.panes[0].command = Some(ExternalCommand {
            argv: vec![
                "tmt".into(),
                "example".into(),
                "ui".into(),
                "--tabs=a,b".into(),
            ],
            owner: ProcessIncarnation::new(20, "owner").unwrap(),
        });
        assert_eq!(
            restore_plan(&snapshot, &records, Some(&[])).panes[0].action,
            WorkspacePaneAction::RelaunchCommand
        );
    }

    #[test]
    fn session_skip_requires_complete_names_and_preserves_linked_window_structure() {
        let mut snapshot = sample();
        snapshot.sessions.push(WorkspaceSession {
            id: "$2".into(),
            name: "other".into(),
            windows: snapshot.sessions[0].windows.clone(),
        });
        let before = snapshot.clone();
        let plan = restore_plan(&snapshot, &[], Some(&["work".into()]));
        assert_eq!(
            plan.sessions,
            [
                ("$1", WorkspaceSessionAction::SkipExisting),
                ("$2", WorkspaceSessionAction::Create)
            ]
        );
        assert!(
            restore_plan(&snapshot, &[], None)
                .sessions
                .iter()
                .all(|(_, action)| *action == WorkspaceSessionAction::Unknown)
        );
        assert_eq!(snapshot, before);
    }
}
