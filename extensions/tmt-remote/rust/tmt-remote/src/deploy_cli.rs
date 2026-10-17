//! Deployment argv and composition through the installed adapter seam;
//! fixtures inject declaration/provider ports, never an alternate deploy algorithm.
use crate::{
    deploy_command::{self, DeployCommandInput, DeployCommandOutput},
    deploy_discovery::{self, DeclarationSource, DiscoveryRefusal},
    deploy_record::DeployRecordStore,
    deploy_run::{DeployPort, SignInProvider},
};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::json;

#[derive(Debug, PartialEq, Eq)]
pub struct FirestoreArgs {
    pub project: String,
    pub remote: Option<String>,
    pub region: String,
    pub sign_in: Vec<SignInProvider>,
    pub authorize: Option<String>,
    pub json: bool,
    pub details: bool,
}
#[derive(Debug)]
pub enum DeployCliError {
    Usage,
    Tool(crate::deploy_tools::ToolDiscoveryError),
    Setup(crate::deploy_firestore::DeploySetupError),
    Local(crate::error::RemoteError),
    Discovery(DiscoveryRefusal),
    Provider(crate::deploy_firestore::DeployPreflightError),
    Command(deploy_command::DeployCommandError, std::path::PathBuf),
}
/// Live Rules are an observation; the engine retains all mutation ownership.
pub trait FirestoreCommandPort: DeployPort {
    const SUPPORTS_HOSTING: bool = false;
    fn prepare_hosting(&mut self, _content: &crate::hosting::HostingComposition) {}
    fn login(&mut self) -> Result<(), crate::deploy_firestore::DeploySetupError>;
    /// Return database presence observed by the existing budget read, if known.
    fn check_index_budget(
        &mut self,
        project: &str,
        plan: &crate::deploy_plan::Plan,
    ) -> Result<Option<bool>, crate::deploy_firestore::DeployPreflightError>;
    fn live_rules(
        &mut self,
        project: &str,
    ) -> Result<Option<Vec<u8>>, crate::deploy_firestore::DeployPreflightError>;
    /// Ports without Hosting support refuse rather than invent an empty inventory.
    fn hosting_inventory(
        &mut self,
        _project: &str,
    ) -> Result<crate::hosting::HostingInventory, crate::deploy_run::DeployRefusal> {
        Err(crate::deploy_run::DeployRefusal::HostingUnavailable)
    }
}
impl FirestoreCommandPort for crate::deploy_firestore::DeployFirestore<'_> {
    const SUPPORTS_HOSTING: bool = true;
    fn prepare_hosting(&mut self, content: &crate::hosting::HostingComposition) {
        self.prepare_content(content);
    }
    fn hosting_inventory(
        &mut self,
        project: &str,
    ) -> Result<crate::hosting::HostingInventory, crate::deploy_run::DeployRefusal> {
        self.hosting_inventory(project)
    }
    fn login(&mut self) -> Result<(), crate::deploy_firestore::DeploySetupError> {
        self.login_account().map(|_| ())
    }
    fn check_index_budget(
        &mut self,
        project: &str,
        plan: &crate::deploy_plan::Plan,
    ) -> Result<Option<bool>, crate::deploy_firestore::DeployPreflightError> {
        self.check_index_budget(project, plan).map(Some)
    }
    fn live_rules(
        &mut self,
        project: &str,
    ) -> Result<Option<Vec<u8>>, crate::deploy_firestore::DeployPreflightError> {
        self.live_rules(project)
    }
}
/// Nested under deploy by the main command owner once concrete discovery is wired.
pub fn command() -> Command {
    tmt_cli_style::command(&tmt_cli_style::CommandSpec {
        name: "firestore", summary: "Plan or explicitly authorize Firestore sharing deployment",
        examples: &[tmt_cli_style::Example { command: "tmt remote deploy firestore --project <project-id> --region <region> --sign-in anonymous --json", note: "Read the exact plan without changing your Firebase project" }],
        outputs: tmt_cli_style::OutputModes::HumanAndJson,
        details: "Needs the Firebase CLI and your own firebase login. At a terminal, review the changes and answer y/N. Rerunning the same unfinished approved deploy continues without another question. Agents, CI, --json and non-interactive input only print the plan unless --authorize names its digest (at least 12 characters); signing in alone never deploys. There is no --yes flag. The plan names any live Rules it will replace; authorizing that plan allows the destructive replacement. Each project keeps its own history on this machine; its region cannot change after authorization. Uses Firebase's free Spark plan; no paid features are set up.",
    })
        .arg(Arg::new("project").long("project"))
        .arg(Arg::new("region").long("region").required(true))
        .arg(
            Arg::new("sign-in")
                .long("sign-in")
                .required(true)
                .value_delimiter(',')
                .action(ArgAction::Append)
                .value_parser(["anonymous", "google.com"]),
        )
        .arg(Arg::new("authorize").long("authorize"))
        .arg(Arg::new("details").long("details").action(ArgAction::SetTrue).help("Show the full plan (fingerprints, extension resources and indexes) before asking."))
}
pub fn arguments(matches: &ArgMatches) -> Result<FirestoreArgs, DeployCliError> {
    let authorize = matches.get_one::<String>("authorize").cloned();
    if authorize.as_ref().is_some_and(|s| !digest(s, 12)) {
        return Err(DeployCliError::Usage);
    }
    let sign_in = matches
        .get_many::<String>("sign-in")
        .ok_or(DeployCliError::Usage)?
        .map(|s| match s.as_str() {
            "anonymous" => Ok(SignInProvider::Anonymous),
            "google.com" => Ok(SignInProvider::Google),
            _ => Err(DeployCliError::Usage),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FirestoreArgs {
        remote: matches
            .try_get_one::<String>("remote")
            .ok()
            .flatten()
            .cloned(),
        project: matches
            .get_one::<String>("project")
            .cloned()
            .unwrap_or_default(),
        region: matches
            .get_one::<String>("region")
            .ok_or(DeployCliError::Usage)?
            .clone(),
        sign_in,
        authorize,
        json: matches.get_flag("json"),
        details: matches.get_flag("details"),
    })
}
fn digest(s: &str, minimum: usize) -> bool {
    s.len() >= minimum
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Read the exact installed snapshot once. A missing declaration never substitutes a
/// fixture or calls the provider; the existing binding/record remain untouched.
pub fn execute<P: FirestoreCommandPort>(
    args: &FirestoreArgs,
    source: &mut dyn DeclarationSource,
    enabled: &[&str],
    provider_factory: impl FnOnce() -> Result<P, DeployCliError>,
    layout_factory: impl FnOnce() -> Result<crate::state::Layout, DeployCliError>,
    now: impl FnOnce() -> Result<u64, DeployCliError>,
) -> Result<DeployCommandOutput, DeployCliError> {
    execute_with_confirmation(
        args,
        source,
        enabled,
        provider_factory,
        layout_factory,
        now,
        None,
    )
}
pub fn execute_with_confirmation<P: FirestoreCommandPort>(
    args: &FirestoreArgs,
    source: &mut dyn DeclarationSource,
    enabled: &[&str],
    provider_factory: impl FnOnce() -> Result<P, DeployCliError>,
    layout_factory: impl FnOnce() -> Result<crate::state::Layout, DeployCliError>,
    now: impl FnOnce() -> Result<u64, DeployCliError>,
    confirmation: Option<&mut PlanConfirmation<'_>>,
) -> Result<DeployCommandOutput, DeployCliError> {
    let discovered =
        deploy_discovery::discover(source, enabled).map_err(DeployCliError::Discovery)?;
    if let Some(output) = unavailable(&discovered) {
        return Ok(output);
    }
    if discovered.hosting.is_some() && !P::SUPPORTS_HOSTING {
        return Err(DeployCliError::Command(
            deploy_command::DeployCommandError::Refused(
                crate::deploy_run::DeployRefusal::HostingUnavailable,
            ),
            std::path::PathBuf::new(),
        ));
    }
    let layout = layout_factory()?;
    let project = crate::remote_selector::owned(
        &layout,
        args.remote.as_deref(),
        (!args.project.is_empty()).then_some(args.project.as_str()),
    )
    .map_err(DeployCliError::Local)?
    .ok_or(DeployCliError::Usage)?;
    let selected = FirestoreArgs {
        project,
        remote: args.remote.clone(),
        region: args.region.clone(),
        sign_in: args.sign_in.clone(),
        authorize: args.authorize.clone(),
        json: args.json,
        details: args.details,
    };
    let args = &selected;
    let mut store = DeployRecordStore::open_for_target(&layout, &args.project, &args.region)
        .map_err(DeployCliError::Local)?;
    let mut provider = provider_factory()?;
    provider.login().map_err(DeployCliError::Setup)?;
    let path = store.record_path();
    let mut output = execute_prepared(
        args,
        &discovered,
        &mut provider,
        &mut store,
        now()?,
        &path,
        confirmation,
    )?;
    if output.json["declined"] != true
        && output.json["record"]["run"]["steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step["state"] == "unknown"))
    {
        output.human.push_str(&uncertain_message(&path));
        output.human.push('\n');
    }
    Ok(output)
}
/// One visible line per read-only round, including elapsed time.
pub fn composite_progress(elapsed: std::time::Duration, ceiling: std::time::Duration) -> String {
    format!(
        "Checking composite indexes ({} s elapsed; up to {} s).",
        elapsed.as_secs(),
        ceiling.as_secs()
    )
}
/// The raw saved record, unlike status --layers, contains each original step outcome.
pub fn uncertain_message(path: &std::path::Path) -> String {
    let quoted = path.to_string_lossy().replace('\'', "'\\''");
    format!(
        "Deployment could not be confirmed; some changes may already be applied. Read the saved outcome (if present) with cat '{quoted}', then rerun the same command to finish this plan."
    )
}
/// No declaration means no provider setup, credential read or record change.
fn unavailable(discovered: &deploy_discovery::DiscoveredPlan) -> Option<DeployCommandOutput> {
    discovered.extensions.view().extensions.is_empty().then(|| DeployCommandOutput { json: json!({"available": false, "reason":"no-declaration", "extensions": discovered.extensions.view()}), human: "Firestore sharing is unavailable: no enabled extension uses Firestore.\nNothing changed in your Firebase project.\n".into() })
}
/// Execute only the already captured declaration snapshot, without rediscovery.
fn execute_prepared<P: FirestoreCommandPort>(
    args: &FirestoreArgs,
    discovered: &deploy_discovery::DiscoveredPlan,
    provider: &mut P,
    store: &mut DeployRecordStore,
    now_ms: u64,
    record_path: &std::path::Path,
    confirmation: Option<&mut PlanConfirmation<'_>>,
) -> Result<DeployCommandOutput, DeployCliError> {
    let database_present = provider
        .check_index_budget(&args.project, &discovered.extensions)
        .map_err(DeployCliError::Provider)?;
    let live_rules = provider
        .live_rules(&args.project)
        .map_err(DeployCliError::Provider)?;
    let mut website_live = false;
    let hosting = discovered
        .hosting
        .as_ref()
        .map(|composition| {
            let inventory = provider
                .hosting_inventory(&args.project)
                .map_err(|reason| {
                    DeployCliError::Command(
                        deploy_command::DeployCommandError::Refused(reason),
                        record_path.to_path_buf(),
                    )
                })?;
            let record = store.load_or_draft().map_err(DeployCliError::Local)?;
            provider.prepare_hosting(composition);
            let mut view = crate::hosting::deployment_view_for_app(
                composition,
                &args.project,
                &record.deployment_id,
                &inventory,
                record
                    .run
                    .as_ref()
                    .and_then(|r| r.hosting.as_ref())
                    .and_then(|h| h.app_id.as_deref()),
            )
            .map_err(|_| {
                DeployCliError::Command(
                    deploy_command::DeployCommandError::Refused(
                        crate::deploy_run::DeployRefusal::HostingUnavailable,
                    ),
                    record_path.to_path_buf(),
                )
            })?;
            website_live = inventory.live.is_some();
            if let Some(checkpoint) = record
                .run
                .as_ref()
                .filter(|r| r.state != crate::deploy_run::RunState::Complete)
                .and_then(|r| r.hosting.as_ref())
                && record.run.as_ref().is_some_and(|r| {
                    r.steps
                        .iter()
                        .find(|s| s.id == "hosting:stage:finalize")
                        .is_some_and(|s| {
                            !matches!(
                                s.state,
                                crate::deploy_run::StepState::Done
                                    | crate::deploy_run::StepState::Adopted
                            )
                        })
                })
                && checkpoint
                    .version_created_ms
                    .is_some_and(|created| now_ms.saturating_sub(created) >= 12 * 60 * 60 * 1000)
            {
                view.abandoned_version = checkpoint.version.clone();
            }
            Ok(view)
        })
        .transpose()?;
    let input = DeployCommandInput {
        extensions: &discovered.extensions,
        project: &args.project,
        location: &args.region,
        sign_in: &args.sign_in,
        rules_body: discovered.artifacts.rules.as_bytes(),
        live_rules: live_rules.as_deref(),
    };
    let map = |error| DeployCliError::Command(error, record_path.to_path_buf());
    let prepared = deploy_command::prepare_with_hosting(&input, store, provider, hosting.as_ref())
        .map_err(map)?;
    let mut continuing = false;
    let authorization = if let Some(typed) = args.authorize.as_deref() {
        Some(
            crate::deploy_run::authorize(prepared.plan(), typed)
                .map_err(|r| map(deploy_command::DeployCommandError::Refused(r)))?,
        )
    } else if let Some(confirm) = confirmation.filter(|_| !args.json) {
        if prepared.record().run.as_ref().is_some_and(|r| {
            r.state != crate::deploy_run::RunState::Complete
                && r.plan_digest == prepared.plan().digest()
        }) {
            continuing = true;
            Some(
                crate::deploy_run::authorize(prepared.plan(), prepared.plan().digest())
                    .map_err(|r| map(deploy_command::DeployCommandError::Refused(r)))?,
            )
        } else {
            if let Some(reason) = crate::deploy_run::interactive_supersession_refusal(
                prepared.plan(),
                prepared.record(),
            ) {
                return Err(map(deploy_command::DeployCommandError::Refused(reason)));
            }
            let mut preview = prepared
                .preview(&input, store, hosting.as_ref())
                .map_err(map)?;
            if args.details {
                preview.human.push_str(&confirmation_notice(
                    prepared.plan(),
                    prepared.record(),
                    website_live,
                ));
            } else {
                preview.human = short_preview(
                    prepared.plan(),
                    prepared.record(),
                    database_present,
                    website_live,
                );
            }
            if !confirm(&preview)? {
                preview.human = format!("{DECLINED}\n");
                preview.json["declined"] = json!(true);
                return Ok(preview);
            }
            Some(
                crate::deploy_run::authorize_interactive(prepared.plan(), prepared.record())
                    .map_err(|r| map(deploy_command::DeployCommandError::Refused(r)))?,
            )
        }
    } else {
        None
    };
    let mut output = deploy_command::execute_prepared(
        &input,
        prepared,
        authorization,
        store,
        provider,
        now_ms,
        hosting.as_ref(),
    )
    .map_err(map)?;
    if continuing {
        output.human.insert_str(0, &format!("{CONTINUING}\n"));
    }
    Ok(output)
}

/// Preserve typed pre-effect refusals from either preparation or the pure runner.
pub fn command_error(
    error: deploy_command::DeployCommandError,
    path: &std::path::Path,
) -> crate::error::RemoteError {
    use deploy_command::DeployCommandError;
    match error {
        DeployCommandError::Record(error) => error,
        DeployCommandError::Refused(reason)
        | DeployCommandError::Run(crate::deploy_run::DeployError::Refused(reason)) => {
            crate::error::RemoteError::new(
                reason.code(),
                &deploy_command::supersession_message(reason, path),
            )
        }
        DeployCommandError::Run(crate::deploy_run::DeployError::Interrupted) => {
            crate::error::RemoteError::new("REMOTE_DEPLOY_UNAVAILABLE", &uncertain_message(path))
        }
    }
}

/// The terminal adapter displays captured bytes and returns one answer; it never
/// supplies a replacement digest, new flags or another provider snapshot.
pub type PlanConfirmation<'a> =
    dyn FnMut(&DeployCommandOutput) -> Result<bool, DeployCliError> + 'a;

pub const DEPLOY_PROMPT: &str = "Deploy these changes to your Firebase project? [y/N]";
pub const DECLINED: &str = "Not deployed. Nothing in your Firebase project changed.";
pub const CONTINUING: &str = "Continuing the deploy you already approved.";
pub const REPLACING: &str = "An earlier deploy did not finish. This one replaces it. Anything the earlier one already changed stays as it is; nothing is undone.";
pub const KNOWN_VERSION: &str = "The earlier deploy left an uploaded website version that is not live. It stays in your project, unchanged.";
pub const UNKNOWN_VERSION: &str = "Your project may already hold one uploaded website version that is not live. It stays that way, and this deploy may upload another.";
/// Interaction owns terminal evidence; agents/CI never become implicit consent.
pub fn may_prompt(interaction: tmt_cli_style::Interaction, agent: bool, ci: bool) -> bool {
    !agent && !ci && interaction.prompt() == tmt_cli_style::Mode::Interactive
}
pub fn confirmation_answer(bytes: &[u8]) -> bool {
    let Ok(answer) = std::str::from_utf8(bytes) else {
        return false;
    };
    answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes")
}

/// Display captured interactive changes, without another provider read.
pub fn confirmation_notice(
    plan: &crate::deploy_run::DeployPlan,
    record: &crate::deploy_run::DeployRecord,
    website_live: bool,
) -> String {
    let mut text = change_lines(plan, website_live);
    text.push_str(&supersession_notice(plan, record));
    text
}

fn supersession_notice(
    plan: &crate::deploy_run::DeployPlan,
    record: &crate::deploy_run::DeployRecord,
) -> String {
    let mut text = String::new();
    if let Some(previous) = crate::deploy_run::superseded_run(plan, record) {
        text.push_str(REPLACING);
        text.push('\n');
        if previous
            .steps
            .iter()
            .any(|s| s.id == "hosting:release" && s.state == crate::deploy_run::StepState::Pending)
        {
            if previous.steps.iter().any(|s| {
                s.id == "hosting:stage:create" && s.state == crate::deploy_run::StepState::Unknown
            }) {
                text.push_str(UNKNOWN_VERSION);
                text.push('\n');
            } else if previous
                .hosting
                .as_ref()
                .is_some_and(|h| h.version.is_some())
            {
                text.push_str(KNOWN_VERSION);
                text.push('\n');
            }
        }
    }
    text
}

/// Both interactive views use the same states and value-column notes.
fn change_lines(plan: &crate::deploy_run::DeployPlan, website_live: bool) -> String {
    use crate::deploy_run::LiveRules;
    let mut text = format!(
        "  Security rules  {}\n",
        match plan.live_rules() {
            LiveRules::Absent => "will be created",
            LiveRules::Current => "no change",
            LiveRules::Own => "will be updated",
            LiveRules::Foreign(_) => "will replace rules set outside TMT",
        },
    );
    if matches!(plan.live_rules(), LiveRules::Foreign(_)) {
        text.push_str("                  Answering y replaces them.\n");
    }
    if let Some(hosting) = &plan.view().hosting {
        let website = if hosting.replaces == "foreign" {
            "will replace the live website".to_owned()
        } else if !website_live {
            "will be published for the first time".to_owned()
        } else if hosting.replaces == "none" {
            "no change".to_owned()
        } else {
            format!(
                "will be updated ({} files)",
                hosting.content["files"].as_array().map_or(0, Vec::len)
            )
        };
        text.push_str(&format!("  Website files   {website}\n"));
        if hosting.replaces == "foreign" {
            text.push_str("                  Answering y replaces it.\n");
        }
    }
    text
}

/// A terminal view of this frozen plan and its existing read-only observations.
/// Unknown live state is a target, never inferred from an earlier completed run.
fn short_preview(
    plan: &crate::deploy_run::DeployPlan,
    record: &crate::deploy_run::DeployRecord,
    database_present: Option<bool>,
    website_live: bool,
) -> String {
    use crate::deploy_run::StepKind;
    let view = plan.view();
    let mut text = format!(
        "Deploy to Firebase\n  Account         {}\n  Project         {} ({})\n\n  Database        {}\n",
        view.account,
        view.project,
        view.database.location,
        match database_present {
            Some(true) => "no change",
            Some(false) => "will be created",
            None => &view.database.location,
        },
    );
    text.push_str(&change_lines(plan, website_live));
    let indexes = plan
        .steps()
        .iter()
        .filter(|step| {
            matches!(
                step.kind,
                StepKind::Index { .. } | StepKind::CompositeIndex(_)
            )
        })
        .count();
    text.push_str(&format!(
        "  Sign-in         {}\n  Indexes         {indexes} needed\n\n",
        view.sign_in.join(", "),
    ));
    text.push_str(&supersession_notice(plan, record));
    text.push_str("Full plan: run the same command with --details.\n");
    text
}
