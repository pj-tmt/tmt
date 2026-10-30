//! Built-in host recovery: which members were running, and where. A restart loses
//! only this; identities, rooms, presets and remembered sessions stay durable in Core.

use std::collections::HashSet;

use super::{MAX_PANES, StoredIdentity, WorkspaceIdentity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinSnapshot {
    pub captured_at_ms: u64,
    pub members: Vec<BuiltinMember>,
}

/// A member whose binding, supervisor and program incarnation agreed at capture.
/// The annotation is display input; restore rechecks every durable coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinMember {
    /// The program's live directory, `None` when it could not be read.
    pub cwd: Option<String>,
    pub identity: WorkspaceIdentity,
}

impl BuiltinSnapshot {
    pub fn agent_count(&self) -> usize {
        self.members
            .iter()
            .map(|member| &member.identity.id)
            .collect::<HashSet<_>>()
            .len()
    }

    /// Whether this capture still holds every identity of `baseline`. Built-in members end one
    /// by one at logout, so any unexplained loss is a shrink and keeps the full baseline. A
    /// member stopped on purpose leaves the baseline first (see [`Self::without`]).
    pub fn covers(&self, baseline: &Self) -> bool {
        let held = self
            .members
            .iter()
            .map(|member| &member.identity.id)
            .collect::<HashSet<_>>();
        baseline
            .members
            .iter()
            .all(|member| held.contains(&member.identity.id))
    }

    /// This recovery input without one identity, for a member stopped or removed on purpose.
    /// `None` when the identity is not here, so nothing needs publishing.
    pub fn without(&self, identity: &str, captured_at_ms: u64) -> Option<Self> {
        let members = self
            .members
            .iter()
            .filter(|member| member.identity.id != identity)
            .cloned()
            .collect::<Vec<_>>();
        (members.len() != self.members.len()).then_some(Self {
            captured_at_ms,
            members,
        })
    }

    /// One entry per binding, within the shared pane bound.
    pub fn is_consistent(&self) -> bool {
        self.members.len() <= MAX_PANES
            && self
                .members
                .iter()
                .map(|member| &member.identity.binding)
                .collect::<HashSet<_>>()
                .len()
                == self.members.len()
    }
}

/// What a restore would do with one recorded member. A preview grants no authority: the
/// restore rechecks every coordinate before it starts anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinMemberAction {
    /// Its UUID is retired or gone; a same-name identity is never substituted.
    IdentityMissing,
    /// Already running on the built-in host; restore leaves it alone.
    Live,
    /// The recorded folder no longer exists.
    FolderMissing,
    NoRememberedSession,
    StaleRequiresRetry,
    Resumable,
}

impl BuiltinMemberAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IdentityMissing => "identity_missing",
            Self::Live => "live",
            Self::FolderMissing => "folder_missing",
            Self::NoRememberedSession => "no_remembered_session",
            Self::StaleRequiresRetry => "stale_requires_retry",
            Self::Resumable => "resumable",
        }
    }
}

pub struct BuiltinMemberPlan<'a> {
    pub member: &'a BuiltinMember,
    pub action: BuiltinMemberAction,
    pub identity: Option<&'a StoredIdentity>,
}

/// Current durable identities and preferences decide, never the recorded annotation. `live`
/// holds the UUIDs running on the built-in host now; `folder_exists` reads the filesystem.
pub fn member_plan<'a>(
    snapshot: &'a BuiltinSnapshot,
    identities: &'a [StoredIdentity],
    live: &HashSet<String>,
    folder_exists: impl Fn(&str) -> bool,
) -> Vec<BuiltinMemberPlan<'a>> {
    snapshot
        .members
        .iter()
        .map(|member| {
            let identity = identities
                .iter()
                .find(|record| record.entry.identity.id == member.identity.id);
            let action = match identity {
                None => BuiltinMemberAction::IdentityMissing,
                Some(_) if live.contains(&member.identity.id) => BuiltinMemberAction::Live,
                Some(_) if member.cwd.as_deref().is_some_and(|cwd| !folder_exists(cwd)) => {
                    BuiltinMemberAction::FolderMissing
                }
                Some(record) => match &record.preferences.remembered {
                    None => BuiltinMemberAction::NoRememberedSession,
                    Some(session) if session.stale_at_ms.is_some() => {
                        BuiltinMemberAction::StaleRequiresRetry
                    }
                    Some(_) => BuiltinMemberAction::Resumable,
                },
            };
            BuiltinMemberPlan {
                member,
                action,
                identity,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, binding: &str) -> BuiltinMember {
        BuiltinMember {
            cwd: Some("/work".into()),
            identity: WorkspaceIdentity {
                id: id.into(),
                name: id.into(),
                lifetime: "saved".into(),
                binding: binding.into(),
                harness: Some("codex".into()),
                session: None,
                mode: None,
                channel: None,
            },
        }
    }

    fn snapshot(at: u64, ids: &[&str]) -> BuiltinSnapshot {
        BuiltinSnapshot {
            captured_at_ms: at,
            members: ids
                .iter()
                .map(|id| member(id, &format!("binding-{id}")))
                .collect(),
        }
    }

    #[test]
    fn coverage_is_by_identity_and_a_stopped_member_leaves_the_baseline() {
        let baseline = snapshot(1, &["a", "b", "c", "d"]);
        // Logout: members end one by one; none of the partial sets covers the baseline.
        for ids in [&["a", "b", "c"][..], &["a", "b"], &["a"], &[]] {
            assert!(!snapshot(2, ids).covers(&baseline), "{ids:?}");
        }
        // Same count, different identity: still a loss.
        assert!(!snapshot(2, &["a", "b", "c", "e"]).covers(&baseline));
        // An explicit stop of d removes it first; the rest then covers, and one more is fine.
        let stopped = baseline.without("d", 3).unwrap();
        assert_eq!(stopped.members.len(), 3);
        assert!(snapshot(4, &["a", "b", "c"]).covers(&stopped));
        assert!(snapshot(4, &["a", "b", "c", "e"]).covers(&stopped));
        assert!(stopped.without("d", 5).is_none());
    }

    #[test]
    fn the_preview_uses_current_durable_state_never_the_recorded_annotation() {
        use crate::{
            binding::{
                BindingEntry,
                session::{
                    HarnessId, ProviderSessionId, RememberedSession, RuntimeMode,
                    SessionPreferences,
                },
            },
            identity::{Identity, Lifetime},
        };
        let snapshot = snapshot(1, &["a", "b", "c", "d", "e"]);
        let record = |id: &str, stale: Option<u64>, remembered: bool| StoredIdentity {
            entry: BindingEntry {
                identity: Identity {
                    id: id.into(),
                    name: id.into(),
                    canonical_name: id.into(),
                    lifetime: Lifetime::Saved,
                    created_at: "date".into(),
                    updated_at: "date".into(),
                },
                binding: None,
            },
            preferences: SessionPreferences {
                remembered: remembered.then(|| RememberedSession {
                    harness: HarnessId::new("codex").unwrap(),
                    mode: RuntimeMode::new("independent").unwrap(),
                    provider_session: ProviderSessionId::new("session").unwrap(),
                    state: None,
                    stale_at_ms: stale,
                    resume_pending_at_ms: None,
                }),
                ..SessionPreferences::default()
            },
        };
        // "a" is gone; "b" runs; "c" lost its folder; "d" is stale; "e" resumes.
        let records = [
            record("b", None, true),
            record("c", None, true),
            record("d", Some(2), true),
            record("e", None, true),
        ];
        let live = HashSet::from(["b".to_owned()]);
        let mut gone = snapshot.clone();
        gone.members[2].cwd = Some("/gone".into());
        let actions: Vec<_> = member_plan(&gone, &records, &live, |cwd| cwd != "/gone")
            .iter()
            .map(|plan| plan.action)
            .collect();
        assert_eq!(
            actions,
            [
                BuiltinMemberAction::IdentityMissing,
                BuiltinMemberAction::Live,
                BuiltinMemberAction::FolderMissing,
                BuiltinMemberAction::StaleRequiresRetry,
                BuiltinMemberAction::Resumable,
            ]
        );
        let unremembered = [record("e", None, false)];
        assert_eq!(
            member_plan(&snapshot, &unremembered, &HashSet::new(), |_| true)[4].action,
            BuiltinMemberAction::NoRememberedSession
        );
    }

    #[test]
    fn counts_distinct_identities_and_refuses_a_repeated_binding() {
        let mut snapshot = BuiltinSnapshot {
            captured_at_ms: 1,
            members: vec![member("a", "b1"), member("b", "b2")],
        };
        assert_eq!(snapshot.agent_count(), 2);
        assert!(snapshot.is_consistent());
        snapshot.members.push(member("c", "b1"));
        assert!(!snapshot.is_consistent());
    }
}
