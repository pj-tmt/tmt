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
    store: &mut DeployRecordStore,
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
    store: &mut DeployRecordStore,
    port: &mut dyn DeployPort,
    now_ms: u64,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
) -> Result<DeployCommandOutput, DeployCommandError> {
    let prepared = prepare_with_hosting(input, store, port, hosting)?;
    let authorization = options
        .authorize
        .map(|typed| deploy_run::authorize(&prepared.plan, typed))
        .transpose()
        .map_err(DeployCommandError::Refused)?;
    execute_prepared(input, prepared, authorization, store, port, now_ms, hosting)
}

/// Immutable inputs retained across the terminal's single decision.
pub struct PreparedDeploy {
    plan: deploy_run::DeployPlan,
    record: DeployRecord,
}
impl PreparedDeploy {
    pub fn plan(&self) -> &deploy_run::DeployPlan {
        &self.plan
    }
    pub fn record(&self) -> &DeployRecord {
        &self.record
    }
    pub fn preview(
        &self,
        input: &DeployCommandInput<'_>,
        store: &mut DeployRecordStore,
        hosting: Option<&crate::hosting::HostingDeploymentView>,
    ) -> Result<DeployCommandOutput, DeployCommandError> {
        store.persist(&self.record)?;
        Ok(render(
            input,
            self,
            &self.record,
            false,
            store,
            hosting,
            false,
        ))
    }
}
pub fn prepare_with_hosting(
    input: &DeployCommandInput<'_>,
    store: &mut DeployRecordStore,
    port: &mut dyn DeployPort,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
) -> Result<PreparedDeploy, DeployCommandError> {
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
    Ok(PreparedDeploy { plan, record })
}
pub fn execute_prepared(
    input: &DeployCommandInput<'_>,
    prepared: PreparedDeploy,
    authorization: Option<deploy_run::Authorization>,
    store: &mut DeployRecordStore,
    port: &mut dyn DeployPort,
    now_ms: u64,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
) -> Result<DeployCommandOutput, DeployCommandError> {
    let plan = &prepared.plan;
    let record = prepared.record.clone();
    let authorized = authorization.is_some();
    let hint = !authorization.as_ref().is_some_and(|a| a.interactive());
    let record = match authorization {
        None => {
            store.persist(&record)?;
            record
        }
        Some(authorization) => {
            if hosting.is_some() && !port.supports_hosting() {
                return Err(DeployCommandError::Refused(
                    DeployRefusal::HostingUnavailable,
                ));
            }
            store.bind_target(input.project, input.location)?;
            deploy_run::run(plan, &authorization, record, port, store, now_ms)
                .map_err(DeployCommandError::Run)?
        }
    };

    Ok(render(
        input, &prepared, &record, authorized, store, hosting, hint,
    ))
}
fn render(
    input: &DeployCommandInput<'_>,
    prepared: &PreparedDeploy,
    record: &DeployRecord,
    authorized: bool,
    store: &DeployRecordStore,
    hosting: Option<&crate::hosting::HostingDeploymentView>,
    hint: bool,
) -> DeployCommandOutput {
    let plan = &prepared.plan;
    let superseded = deploy_run::superseded_run(plan, &prepared.record).map(|run| {
        (
            run.plan_digest.clone(),
            deploy_run::supersession_refusal(plan, &prepared.record),
        )
    });
    let mut json = json!({ "authorized": authorized, "plan": plan.view(), "planDigest": plan.digest(),
        "extensions": input.extensions.view(), "record": record });
    let mut human = format!(
        "Firestore sharing plan\nAccount: {}\nProject: {}\nDatabase: {} ({}), {}\nSign-in: {}\nRules: {} ({})\nIndex configs: {}\nRoles: none created\nTTL: not set up\nPlan digest: {}\n",
        plan.account(),
        input.project,
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
        writeln!(human, "DESTRUCTIVE: Replace the live Rules for project {}. This affects every tenant using its Rules.\nExisting Rules fingerprint: {}", input.project, &replaced[..12]).expect("String write");
        if hint {
            writeln!(
                human,
                "Authorizing plan {} allows this replacement.",
                &plan.digest()[..12]
            )
            .expect("String write");
        } else {
            human.push_str("Answering y replaces them.\n");
        }
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
            for index in &resource.composite_indexes {
                let fields = index
                    .fields
                    .iter()
                    .map(|field| format!("{} asc", field.field_path))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    human,
                    "    Composite index: {} ({fields})",
                    index.collection_group
                )
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
    if let Some((digest, refusal)) = superseded {
        let next = refusal.map(|reason| supersession_message(reason, &store.record_path()));
        json["supersededRun"] = json!({"planDigest":digest,"safe":refusal.is_none(),"reason":refusal.map(|r| r.code()),"next":next});
        if hint {
            human.push_str("An unfinished run from an older plan exists. ");
            if let Some(next) = next {
                human.push_str(&next);
            } else {
                human.push_str(
                    "Authorizing this plan supersedes it and keeps its recorded outcomes.",
                );
            }
            human.push_str(" Nothing has been undone.\n");
        }
    }
    if authorized {
        if !hint {
            human.clear();
        }
        describe_record(&mut human, record, &store.record_path());
    } else if hint {
        writeln!(human, "Not authorized; nothing changed in your Firebase project.\nTo deploy this plan{}, run the same command with --authorize {}", if plan.view().rules.replaced_digest.is_some() { " and replace the live Rules" } else { "" }, &plan.digest()[..12]).expect("String write");
    }
    if authorized && let Ok(link) = crate::remote_link::from_record(record) {
        writeln!(human, "Remote link: {link}").expect("String write");
        json["remoteLink"] = Value::String(link);
    }
    DeployCommandOutput { json, human }
}
/// One recovery instruction for both an unknown creation and a refused replacement.
pub fn unconfirmed_creation_message(path: &std::path::Path) -> String {
    format!(
        "Firebase did not confirm the original creation; tmt will not repeat it. Rerun the same command to observe its result. If it is still unconfirmed, inspect its cause and the Firebase state. Keep {} so the original creation can be identified; do not delete it to bypass this check.",
        path.display()
    )
}
pub fn supersession_message(reason: DeployRefusal, path: &std::path::Path) -> String {
    match reason {
        DeployRefusal::PreviousCreateUnconfirmed => unconfirmed_creation_message(path),
        DeployRefusal::PreviousVersionStaged => "An earlier run has a staged Hosting version. Keep the saved record and restore the earlier extension build to finish that plan. An unfinalized version at least 12 hours old can instead be left unreleased by authorizing the displayed abandonment plan.".into(),
        _ => "The plan or its authorization was refused. Nothing changed in your Firebase project. Run without --authorize to read the current plan, then authorize its digest.".into(),
    }
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
fn describe_cause(text: &mut String, cause: Option<&deploy_run::StepCause>, label: &str) {
    if let Some(cause) = cause {
        let value = serde_json::to_value(cause).expect("fixed step cause serializes");
        write!(
            text,
            "{label}: {} (phase {}",
            value["code"].as_str().expect("cause code"),
            value["phase"].as_str().expect("cause phase")
        )
        .expect("String write");
        if let Some(class) = value["httpStatusClass"].as_str() {
            write!(text, "; HTTP {class}").expect("String write");
        }
        text.push_str(").\n");
        for sample in cause.file_samples.iter().flatten() {
            let row = serde_json::to_value(sample).expect("bounded sample serializes");
            writeln!(
                text,
                "  File: {}; {}; hash {}; {}.",
                sample.path,
                row["status"]
                    .as_str()
                    .expect("sample status")
                    .to_ascii_lowercase(),
                sample.hash_prefix,
                row["relation"].as_str().expect("sample relation")
            )
            .expect("String write");
        }
        if let (Some(files), Some(samples)) = (&cause.files, &cause.file_samples) {
            let shown = samples
                .iter()
                .filter(|sample| sample.relation != deploy_run::FileRelation::Exact)
                .count();
            let remaining = usize::from(files.extra).saturating_sub(shown);
            if remaining > 0 {
                writeln!(text, "  …and {remaining} more files.").expect("String write");
            }
        }
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
            deploy_run::RunState::Superseded => "superseded",
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
            describe_cause(text, step.cause.as_ref(), "Hosting upload cause");
            continue;
        }
        writeln!(text, "{}: {}", step.id, state).expect("String write");
        describe_cause(text, step.cause.as_ref(), "Cause");
        if step.id == "web-app:create" && step.state == deploy_run::StepState::Building {
            text.push_str(
                "Firebase is still creating the web app. Rerun the same command to check it.\n",
            );
        }
        if matches!(step.id.as_str(), "web-app:create" | "hosting:stage:create")
            && step.state == deploy_run::StepState::Unknown
        {
            writeln!(text, "{}", unconfirmed_creation_message(record_path)).expect("String write");
        }
        if let deploy_run::StepState::OwnerAction(action) = step.state {
            writeln!(text, "{}: {}", action.code(), action.instruction()).expect("String write");
        }
    }
    let composites: Vec<_> = run
        .steps
        .iter()
        .filter(|step| step.id.starts_with("composite-index:"))
        .collect();
    if let Some(index) = composites
        .iter()
        .position(|step| step.state == deploy_run::StepState::Building)
    {
        writeln!(
            text,
            "Index {} of {} is still building. Rerun the same command in a minute.",
            index + 1,
            composites.len()
        )
        .expect("String write");
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
    fn composite_round_copy_displays_elapsed_and_shared_ceiling() {
        assert_eq!(
            crate::deploy_cli::composite_progress(
                std::time::Duration::from_secs(20),
                std::time::Duration::from_secs(180)
            ),
            "Checking composite indexes (20 s elapsed; up to 180 s)."
        );
    }

    #[test]
    fn known_building_composite_names_its_ordinal_and_rerun() {
        let mut record = DeployRecord::new("fixture");
        record.run = Some(Run {
            plan_digest: "a".repeat(64),
            account: "fixture".into(),
            authorized_at_ms: 0,
            state: RunState::Partial,
            rules_attempted: false,
            hosting: None,
            steps: (1..=4)
                .map(|n| StepRecord {
                    cause: None,
                    id: format!("composite-index:{n}"),
                    state: if n == 2 {
                        StepState::Building
                    } else {
                        StepState::Done
                    },
                })
                .collect(),
        });
        let mut text = String::new();
        describe_record(
            &mut text,
            &record,
            std::path::Path::new("/fixture/deploy.json"),
        );
        assert!(
            text.contains("Index 2 of 4 is still building. Rerun the same command in a minute.")
        );
        assert!(!text.contains("could not be confirmed"));
    }

    #[test]
    fn human_cause_line_stays_unchanged_with_file_counts() {
        let cause = serde_json::from_str(r#"{"code":"files-mismatch","phase":"list","httpStatusClass":"2xx","files":{"planned":15,"expected":10,"active":4,"listed":14,"missing":1,"extra":0}}"#).unwrap();
        let mut text = String::new();
        describe_cause(&mut text, Some(&cause), "Cause");
        assert_eq!(text, "Cause: files-mismatch (phase list; HTTP 2xx).\n");
    }

    #[test]
    fn human_file_samples_use_only_the_closed_public_diagnostic() {
        let cause = serde_json::from_str(r#"{"code":"files-mismatch","phase":"list","httpStatusClass":"2xx","fileSamples":[{"path":"/404.html","status":"ACTIVE","hashPrefix":"abcdef01","relation":"path-missing"},{"path":"<redacted>","status":"EXPECTED","hashPrefix":"01234567","relation":"hash-different"}]}"#).unwrap();
        let mut text = String::new();
        describe_cause(&mut text, Some(&cause), "Cause");
        assert_eq!(
            text,
            "Cause: files-mismatch (phase list; HTTP 2xx).\n  File: /404.html; active; hash abcdef01; path-missing.\n  File: <redacted>; expected; hash 01234567; hash-different.\n"
        );
    }

    #[test]
    fn human_file_samples_count_only_unshown_non_exact_rows_without_changing_json() {
        let mut value = serde_json::json!({"code":"files-mismatch","phase":"list","files":{"planned":15,"expected":0,"active":7,"listed":7,"missing":5,"extra":5},"fileSamples":[
            {"path":"/404.html","status":"ACTIVE","hashPrefix":"abcdef01","relation":"path-missing"},
            {"path":"/index.html","status":"ACTIVE","hashPrefix":"01234567","relation":"hash-different"},
            {"path":"/same.html","status":"EXPECTED","hashPrefix":"12345678","relation":"exact"}
        ]});
        for (extra, tail) in [(5, Some("  …and 3 more files.\n")), (2, None), (1, None)] {
            value["files"]["extra"] = extra.into();
            let cause: deploy_run::StepCause = serde_json::from_value(value.clone()).unwrap();
            let mut text = String::new();
            describe_cause(&mut text, Some(&cause), "Cause");
            if let Some(tail) = tail {
                assert!(text.ends_with(tail), "{text}");
            } else {
                assert!(!text.contains("more files"), "{text}");
            }
            assert_eq!(serde_json::to_value(&cause).unwrap(), value);
        }
    }

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
                    cause: None,
                    id: "web-app:create".into(),
                    state: StepState::Unknown,
                },
                StepRecord {
                    cause: Some(deploy_run::StepCause {
                        code: deploy_run::CauseCode::Deadline,
                        phase: deploy_run::CausePhase::Body,
                        http_status_class: Some(deploy_run::HttpStatusClass::Success),
                        files: None,
                        file_samples: None,
                    }),
                    id: "hosting:stage:create".into(),
                    state: StepState::Unknown,
                },
                StepRecord {
                    cause: None,
                    id: format!("hosting:stage:upload:{}", "a".repeat(64)),
                    state: StepState::Done,
                },
                StepRecord {
                    cause: None,
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
        assert!(text.contains("Keep /fixture/remote/deploy.json"));
        assert!(text.contains("Cause: deadline (phase body; HTTP 2xx)."));
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
