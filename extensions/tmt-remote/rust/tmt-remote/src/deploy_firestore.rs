//! Firestore and Hosting provider adapter. The embedded helper owns the installed
//! firebase-tools login; the existing deploy engine owns authorization and recovery.
//! The CLI composition wires it; verified publication does not enable cloud routes.
use crate::{
    deploy_run::{
        self, DeployApplied, DeployFault, DeployObserved, DeployOwnerAction, DeployPlan,
        DeployPort, DeployProviderError, DeployStep, StepKind,
    },
    limits, wire,
};
use serde_json::{Value, json};
use std::{ffi::OsString, fmt, path::PathBuf, sync::atomic::AtomicBool, time::Instant};
use tmt_invoke::{EnvironmentPolicy, LaunchOptions, Request};

const HELPER: &str = include_str!("deploy_firestore/helper.cjs");
/// Versions whose private auth/layout interface this release has qualified.
pub const FIREBASE_TOOLS_VERSIONS: &[&str] = &["15.29.0"];

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

/// Uses one waited child per bounded request. Helper bytes are embedded in this binary,
/// never loaded from cwd. Package paths select the explicitly installed external tool.
pub struct DeployFirestore<'a> {
    node: PathBuf,
    package: PathBuf,
    stop: &'a AtomicBool,
    hosting: Option<deploy_run::HostingCheckpoint>,
    compressed: std::collections::BTreeMap<String, Vec<u8>>,
    steps: Vec<deploy_run::StepRecord>,
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
            != json!({"versions":FIREBASE_TOOLS_VERSIONS,"maxBytes":limits::DEPLOY_PROVIDER_BYTES,"maxPages":limits::DEPLOY_PROVIDER_PAGES})
        {
            return Err(DeploySetupError::UnsupportedTool);
        }
        Ok(adapter)
    }
    pub fn environment_names() -> [OsString; 3] {
        ["HOME", "PATH", "XDG_CONFIG_HOME"].map(OsString::from)
    }
    /// Read-only plan inputs; no deployment permission is implied by a successful read.
    pub fn live_rules(&mut self, project: &str) -> Result<Option<Vec<u8>>, DeployProviderError> {
        let value = self
            .exchange(
                "live-rules",
                json!({"project":project}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(provider_error)?;
        rules_bytes(&value)
    }
    /// Read-only live+planned field-config ceiling, including unrelated configurations.
    pub fn check_index_budget(
        &mut self,
        project: &str,
        plan: &crate::deploy_plan::Plan,
    ) -> Result<(), DeployProviderError> {
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
                json!({"project": project, "fields": fields}),
                None,
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
            )
            .map_err(provider_error)?;
        if value != "within-budget" {
            return Err(unknown());
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
    ) -> Result<Value, String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("unknown")?;
        let request = json!({"version":1,"operation":operation,"input":input,"account":account,
            "budgetMs":remaining.as_millis().min(30_000)});
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
        .map_err(|_| "unknown")?;
        // Never expose exception text, stderr, a failed child's JSON, or its exit cause.
        if !output.status.success() {
            return Err("unknown".into());
        }
        if output.stdout.len() > limits::DEPLOY_PROVIDER_BYTES {
            return Err("unknown".into());
        }
        let value = wire::strict_json(&output.stdout).ok_or("unknown")?;
        let object = value.as_object().ok_or("unknown")?;
        if object.len() != 2 || object.get("version") != Some(&json!(1)) {
            return Err("unknown".into());
        }
        if let Some(result) = object.get("result") {
            return Ok(result.clone());
        }
        let code = object
            .get("error")
            .and_then(Value::as_str)
            .ok_or("unknown")?;
        // Only fixed codes cross this process boundary. All other strings are discarded.
        match code {
            "unsupported-tool" | "login-required" | "account-changed" | "permission-denied"
            | "api-disabled" | "quota-exceeded" | "database-mismatch" | "rules-foreign"
            | "verify-failed" | "provider-rejected" | "unknown" => Err(code.into()),
            _ => Err("unknown".into()),
        }
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
                Instant::now() + limits::DEPLOY_PROVIDER_CALL,
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
fn setup_error(code: String) -> DeploySetupError {
    match code.as_str() {
        "unsupported-tool" => DeploySetupError::UnsupportedTool,
        "login-required" => DeploySetupError::LoginRequired,
        _ => DeploySetupError::Unavailable,
    }
}
fn provider_error(code: String) -> DeployProviderError {
    let fault = match code.as_str() {
        "permission-denied" => DeployFault::PermissionDenied,
        "api-disabled" => DeployFault::ApiDisabled,
        "quota-exceeded" => DeployFault::QuotaExceeded,
        "database-mismatch" => DeployFault::DatabaseMismatch,
        "rules-foreign" => DeployFault::RulesForeign,
        "verify-failed" => return unknown(),
        "provider-rejected" | "unsupported-tool" | "login-required" | "account-changed" => {
            DeployFault::ProviderRejected
        }
        _ => return unknown(),
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
            let parsed: deploy_run::HostingCheckpoint =
                serde_json::from_value(state.clone()).map_err(|_| unknown())?;
            if parsed.envelope != checkpoint.envelope {
                return Err(unknown());
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
