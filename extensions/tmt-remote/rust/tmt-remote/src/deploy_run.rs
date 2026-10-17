//! The authorized deploy run for the Firestore sharing layer (contract: Backends and
//! deploy, "authorized deploy"). Pure: the caller reads the live Rules and the account
//! through a `DeployPort`, persists the record through a `DeploySink` and supplies the
//! clock; nothing here talks to a provider, reads a file or holds a credential, so no
//! credential choice can leak into it.
//!
//! The owner authorizes one fixed envelope: additive prerequisites and optional Hosting
//! stage precede Rules, Hosting release follows Rules, and joint verification is last.
//! A usable binding exists only when every step finished and the final read-back passed.
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
    HostingUnavailable,
    PreviousCreateUnconfirmed,
    PreviousVersionStaged,
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
            Self::HostingUnavailable => "REMOTE_DEPLOY_HOSTING_UNAVAILABLE",
            Self::PreviousCreateUnconfirmed => "REMOTE_DEPLOY_PREVIOUS_CREATE_UNCONFIRMED",
            Self::PreviousVersionStaged => "REMOTE_DEPLOY_PREVIOUS_VERSION_STAGED",
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
    CompositeIndex(crate::deploy_plan::CompositeIndexPlan),
    Hosting(HostingStep),
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployView {
    pub version: u8,
    pub backend: String,
    pub profile: String,
    pub account: String,
    pub project: String,
    pub deployment_id: String,
    pub database: DatabaseView,
    pub sign_in: Vec<String>,
    pub rules: RulesView,
    pub index_configs: usize,
    pub steps: Vec<String>,
    /// Digest of the extension plan (declarations, resources, limits, indexes, TTL).
    pub extension_plan_digest: String,
    pub destructive: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosting: Option<crate::hosting::HostingDeploymentView>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseView {
    pub name: String,
    pub edition: String,
    pub location: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesView {
    /// Digest of the composed body (without the marker line).
    pub digest: String,
    pub replaces: String,
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
/// shared by plan time and every adapter, so both read the same ownership. A release is
/// Remote's only while its marker names this deployment and the digest in the marker is
/// the digest of the bytes after the marker line: an edited release is no longer Remote's.
pub fn classify_live_rules(live: Option<&[u8]>, deployment_id: &str, publish: &[u8]) -> LiveRules {
    let Some(live) = live else {
        return LiveRules::Absent;
    };
    if live == publish {
        return LiveRules::Current;
    }
    let own_prefix = format!("{MARKER_PREFIX}{deployment_id} rules ");
    let intact = live
        .strip_prefix(own_prefix.as_bytes())
        .and_then(|rest| {
            let end = rest.iter().position(|byte| *byte == b'\n')?;
            let (digest, body) = (&rest[..end], &rest[end + 1..]);
            Some(digest == rules_digest(body).as_bytes())
        })
        .unwrap_or(false);
    if intact {
        LiveRules::Own
    } else {
        LiveRules::Foreign(sha256_hex(live))
    }
}

pub(crate) fn project_ok(value: &str) -> bool {
    // Firebase project ids: 6-30 of lowercase letters, digits, hyphens; starts with a letter.
    (6..=30).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub(crate) fn location_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Compose the envelope the owner authorizes. No provider effect happens here.
pub fn prepare(plan: &Plan, input: &DeployInput<'_>) -> Result<DeployPlan, DeployRefusal> {
    prepare_with_hosting(plan, input, None)
}

/// Include the exact read-only Hosting inventory in the one authorization envelope.
pub fn prepare_with_hosting(
    plan: &Plan,
    input: &DeployInput<'_>,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
) -> Result<DeployPlan, DeployRefusal> {
    if plan
        .view()
        .extensions
        .iter()
        .any(|extension| extension.hosting.is_some())
        != hosting.is_some()
    {
        return Err(DeployRefusal::HostingUnavailable);
    }
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
    for index in crate::deploy_plan::composite_indexes(plan.view()) {
        let bytes = serde_json::to_vec(&index).expect("index serializes");
        steps.push(step(
            &format!(
                "composite-index:{}:{}",
                index.collection_group,
                sha256_hex(&bytes)
            ),
            StepKind::CompositeIndex(index),
        ));
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
    let mut view = DeployView {
        version: 1,
        backend: "firestore".into(),
        profile: "sharing".into(),
        account: input.account.to_owned(),
        project: input.project.to_owned(),
        deployment_id: input.deployment_id.to_owned(),
        database: DatabaseView {
            name: DATABASE.into(),
            edition: EDITION.into(),
            location: input.location.to_owned(),
        },
        sign_in: providers
            .iter()
            .map(|provider| provider.name().to_owned())
            .collect(),
        rules: RulesView {
            digest: rules_digest(input.rules_body),
            replaces: replaces.into(),
            replaced_digest,
        },
        index_configs,
        steps: steps.iter().map(|step| step.id.clone()).collect(),
        extension_plan_digest: plan.digest().to_owned(),
        destructive,
        hosting: hosting.cloned(),
    };
    if let Some(hosting) = hosting {
        if hosting.site != input.project
            || hosting.public_url != format!("https://{}.web.app", input.project)
        {
            return Err(DeployRefusal::HostingUnavailable);
        }
        steps = hosting_steps(steps, hosting)?;
        view.steps = steps.iter().map(|step| step.id.clone()).collect();
        if let Some(fingerprint) = &hosting.replaced_fingerprint {
            view.destructive
                .push(format!("replaces-hosting:{fingerprint}"));
        }
    }
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
    abandon_unreleased_hosting: bool,
}
impl Authorization {
    pub(crate) fn interactive(&self) -> bool {
        self.abandon_unreleased_hosting
    }
}
/// Authorize a plan from the digest (or a prefix of at least 12 hex characters) the
/// owner typed. The plan digest also covers any foreign Rules replacement fingerprint.
pub fn authorize(plan: &DeployPlan, typed_digest: &str) -> Result<Authorization, DeployRefusal> {
    if typed_digest.len() < MIN_DIGEST_PREFIX || !plan.digest.starts_with(typed_digest) {
        return Err(DeployRefusal::AuthorizationStale);
    }
    Ok(Authorization {
        plan_digest: plan.digest.clone(),
        abandon_unreleased_hosting: false,
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
    Building,
    OwnerAction(DeployOwnerAction),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployProviderError {
    Rejected(DeployFault),
    /// The outcome is unknown: the call may have taken effect.
    Unknown,
    /// A bounded adapter diagnosis; never provider text or resource identifiers.
    Unconfirmed(StepCause),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CauseCode {
    Deadline,
    Transport,
    InvalidResponse,
    BodyInvalid,
    BodyTooLarge,
    InventoryIncomplete,
    FilesMismatch,
    HttpRejected,
    ChildFailed,
    OutputInvalid,
    CallIncomplete,
    UnconfirmedCreate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CausePhase {
    Credentials,
    List,
    Mutation,
    Body,
    ChildExit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpStatusClass {
    #[serde(rename = "1xx")]
    Informational,
    #[serde(rename = "2xx")]
    Success,
    #[serde(rename = "3xx")]
    Redirect,
    #[serde(rename = "4xx")]
    ClientError,
    #[serde(rename = "5xx")]
    ServerError,
}
/// Identifier-free union counts, bounded by the existing Hosting file cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileCounts {
    pub planned: u16,
    pub expected: u16,
    pub active: u16,
    pub listed: u16,
    pub missing: u16,
    pub extra: u16,
}
fn file_counts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<FileCounts>, D::Error> {
    let counts = Option::<FileCounts>::deserialize(d)?;
    if counts.is_some_and(|c| {
        [
            c.planned, c.expected, c.active, c.listed, c.missing, c.extra,
        ]
        .into_iter()
        .any(|n| usize::from(n) > crate::limits::HOSTING_FILES)
    }) {
        return Err(serde::de::Error::custom(
            "file counts exceed inventory bound",
        ));
    }
    Ok(counts)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepCause {
    pub code: CauseCode,
    pub phase: CausePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status_class: Option<HttpStatusClass>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "file_counts"
    )]
    pub files: Option<FileCounts>,
}
impl StepCause {
    pub fn new(code: CauseCode, phase: CausePhase) -> Self {
        Self {
            code,
            phase,
            http_status_class: None,
            files: None,
        }
    }
}

/// The provider seam. Adapters implement it; fixtures fake it.
pub trait DeployPort {
    fn supports_hosting(&self) -> bool {
        false
    }
    /// Restore only the retained original; the adapter never owns persistence.
    fn restore_hosting(&mut self, _checkpoint: &HostingCheckpoint, _steps: &[StepRecord]) {}
    fn hosting_checkpoint(&self) -> Option<HostingCheckpoint> {
        None
    }

    /// The account the credential resolves to. Read-only.
    fn account(&mut self) -> Result<String, DeployProviderError>;
    fn observe(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError>;
    /// Start a read-only composite check round. The adapter owns its bounded wait,
    /// cancellation and ceiling; false leaves accepted indexes truthfully building.
    /// Round zero starts a fresh wait for this run, including an exact-plan resume.
    fn composite_check(&mut self, _round: usize) -> Result<bool, DeployProviderError> {
        Ok(false)
    }
    /// Read within the adapter's shared check ceiling; this must never create an index.
    fn observe_composite(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError> {
        self.observe(plan, step)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<StepCause>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Applying,
    Complete,
    Partial,
    Superseded,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosting: Option<HostingCheckpoint>,
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
    /// Prior unfinished outcomes, retained without deleting any provider resources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_runs: Vec<Run>,
}
impl DeployRecord {
    pub fn new(deployment_id: &str) -> Self {
        Self {
            deployment_id: deployment_id.to_owned(),
            binding: None,
            run: None,
            superseded_runs: Vec::new(),
        }
    }
    /// The binding routes may use. Once a run may have switched the Rules and has not
    /// completed, the project is partial and no binding is usable.
    pub fn usable_binding(&self) -> Option<&DeployBinding> {
        if self.run.as_ref().is_some_and(|run| {
            run.publication_may_have_changed() && run.state != RunState::Complete
        }) {
            None
        } else {
            self.binding.as_ref()
        }
    }
    /// A consumer may publish a link only from the atomically completed joint result.
    pub fn verified_publication(&self) -> Option<&VerifiedHostingPublication> {
        let run = self.run.as_ref()?;
        if run.state != RunState::Complete || self.usable_binding()?.plan_digest != run.plan_digest
        {
            return None;
        }
        run.hosting.as_ref()?.publication.as_ref()
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
    // A declared Hosting plan cannot fall through to a Rules-only adapter.
    if plan.view.hosting.is_some() && !port.supports_hosting() {
        return Err(DeployError::Refused(DeployRefusal::HostingUnavailable));
    }
    if authorization.plan_digest != plan.digest || record.deployment_id != plan.deployment_id() {
        return Err(DeployError::Refused(DeployRefusal::AuthorizationStale));
    }
    if let Some(reason) = if authorization.abandon_unreleased_hosting {
        interactive_supersession_refusal(plan, &record)
    } else {
        supersession_refusal(plan, &record)
    } {
        return Err(DeployError::Refused(reason));
    }
    match port.account() {
        Ok(account) if account == plan.account() => {}
        Ok(_) => return Err(DeployError::Refused(DeployRefusal::AccountChanged)),
        Err(_) => return Err(DeployError::Refused(DeployRefusal::AccountUnreadable)),
    }
    begin(&mut record, plan, now_ms);
    save(sink, &record)?;
    if let Some(checkpoint) = &record.run.as_ref().expect("begun").hosting {
        port.restore_hosting(checkpoint, &record.run.as_ref().expect("begun").steps);
    }
    // Finished steps are observed again too, so drift since the last run is noticed.
    for (index, step) in plan.steps.iter().enumerate() {
        if !advance(plan, step, index, &mut record, port, sink)? {
            return finish(record, sink, now_ms, plan);
        }
        let batch_end = matches!(step.kind, StepKind::CompositeIndex(_))
            && !plan
                .steps
                .get(index + 1)
                .is_some_and(|next| matches!(next.kind, StepKind::CompositeIndex(_)));
        if batch_end && !check_composites(plan, &mut record, port, sink)? {
            return finish(record, sink, now_ms, plan);
        }
    }
    finish(record, sink, now_ms, plan)
}

/// All composite creations have finished before this read-only barrier. Neither a
/// missing read nor an exhausted wait ever authorizes another mutation in this run.
fn check_composites(
    plan: &DeployPlan,
    record: &mut DeployRecord,
    port: &mut dyn DeployPort,
    sink: &mut dyn DeploySink,
) -> Result<bool, DeployError> {
    let mut round = 0;
    loop {
        let building: Vec<_> = plan
            .steps
            .iter()
            .enumerate()
            .filter(|(i, step)| {
                matches!(step.kind, StepKind::CompositeIndex(_))
                    && record.run.as_ref().expect("begun").steps[*i].state == StepState::Building
            })
            .collect();
        let Some((first, _)) = building.first() else {
            return Ok(true);
        };
        match port.composite_check(round) {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(error) => {
                set(record, sink, *first, from_error(error))?;
                return Ok(false);
            }
        }
        for (index, step) in building {
            let state = match port.observe_composite(plan, step) {
                Ok(DeployObserved::Satisfied) => StepState::Done,
                Ok(DeployObserved::Adopted) => StepState::Adopted,
                Ok(DeployObserved::Building) => StepState::Building,
                // Lost or invalid read evidence cannot be used as an absent/create signal.
                Ok(_) => StepState::Unknown,
                Err(error) => from_error(error),
            };
            let can_continue = state.finished() || state == StepState::Building;
            set(record, sink, index, state)?;
            if !can_continue {
                return Ok(false);
            }
        }
        round += 1;
    }
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
    if state != StepState::Unknown {
        run_of(record).steps[index].cause = None;
    }
    run_of(record).steps[index].state = state;
    save(sink, record)
}
fn save(sink: &mut dyn DeploySink, record: &DeployRecord) -> Result<(), DeployError> {
    sink.save(record).map_err(|_| DeployError::Interrupted)
}

/// Start the run for this plan, or resume the saved one when it is the same plan and
/// unfinished. A finished run is never resumed: running the same plan again is a fresh
/// check that keeps the binding until a Rules call is actually made.
fn begin(record: &mut DeployRecord, plan: &DeployPlan, now_ms: u64) {
    if record
        .run
        .as_ref()
        .is_some_and(|run| run.plan_digest == plan.digest && run.state != RunState::Complete)
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
        .is_some_and(|run| run.publication_may_have_changed() && run.state != RunState::Complete)
    {
        record.binding = None;
    }
    if let Some(mut previous) = record.run.take()
        && previous.state != RunState::Complete
    {
        previous.state = RunState::Superseded;
        record.superseded_runs.push(previous);
    }
    record.run = Some(Run {
        plan_digest: plan.digest.clone(),
        account: plan.account().to_owned(),
        authorized_at_ms: now_ms,
        state: RunState::Applying,
        rules_attempted: false,
        hosting: plan.view.hosting.as_ref().map(|hosting| HostingCheckpoint {
            envelope: serde_json::to_value(&plan.view).expect("view serializes"),
            app_id: hosting.web_app.clone(),
            operation: None,
            version: None,
            version_created_ms: None,
            publication: None,
        }),
        steps: plan
            .steps
            .iter()
            .map(|step| StepRecord {
                cause: None,
                id: step.id.clone(),
                state: StepState::Pending,
            })
            .collect(),
    });
}

/// Observe, then apply when needed. Returns whether the run may go on to the next step.
fn advance(
    plan: &DeployPlan,
    step: &DeployStep,
    index: usize,
    record: &mut DeployRecord,
    port: &mut dyn DeployPort,
    sink: &mut dyn DeploySink,
) -> Result<bool, DeployError> {
    if let Some(checkpoint) = &record.run.as_ref().expect("begun").hosting {
        port.restore_hosting(checkpoint, &record.run.as_ref().expect("begun").steps);
    }
    let observation = port.observe(plan, step);
    capture_checkpoint(record, port);
    let observed = match observation {
        Ok(observed) => observed,
        Err(error) => {
            set_failure(record, sink, index, error, step, CausePhase::List)?;
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
            return Ok(!matches!(step.kind, StepKind::Hosting(_)));
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
            if plan.view.rules.replaced_digest.as_deref() != Some(digest.as_str()) =>
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
    if matches!(
        step.kind,
        StepKind::Hosting(HostingStep::WebAppCreate | HostingStep::VersionCreate)
    ) && record.run.as_ref().expect("begun").steps[index].state == StepState::Unknown
    {
        // Absence after an ambiguous mutation is not proof it did not happen.
        if run_of(record).steps[index].cause.is_none() {
            run_of(record).steps[index].cause = Some(StepCause::new(
                CauseCode::UnconfirmedCreate,
                CausePhase::List,
            ));
            save(sink, record)?;
        }
        return Ok(false);
    }
    // Recorded before the call, so a crash or a lost answer leaves `unknown`, never a gap.
    if step.kind == StepKind::Rules {
        run_of(record).rules_attempted = true;
    }
    run_of(record).steps[index].cause = matches!(step.kind, StepKind::Hosting(_))
        .then(|| StepCause::new(CauseCode::CallIncomplete, CausePhase::Mutation));
    set(record, sink, index, StepState::Unknown)?;
    if let Some(checkpoint) = &record.run.as_ref().expect("begun").hosting {
        port.restore_hosting(checkpoint, &record.run.as_ref().expect("begun").steps);
    }
    let application = port.apply(plan, step);
    capture_checkpoint(record, port);
    match application {
        Ok(DeployApplied::Done) => {
            set(record, sink, index, StepState::Done)?;
            Ok(true)
        }
        Ok(DeployApplied::Building) => {
            set(record, sink, index, StepState::Building)?;
            Ok(matches!(step.kind, StepKind::CompositeIndex(_)))
        }
        Ok(DeployApplied::OwnerAction(action)) => {
            set(record, sink, index, StepState::OwnerAction(action))?;
            Ok(false)
        }
        Err(error) => {
            set_failure(record, sink, index, error, step, CausePhase::Mutation)?;
            Ok(false)
        }
    }
}

fn set_failure(
    record: &mut DeployRecord,
    sink: &mut dyn DeploySink,
    index: usize,
    error: DeployProviderError,
    step: &DeployStep,
    phase: CausePhase,
) -> Result<(), DeployError> {
    run_of(record).steps[index].cause = if matches!(step.kind, StepKind::Hosting(_)) {
        match error {
            DeployProviderError::Unconfirmed(cause) => Some(cause),
            DeployProviderError::Unknown => Some(StepCause::new(CauseCode::InvalidResponse, phase)),
            DeployProviderError::Rejected(_) => None,
        }
    } else {
        None
    };
    set(record, sink, index, from_error(error))
}

fn from_error(error: DeployProviderError) -> StepState {
    match error {
        DeployProviderError::Rejected(fault) => StepState::Failed(fault),
        DeployProviderError::Unknown | DeployProviderError::Unconfirmed(_) => StepState::Unknown,
    }
}

/// Publish the binding only when every step finished, and in the same save.
fn finish(
    mut record: DeployRecord,
    sink: &mut dyn DeploySink,
    now_ms: u64,
    plan: &DeployPlan,
) -> Result<DeployRecord, DeployError> {
    let complete = run_of(&mut record).steps.iter().all(|s| s.state.finished())
        && (plan.view.hosting.is_none()
            || run_of(&mut record)
                .hosting
                .as_ref()
                .is_some_and(|h| h.publication.as_ref().is_some_and(|p| p.matches(plan))));
    run_of(&mut record).state = if complete {
        RunState::Complete
    } else {
        RunState::Partial
    };
    if complete {
        if let Some(publication) = run_of(&mut record)
            .hosting
            .as_mut()
            .and_then(|h| h.publication.as_mut())
        {
            publication.verified_at_ms = now_ms;
        }
        record.binding = Some(DeployBinding {
            plan_digest: plan.digest.clone(),
            project: plan.view.project.clone(),
            completed_at_ms: now_ms,
        });
    }
    save(sink, &record)?;
    Ok(record)
}

/// Concrete stage effects all remain in the existing ordered deploy engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostingStep {
    WebAppCreate,
    SiteCreate,
    SiteConfigure,
    VersionCreate,
    Populate,
    Upload { path: String, hash: String },
    Finalize,
    Release,
}
fn hosting_steps(
    mut steps: Vec<DeployStep>,
    hosting: &crate::hosting::HostingDeploymentView,
) -> Result<Vec<DeployStep>, DeployRefusal> {
    let mut prefix = Vec::new();
    if hosting.create_web_app {
        prefix.push(step(
            "web-app:create",
            StepKind::Hosting(HostingStep::WebAppCreate),
        ));
    }
    if hosting.create_site {
        prefix.push(step(
            "hosting-site:create",
            StepKind::Hosting(HostingStep::SiteCreate),
        ));
    }
    if hosting.configure_site {
        prefix.push(step(
            "hosting-site:configure",
            StepKind::Hosting(HostingStep::SiteConfigure),
        ));
    }
    prefix.append(&mut steps);
    let rules = prefix
        .iter()
        .position(|s| s.kind == StepKind::Rules)
        .expect("Rules");
    let mut stage = vec![
        step(
            "hosting:stage:create",
            StepKind::Hosting(HostingStep::VersionCreate),
        ),
        step(
            "hosting:stage:populate",
            StepKind::Hosting(HostingStep::Populate),
        ),
    ];
    let files = hosting.content["files"]
        .as_array()
        .ok_or(DeployRefusal::HostingUnavailable)?;
    let mut hashes = std::collections::BTreeSet::new();
    for file in files {
        let hash = file["gzipDigest"]
            .as_str()
            .ok_or(DeployRefusal::HostingUnavailable)?;
        let path = file["path"]
            .as_str()
            .ok_or(DeployRefusal::HostingUnavailable)?;
        if hashes.insert(hash) {
            stage.push(step(
                &format!("hosting:stage:upload:{hash}"),
                StepKind::Hosting(HostingStep::Upload {
                    path: path.into(),
                    hash: hash.into(),
                }),
            ));
        }
    }
    stage.push(step(
        "hosting:stage:finalize",
        StepKind::Hosting(HostingStep::Finalize),
    ));
    prefix.splice(rules..rules, stage);
    let rules = prefix
        .iter()
        .position(|s| s.kind == StepKind::Rules)
        .expect("Rules");
    prefix.insert(
        rules + 1,
        step("hosting:release", StepKind::Hosting(HostingStep::Release)),
    );
    Ok(prefix)
}
/// Only the frozen authorization and provider-generated recovery facts extend the record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostingCheckpoint {
    pub envelope: serde_json::Value,
    pub app_id: Option<String>,
    pub operation: Option<String>,
    pub version: Option<String>,
    pub version_created_ms: Option<u64>,
    pub publication: Option<VerifiedHostingPublication>,
}
/// Constructed by final joint read-back, never by staging or a Rules-only outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifiedHostingPublication {
    project: String,
    region: String,
    deployment_id: String,
    plan_digest: String,
    app_id: String,
    site_app_id: String,
    public_config: serde_json::Value,
    entry_url: String,
    version: String,
    release: String,
    #[serde(default)]
    verified_at_ms: u64,
    rules_digest: String,
    content_digest: String,
}
impl VerifiedHostingPublication {
    pub fn public_config(&self) -> &serde_json::Value {
        &self.public_config
    }
    pub fn entry_url(&self) -> &str {
        &self.entry_url
    }
    pub(crate) fn from_verified(
        plan: &DeployPlan,
        checkpoint: &HostingCheckpoint,
        value: &serde_json::Value,
    ) -> Option<Self> {
        let project = &plan.view.project;
        let app_id = checkpoint.app_id.as_ref()?;
        let version = checkpoint.version.as_ref()?;
        let config = &value["publicConfig"];
        if value["siteAppId"] != *app_id
            || !crate::hosting::public_config_ok(Some(config), project, app_id)
        {
            return None;
        }
        let release = value["release"].as_str()?;
        if !release.starts_with(&format!("sites/{project}/releases/")) || release.len() > 256 {
            return None;
        }
        Some(Self {
            project: project.clone(),
            region: plan.view.database.location.clone(),
            deployment_id: plan.deployment_id().into(),
            plan_digest: plan.digest().into(),
            app_id: app_id.clone(),
            site_app_id: app_id.clone(),
            public_config: config.clone(),
            entry_url: format!("https://{project}.web.app"),
            version: version.clone(),
            release: release.into(),
            verified_at_ms: 0,
            rules_digest: plan.view.rules.digest.clone(),
            content_digest: plan.view.hosting.as_ref()?.content["digest"]
                .as_str()?
                .into(),
        })
    }
    pub(crate) fn valid_for(
        &self,
        view: &DeployView,
        checkpoint: &HostingCheckpoint,
        digest: &str,
    ) -> bool {
        self.project == view.project
            && self.region == view.database.location
            && self.deployment_id == view.deployment_id
            && self.plan_digest == digest
            && self.site_app_id == self.app_id
            && Some(&self.app_id) == checkpoint.app_id.as_ref()
            && Some(&self.version) == checkpoint.version.as_ref()
            && self.entry_url == format!("https://{}.web.app", view.project)
            && self
                .version
                .starts_with(&format!("sites/{}/versions/", view.project))
            && self
                .release
                .starts_with(&format!("sites/{}/releases/", view.project))
            && self.rules_digest == view.rules.digest
            && view
                .hosting
                .as_ref()
                .is_some_and(|h| h.content["digest"] == self.content_digest)
            && crate::hosting::public_config_ok(
                Some(&self.public_config),
                &self.project,
                &self.app_id,
            )
    }
    fn matches(&self, plan: &DeployPlan) -> bool {
        self.project == plan.view.project
            && self.plan_digest == plan.digest
            && self.deployment_id == plan.deployment_id()
    }
}
fn capture_checkpoint(record: &mut DeployRecord, port: &dyn DeployPort) {
    if let Some(checkpoint) = port.hosting_checkpoint() {
        run_of(record).hosting = Some(checkpoint);
    }
}
/// Resume the original authorization even after its own effects change the live inventory.
/// Current explicit inputs must still describe exactly that original intent.
pub fn retain_hosting_plan(
    mut candidate: DeployPlan,
    record: &DeployRecord,
) -> Result<DeployPlan, DeployRefusal> {
    if candidate
        .view
        .hosting
        .as_ref()
        .is_some_and(|h| h.abandoned_version.is_some())
    {
        return Ok(candidate);
    }
    let Some(run) = record
        .run
        .as_ref()
        .filter(|r| r.state != RunState::Complete)
    else {
        return Ok(candidate);
    };
    let Some(checkpoint) = &run.hosting else {
        return Ok(candidate);
    };
    let view: DeployView = serde_json::from_value(checkpoint.envelope.clone())
        .map_err(|_| DeployRefusal::AuthorizationStale)?;
    if sha256_hex(&serde_json::to_vec(&view).map_err(|_| DeployRefusal::AuthorizationStale)?)
        != run.plan_digest
    {
        return Err(DeployRefusal::AuthorizationStale);
    }
    if view.account != candidate.view.account
        || view.project != candidate.view.project
        || view.database != candidate.view.database
        || view.deployment_id != candidate.view.deployment_id
        || view.extension_plan_digest != candidate.view.extension_plan_digest
        || view.sign_in != candidate.view.sign_in
        || view.rules.digest != candidate.view.rules.digest
        || view.hosting.as_ref().map(|h| &h.content)
            != candidate.view.hosting.as_ref().map(|h| &h.content)
    {
        return Ok(candidate);
    }
    candidate
        .steps
        .retain(|s| !matches!(s.kind, StepKind::Hosting(_)));
    candidate.steps = hosting_steps(
        candidate.steps,
        view.hosting
            .as_ref()
            .ok_or(DeployRefusal::HostingUnavailable)?,
    )?;
    candidate.bytes = serde_json::to_vec(&view).expect("view serializes");
    candidate.digest = run.plan_digest.clone();
    candidate.view = view;
    Ok(candidate)
}

/// Whether this plan would replace an unfinished authorization. No provider work.
pub fn superseded_run<'a>(plan: &DeployPlan, record: &'a DeployRecord) -> Option<&'a Run> {
    record
        .run
        .as_ref()
        .filter(|run| run.state != RunState::Complete && run.plan_digest != plan.digest)
}
/// A changed intent cannot discard an ambiguous creation or a staged Version.
/// The existing explicit expired-Version plan is the sole abandonment path here.
pub fn supersession_refusal(plan: &DeployPlan, record: &DeployRecord) -> Option<DeployRefusal> {
    let previous = superseded_run(plan, record)?;
    if previous.steps.iter().any(|step| {
        matches!(
            step.id.as_str(),
            "web-app:create"
                | "hosting-site:create"
                | "hosting-site:configure"
                | "hosting:stage:create"
        ) && (step.state == StepState::Unknown
            || (step.id == "web-app:create" && step.state == StepState::Building))
    }) {
        return Some(DeployRefusal::PreviousCreateUnconfirmed);
    }
    let abandoned = plan
        .view
        .hosting
        .as_ref()
        .and_then(|h| h.abandoned_version.as_ref());
    let version = previous.hosting.as_ref().and_then(|h| h.version.as_ref());
    if version.is_some() && abandoned == version {
        return None;
    }
    if version.is_some()
        || previous.steps.iter().any(|step| {
            step.id == "hosting:stage:create"
                && matches!(
                    step.state,
                    StepState::Done | StepState::Adopted | StepState::Building
                )
        })
    {
        return Some(DeployRefusal::PreviousVersionStaged);
    }
    None
}

impl Run {
    pub(crate) fn publication_may_have_changed(&self) -> bool {
        self.rules_attempted
            || self
                .steps
                .iter()
                .any(|s| s.id == "hosting:release" && s.state != StepState::Pending)
    }
}

/// A terminal confirmation of these captured bytes may leave an unreleased Version
/// behind, but never discards an unconfirmed web-app/site/config creation.
pub fn authorize_interactive(
    plan: &DeployPlan,
    record: &DeployRecord,
) -> Result<Authorization, DeployRefusal> {
    if let Some(reason) = interactive_supersession_refusal(plan, record) {
        return Err(reason);
    }
    Ok(Authorization {
        plan_digest: plan.digest.clone(),
        abandon_unreleased_hosting: true,
    })
}
pub fn interactive_supersession_refusal(
    plan: &DeployPlan,
    record: &DeployRecord,
) -> Option<DeployRefusal> {
    let previous = superseded_run(plan, record)?;
    if previous.steps.iter().any(|step| {
        matches!(
            step.id.as_str(),
            "web-app:create" | "hosting-site:create" | "hosting-site:configure"
        ) && (step.state == StepState::Unknown
            || (step.id == "web-app:create" && step.state == StepState::Building))
    }) {
        return Some(DeployRefusal::PreviousCreateUnconfirmed);
    }
    if previous
        .steps
        .iter()
        .any(|step| step.id == "hosting:release" && step.state == StepState::Pending)
    {
        None
    } else {
        supersession_refusal(plan, record)
    }
}
