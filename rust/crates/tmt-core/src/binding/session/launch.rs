//! Bounded observations of a managed launch, never admission or presence authority.
use super::{DriverState, HarnessId, ProviderSessionId, RuntimeMode, SessionTransition};
use crate::endpoint::ProcessIncarnation;

pub const RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;
pub const MAXIMUM_PER_BINDING: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStart {
    pub process: ProcessIncarnation,
    pub transition: SessionTransition,
    pub session: ProviderSessionId,
    pub mode: RuntimeMode,
    pub state: Option<DriverState>,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningStep {
    Input,
    Decode,
    DriverAdmission,
    HostDiscovery,
    PaneSnapshot,
    ProviderVerification,
    Storage,
    Context,
}
impl SessionLearningStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Decode => "decode",
            Self::DriverAdmission => "driver_admission",
            Self::HostDiscovery => "host_discovery",
            Self::PaneSnapshot => "pane_snapshot",
            Self::ProviderVerification => "provider_verification",
            Self::Storage => "storage",
            Self::Context => "context",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "input" => Self::Input,
            "decode" => Self::Decode,
            "driver_admission" => Self::DriverAdmission,
            "host_discovery" => Self::HostDiscovery,
            "pane_snapshot" => Self::PaneSnapshot,
            "provider_verification" => Self::ProviderVerification,
            "storage" => Self::Storage,
            "context" => Self::Context,
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningReason {
    InProgress,
    NotAdmitted,
    WorkerError,
    BudgetTimeout,
    SpawnFailed,
    LaunchEnded,
    Conflict,
}
impl SessionLearningReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::NotAdmitted => "not_admitted",
            Self::WorkerError => "worker_error",
            Self::BudgetTimeout => "budget_timeout",
            Self::SpawnFailed => "spawn_failed",
            Self::LaunchEnded => "launch_ended",
            Self::Conflict => "conflict",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "in_progress" => Self::InProgress,
            "not_admitted" => Self::NotAdmitted,
            "worker_error" => Self::WorkerError,
            "budget_timeout" => Self::BudgetTimeout,
            "spawn_failed" => Self::SpawnFailed,
            "launch_ended" => Self::LaunchEnded,
            "conflict" => Self::Conflict,
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLearningDiagnostic {
    pub step: SessionLearningStep,
    pub reason: SessionLearningReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStatus {
    Intent,
    Pending,
    Admitted,
    Settled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchObservation {
    pub binding_id: String,
    pub preferences_fingerprint: String,
    pub owner: ProcessIncarnation,
    pub driver: HarnessId,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub pending: Option<PendingStart>,
    pub diagnostic: Option<SessionLearningDiagnostic>,
    pub status: LaunchStatus,
}
impl LaunchObservation {
    /// A match only selects evidence; the caller must still use the ordinary
    /// live child/owner verification and binding/session admission CAS.
    pub fn pending_for(
        &self,
        owner: &ProcessIncarnation,
        child: &ProcessIncarnation,
        driver: &HarnessId,
        now_ms: u64,
    ) -> Option<&PendingStart> {
        if matches!(self.status, LaunchStatus::Settled | LaunchStatus::Admitted)
            || &self.owner != owner
            || &self.driver != driver
            || now_ms < self.created_at_ms
            || now_ms >= self.expires_at_ms
        {
            return None;
        }
        self.pending
            .as_ref()
            .filter(|pending| &pending.process == child)
    }
}

/// Recovery is presentation only; this command is never executed automatically.
pub fn session_learning_recovery(
    identity: &crate::identity::Identity,
    driver: &HarnessId,
    preferences: &super::SessionPreferences,
) -> String {
    if preferences
        .remembered
        .as_ref()
        .is_some_and(|remembered| &remembered.harness == driver)
    {
        return format!(
            "Exit the agent in its pane, then run: tmt resume -- {}",
            identity.id
        );
    }
    let command = crate::driver::descriptor::named(driver.as_str())
        .and_then(|driver| driver.executables.first())
        .map(|executable| {
            format!(
                "Exit the agent in its pane, then run: tmt run -- {} {}",
                crate::names::command_word(&identity.name),
                crate::names::command_word(executable)
            )
        });
    command
        .filter(|command| command.len() <= 256)
        .unwrap_or_else(|| {
            "Exit the agent in its pane, then start it again with tmt run in that pane.".into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_never_suggests_resume_without_coordinates_or_truncates_a_command() {
        let mut identity = crate::identity::Identity {
            id: "123e4567-e89b-42d3-a456-426614174000".into(),
            name: "Alpha".into(),
            canonical_name: "alpha".into(),
            lifetime: crate::identity::Lifetime::Saved,
            created_at: "created".into(),
            updated_at: "updated".into(),
        };
        let driver = HarnessId::new("codex").unwrap();
        let mut preferences = super::super::SessionPreferences::default();
        assert!(
            session_learning_recovery(&identity, &driver, &preferences)
                .ends_with("tmt run -- Alpha codex")
        );
        identity.name = "🧑".repeat(64);
        let fresh = session_learning_recovery(&identity, &driver, &preferences);
        assert!(fresh.len() <= 256 && !fresh.contains('🧑'));
        preferences.remember(
            driver.clone(),
            RuntimeMode::new("default").unwrap(),
            ProviderSessionId::new("known").unwrap(),
        );
        assert!(
            session_learning_recovery(&identity, &driver, &preferences)
                .ends_with(&format!("tmt resume -- {}", identity.id))
        );
        assert!(
            !session_learning_recovery(&identity, &HarnessId::new("claude").unwrap(), &preferences)
                .contains("tmt resume")
        );
    }

    #[test]
    fn a_pending_start_selects_only_its_exact_live_launch_coordinates() {
        let owner = ProcessIncarnation::new(10, "owner").unwrap();
        let child = ProcessIncarnation::new(20, "child").unwrap();
        let driver = HarnessId::new("claude").unwrap();
        let mut observation = LaunchObservation {
            binding_id: "binding".into(),
            preferences_fingerprint: "test-fingerprint".into(),
            owner: owner.clone(),
            driver: driver.clone(),
            created_at_ms: 100,
            expires_at_ms: 200,
            status: LaunchStatus::Pending,
            diagnostic: None,
            pending: Some(PendingStart {
                process: child.clone(),
                transition: SessionTransition::Started,
                session: ProviderSessionId::new("session").unwrap(),
                mode: RuntimeMode::new("default").unwrap(),
                state: None,
                observed_at_ms: 110,
            }),
        };
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_some()
        );
        for time in [99, 200, u64::MAX] {
            assert!(
                observation
                    .pending_for(&owner, &child, &driver, time)
                    .is_none()
            );
        }
        assert!(
            observation
                .pending_for(
                    &ProcessIncarnation::new(10, "reused").unwrap(),
                    &child,
                    &driver,
                    150
                )
                .is_none()
        );
        assert!(
            observation
                .pending_for(
                    &owner,
                    &ProcessIncarnation::new(20, "reused").unwrap(),
                    &driver,
                    150
                )
                .is_none()
        );
        assert!(
            observation
                .pending_for(&owner, &child, &HarnessId::new("codex").unwrap(), 150)
                .is_none()
        );
        observation.status = LaunchStatus::Settled;
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_none()
        );
        observation.status = LaunchStatus::Admitted;
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_none()
        );
    }
}
