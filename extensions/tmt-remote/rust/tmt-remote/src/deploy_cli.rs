//! Deployment argv and composition through the installed adapter seam;
//! fixtures inject declaration/provider ports, never an alternate deploy algorithm.
use crate::{
    deploy_command::{self, DeployCommandInput, DeployCommandOptions, DeployCommandOutput},
    deploy_discovery::{self, DeclarationSource, DiscoveryRefusal},
    deploy_record::DeployRecordStore,
    deploy_run::{DeployPort, DeployProviderError, SignInProvider},
};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::json;

#[derive(Debug, PartialEq, Eq)]
pub struct FirestoreArgs {
    pub project: String,
    pub region: String,
    pub sign_in: Vec<SignInProvider>,
    pub authorize: Option<String>,
    pub json: bool,
}
#[derive(Debug)]
pub enum DeployCliError {
    Usage,
    Tool(crate::deploy_tools::ToolDiscoveryError),
    Setup(crate::deploy_firestore::DeploySetupError),
    Local(crate::error::RemoteError),
    Discovery(DiscoveryRefusal),
    Provider(DeployProviderError),
    Command(deploy_command::DeployCommandError, std::path::PathBuf),
}
/// Live Rules are an observation; the engine retains all mutation ownership.
pub trait FirestoreCommandPort: DeployPort {
    const SUPPORTS_HOSTING: bool = false;
    fn prepare_hosting(&mut self, _content: &crate::hosting::HostingComposition) {}
    fn login(&mut self) -> Result<(), crate::deploy_firestore::DeploySetupError>;
    fn check_index_budget(
        &mut self,
        project: &str,
        plan: &crate::deploy_plan::Plan,
    ) -> Result<(), DeployProviderError>;
    fn live_rules(&mut self, project: &str) -> Result<Option<Vec<u8>>, DeployProviderError>;
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
    ) -> Result<(), DeployProviderError> {
        self.check_index_budget(project, plan)
    }
    fn live_rules(&mut self, project: &str) -> Result<Option<Vec<u8>>, DeployProviderError> {
        self.live_rules(project)
    }
}
/// Nested under deploy by the main command owner once concrete discovery is wired.
pub fn command() -> Command {
    tmt_cli_style::command(&tmt_cli_style::CommandSpec {
        name: "firestore", summary: "Plan or explicitly authorize Firestore sharing deployment",
        examples: &[tmt_cli_style::Example { command: "tmt remote deploy firestore --project <project-id> --region <region> --sign-in anonymous --json", note: "Read the exact plan without changing your Firebase project" }],
        outputs: tmt_cli_style::OutputModes::HumanAndJson,
        details: "Needs the Firebase CLI and your own firebase login. Without --authorize it only prints the plan. To deploy, pass --authorize with the first 12 or more characters of the plan digest; signing in alone never deploys. The plan names any live Rules it will replace; authorizing that plan allows the destructive replacement. One Remote home deploys to one project and region. Uses Firebase's free Spark plan; no paid features are set up.",
    })
        .arg(Arg::new("project").long("project").required(true))
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
        project: matches
            .get_one::<String>("project")
            .ok_or(DeployCliError::Usage)?
            .clone(),
        region: matches
            .get_one::<String>("region")
            .ok_or(DeployCliError::Usage)?
            .clone(),
        sign_in,
        authorize,
        json: matches.get_flag("json"),
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
    let mut store = DeployRecordStore::open_for_target(&layout, &args.project, &args.region)
        .map_err(DeployCliError::Local)?;
    let mut provider = provider_factory()?;
    provider.login().map_err(DeployCliError::Setup)?;
    let path = layout.directory.join("deploy.json");
    let mut output = execute_prepared(args, &discovered, &mut provider, &mut store, now()?, &path)?;
    if output.json["record"]["run"]["state"] == "partial" {
        output.human.push_str(&uncertain_message(&path));
        output.human.push('\n');
    }
    Ok(output)
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
    store: &mut DeployRecordStore<'_>,
    now_ms: u64,
    record_path: &std::path::Path,
) -> Result<DeployCommandOutput, DeployCliError> {
    provider
        .check_index_budget(&args.project, &discovered.extensions)
        .map_err(DeployCliError::Provider)?;
    let live_rules = provider
        .live_rules(&args.project)
        .map_err(DeployCliError::Provider)?;
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
    deploy_command::execute_with_hosting(
        &DeployCommandInput {
            extensions: &discovered.extensions,
            project: &args.project,
            location: &args.region,
            sign_in: &args.sign_in,
            rules_body: discovered.artifacts.rules.as_bytes(),
            live_rules: live_rules.as_deref(),
        },
        &DeployCommandOptions {
            authorize: args.authorize.as_deref(),
        },
        store,
        provider,
        now_ms,
        hosting.as_ref(),
    )
    .map_err(|error| DeployCliError::Command(error, record_path.to_path_buf()))
}
