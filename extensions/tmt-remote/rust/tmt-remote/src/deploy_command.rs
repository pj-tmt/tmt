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
    execute_with_hosting(input, options, store, port, now_ms, None)
}
/// The same command owner with a captured Hosting plan; unsupported ports refuse
/// execution rather than falling through to a Rules-only run.
pub fn execute_with_hosting(
    input: &DeployCommandInput<'_>,
    options: &DeployCommandOptions<'_>,
    store: &mut DeployRecordStore<'_>,
    port: &mut dyn DeployPort,
    now_ms: u64,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
) -> Result<DeployCommandOutput, DeployCommandError> {
    store.check_target(input.project, input.location)?;
    let record = store.load_or_draft()?;
    let account = port
        .account()
        .map_err(|_| DeployCommandError::Refused(DeployRefusal::AccountUnreadable))?;
    if account.is_empty() || account.len() > 1024 || account.chars().any(char::is_control) {
        return Err(DeployCommandError::Refused(
            DeployRefusal::AccountUnreadable,
        ));
    }
    let plan = deploy_run::prepare_with_hosting(
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
        hosting,
    )
    .map_err(DeployCommandError::Refused)?;
    let plan =
        deploy_run::retain_hosting_plan(plan, &record).map_err(DeployCommandError::Refused)?;
    let record = match options.authorize {
        None => {
            store.persist(&record)?;
            record
        }
        Some(typed) => {
            let authorization =
                deploy_run::authorize(&plan, typed).map_err(DeployCommandError::Refused)?;
            if hosting.is_some() && !port.supports_hosting() {
                return Err(DeployCommandError::Refused(
                    DeployRefusal::HostingUnavailable,
                ));
            }
            store.bind_target(input.project, input.location)?;
            deploy_run::run(&plan, &authorization, record, port, store, now_ms)
                .map_err(DeployCommandError::Run)?
        }
    };
    let authorized = options.authorize.is_some();
    let mut json = json!({ "authorized": authorized, "plan": plan.view(), "planDigest": plan.digest(),
        "extensions": input.extensions.view(), "record": record });
    let mut human = format!(
        "Firestore sharing plan\nAccount: {}\nProject: {}\nDeployment: {}\nDatabase: {} ({}), {}\nSign-in: {}\nRules: {} ({})\nIndex configs: {}\nRoles: none created\nTTL: not set up\nPlan digest: {}\n",
        plan.account(),
        input.project,
        plan.deployment_id(),
        plan.view().database.name,
        plan.view().database.edition,
        input.location,
        plan.view().sign_in.join(", "),
        &plan.view().rules.digest[..12],
        if plan.view().rules.replaces == "none" {
            "no existing Rules"
        } else if plan.view().rules.replaces == "foreign" {
            "replaces the live Rules"
        } else {
            &plan.view().rules.replaces
        },
        plan.view().index_configs,
        &plan.digest()[..12]
    );
    if let Some(replaced) = &plan.view().rules.replaced_digest {
        writeln!(human, "DESTRUCTIVE: Replace the live Rules for project {}. This affects every tenant using its Rules.\nExisting Rules fingerprint: {}\nAuthorizing plan {} allows this replacement.", input.project, &replaced[..12], &plan.digest()[..12]).expect("String write");
    }
    if let Some(hosting) = hosting {
        writeln!(
            human,
            "Hosting: {}\nPublic URL: {}\nCreate site: {}\nCreate web app: {}\nPublic files:",
            hosting.site,
            hosting.public_url,
            if hosting.create_site { "yes" } else { "no" },
            if hosting.create_web_app { "yes" } else { "no" }
        )
        .expect("String write");
        writeln!(
            human,
            "Link Hosting site to the selected web app: {}",
            if hosting.configure_site { "yes" } else { "no" }
        )
        .expect("String write");
        for file in hosting.content["files"]
            .as_array()
            .expect("validated content files")
        {
            writeln!(
                human,
                "  {} ({} raw bytes, {} gzip bytes)",
                file["path"].as_str().expect("path"),
                file["rawLength"],
                file["gzipLength"]
            )
            .expect("String write");
        }
        if let Some(fingerprint) = &hosting.replaced_fingerprint {
            writeln!(human, "DESTRUCTIVE: Replace the live Hosting release for project {}. This replaces public content for every tenant.\nExisting Hosting fingerprint: {}", input.project, &fingerprint[..12]).expect("String write");
        }
    }
    for item in plan.view().destructive.iter().filter(|item| {
        !item.starts_with("replaces-rules:") && !item.starts_with("replaces-hosting:")
    }) {
        writeln!(human, "Destructive change: {item}").expect("String write");
    }
    human.push_str("Extensions and resources:\n");
    for extension in &input.extensions.view().extensions {
        writeln!(
            human,
            "{} (declaration {})",
            extension.name,
            &extension.declaration_digest[..12]
        )
        .expect("String write");
        writeln!(
            human,
            "  Admission: {} (artifact {})",
            extension.admission.entry_point,
            &extension.admission.artifact_digest[..12]
        )
        .expect("String write");
        for resource in &extension.resources {
            writeln!(
                human,
                "  {}: {} at {}; object {}, namespace {}, entries {}, TTL: {}",
                resource.name,
                resource.kind,
                resource.path,
                readable_bytes(resource.limits.max_object_bytes),
                readable_bytes(resource.limits.max_namespace_bytes),
                resource.limits.max_entries,
                if resource.ttl == "none" {
                    "none"
                } else {
                    "not set up"
                }
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
        describe_record(&mut human, &record, &store.record_path());
    } else {
        writeln!(human, "Not authorized; nothing changed in your Firebase project.\nTo deploy this plan{}, run the same command with --authorize {}", if plan.view().rules.replaced_digest.is_some() { " and replace the live Rules" } else { "" }, &plan.digest()[..12]).expect("String write");
    }
    if authorized && let Ok(link) = crate::remote_link::from_record(&record) {
        writeln!(human, "Remote link: {link}").expect("String write");
        json["remoteLink"] = Value::String(link);
    }
    Ok(DeployCommandOutput { json, human })
}
fn readable_bytes(bytes: u64) -> String {
    let (unit, divisor) = if bytes >= 1024 * 1024 {
        ("MiB", 1024 * 1024)
    } else if bytes >= 1024 {
        ("KiB", 1024)
    } else {
        return format!("{bytes} B");
    };
    if bytes.is_multiple_of(divisor) {
        format!("{} {unit}", bytes / divisor)
    } else {
        format!("{:.2} {unit}", bytes as f64 / divisor as f64)
    }
}
fn describe_record(text: &mut String, record: &DeployRecord, record_path: &std::path::Path) {
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
    let mut uploads = std::collections::BTreeMap::new();
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
        if step.id.starts_with("hosting:stage:upload:") {
            *uploads
                .entry(if state == "unknown" {
                    "unconfirmed"
                } else {
                    state
                })
                .or_insert(0usize) += 1;
            continue;
        }
        writeln!(text, "{}: {}", step.id, state).expect("String write");
        if step.id == "web-app:create" && step.state == deploy_run::StepState::Building {
            text.push_str(
                "Firebase is still creating the web app. Rerun the same command to check it.\n",
            );
        }
        if matches!(step.id.as_str(), "web-app:create" | "hosting:stage:create")
            && step.state == deploy_run::StepState::Unknown
        {
            writeln!(text, "Firebase did not confirm the original creation; tmt will not repeat it. Rerun the same command to check its result. If it is still unconfirmed, check the Firebase project, delete {}, then run without --authorize to read a fresh plan. Existing Firebase resources stay; authorizing a new plan may create a second one.", record_path.display()).expect("String write");
        }
        if let deploy_run::StepState::OwnerAction(action) = step.state {
            writeln!(text, "{}: {}", action.code(), action.instruction()).expect("String write");
        }
    }
    if !uploads.is_empty() {
        let summary = uploads
            .into_iter()
            .map(|(state, n)| format!("{n} {state}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(text, "Hosting file uploads: {summary}.").expect("String write");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deploy_run::{Run, RunState, StepRecord, StepState};
    #[test]
    fn human_progress_collapses_uploads_and_names_the_original_create_exit() {
        let mut record = DeployRecord::new("fixture");
        record.run = Some(Run {
            plan_digest: "a".repeat(64),
            account: "fixture".into(),
            authorized_at_ms: 0,
            state: RunState::Partial,
            rules_attempted: false,
            hosting: None,
            steps: vec![
                StepRecord {
                    id: "web-app:create".into(),
                    state: StepState::Unknown,
                },
                StepRecord {
                    id: "hosting:stage:create".into(),
                    state: StepState::Unknown,
                },
                StepRecord {
                    id: format!("hosting:stage:upload:{}", "a".repeat(64)),
                    state: StepState::Done,
                },
                StepRecord {
                    id: format!("hosting:stage:upload:{}", "b".repeat(64)),
                    state: StepState::Unknown,
                },
            ],
        });
        let mut text = String::new();
        describe_record(
            &mut text,
            &record,
            std::path::Path::new("/fixture/remote/deploy.json"),
        );
        assert!(text.contains("Hosting file uploads: 1 done, 1 unconfirmed."));
        assert!(!text.contains("hosting:stage:upload:"));
        assert_eq!(
            text.matches("Firebase did not confirm the original creation; tmt will not repeat it.")
                .count(),
            2
        );
        assert!(text.contains("delete /fixture/remote/deploy.json, then run without --authorize"));
        record.run.as_mut().unwrap().steps[0].state = StepState::Building;
        let mut pending = String::new();
        describe_record(
            &mut pending,
            &record,
            std::path::Path::new("/fixture/remote/deploy.json"),
        );
        assert!(pending.contains(
            "Firebase is still creating the web app. Rerun the same command to check it."
        ));
    }
}
