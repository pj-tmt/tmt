//! A fake provider for the deploy run (#2164): it holds the project's objects, logs every
//! call and can fail a step before or after its effect. It has no delete operation
//! because the port has none; unrelated objects are checked for equality instead.
use std::collections::{BTreeMap, BTreeSet};
use tmt_remote::deploy_run::{
    DeployApplied, DeployObserved, DeployOwnerAction, DeployPlan, DeployPort, DeployProviderError,
    DeployRecord, DeploySink, DeploySinkError, DeployStep, StepKind, classify_live_rules,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// The call fails and nothing changed.
    BeforeEffect,
    /// The effect lands and the answer is lost: the outcome is unknown.
    AfterEffectUnknown,
}
#[derive(Default)]
pub struct Fake {
    pub account: String,
    /// Objects present on the project, by step id.
    pub present: BTreeSet<String>,
    /// Present objects Remote did not make.
    pub adopted: BTreeSet<String>,
    pub rules: Option<Vec<u8>>,
    pub database_mismatch: bool,
    /// Indexes the provider accepted that are still building.
    pub building: BTreeSet<String>,
    /// Replace the Rules with this release right after the named step lands.
    pub drift_after: Option<(String, Vec<u8>)>,
    /// An outstanding owner step, by step id.
    pub owner_pending: BTreeMap<String, DeployOwnerAction>,
    /// One-shot scripted failures by step id.
    pub faults: BTreeMap<String, When>,
    /// One-shot failures of the read-only look at a step.
    pub observe_faults: BTreeMap<String, DeployProviderError>,
    /// Objects the deploy must never touch.
    pub unrelated: BTreeMap<String, String>,
    /// Times each step's effect landed.
    pub effects: BTreeMap<String, u32>,
    /// Every observe/apply call as `observe:<id>` / `apply:<id>`.
    pub calls: Vec<String>,
}
impl Fake {
    pub fn new(account: &str) -> Self {
        Self {
            account: account.to_owned(),
            ..Self::default()
        }
    }
    pub fn effects_of(&self, id: &str) -> u32 {
        self.effects.get(id).copied().unwrap_or(0)
    }
    fn land(&mut self, plan: &DeployPlan, step: &DeployStep) {
        *self.effects.entry(step.id.clone()).or_default() += 1;
        if step.kind == StepKind::Rules {
            self.rules = Some(plan.deployed_rules().to_vec());
        }
        self.present.insert(step.id.clone());
    }
}
impl DeployPort for Fake {
    fn account(&mut self) -> Result<String, DeployProviderError> {
        Ok(self.account.clone())
    }
    fn observe(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError> {
        self.calls.push(format!("observe:{}", step.id));
        if let Some(error) = self.observe_faults.remove(&step.id) {
            return Err(error);
        }
        if let Some(action) = self.owner_pending.get(&step.id) {
            return Ok(DeployObserved::OwnerAction(*action));
        }
        Ok(match &step.kind {
            StepKind::Rules => DeployObserved::Rules(classify_live_rules(
                self.rules.as_deref(),
                plan.deployment_id(),
                plan.deployed_rules(),
            )),
            StepKind::Verify => {
                let rules_current = self.rules.as_deref() == Some(plan.deployed_rules());
                let all = plan
                    .steps()
                    .iter()
                    .filter(|s| s.kind != StepKind::Verify)
                    .all(|s| self.present.contains(&s.id));
                if !self.building.is_empty() {
                    DeployObserved::Building
                } else if rules_current && all {
                    DeployObserved::Satisfied
                } else {
                    DeployObserved::Absent
                }
            }
            StepKind::Database if self.database_mismatch => DeployObserved::Mismatch,
            _ if self.adopted.contains(&step.id) => DeployObserved::Adopted,
            _ if self.present.contains(&step.id) && self.building.contains(&step.id) => {
                DeployObserved::Building
            }
            _ if self.present.contains(&step.id) => DeployObserved::Satisfied,
            _ => DeployObserved::Absent,
        })
    }
    fn apply(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployApplied, DeployProviderError> {
        self.calls.push(format!("apply:{}", step.id));
        match self.faults.remove(&step.id) {
            Some(When::BeforeEffect) => {
                return Err(DeployProviderError::Rejected(
                    tmt_remote::deploy_run::DeployFault::ProviderRejected,
                ));
            }
            Some(When::AfterEffectUnknown) => {
                self.land(plan, step);
                return Err(DeployProviderError::Unknown);
            }
            None => {}
        }
        self.land(plan, step);
        if let Some((after, release)) = &self.drift_after
            && *after == step.id
        {
            self.rules = Some(release.clone());
        }
        Ok(DeployApplied::Done)
    }
}

/// Keeps every saved record; can fail from the n-th save on, as a crash would.
#[derive(Default)]
pub struct Mem {
    pub saved: Vec<DeployRecord>,
    pub fail_from: Option<usize>,
}
impl DeploySink for Mem {
    fn save(&mut self, record: &DeployRecord) -> Result<(), DeploySinkError> {
        if self.fail_from.is_some_and(|n| self.saved.len() >= n) {
            return Err(DeploySinkError);
        }
        self.saved.push(record.clone());
        Ok(())
    }
}
