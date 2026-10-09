//! The authorized deploy run for the Firestore sharing layer (contract: Backends and
//! deploy, "authorized deploy"). Pure: the caller reads the live Rules and the account
//! through a `DeployPort`, persists the record through a `DeploySink` and supplies the
//! clock; nothing here talks to a provider, reads a file or holds a credential, so no
//! credential choice can leak into it.
//!
//! A deploy is a fixed, ordered list of steps (database, sign-in providers, indexes,
//! Rules, verify). The owner authorizes the digest of the whole envelope; any change
//! needs a new authorization. The Rules release is the one step that switches what the
//! project serves, so it runs after every additive step. A usable binding exists only
//! when every step finished and the final read-back passed.
use crate::{
    canonical,
    deploy_plan::{Plan, sha256_hex},
};
use serde::{Deserialize, Serialize};

/// The only database a free project has, and the only edition the free plan prices.
const DATABASE: &str = "(default)";
const EDITION: &str = "standard";
/// A Remote-owned first line of the deployed Rules, naming the deployment and the digest
/// of the composed body that follows it.
const MARKER_PREFIX: &str = "// tmt-remote deployment ";
/// The shortest accepted prefix of a plan digest when authorizing.
const MIN_DIGEST_PREFIX: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignInProvider {
    Anonymous,
    Google,
}
impl SignInProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Google => "google.com",
        }
    }
}

/// What a deploy refuses before any provider effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployRefusal {
    InvalidProject,
    InvalidLocation,
    InvalidDeployment,
    NoSignIn,
    /// The authorization does not name this plan: something it covers has changed.
    AuthorizationStale,
    AccountChanged,
    AccountUnreadable,
    /// The plan replaces Rules Remote does not own and the digest was not typed.
    ReplaceRulesRequired,
}
impl DeployRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidProject => "REMOTE_DEPLOY_PROJECT_INVALID",
            Self::InvalidLocation => "REMOTE_DEPLOY_LOCATION_INVALID",
            Self::InvalidDeployment => "REMOTE_DEPLOY_DEPLOYMENT_INVALID",
            Self::NoSignIn => "REMOTE_DEPLOY_SIGN_IN_MISSING",
            Self::AuthorizationStale => "REMOTE_DEPLOY_AUTHORIZATION_STALE",
            Self::AccountChanged => "REMOTE_DEPLOY_ACCOUNT_CHANGED",
            Self::AccountUnreadable => "REMOTE_DEPLOY_ACCOUNT_UNREADABLE",
            Self::ReplaceRulesRequired => "REMOTE_DEPLOY_REPLACE_RULES_REQUIRED",
        }
    }
}

/// What the live project holds for the Rules, from a read-only observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveRules {
    Absent,
    /// Exactly the bytes this deploy would publish.
    Current,
    /// An earlier release of this same deployment.
    Own,
    /// Anything else, named by the digest of its bytes.
    Foreign(String),
}

#[derive(Clone, Copy, Debug)]
pub struct DeployInput<'a> {
    pub account: &'a str,
    pub project: &'a str,
    pub deployment_id: &'a str,
    pub location: &'a str,
    pub sign_in: &'a [SignInProvider],
    /// The composed Rules, without the Remote marker line.
    pub rules_body: &'a [u8],
    /// The live Rules bytes at plan time (`None` when the project has no release).
    pub live_rules: Option<&'a [u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    Database,
    SignIn(SignInProvider),
    Index {
        path: String,
        field: String,
        direction: &'static str,
    },
    Rules,
    /// Reads back every other step; the last gate before a binding.
    Verify,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployStep {
    /// Stable across runs: retry matches provider objects by this identity.
    pub id: String,
    pub kind: StepKind,
}

/// Everything the owner reads before authorizing.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployView {
    pub version: u8,
    pub backend: &'static str,
    pub profile: &'static str,
    pub account: String,
    pub project: String,
    pub deployment_id: String,
    pub database: DatabaseView,
    pub sign_in: Vec<&'static str>,
    pub rules: RulesView,
    pub index_configs: usize,
    pub steps: Vec<String>,
    /// Digest of the extension plan (declarations, resources, limits, indexes, TTL).
    pub extension_plan_digest: String,
    pub destructive: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseView {
    pub name: &'static str,
    pub edition: &'static str,
    pub location: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesView {
    /// Digest of the composed body (without the marker line).
    pub digest: String,
    pub replaces: &'static str,
    pub replaced_digest: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DeployPlan {
    view: DeployView,
    bytes: Vec<u8>,
    digest: String,
    steps: Vec<DeployStep>,
    rules: Vec<u8>,
    live: LiveRules,
}
impl DeployPlan {
    pub fn view(&self) -> &DeployView {
        &self.view
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn steps(&self) -> &[DeployStep] {
        &self.steps
    }
    /// The exact bytes the Rules step publishes: the marker line, then the composed body.
    pub fn deployed_rules(&self) -> &[u8] {
        &self.rules
    }
    pub fn deployment_id(&self) -> &str {
        &self.view.deployment_id
    }
    pub fn account(&self) -> &str {
        &self.view.account
    }
    pub fn live_rules(&self) -> &LiveRules {
        &self.live
    }
}

/// The digest of a Rules body: what the plan view shows and the marker line names.
pub fn rules_digest(body: &[u8]) -> String {
    sha256_hex(body)
}

fn marker_line(deployment_id: &str, body: &[u8]) -> String {
    format!(
        "{MARKER_PREFIX}{deployment_id} rules {}\n",
        rules_digest(body)
    )
}
/// The marker line followed by the body: the single definition of the deployed bytes.
fn deployed(deployment_id: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = marker_line(deployment_id, body).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Classify live Rules against the bytes this deploy would publish. The one definition
/// shared by plan time and every adapter, so both read the same ownership.
pub fn classify_live_rules(live: Option<&[u8]>, deployment_id: &str, publish: &[u8]) -> LiveRules {
    let Some(live) = live else {
        return LiveRules::Absent;
    };
    if live == publish {
        return LiveRules::Current;
    }
    let own_prefix = format!("{MARKER_PREFIX}{deployment_id} rules ");
    if live.starts_with(own_prefix.as_bytes()) {
        return LiveRules::Own;
    }
    LiveRules::Foreign(sha256_hex(live))
}

fn project_ok(value: &str) -> bool {
    // Firebase project ids: 6-30 of lowercase letters, digits, hyphens; starts with a letter.
    (6..=30).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn location_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Compose the envelope the owner authorizes. No provider effect happens here.
pub fn prepare(plan: &Plan, input: &DeployInput<'_>) -> Result<DeployPlan, DeployRefusal> {
    if !project_ok(input.project) {
        return Err(DeployRefusal::InvalidProject);
    }
    if !location_ok(input.location) {
        return Err(DeployRefusal::InvalidLocation);
    }
    if canonical::uuid(input.deployment_id).is_err() {
        return Err(DeployRefusal::InvalidDeployment);
    }
    let mut providers = input.sign_in.to_vec();
    providers.sort_by_key(|provider| provider.name());
    providers.dedup();
    if providers.is_empty() {
        return Err(DeployRefusal::NoSignIn);
    }
    let rules = deployed(input.deployment_id, input.rules_body);
    let live = classify_live_rules(input.live_rules, input.deployment_id, &rules);
    let mut steps = vec![step("database", StepKind::Database)];
    for provider in &providers {
        steps.push(step(
            &format!("sign-in:{}", provider.name()),
            StepKind::SignIn(*provider),
        ));
    }
    let mut index_configs = 0;
    for extension in &plan.view().extensions {
        for resource in &extension.resources {
            for index in &resource.indexes {
                index_configs += 1;
                steps.push(step(
                    &format!(
                        "index:{}#{}:{}",
                        resource.path, index.field, index.direction
                    ),
                    StepKind::Index {
                        path: resource.path.clone(),
                        field: index.field.clone(),
                        direction: index.direction,
                    },
                ));
            }
        }
    }
    steps.push(step("rules", StepKind::Rules));
    steps.push(step("verify", StepKind::Verify));
    let (replaces, replaced_digest, destructive) = match &live {
        LiveRules::Absent | LiveRules::Current => ("none", None, Vec::new()),
        LiveRules::Own => ("own", None, Vec::new()),
        LiveRules::Foreign(digest) => (
            "foreign",
            Some(digest.clone()),
            vec![format!("replaces-rules:{digest}")],
        ),
    };
    let view = DeployView {
        version: 1,
        backend: "firestore",
        profile: "sharing",
        account: input.account.to_owned(),
        project: input.project.to_owned(),
        deployment_id: input.deployment_id.to_owned(),
        database: DatabaseView {
            name: DATABASE,
            edition: EDITION,
            location: input.location.to_owned(),
        },
        sign_in: providers.iter().map(|provider| provider.name()).collect(),
        rules: RulesView {
            digest: rules_digest(input.rules_body),
            replaces,
            replaced_digest,
        },
        index_configs,
        steps: steps.iter().map(|step| step.id.clone()).collect(),
        extension_plan_digest: plan.digest().to_owned(),
        destructive,
    };
    let bytes = serde_json::to_vec(&view).expect("a deploy view serializes");
    let digest = sha256_hex(&bytes);
    Ok(DeployPlan {
        view,
        bytes,
        digest,
        steps,
        rules,
        live,
    })
}
impl DeployPlan {
    /// The canonical bytes the digest covers.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
fn step(id: &str, kind: StepKind) -> DeployStep {
    DeployStep {
        id: id.to_owned(),
        kind,
    }
}

/// The owner's authorization of one plan digest for the plan's account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    plan_digest: String,
    replace_rules: Option<String>,
}
/// Authorize a plan from the digest (or a prefix of at least 12 hex characters) the
/// owner typed, plus the digest of the foreign Rules when the plan replaces them.
pub fn authorize(
    plan: &DeployPlan,
    typed_digest: &str,
    replace_rules: Option<&str>,
) -> Result<Authorization, DeployRefusal> {
    if typed_digest.len() < MIN_DIGEST_PREFIX || !plan.digest.starts_with(typed_digest) {
        return Err(DeployRefusal::AuthorizationStale);
    }
    if let LiveRules::Foreign(digest) = &plan.live
        && replace_rules != Some(digest.as_str())
    {
        return Err(DeployRefusal::ReplaceRulesRequired);
    }
    Ok(Authorization {
        plan_digest: plan.digest.clone(),
        replace_rules: replace_rules.map(str::to_owned),
    })
}

/// A fixed reason a provider step did not finish. Never provider text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeployFault {
    PermissionDenied,
    ApiDisabled,
    QuotaExceeded,
    ProviderRejected,
    /// The database exists with another edition or location, or a second one was refused.
    DatabaseMismatch,
    /// The live Rules are not Remote's and the authorization did not name them.
    RulesForeign,
    /// The read-back did not match what was applied.
    VerifyFailed,
}
impl DeployFault {
    pub fn code(self) -> &'static str {
        match self {
            Self::PermissionDenied => "REMOTE_DEPLOY_PERMISSION_DENIED",
            Self::ApiDisabled => "REMOTE_DEPLOY_API_DISABLED",
            Self::QuotaExceeded => "REMOTE_DEPLOY_QUOTA_EXCEEDED",
            Self::ProviderRejected => "REMOTE_DEPLOY_PROVIDER_REJECTED",
            Self::DatabaseMismatch => "REMOTE_DEPLOY_DATABASE_MISMATCH",
            Self::RulesForeign => "REMOTE_DEPLOY_RULES_FOREIGN",
            Self::VerifyFailed => "REMOTE_DEPLOY_VERIFY_FAILED",
        }
    }
}
/// A console step only the owner can do. Fixed text, no provider wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeployOwnerAction {
    /// Authentication is not initialized for the project.
    InitializeAuth,
    /// The Google sign-in provider needs an OAuth client that only the console creates.
    EnableGoogleSignIn,
}
impl DeployOwnerAction {
    pub fn code(self) -> &'static str {
        match self {
            Self::InitializeAuth => "REMOTE_DEPLOY_OWNER_INITIALIZE_AUTH",
            Self::EnableGoogleSignIn => "REMOTE_DEPLOY_OWNER_ENABLE_GOOGLE_SIGN_IN",
        }
    }
    pub fn instruction(self) -> &'static str {
        match self {
            Self::InitializeAuth => {
                "In the Firebase console open Authentication and choose Get started, then run the deploy again."
            }
            Self::EnableGoogleSignIn => {
                "In the Firebase console open Authentication, Sign-in method, enable Google, then run the deploy again."
            }
        }
    }
}

/// What a read-only look at one step found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployObserved {
    Absent,
    /// Present and exactly what this deploy would make.
    Satisfied,
    /// Present, equivalent, and not made by this deployment: kept, never deleted.
    Adopted,
    /// The database exists with another edition or location.
    Mismatch,
    /// Accepted by the provider and still building.
    Building,
    Rules(LiveRules),
    /// An owner step is outstanding.
    OwnerAction(DeployOwnerAction),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployApplied {
    Done,
    OwnerAction(DeployOwnerAction),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployProviderError {
    Rejected(DeployFault),
    /// The outcome is unknown: the call may have taken effect.
    Unknown,
}

/// The provider seam. Adapters implement it; fixtures fake it.
pub trait DeployPort {
    /// The account the credential resolves to. Read-only.
    fn account(&mut self) -> Result<String, DeployProviderError>;
    fn observe(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError>;
    fn apply(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployApplied, DeployProviderError>;
}
/// Durable storage for the record. A failure stops the run; the last saved record rules.
pub trait DeploySink {
    fn save(&mut self, record: &DeployRecord) -> Result<(), DeploySinkError>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeploySinkError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "code", rename_all = "kebab-case")]
pub enum StepState {
    Pending,
    Done,
    Adopted,
    /// A call may have taken effect; the next run observes before doing anything.
    Unknown,
    Building,
    Failed(DeployFault),
    OwnerAction(DeployOwnerAction),
}
impl StepState {
    fn finished(&self) -> bool {
        matches!(self, Self::Done | Self::Adopted)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub id: String,
    #[serde(flatten)]
    pub state: StepState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Applying,
    Complete,
    Partial,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub plan_digest: String,
    pub account: String,
    pub authorized_at_ms: u64,
    pub state: RunState,
    /// Set before the Rules call is made: from then on the project may serve new Rules.
    pub rules_attempted: bool,
    pub steps: Vec<StepRecord>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployBinding {
    pub plan_digest: String,
    pub project: String,
    pub completed_at_ms: u64,
}
/// The durable deployment record. Slice 2 persists it; this module only transitions it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRecord {
    pub deployment_id: String,
    /// The last deploy that finished completely, while it still describes the project.
    pub binding: Option<DeployBinding>,
    pub run: Option<Run>,
}
impl DeployRecord {
    pub fn new(deployment_id: &str) -> Self {
        Self {
            deployment_id: deployment_id.to_owned(),
            binding: None,
            run: None,
        }
    }
    /// The binding routes may use. Once a run may have switched the Rules and has not
    /// completed, the project is partial and no binding is usable.
    pub fn usable_binding(&self) -> Option<&DeployBinding> {
        match &self.run {
            Some(run) if run.rules_attempted && run.state != RunState::Complete => None,
            _ => self.binding.as_ref(),
        }
    }
}

/// Why a run did not start or could not continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployError {
    /// Refused before any observe or apply call.
    Refused(DeployRefusal),
    /// The sink failed; the last saved record is the truth.
    Interrupted,
}

/// Run the plan to the end or to the first step that cannot finish, saving the record
/// before and after every provider call. Retry with the same authorization resumes.
pub fn run(
    plan: &DeployPlan,
    authorization: &Authorization,
    mut record: DeployRecord,
    port: &mut dyn DeployPort,
    sink: &mut dyn DeploySink,
    now_ms: u64,
) -> Result<DeployRecord, DeployError> {
    if authorization.plan_digest != plan.digest || record.deployment_id != plan.deployment_id() {
        return Err(DeployError::Refused(DeployRefusal::AuthorizationStale));
    }
    match port.account() {
        Ok(account) if account == plan.account() => {}
        Ok(_) => return Err(DeployError::Refused(DeployRefusal::AccountChanged)),
        Err(_) => return Err(DeployError::Refused(DeployRefusal::AccountUnreadable)),
    }
    begin(&mut record, plan, now_ms);
    save(sink, &record)?;
    // Finished steps are observed again too, so drift since the last run is noticed.
    for (index, step) in plan.steps.iter().enumerate() {
        if !advance(plan, authorization, step, index, &mut record, port, sink)? {
            return finish(record, sink, now_ms, plan);
        }
    }
    finish(record, sink, now_ms, plan)
}

fn run_of(record: &mut DeployRecord) -> &mut Run {
    record.run.as_mut().expect("a run was begun")
}
fn set(
    record: &mut DeployRecord,
    sink: &mut dyn DeploySink,
    index: usize,
    state: StepState,
) -> Result<(), DeployError> {
    run_of(record).steps[index].state = state;
    save(sink, record)
}
fn save(sink: &mut dyn DeploySink, record: &DeployRecord) -> Result<(), DeployError> {
    sink.save(record).map_err(|_| DeployError::Interrupted)
}

/// Start the run for this plan, or resume the saved one when it is the same plan.
fn begin(record: &mut DeployRecord, plan: &DeployPlan, now_ms: u64) {
    if record
        .run
        .as_ref()
        .is_some_and(|run| run.plan_digest == plan.digest)
    {
        let run = run_of(record);
        run.state = RunState::Applying;
        return;
    }
    // A different plan replaces the run. A partial earlier run that may have switched the
    // Rules also withdraws the old binding: it no longer describes the project.
    if record
        .run
        .as_ref()
        .is_some_and(|run| run.rules_attempted && run.state != RunState::Complete)
    {
        record.binding = None;
    }
    record.run = Some(Run {
        plan_digest: plan.digest.clone(),
        account: plan.account().to_owned(),
        authorized_at_ms: now_ms,
        state: RunState::Applying,
        rules_attempted: false,
        steps: plan
            .steps
            .iter()
            .map(|step| StepRecord {
                id: step.id.clone(),
                state: StepState::Pending,
            })
            .collect(),
    });
}

/// Observe, then apply when needed. Returns whether the run may go on to the next step.
fn advance(
    plan: &DeployPlan,
    authorization: &Authorization,
    step: &DeployStep,
    index: usize,
    record: &mut DeployRecord,
    port: &mut dyn DeployPort,
    sink: &mut dyn DeploySink,
) -> Result<bool, DeployError> {
    let observed = match port.observe(plan, step) {
        Ok(observed) => observed,
        Err(error) => {
            set(record, sink, index, from_error(error))?;
            return Ok(false);
        }
    };
    if step.kind == StepKind::Verify {
        // Only a full read-back passes; nothing is applied to make it pass.
        let state = match observed {
            DeployObserved::Satisfied => StepState::Done,
            DeployObserved::Building => StepState::Building,
            _ => StepState::Failed(DeployFault::VerifyFailed),
        };
        let finished = state.finished();
        set(record, sink, index, state)?;
        return Ok(finished);
    }
    match observed {
        DeployObserved::Satisfied => {
            set(record, sink, index, StepState::Done)?;
            return Ok(true);
        }
        DeployObserved::Adopted => {
            set(record, sink, index, StepState::Adopted)?;
            return Ok(true);
        }
        DeployObserved::Mismatch => {
            set(
                record,
                sink,
                index,
                StepState::Failed(DeployFault::DatabaseMismatch),
            )?;
            return Ok(false);
        }
        DeployObserved::Building => {
            set(record, sink, index, StepState::Building)?;
            return Ok(true);
        }
        DeployObserved::OwnerAction(action) => {
            set(record, sink, index, StepState::OwnerAction(action))?;
            return Ok(false);
        }
        DeployObserved::Rules(LiveRules::Current) => {
            set(record, sink, index, StepState::Done)?;
            return Ok(true);
        }
        DeployObserved::Rules(LiveRules::Foreign(digest))
            if authorization.replace_rules.as_deref() != Some(digest.as_str()) =>
        {
            set(
                record,
                sink,
                index,
                StepState::Failed(DeployFault::RulesForeign),
            )?;
            return Ok(false);
        }
        DeployObserved::Absent | DeployObserved::Rules(_) => {}
    }
    // Recorded before the call, so a crash or a lost answer leaves `unknown`, never a gap.
    if step.kind == StepKind::Rules {
        run_of(record).rules_attempted = true;
    }
    set(record, sink, index, StepState::Unknown)?;
    match port.apply(plan, step) {
        Ok(DeployApplied::Done) => {
            set(record, sink, index, StepState::Done)?;
            Ok(true)
        }
        Ok(DeployApplied::OwnerAction(action)) => {
            set(record, sink, index, StepState::OwnerAction(action))?;
            Ok(false)
        }
        Err(error) => {
            set(record, sink, index, from_error(error))?;
            Ok(false)
        }
    }
}

fn from_error(error: DeployProviderError) -> StepState {
    match error {
        DeployProviderError::Rejected(fault) => StepState::Failed(fault),
        DeployProviderError::Unknown => StepState::Unknown,
    }
}

/// Publish the binding only when every step finished, and in the same save.
fn finish(
    mut record: DeployRecord,
    sink: &mut dyn DeploySink,
    now_ms: u64,
    plan: &DeployPlan,
) -> Result<DeployRecord, DeployError> {
    let complete = run_of(&mut record).steps.iter().all(|s| s.state.finished());
    run_of(&mut record).state = if complete {
        RunState::Complete
    } else {
        RunState::Partial
    };
    if complete {
        record.binding = Some(DeployBinding {
            plan_digest: plan.digest.clone(),
            project: plan.view.project.clone(),
            completed_at_ms: now_ms,
        });
    }
    save(sink, &record)?;
    Ok(record)
}
