//! Deployment plan/authorization/output owner over captured inputs and a provider port.
//! The CLI composition owns installed discovery; this owner never retries an effect.
use crate::{
    deploy_plan::Plan,
    deploy_record::DeployRecordStore,
    deploy_run::{
        self, DeployError, DeployInput, DeployPort, DeployRecord, DeployRefusal, SignInProvider,
    },
    error::RemoteError,
};
use serde_json::{Value, json};
use std::fmt::Write;

/// Approved declaration/Rules bytes and read-only live Rules supplied by the caller.
/// The account is always resolved by the injected provider, never supplied as a flag.
pub struct DeployCommandInput<'a> {
    pub extensions: &'a Plan,
    pub project: &'a str,
    pub location: &'a str,
    pub sign_in: &'a [SignInProvider],
    pub rules_body: &'a [u8],
    pub live_rules: Option<&'a [u8]>,
}
/// Flag-only authorization; None is plan-only, never an implicit prompt or yes.
#[derive(Default)]
pub struct DeployCommandOptions<'a> {
    pub authorize: Option<&'a str>,
    pub replace_rules: Option<&'a str>,
}
#[derive(Debug)]
pub enum DeployCommandError {
    Record(RemoteError),
    Refused(DeployRefusal),
    Run(DeployError),
}
impl From<RemoteError> for DeployCommandError {
    fn from(error: RemoteError) -> Self {
        Self::Record(error)
    }
}
/// Human and JSON projections of the same plan and actual saved outcome.
pub struct DeployCommandOutput {
    pub json: Value,
    pub human: String,
}

/// Prepare the exact plan, retain its draft identity and optionally execute only its
/// explicit authorization. The caller holds one record owner throughout this call.
pub fn execute(
    input: &DeployCommandInput<'_>,
    options: &DeployCommandOptions<'_>,
    store: &mut DeployRecordStore<'_>,
    port: &mut dyn DeployPort,
    now_ms: u64,
) -> Result<DeployCommandOutput, DeployCommandError> {
    let record = store.load_or_draft()?;
    let account = port
        .account()
        .map_err(|_| DeployCommandError::Refused(DeployRefusal::AccountUnreadable))?;
    if account.is_empty() || account.len() > 1024 || account.chars().any(char::is_control) {
        return Err(DeployCommandError::Refused(
            DeployRefusal::AccountUnreadable,
        ));
    }
    let plan = deploy_run::prepare(
        input.extensions,
        &DeployInput {
            account: &account,
            project: input.project,
            deployment_id: &record.deployment_id,
            location: input.location,
            sign_in: input.sign_in,
            rules_body: input.rules_body,
            live_rules: input.live_rules,
        },
    )
    .map_err(DeployCommandError::Refused)?;
    let record = match options.authorize {
        None => {
            if options.replace_rules.is_some() {
                return Err(DeployCommandError::Refused(
                    DeployRefusal::AuthorizationStale,
                ));
            }
            store.persist(&record)?;
            record
        }
        Some(typed) => {
            let authorization = deploy_run::authorize(&plan, typed, options.replace_rules)
                .map_err(DeployCommandError::Refused)?;
            deploy_run::run(&plan, &authorization, record, port, store, now_ms)
                .map_err(DeployCommandError::Run)?
        }
    };
    let authorized = options.authorize.is_some();
    let json = json!({ "authorized": authorized, "plan": plan.view(), "planDigest": plan.digest(),
        "extensions": input.extensions.view(), "record": record });
    let mut human = format!(
        "Firestore sharing plan\nAccount: {}\nProject: {}\nDeployment: {}\nDatabase: {} ({}), {}\nSign-in: {}\nRules: {} (replaces {})\nIndex configs: {}\nRoles: none created\nPhysical TTL: not provisioned\nPlan digest: {}\n",
        plan.account(),
        input.project,
        plan.deployment_id(),
        plan.view().database.name,
        plan.view().database.edition,
        input.location,
        plan.view().sign_in.join(", "),
        plan.view().rules.digest,
        plan.view().rules.replaces,
        plan.view().index_configs,
        plan.digest()
    );
    if let Some(replaced) = &plan.view().rules.replaced_digest {
        writeln!(human, "Replaced Rules digest: {replaced}").expect("String write");
    }
    for item in &plan.view().destructive {
        writeln!(human, "Destructive change: {item}").expect("String write");
    }
    human.push_str("Extensions and resources:\n");
    for extension in &input.extensions.view().extensions {
        writeln!(
            human,
            "{} (declaration {})",
            extension.name, extension.declaration_digest
        )
        .expect("String write");
        writeln!(
            human,
            "  Admission: {} (artifact {})",
            extension.admission.entry_point, extension.admission.artifact_digest
        )
        .expect("String write");
        for resource in &extension.resources {
            writeln!(
                human,
                "  {}: {} at {}; object {} B, namespace {} B, entries {}, TTL {}",
                resource.name,
                resource.kind,
                resource.path,
                resource.limits.max_object_bytes,
                resource.limits.max_namespace_bytes,
                resource.limits.max_entries,
                resource.ttl
            )
            .expect("String write");
            for index in &resource.indexes {
                writeln!(human, "    Index: {} {}", index.field, index.direction)
                    .expect("String write");
            }
        }
    }
    for unavailable in &input.extensions.view().unavailable {
        writeln!(
            human,
            "{}: unavailable ({})",
            unavailable.name, unavailable.reason
        )
        .expect("String write");
    }
    if authorized {
        describe_record(&mut human, &record);
    } else {
        human.push_str("Not authorized; nothing changed in your Firebase project.\nAuthorize this exact plan with --authorize <plan-digest-prefix>.\n");
    }
    Ok(DeployCommandOutput { json, human })
}
fn describe_record(text: &mut String, record: &DeployRecord) {
    let run = record.run.as_ref().expect("authorized run");
    writeln!(
        text,
        "Run: {}; usable binding: {}",
        match run.state {
            deploy_run::RunState::Applying => "applying",
            deploy_run::RunState::Complete => "complete",
            deploy_run::RunState::Partial => "partial",
        },
        record.usable_binding().is_some()
    )
    .expect("String write");
    for step in &run.steps {
        let state = match step.state {
            deploy_run::StepState::Pending => "pending",
            deploy_run::StepState::Done => "done",
            deploy_run::StepState::Adopted => "adopted",
            deploy_run::StepState::Unknown => "unknown",
            deploy_run::StepState::Building => "building",
            deploy_run::StepState::Failed(fault) => fault.code(),
            deploy_run::StepState::OwnerAction(action) => action.code(),
        };
        writeln!(text, "{}: {}", step.id, state).expect("String write");
        if let deploy_run::StepState::OwnerAction(action) = step.state {
            writeln!(text, "{}: {}", action.code(), action.instruction()).expect("String write");
        }
    }
}
