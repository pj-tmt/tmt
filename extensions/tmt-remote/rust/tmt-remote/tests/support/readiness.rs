//! The fixture evidence source for layered Firestore readiness: it produces every
//! prerequisite state without an account, a network or any provider.
use std::sync::Arc;
use tmt_remote::readiness::{
    FirestoreEvidence, FirestoreEvidenceSource, FirestoreReason as R, FirestoreTier, Observed,
};

pub struct FixtureEvidence(pub Option<FirestoreEvidence>);
impl FixtureEvidence {
    pub fn shared(evidence: FirestoreEvidence) -> Arc<dyn FirestoreEvidenceSource> {
        Arc::new(Self(Some(evidence)))
    }
}
impl FirestoreEvidenceSource for FixtureEvidence {
    fn evidence(&self) -> Option<FirestoreEvidence> {
        self.0
    }
}

/// Everything recorded as working, on a paid plan.
pub const ALL_ENABLED: FirestoreEvidence = FirestoreEvidence {
    project: Observed::Enabled,
    sign_in: Observed::Enabled,
    rules: Observed::Enabled,
    quota: Observed::Enabled,
    tier: FirestoreTier::Paid,
};
/// One scenario per prerequisite state: the control with a single item changed.
pub fn scenarios() -> Vec<(&'static str, FirestoreEvidence)> {
    let mut all = vec![("all enabled, paid", ALL_ENABLED)];
    let with = |f: fn(&mut FirestoreEvidence)| {
        let mut evidence = ALL_ENABLED;
        f(&mut evidence);
        evidence
    };
    all.push(("free tier", with(|e| e.tier = FirestoreTier::Free)));
    all.push(("tier unknown", with(|e| e.tier = FirestoreTier::Unknown)));
    all.push((
        "project not configured",
        with(|e| e.project = Observed::Off(R::NotConfigured)),
    ));
    all.push((
        "project access lost",
        with(|e| e.project = Observed::Off(R::AccessLost)),
    ));
    all.push(("project unknown", with(|e| e.project = Observed::Unknown)));
    all.push((
        "sign-in provider disabled",
        with(|e| e.sign_in = Observed::Off(R::ProviderDisabled)),
    ));
    all.push((
        "sign-in permission missing",
        with(|e| e.sign_in = Observed::Off(R::PermissionMissing)),
    ));
    all.push(("sign-in unknown", with(|e| e.sign_in = Observed::Unknown)));
    all.push((
        "rules not deployed",
        with(|e| e.rules = Observed::Off(R::NotDeployed)),
    ));
    all.push((
        "rules out of date",
        with(|e| e.rules = Observed::Off(R::OutOfDate)),
    ));
    all.push((
        "rules partial",
        with(|e| e.rules = Observed::Off(R::Partial)),
    ));
    all.push(("rules unknown", with(|e| e.rules = Observed::Unknown)));
    all.push((
        "quota headroom low",
        with(|e| e.quota = Observed::Off(R::HeadroomLow)),
    ));
    all.push((
        "quota exhausted",
        with(|e| e.quota = Observed::Off(R::Exhausted)),
    ));
    all.push(("quota unknown", with(|e| e.quota = Observed::Unknown)));
    all
}
