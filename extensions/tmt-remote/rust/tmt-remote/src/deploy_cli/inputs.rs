//! Firestore's authored input/permission metadata and pure selection. Permission
//! probing belongs to the separate preflight adapter; no provider call lives here.
use super::{DeployCliError, FirestoreArgs};
use crate::{
    deploy_input::{self, DefaultChoice, InputError, InputKind, InputSpec, SelectedValue, Values},
    deploy_run::{DeployRecord, ResourceInputs, SignInProvider},
    error::RemoteError,
};

const PROJECT: &[&str] = &["firebase.projects.get"];
const DATABASE: &[&str] = &[
    "datastore.databases.getMetadata",
    "datastore.databases.create",
];
const APP: &[&str] = &[
    "firebase.clients.get",
    "firebase.clients.list",
    "firebase.clients.create",
];
const SITE: &[&str] = &[
    "firebasehosting.sites.get",
    "firebasehosting.sites.create",
    "firebasehosting.sites.update",
];
const SIGN_IN: &[&str] = &["firebaseauth.configs.get", "firebaseauth.configs.update"];
const RULES: &[&str] = &[
    "firebaserules.rulesets.list",
    "firebaserules.rulesets.get",
    "firebaserules.rulesets.create",
    "firebaserules.releases.get",
    "firebaserules.releases.create",
    "firebaserules.releases.update",
];
const INDEXES: &[&str] = &[
    "datastore.indexes.list",
    "datastore.indexes.get",
    "datastore.indexes.create",
    "datastore.indexes.update",
];
const OWNER: &[&str] = &["datastore.entities.get", "datastore.entities.create"];

const MANIFEST: &[InputSpec] = &[
    InputSpec {
        input: "project",
        flag: "project",
        kind: InputKind::SelectExisting,
        default: DefaultChoice::MustProvide,
        creates: None,
        cost: "Existing Spark project; never created",
        permissions: PROJECT,
    },
    InputSpec {
        input: "region",
        flag: "region",
        kind: InputKind::SelectExisting,
        default: DefaultChoice::MustProvide,
        creates: None,
        cost: "Database region; cannot change after authorization",
        permissions: DATABASE,
    },
    InputSpec {
        input: "database",
        flag: "database",
        kind: InputKind::CreateOrAdopt,
        default: DefaultChoice::Value("(default)"),
        creates: Some("the (default) database if absent"),
        cost: "Only (default) is supported on the free plan",
        permissions: DATABASE,
    },
    InputSpec {
        input: "web-app",
        flag: "web-app",
        kind: InputKind::CreateOrAdopt,
        default: DefaultChoice::DuringPlanning("new"),
        creates: Some("a web app named tmt Remote (<deployment>) when Hosting is needed"),
        cost: "Free-plan web app; an explicit app ID must already exist",
        permissions: APP,
    },
    InputSpec {
        input: "site",
        flag: "site",
        kind: InputKind::CreateOrAdopt,
        default: DefaultChoice::DuringPlanning("new"),
        creates: Some("a dedicated TMT Hosting site when Hosting is needed"),
        cost: "Free-plan Hosting; new never selects the project default site",
        permissions: SITE,
    },
    InputSpec {
        input: "sign-in",
        flag: "sign-in",
        kind: InputKind::CreateOrAdopt,
        default: DefaultChoice::Value("anonymous"),
        creates: Some("the selected sign-in configuration if not enabled"),
        cost: "Free-plan anonymous or google.com sign-in",
        permissions: SIGN_IN,
    },
];
pub fn manifest() -> &'static [InputSpec] {
    MANIFEST
}

/// Attribution labels denote existing deployment operation families; they are not
/// new runner steps. These requirements describe potential work before inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequirement {
    pub permission: &'static str,
    pub step: &'static str,
}
pub fn permission_requirements(
    plan: &crate::deploy_plan::Plan,
    hosting: bool,
) -> Vec<PermissionRequirement> {
    if plan.view().extensions.is_empty() {
        return vec![];
    }
    let mut requirements = Vec::new();
    let mut add = |step, permissions: &'static [&'static str]| {
        for permission in permissions {
            let requirement = PermissionRequirement { permission, step };
            if !requirements.contains(&requirement) {
                requirements.push(requirement);
            }
        }
    };
    add("project", PROJECT);
    add("database", DATABASE);
    add("sign-in", SIGN_IN);
    add("rules", RULES);
    if plan
        .view()
        .extensions
        .iter()
        .flat_map(|extension| &extension.resources)
        .any(|resource| !resource.indexes.is_empty() || !resource.composite_indexes.is_empty())
    {
        add("indexes", INDEXES);
    }
    if hosting {
        add("web-app", APP);
        add("hosting", SITE);
        add("owner:register", OWNER);
    }
    requirements
}

pub fn supplied(args: &FirestoreArgs) -> Values {
    let mut values = Values::new();
    for (name, value) in [
        ("project", Some(args.project.as_str())),
        ("region", Some(args.region.as_str())),
        ("database", args.database.as_deref()),
        ("web-app", args.web_app.as_deref()),
        ("site", args.site.as_deref()),
    ] {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            values.insert(name.into(), value.into());
        }
    }
    if !args.sign_in.is_empty() {
        values.insert("sign-in".into(), sign_in_names(&args.sign_in));
    }
    values
}
pub fn sign_in_names(providers: &[SignInProvider]) -> String {
    let mut names: Vec<_> = providers.iter().map(|provider| provider.name()).collect();
    names.sort_unstable();
    names.dedup();
    names.join(",")
}
fn invalid(flag: &str, message: &str) -> DeployCliError {
    DeployCliError::Local(RemoteError::new(
        "REMOTE_DEPLOY_INPUT_INVALID",
        &format!("{flag}: {message}"),
    ))
}
pub fn validate_supplied(args: &FirestoreArgs) -> Result<(), DeployCliError> {
    for (name, value) in supplied(args) {
        if value.contains('<') || value.contains('>') {
            return Err(invalid(
                &format!("--{name}"),
                "replace the placeholder before deploying.",
            ));
        }
    }
    if !args.project.is_empty() && !crate::deploy_run::project_ok(&args.project) {
        return Err(invalid("--project", "invalid project ID."));
    }
    if !args.region.is_empty() && !crate::deploy_run::location_ok(&args.region) {
        return Err(invalid("--region", "invalid database region."));
    }
    if args
        .database
        .as_deref()
        .is_some_and(|value| value != "(default)")
    {
        return Err(invalid(
            "--database",
            "only (default) is supported on the free plan.",
        ));
    }
    if args
        .site
        .as_deref()
        .is_some_and(|value| value != "new" && !crate::hosting::site_id_ok(value))
    {
        return Err(invalid("--site", "use an existing site ID or new."));
    }
    if args
        .web_app
        .as_deref()
        .is_some_and(|value| value != "new" && !crate::hosting::app_id_ok(value))
    {
        return Err(invalid("--web-app", "use an existing app ID or new."));
    }
    Ok(())
}
pub fn record_values(record: &DeployRecord, project: &str, region: Option<&str>) -> Values {
    if let Some(inputs) = &record.inputs {
        return resource_values(inputs);
    }
    let mut values = Values::new();
    values.insert("project".into(), project.into());
    if let Some(region) = region {
        values.insert("region".into(), region.into());
    }
    if let Some(run) = &record.run {
        values.insert("database".into(), "(default)".into());
        let hosting = run.hosting.as_ref();
        values.insert(
            "site".into(),
            hosting
                .and_then(|hosting| hosting.envelope["hosting"]["site"].as_str())
                .unwrap_or(project)
                .into(),
        );
        values.insert(
            "web-app".into(),
            hosting
                .and_then(|hosting| {
                    hosting
                        .app_id
                        .as_deref()
                        .or_else(|| hosting.envelope["hosting"]["webApp"].as_str())
                })
                .unwrap_or("new")
                .into(),
        );
        let mut providers: Vec<_> = run
            .steps
            .iter()
            .filter_map(|step| step.id.strip_prefix("sign-in:"))
            .collect();
        providers.sort_unstable();
        providers.dedup();
        if !providers.is_empty() {
            values.insert("sign-in".into(), providers.join(","));
        }
    }
    values
}
pub fn resource_values(inputs: &ResourceInputs) -> Values {
    [
        ("project", inputs.project.clone()),
        ("region", inputs.region.clone()),
        ("database", inputs.database.clone()),
        ("web-app", inputs.web_app.clone()),
        ("site", inputs.site.clone()),
        ("sign-in", sign_in_names(&inputs.sign_in)),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value))
    .collect()
}
pub fn resolve(
    args: &FirestoreArgs,
    project: Option<&str>,
    recorded: &Values,
    requirements: &[PermissionRequirement],
) -> Result<ResourceInputs, DeployCliError> {
    let mut given = supplied(args);
    if let Some(project) = project {
        given.insert("project".into(), project.into());
    }
    let selected = deploy_input::select(MANIFEST, &given, recorded, args.defaults).map_err(
        |error| match error {
            InputError::Required(mut required) => {
                required.permissions.clear();
                for requirement in requirements {
                    if !required.permissions.contains(&requirement.permission) {
                        required.permissions.push(requirement.permission);
                    }
                }
                let hint = deploy_input::defaults_hint("firestore", MANIFEST, &given, &required);
                DeployCliError::Local(RemoteError::deploy_inputs(required, &hint))
            }
            InputError::RecordedConflict { flag, .. } => DeployCliError::Local(RemoteError::new(
                "REMOTE_DEPLOY_PROJECT_CONFLICT",
                &format!(
                    "{flag} differs from the recorded deployment; its resources were not changed."
                ),
            )),
        },
    )?;
    let values: Values = selected
        .into_iter()
        .map(|selected| {
            (
                selected.input.into(),
                match selected.value {
                    SelectedValue::Value(value) => value,
                    SelectedValue::DuringPlanning(value) => value.into(),
                },
            )
        })
        .collect();
    let sign_in = values["sign-in"]
        .split(',')
        .map(|provider| match provider {
            "anonymous" => Ok(SignInProvider::Anonymous),
            "google.com" => Ok(SignInProvider::Google),
            _ => Err(invalid("--sign-in", "use anonymous or google.com.")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let inputs = ResourceInputs {
        project: values["project"].clone(),
        region: values["region"].clone(),
        database: values["database"].clone(),
        web_app: values["web-app"].clone(),
        site: values["site"].clone(),
        sign_in,
    };
    if !inputs.valid() {
        return Err(invalid("deployment inputs", "invalid resource selection."));
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests;
pub fn site(
    inputs: &ResourceInputs,
    deployment: &str,
    recorded_site: Option<&str>,
) -> Result<String, DeployCliError> {
    if inputs.site != "new" {
        return Ok(inputs.site.clone());
    }
    if let Some(site) = recorded_site {
        if !crate::hosting::site_id_ok(site) {
            return Err(invalid("--site", "invalid recorded site."));
        }
        return Ok(site.to_owned());
    }
    if crate::canonical::uuid(deployment).is_err() {
        return Err(invalid("--site", "invalid deployment identity."));
    }
    let hex: String = deployment
        .chars()
        .filter(|character| *character != '-')
        .collect();
    let candidate = format!("tmt-{}", &hex[..crate::limits::HOSTING_SITE_ID_BYTES - 4]);
    if candidate == inputs.project || !crate::hosting::site_id_ok(&candidate) {
        return Err(invalid("--site", "could not select a dedicated TMT site."));
    }
    Ok(candidate)
}
