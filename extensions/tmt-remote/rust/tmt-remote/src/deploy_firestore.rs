//! Firestore and Hosting provider adapter. The embedded helper owns the installed
//! firebase-tools login; the existing deploy engine owns authorization and recovery.
//! The CLI composition wires it; verified publication does not enable cloud routes.
use crate::error::DeployProviderDiagnostic;
use crate::{
    deploy_run::{
        self, CauseCode, CausePhase, DeployApplied, DeployFault, DeployObserved, DeployOwnerAction,
        DeployPlan, DeployPort, DeployProviderError, DeployStep, StepCause, StepKind,
    },
    limits, wire,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fmt,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tmt_invoke::{EnvironmentPolicy, LaunchOptions, Request};

const HELPER: &str = include_str!("deploy_firestore/helper.cjs");
/// Versions whose private auth/layout interface this release has qualified.
pub const FIREBASE_TOOLS_VERSIONS: &[&str] = &["15.29.0"];

// One fixed adapter policy: at most eighteen read-only rounds within three minutes.
const COMPOSITE_INDEX_WAIT: CompositeWaitPolicy = CompositeWaitPolicy {
    interval: Duration::from_secs(10),
    rounds: 18,
    cancel_tick: Duration::from_millis(100),
};
struct CompositeWaitPolicy {
    interval: Duration,
    rounds: u32,
    cancel_tick: Duration,
}
impl CompositeWaitPolicy {
    fn ceiling(&self) -> Duration {
        self.interval * self.rounds
    }
    fn wait_round(
        &self,
        start: Instant,
        round: usize,
        stop: &AtomicBool,
        mut now: impl FnMut() -> Instant,
        mut sleep: impl FnMut(Duration),
    ) -> Result<Option<Duration>, DeployProviderError> {
        if round >= self.rounds as usize {
            return Ok(None);
        }
        let target = if round == 0 {
            start
        } else {
            now() + self.interval
        };
        loop {
            if stop.load(Ordering::Acquire) {
                return Err(unknown());
            }
            let current = now();
            if current >= start + self.ceiling() {
                return Ok(None);
            }
            if current >= target {
                return Ok(Some(current.duration_since(start)));
            }
            sleep(
                target
                    .min(start + self.ceiling())
                    .duration_since(current)
                    .min(self.cancel_tick),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeploySetupError {
    UnsupportedTool,
    LoginRequired,
    Unavailable,
}
impl fmt::Display for DeploySetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedTool => "This Remote release needs firebase-tools 15.29.0. Install it with: npm install -g firebase-tools@15.29.0",
            Self::LoginRequired => "Firebase is not signed in. Run firebase login with your own account, then try again.",
            Self::Unavailable => "Firebase setup could not be confirmed. Check that firebase projects:list works in this terminal, then try again.",
        })
    }
}
impl std::error::Error for DeploySetupError {}

/// Preflight has no effects; its HTTP evidence is separate from persisted step outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployPreflightError {
    pub reason: DeployProviderError,
    pub provider: Option<DeployProviderDiagnostic>,
}
impl From<DeployProviderError> for DeployPreflightError {
    fn from(reason: DeployProviderError) -> Self {
        Self {
            reason,
            provider: None,
        }
    }
}
impl DeployPreflightError {
    pub fn remote_error(self) -> crate::error::RemoteError {
        let code = match self.reason {
            DeployProviderError::Rejected(fault) => fault.code(),
            DeployProviderError::Unknown | DeployProviderError::Unconfirmed(_) => {
                "REMOTE_DEPLOY_UNAVAILABLE"
            }
        };
        let message = match self.provider {
            Some(provider) => format!("Firebase could not read {} (HTTP {}). Nothing changed in your Firebase project.", provider.operation.label(), provider.status),
            None => "Firebase refused to list the project's current setup. Nothing changed in your Firebase project.".into(),
        };
        let mut error = crate::error::RemoteError::new(code, &message);
        error.provider = self.provider;
        error
    }
}
#[derive(Debug)]
struct HelperFailure {
    code: String,
    provider: Option<DeployProviderDiagnostic>,
    cause: Option<StepCause>,
}
impl From<&str> for HelperFailure {
    fn from(code: &str) -> Self {
        Self {
            code: code.into(),
            provider: None,
            cause: None,
        }
    }
}
fn preflight_error(error: HelperFailure) -> DeployPreflightError {
    let provider = error.provider;
    DeployPreflightError {
        reason: provider_error(error),
        provider,
    }
}
fn parse_reply(bytes: &[u8]) -> Result<Value, HelperFailure> {
    let value = wire::strict_json(bytes).ok_or("unknown")?;
    let object = value.as_object().ok_or("unknown")?;
    if object.get("version") != Some(&json!(1)) {
        return Err("unknown".into());
    }
    if let Some(result) = object.get("result") {
        if object.len() != 2 {
            return Err("unknown".into());
        }
        return Ok(result.clone());
    }
    let code = object
        .get("error")
        .and_then(Value::as_str)
        .ok_or("unknown")?;
    let cause = object
        .get("cause")
        .map(|v| {
            serde_json::from_value::<StepCause>(v.clone())
                .map_err(|_| HelperFailure::from("unknown"))
        })
        .transpose()?;
    let provider = match object.get("provider") {
        Some(value) => {
            let diagnostic: DeployProviderDiagnostic =
                serde_json::from_value(value.clone()).map_err(|_| "unknown")?;
            if !diagnostic.valid() {
                return Err("unknown".into());
            }
            Some(diagnostic)
        }
        None => None,
    };
    if object.len() != 2 + usize::from(provider.is_some()) + usize::from(cause.is_some()) {
        return Err("unknown".into());
    }
    // Only fixed codes cross this process boundary. All other strings are discarded.
    match code {
        "unsupported-tool" | "login-required" | "account-changed" | "permission-denied"
        | "api-disabled" | "quota-exceeded" | "database-mismatch" | "rules-foreign"
        | "verify-failed" | "provider-rejected" | "unknown" => Err(HelperFailure {
            code: code.into(),
            provider,
            cause,
        }),
        _ => Err("unknown".into()),
    }
}

/// Uses one waited child per bounded request. Helper bytes are embedded in this binary,
/// never loaded from cwd. Package paths select the explicitly installed external tool.
pub struct DeployFirestore<'a> {
    node: PathBuf,
    package: PathBuf,
    stop: &'a AtomicBool,
    hosting: Option<deploy_run::HostingCheckpoint>,
    compressed: std::collections::BTreeMap<String, Vec<u8>>,
    steps: Vec<deploy_run::StepRecord>,
    composite_started: Option<Instant>,
    composite_progress: Option<Box<dyn FnMut(Duration, Duration) + Send + 'a>>,
}
impl<'a> DeployFirestore<'a> {
    /// Check compatibility without loading auth, reading credentials or contacting Google.
    /// Executable/package discovery belongs to the CLI's installed-tool factory.
    pub fn at(
        node: PathBuf,
        package: PathBuf,
        stop: &'a AtomicBool,
    ) -> Result<Self, DeploySetupError> {
        if !node.is_absolute() || !package.is_absolute() {
            return Err(DeploySetupError::UnsupportedTool);
        }
        let mut adapter = Self {
            node,
            package,
            stop,
            hosting: None,
            compressed: std::collections::BTreeMap::new(),
            steps: Vec::new(),
            composite_started: None,
            composite_progress: None,
        };
        let capabilities = adapter
            .exchange(
                "compatibility",
                json!({}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(setup_error)?;
        if capabilities
            != json!({"versions":FIREBASE_TOOLS_VERSIONS,"maxBytes":limits::DEPLOY_PROVIDER_BYTES,"maxPages":limits::DEPLOY_PROVIDER_PAGES,
                "hostingPhaseMs":limits::DEPLOY_PROVIDER_CALL.as_millis(),"hostingApplyMs":limits::HOSTING_APPLY_CALL.as_millis()})
        {
            return Err(DeploySetupError::UnsupportedTool);
        }
        Ok(adapter)
    }
    /// CLI composition supplies presentation; the provider never writes output.
    pub fn with_composite_progress(
        mut self,
        progress: impl FnMut(Duration, Duration) + Send + 'a,
    ) -> Self {
        self.composite_progress = Some(Box::new(progress));
        self
    }
    pub fn environment_names() -> [OsString; 3] {
        ["HOME", "PATH", "XDG_CONFIG_HOME"].map(OsString::from)
    }
    /// Read-only plan inputs; no deployment permission is implied by a successful read.
    pub fn live_rules(&mut self, project: &str) -> Result<Option<Vec<u8>>, DeployPreflightError> {
        let value = self
            .exchange(
                "live-rules",
                json!({"project":project}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(preflight_error)?;
        rules_bytes(&value).map_err(Into::into)
    }
    /// Read-only live+planned field-config ceiling, including unrelated configurations.
    pub fn check_index_budget(
        &mut self,
        project: &str,
        plan: &crate::deploy_plan::Plan,
    ) -> Result<(), DeployPreflightError> {
        let fields: std::collections::BTreeSet<_> = plan
            .view()
            .extensions
            .iter()
            .flat_map(|extension| &extension.resources)
            .flat_map(|resource| {
                resource.indexes.iter().map(move |index| {
                    format!(
                        "{}/{}",
                        resource.path.rsplit('/').next().expect("validated path"),
                        index.field
                    )
                })
            })
            .collect();
        let value = self
            .exchange(
                "index-budget",
                json!({"project": project, "fields": fields, "composites": crate::deploy_plan::composite_indexes(plan.view())}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(preflight_error)?;
        if value != "within-budget" {
            return Err(unknown().into());
        }
        Ok(())
    }
    pub fn login_account(&mut self) -> Result<String, DeploySetupError> {
        let value = self
            .exchange(
                "account",
                json!({}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(setup_error)?;
        value
            .as_str()
            .filter(|v| account_ok(v))
            .map(str::to_owned)
            .ok_or(DeploySetupError::Unavailable)
    }
    fn exchange(
        &mut self,
        operation: &str,
        input: Value,
        account: Option<&str>,
        deadline: Instant,
    ) -> Result<Value, HelperFailure> {
        let failure = |code, phase| HelperFailure {
            code: "unknown".into(),
            provider: None,
            cause: Some(StepCause::new(code, phase)),
        };
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| failure(CauseCode::Deadline, CausePhase::ChildExit))?;
        let request = json!({"version":1,"operation":operation,"input":input,"account":account,
            "budgetMs":remaining.as_millis().min(if operation == "apply-hosting" { limits::HOSTING_APPLY_CALL.as_millis() } else { limits::DEPLOY_PROVIDER_CALL.as_millis() })});
        let input = serde_json::to_vec(&request).map_err(|_| "unknown")?;
        let input_bound = if operation == "apply-hosting" {
            limits::HOSTING_UPLOAD_REQUEST_BYTES
        } else {
            limits::DEPLOY_PROVIDER_BYTES
        };
        if input.len() > input_bound {
            return Err("provider-rejected".into());
        }
        let args = [
            OsString::from("--eval"),
            OsString::from(HELPER),
            self.package.as_os_str().to_owned(),
        ];
        let env = Self::environment_names();
        let output = tmt_invoke::invoke(
            Request {
                program: &self.node,
                args: &args,
                input: &input,
                deadline,
                max_stream_bytes: limits::DEPLOY_PROVIDER_BYTES,
                launch: LaunchOptions {
                    environment: EnvironmentPolicy::ClearAllowlist(&env),
                    current_dir: Some(PathBuf::from("/")),
                    ..LaunchOptions::default()
                },
            },
            Some(self.stop),
        )
        .map_err(|error| {
            failure(
                match error.kind {
                    tmt_invoke::FailureKind::Deadline => CauseCode::Deadline,
                    tmt_invoke::FailureKind::OutputLimit(_) => CauseCode::OutputInvalid,
                    _ => CauseCode::ChildFailed,
                },
                CausePhase::ChildExit,
            )
        })?;
        // Never expose exception text, stderr, a failed child's JSON, or its exit cause.
        if !output.status.success() {
            return Err(failure(CauseCode::ChildFailed, CausePhase::ChildExit));
        }
        if output.stdout.len() > limits::DEPLOY_PROVIDER_BYTES {
            return Err(failure(CauseCode::OutputInvalid, CausePhase::ChildExit));
        }
        parse_reply(&output.stdout).map_err(|mut error| {
            if error.code == "unknown" && error.cause.is_none() {
                error.cause = Some(StepCause::new(
                    CauseCode::OutputInvalid,
                    CausePhase::ChildExit,
                ));
            }
            error
        })
    }

    fn step(
        &mut self,
        operation: &str,
        plan: &DeployPlan,
        step: &DeployStep,
        deadline: Instant,
    ) -> Result<Value, DeployProviderError> {
        let input = match &step.kind {
            StepKind::Database => {
                json!({"project":plan.view().project,"location":plan.view().database.location})
            }
            StepKind::SignIn(provider) => {
                json!({"project":plan.view().project,"provider":provider.name()})
            }
            StepKind::Index {
                path,
                field,
                direction,
            } => json!({"project":plan.view().project,
                "collection":path.rsplit('/').next(),"field":field,"direction":direction}),
            StepKind::CompositeIndex(index) => json!({"project":plan.view().project,"index":index}),
            StepKind::Rules => {
                json!({"project":plan.view().project,"source":std::str::from_utf8(plan.deployed_rules()).map_err(|_| unknown())?,
                "deployment":plan.deployment_id(),"replacedDigest":plan.view().rules.replaced_digest})
            }
            StepKind::Hosting(_) => return self.hosting_step(operation, plan, step, deadline),
            StepKind::Verify => return Err(unknown()),
        };
        let kind = match step.kind {
            StepKind::Database => "database",
            StepKind::SignIn(_) => "sign-in",
            StepKind::Index { .. } => "index",
            StepKind::CompositeIndex(_) => "composite",
            StepKind::Rules => "rules",
            StepKind::Hosting(_) | StepKind::Verify => unreachable!(),
        };
        self.exchange(
            &format!("{operation}-{kind}"),
            input,
            Some(plan.account()),
            deadline,
        )
        .map_err(provider_error)
    }
    fn observe_until(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
        deadline: Instant,
    ) -> Result<DeployObserved, DeployProviderError> {
        let result = self.step("observe", plan, step, deadline)?;
        if step.kind == StepKind::Rules {
            return Ok(DeployObserved::Rules(deploy_run::classify_live_rules(
                rules_bytes(&result)?.as_deref(),
                plan.deployment_id(),
                plan.deployed_rules(),
            )));
        }
        observed(&result)
    }
}
impl DeployPort for DeployFirestore<'_> {
    fn supports_hosting(&self) -> bool {
        true
    }
    fn restore_hosting(
        &mut self,
        checkpoint: &deploy_run::HostingCheckpoint,
        steps: &[deploy_run::StepRecord],
    ) {
        self.hosting = Some(checkpoint.clone());
        self.steps = steps.to_vec();
    }
    fn hosting_checkpoint(&self) -> Option<deploy_run::HostingCheckpoint> {
        self.hosting.clone()
    }
    fn account(&mut self) -> Result<String, DeployProviderError> {
        self.login_account().map_err(|_| unknown())
    }
    fn observe(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError> {
        if step.kind == StepKind::Verify {
            let verified = verify_steps(
                plan.steps(),
                |other, deadline| self.observe_until(plan, other, deadline),
                Instant::now,
            )?;
            if verified == DeployObserved::Satisfied && plan.view().hosting.is_some() {
                let value = self.hosting_exchange(
                    "verify-hosting",
                    plan,
                    "verify",
                    None,
                    None,
                    Instant::now() + limits::DEPLOY_PROVIDER_CALL,
                )?;
                let checkpoint = self.hosting.as_ref().ok_or_else(unknown)?;
                let publication =
                    deploy_run::VerifiedHostingPublication::from_verified(plan, checkpoint, &value)
                        .ok_or_else(unknown)?;
                self.hosting.as_mut().expect("checkpoint").publication = Some(publication);
            }
            return Ok(verified);
        }
        self.observe_until(plan, step, Instant::now() + limits::DEPLOY_PROVIDER_CALL)
    }
    fn composite_check(&mut self, round: usize) -> Result<bool, DeployProviderError> {
        if round == 0 {
            self.composite_started = Some(Instant::now());
        }
        let start = self.composite_started.ok_or_else(unknown)?;
        let elapsed = COMPOSITE_INDEX_WAIT.wait_round(
            start,
            round,
            self.stop,
            Instant::now,
            std::thread::sleep,
        )?;
        if let Some(elapsed) = elapsed {
            if let Some(progress) = &mut self.composite_progress {
                progress(elapsed, COMPOSITE_INDEX_WAIT.ceiling());
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn observe_composite(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError> {
        let ceiling = self.composite_started.ok_or_else(unknown)? + COMPOSITE_INDEX_WAIT.ceiling();
        composite_observation(Instant::now(), ceiling, self.stop, |deadline| {
            self.observe_until(plan, step, deadline)
        })
    }
    fn apply(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployApplied, DeployProviderError> {
        match self
            .step(
                "apply",
                plan,
                step,
                Instant::now()
                    + if matches!(step.kind, StepKind::Hosting(_)) {
                        limits::HOSTING_APPLY_CALL
                    } else {
                        limits::DEPLOY_PROVIDER_CALL
                    },
            )?
            .as_str()
        {
            Some("done") => Ok(DeployApplied::Done),
            Some("building") => Ok(DeployApplied::Building),
            Some("initialize-auth") => Ok(DeployApplied::OwnerAction(
                DeployOwnerAction::InitializeAuth,
            )),
            Some("enable-google-sign-in") => Ok(DeployApplied::OwnerAction(
                DeployOwnerAction::EnableGoogleSignIn,
            )),
            _ => Err(unknown()),
        }
    }
}
// Inject the one read so the shared ceiling and cancellation can be tested without
// sleeping or launching a child. This owner never applies an index while checking it.
fn composite_observation(
    now: Instant,
    ceiling: Instant,
    stop: &AtomicBool,
    read: impl FnOnce(Instant) -> Result<DeployObserved, DeployProviderError>,
) -> Result<DeployObserved, DeployProviderError> {
    if stop.load(Ordering::Acquire) {
        return Err(unknown());
    }
    if now >= ceiling {
        return Ok(DeployObserved::Building);
    }
    let deadline = (now + limits::DEPLOY_PROVIDER_CALL).min(ceiling);
    let observed = read(deadline);
    if observed.is_err() && deadline == ceiling && !stop.load(Ordering::Acquire) {
        // A failed final read does not make the accepted creation unconfirmed.
        // Keep the last accepted Building state; an exact-plan rerun observes it again.
        Ok(DeployObserved::Building)
    } else {
        observed
    }
}

fn account_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)
}
fn unknown() -> DeployProviderError {
    DeployProviderError::Unknown
}
fn rules_bytes(value: &Value) -> Result<Option<Vec<u8>>, DeployProviderError> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.as_bytes().to_vec())),
        _ => Err(unknown()),
    }
}
fn observed(value: &Value) -> Result<DeployObserved, DeployProviderError> {
    match value.as_str() {
        Some("absent") => Ok(DeployObserved::Absent),
        Some("satisfied") => Ok(DeployObserved::Satisfied),
        Some("adopted") => Ok(DeployObserved::Adopted),
        Some("building") => Ok(DeployObserved::Building),
        Some("mismatch") => Ok(DeployObserved::Mismatch),
        Some("initialize-auth") => Ok(DeployObserved::OwnerAction(
            DeployOwnerAction::InitializeAuth,
        )),
        Some("enable-google-sign-in") => Ok(DeployObserved::OwnerAction(
            DeployOwnerAction::EnableGoogleSignIn,
        )),
        _ => Err(unknown()),
    }
}
fn setup_error(error: HelperFailure) -> DeploySetupError {
    match error.code.as_str() {
        "unsupported-tool" => DeploySetupError::UnsupportedTool,
        "login-required" => DeploySetupError::LoginRequired,
        _ => DeploySetupError::Unavailable,
    }
}
fn provider_error(error: HelperFailure) -> DeployProviderError {
    let fault = match error.code.as_str() {
        "permission-denied" => DeployFault::PermissionDenied,
        "api-disabled" => DeployFault::ApiDisabled,
        "quota-exceeded" => DeployFault::QuotaExceeded,
        "database-mismatch" => DeployFault::DatabaseMismatch,
        "rules-foreign" => DeployFault::RulesForeign,
        "verify-failed" => return unknown(),
        "provider-rejected" | "unsupported-tool" | "login-required" | "account-changed" => {
            DeployFault::ProviderRejected
        }
        _ => {
            return error
                .cause
                .map(DeployProviderError::Unconfirmed)
                .unwrap_or_else(unknown);
        }
    };
    DeployProviderError::Rejected(fault)
}

// Verification is read-only and plan-bounded; each child gets a full operation budget.
fn verify_steps(
    steps: &[DeployStep],
    mut observe: impl FnMut(&DeployStep, Instant) -> Result<DeployObserved, DeployProviderError>,
    mut now: impl FnMut() -> Instant,
) -> Result<DeployObserved, DeployProviderError> {
    for step in steps.iter().filter(|s| s.kind != StepKind::Verify) {
        match observe(step, now() + limits::DEPLOY_PROVIDER_CALL)? {
            DeployObserved::Satisfied
            | DeployObserved::Adopted
            | DeployObserved::Rules(deploy_run::LiveRules::Current) => {}
            DeployObserved::Building => return Ok(DeployObserved::Building),
            _ => return Err(unknown()),
        }
    }
    Ok(DeployObserved::Satisfied)
}

impl DeployFirestore<'_> {
    pub fn prepare_content(&mut self, composition: &crate::hosting::HostingComposition) {
        self.compressed = composition
            .files()
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    composition
                        .gzip(&f.path)
                        .expect("validated content")
                        .to_vec(),
                )
            })
            .collect();
    }
    pub fn hosting_inventory(
        &mut self,
        project: &str,
    ) -> Result<crate::hosting::HostingInventory, deploy_run::DeployRefusal> {
        let value = self
            .exchange(
                "hosting-inventory",
                json!({"project": project}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(|_| deploy_run::DeployRefusal::HostingUnavailable)?;
        serde_json::from_value(value).map_err(|_| deploy_run::DeployRefusal::HostingUnavailable)
    }
    fn hosting_exchange(
        &mut self,
        operation: &str,
        plan: &DeployPlan,
        action: &str,
        hash: Option<&str>,
        bytes: Option<&[u8]>,
        deadline: Instant,
    ) -> Result<Value, DeployProviderError> {
        use base64::Engine;
        let checkpoint = self.hosting.as_ref().ok_or_else(unknown)?.clone();
        let value = self.exchange(operation, json!({"project": plan.view().project, "deployment": plan.deployment_id(), "planDigest": plan.digest(), "hosting": plan.view().hosting, "checkpoint": checkpoint, "steps": self.steps, "action": action, "hash": hash, "bytesBase64": bytes.map(|b| base64::engine::general_purpose::STANDARD.encode(b)), "source": std::str::from_utf8(plan.deployed_rules()).map_err(|_| unknown())?}), Some(plan.account()), deadline).map_err(provider_error)?;
        if let Some(state) = value.get("checkpoint") {
            let output_error = || {
                DeployProviderError::Unconfirmed(StepCause::new(
                    CauseCode::OutputInvalid,
                    CausePhase::ChildExit,
                ))
            };
            let parsed: deploy_run::HostingCheckpoint =
                serde_json::from_value(state.clone()).map_err(|_| output_error())?;
            if parsed.envelope != checkpoint.envelope {
                return Err(output_error());
            }
            self.hosting = Some(parsed);
        }
        Ok(value)
    }
    fn hosting_step(
        &mut self,
        operation: &str,
        plan: &DeployPlan,
        step: &DeployStep,
        deadline: Instant,
    ) -> Result<Value, DeployProviderError> {
        use deploy_run::HostingStep;
        let (action, hash, bytes) = match &step.kind {
            StepKind::Hosting(HostingStep::WebAppCreate) => ("web-app", None, None),
            StepKind::Hosting(HostingStep::SiteCreate) => ("site", None, None),
            StepKind::Hosting(HostingStep::SiteConfigure) => ("configure", None, None),
            StepKind::Hosting(HostingStep::VersionCreate) => ("create", None, None),
            StepKind::Hosting(HostingStep::Populate) => ("populate", None, None),
            StepKind::Hosting(HostingStep::Upload { path, hash }) => (
                "upload",
                Some(hash.as_str()),
                Some(self.compressed.get(path).ok_or_else(unknown)?.clone()),
            ),
            StepKind::Hosting(HostingStep::Finalize) => ("finalize", None, None),
            StepKind::Hosting(HostingStep::Release) => ("release", None, None),
            _ => return Err(unknown()),
        };
        let value = self.hosting_exchange(
            &format!("{operation}-hosting"),
            plan,
            action,
            hash,
            if operation == "apply" {
                bytes.as_deref()
            } else {
                None
            },
            deadline,
        )?;
        Ok(value["state"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn clipped_composite_read_failure_keeps_last_building_state() {
        let start = Instant::now();
        let ceiling = start + Duration::from_secs(180);
        let stop = AtomicBool::new(false);
        for error in [
            DeployProviderError::Unknown,
            DeployProviderError::Rejected(DeployFault::PermissionDenied),
        ] {
            let result = composite_observation(
                start + Duration::from_secs(179),
                ceiling,
                &stop,
                |deadline| {
                    assert_eq!(deadline, ceiling);
                    Err(error)
                },
            );
            assert_eq!(
                result,
                Ok(DeployObserved::Building),
                "the wait ceiling is not an unconfirmed mutation"
            );
            assert_eq!(
                composite_observation(start, ceiling, &stop, |deadline| {
                    assert_eq!(deadline, start + limits::DEPLOY_PROVIDER_CALL);
                    Err(error)
                }),
                Err(error),
                "a failure before the clipped final round stays a failure"
            );
        }
        assert_eq!(
            composite_observation(start + Duration::from_secs(179), ceiling, &stop, |_| {
                stop.store(true, Ordering::Release);
                Err(DeployProviderError::Unknown)
            }),
            Err(DeployProviderError::Unknown),
            "cancellation during the clipped read stays unknown"
        );
        assert_eq!(
            composite_observation(start, ceiling, &stop, |_| panic!(
                "stopped reads never start"
            )),
            Err(DeployProviderError::Unknown)
        );
        stop.store(false, Ordering::Release);
        assert_eq!(
            composite_observation(ceiling, ceiling, &stop, |_| panic!(
                "exhausted ceiling never starts a read"
            )),
            Ok(DeployObserved::Building)
        );
        for observed in [
            DeployObserved::Satisfied,
            DeployObserved::Adopted,
            DeployObserved::Building,
            DeployObserved::Absent,
        ] {
            assert_eq!(
                composite_observation(start + Duration::from_secs(179), ceiling, &stop, |_| Ok(
                    observed.clone()
                )),
                Ok(observed)
            );
        }
    }

    #[test]
    fn composite_wait_is_visible_bounded_and_cancellation_wakeable() {
        use std::cell::Cell;
        let start = Instant::now();
        let clock = Cell::new(start);
        let stop = AtomicBool::new(false);
        let mut elapsed = Vec::new();
        for round in 0..18 {
            elapsed.push(
                COMPOSITE_INDEX_WAIT
                    .wait_round(
                        start,
                        round,
                        &stop,
                        || clock.get(),
                        |delay| clock.set(clock.get() + delay),
                    )
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(COMPOSITE_INDEX_WAIT.ceiling(), Duration::from_secs(180));
        assert_eq!(
            elapsed,
            (0..18)
                .map(|round| Duration::from_secs(round * 10))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            COMPOSITE_INDEX_WAIT
                .wait_round(
                    start,
                    18,
                    &stop,
                    || clock.get(),
                    |_| panic!("ceiling never sleeps")
                )
                .unwrap(),
            None
        );
        // Slow reads consume the same ceiling, rather than adding another call budget.
        clock.set(start + Duration::from_secs(175));
        assert_eq!(
            COMPOSITE_INDEX_WAIT
                .wait_round(
                    start,
                    1,
                    &stop,
                    || clock.get(),
                    |delay| clock.set(clock.get() + delay)
                )
                .unwrap(),
            None
        );
        assert_eq!(clock.get(), start + Duration::from_secs(180));
        clock.set(start);
        assert_eq!(
            COMPOSITE_INDEX_WAIT.wait_round(
                start,
                1,
                &stop,
                || clock.get(),
                |delay| {
                    clock.set(clock.get() + delay);
                    stop.store(true, Ordering::Release);
                }
            ),
            Err(DeployProviderError::Unknown)
        );
        assert_eq!(clock.get() - start, Duration::from_millis(100));
    }

    #[test]
    fn strict_diagnostic_reply_keeps_fixed_codes_and_rejects_untrusted_evidence() {
        use crate::error::DeployOperation;
        for (code, expected) in [
            ("provider-rejected", "REMOTE_DEPLOY_PROVIDER_REJECTED"),
            ("permission-denied", "REMOTE_DEPLOY_PERMISSION_DENIED"),
            ("quota-exceeded", "REMOTE_DEPLOY_QUOTA_EXCEEDED"),
            ("unknown", "REMOTE_DEPLOY_UNAVAILABLE"),
        ] {
            let raw =
                json!({"version":1,"error":code,"provider":{"operation":"fields","status":400}});
            let failure = parse_reply(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
            let error = preflight_error(failure).remote_error();
            assert_eq!(error.code, expected);
            assert_eq!(error.provider.unwrap().operation, DeployOperation::Fields);
            assert_eq!(error.json_value()["error"]["provider"], raw["provider"]);
        }
        for provider in [
            json!({"operation":"https://secret.invalid/project", "status":400}),
            json!({"operation":"fields", "status":"400"}),
            json!({"operation":"fields", "status":200}),
            json!({"operation":"fields", "status":600}),
            json!({"operation":"fields", "status":400,"body":"TOKEN_CANARY"}),
        ] {
            let raw = json!({"version":1,"error":"provider-rejected","provider":provider});
            let failure = parse_reply(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
            assert_eq!(failure.code, "unknown");
            assert!(failure.provider.is_none());
        }
        for raw in [
            br#"{"version":1,"error":"provider-rejected","provider":{"operation":"fields","status":400,"status":404}}"#.as_slice(),
            br#"{"version":1,"result":null,"provider":{"operation":"fields","status":400}}"#.as_slice(),
        ] {
            let failure = parse_reply(raw).unwrap_err();
            assert_eq!(failure.code, "unknown");
            assert!(failure.provider.is_none());
        }
        let error =
            preflight_error(parse_reply(br#"{"version":1,"error":"login-required"}"#).unwrap_err())
                .remote_error();
        assert_eq!(error.code, "REMOTE_DEPLOY_PROVIDER_REJECTED");
        assert!(error.json_value()["error"].get("provider").is_none());
    }

    #[test]
    fn verify_gives_each_read_its_own_absolute_deadline_after_slow_prior_reads() {
        let steps: Vec<_> = [StepKind::Database, StepKind::Rules, StepKind::Verify]
            .into_iter()
            .enumerate()
            .map(|(i, kind)| DeployStep {
                id: i.to_string(),
                kind,
            })
            .collect();
        let start = Instant::now();
        let mut clock = start;
        let mut deadlines = Vec::new();
        let outcome = verify_steps(
            &steps,
            |step, deadline| {
                deadlines.push(deadline);
                Ok(if step.kind == StepKind::Rules {
                    DeployObserved::Rules(deploy_run::LiveRules::Current)
                } else {
                    DeployObserved::Adopted
                })
            },
            || {
                let current = clock;
                clock += Duration::from_secs(20);
                current
            },
        );
        assert_eq!(outcome.unwrap(), DeployObserved::Satisfied);
        assert_eq!(
            deadlines,
            [
                start + limits::DEPLOY_PROVIDER_CALL,
                start + Duration::from_secs(20) + limits::DEPLOY_PROVIDER_CALL
            ]
        );
    }
}

#[cfg(test)]
mod cause_tests {
    use super::*;
    #[test]
    fn child_exit_and_invalid_output_have_bounded_causes() {
        let stop = AtomicBool::new(false);
        for (program, expected) in [
            ("/bin/false", CauseCode::ChildFailed),
            ("/bin/echo", CauseCode::OutputInvalid),
        ] {
            let mut adapter = DeployFirestore {
                node: program.into(),
                package: "/unused".into(),
                stop: &stop,
                composite_progress: None,
                composite_started: None,
                hosting: None,
                compressed: Default::default(),
                steps: Vec::new(),
            };
            let error = adapter
                .exchange(
                    "apply-hosting",
                    json!({}),
                    None,
                    Instant::now() + limits::DEPLOY_PROVIDER_CALL,
                )
                .unwrap_err();
            assert_eq!(
                error.cause,
                Some(StepCause::new(expected, CausePhase::ChildExit))
            );
            assert!(matches!(
                provider_error(error),
                DeployProviderError::Unconfirmed(_)
            ));
        }
    }

    #[test]
    fn helper_causes_are_closed_and_preserved_without_provider_text() {
        let failure = parse_reply(br#"{"version":1,"error":"unknown","cause":{"code":"deadline","phase":"body","httpStatusClass":"2xx"}}"#).unwrap_err();
        assert!(matches!(
            provider_error(failure),
            DeployProviderError::Unconfirmed(StepCause {
                code: CauseCode::Deadline,
                phase: CausePhase::Body,
                ..
            })
        ));
        for bytes in [
            br#"{"version":1,"error":"unknown","cause":{"code":"SECRET","phase":"body"}}"#.as_slice(),
            br#"{"version":1,"error":"unknown","cause":{"code":"deadline","phase":"body","message":"SECRET"}}"#.as_slice(),
        ] {
            let failure = parse_reply(bytes).unwrap_err();
            assert!(failure.cause.is_none());
            assert!(!format!("{failure:?}").contains("SECRET"));
        }
    }
}
