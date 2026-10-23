//! A restore preview uses current durable preferences, without authorizing launch.

use super::{StoredIdentity, WorkspaceSnapshot};
use crate::binding::session::RememberedSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSessionAction {
    Create,
    /// A live session already has the name. The restore decides from the live session itself:
    /// it adds the saved windows to the server's fresh default session, restores beside any
    /// other session in use, and skips a session this snapshot's restore made earlier.
    NameInUse,
    Unknown,
}

impl WorkspaceSessionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::NameInUse => "name_in_use",
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
    /// A tmux 3.8 floating pane: restore recreates the tiled layout around it and
    /// reports it skipped. Its annotations stay in the plan for the report.
    Floating,
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
            Self::Floating => "floating",
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
                Some(names) if names.contains(&session.name) => WorkspaceSessionAction::NameInUse,
                Some(_) => WorkspaceSessionAction::Create,
                None => WorkspaceSessionAction::Unknown,
            };
            (session.id.as_str(), action)
        })
        .collect();
    let floating = super::layout::floating_pane_ids(snapshot);
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
            let action = if floating.contains(&pane.id) {
                WorkspacePaneAction::Floating
            } else if pane.command.is_some() {
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
    fn a_floating_pane_is_planned_as_left_out_and_keeps_its_annotations_for_the_report() {
        let mut snapshot = sample();
        // %2 floats above the tiled %1 (the producer's shape: a `z` cell under the root).
        let layout = r#"{"V":2,"L":{"t":"v","w":80,"h":24,"x":0,"y":0,"c":[{"t":"p","w":80,"h":24,"x":0,"y":0,"a":true,"i":0,"I":"%1"},{"t":"p","w":20,"h":5,"x":2,"y":2,"i":1,"z":0,"I":"%2"}]}}"#;
        snapshot.windows[0].layout = layout.into();
        snapshot.windows[0].visible_layout = layout.into();
        snapshot.panes.push(WorkspacePane {
            id: "%2".into(),
            index: 1,
            identity: Some(WorkspaceIdentity {
                id: "float-uuid".into(),
                name: "float".into(),
                lifetime: "saved".into(),
                binding: "old-binding".into(),
                harness: None,
                session: None,
                mode: None,
                channel: None,
            }),
            ..snapshot.panes[0].clone()
        });
        let plan = restore_plan(&snapshot, &[], Some(&[]));
        let actions: Vec<_> = plan
            .panes
            .iter()
            .map(|pane| (pane.pane, pane.action))
            .collect();
        assert_eq!(
            actions,
            [
                ("%1", WorkspacePaneAction::Shell),
                ("%2", WorkspacePaneAction::Floating)
            ]
        );
        assert_eq!(WorkspacePaneAction::Floating.as_str(), "floating");
        // A layout that does not parse names no float; restore refuses it on its own.
        snapshot.windows[0].layout = "layout".into();
        assert_eq!(
            restore_plan(&snapshot, &[], Some(&[])).panes[1].action,
            WorkspacePaneAction::IdentityMissing
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
                ("$1", WorkspaceSessionAction::NameInUse),
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
