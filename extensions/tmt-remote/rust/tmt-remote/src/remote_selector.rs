//! One local selection owner; a public reference is never proof of an owned project.
use crate::{deploy_record, error::RemoteError, remote_catalog, settings, state::Layout};
pub fn missing(reference: &str) -> RemoteError {
    RemoteError::new(
        "REMOTE_SELECTOR_NOT_FOUND",
        &format!("Remote '{reference}' was not found."),
    )
    .with_hint("Run tmt remote ls")
}
fn resolve(
    layout: &Layout,
    reference: &str,
) -> Result<Option<remote_catalog::RemoteReference>, RemoteError> {
    if reference == "local" {
        return Ok(None);
    }
    if !remote_catalog::name_ok(reference) {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "Invalid remote name or project id.",
        )
        .with_hint("Run tmt remote ls"));
    }
    let entries = remote_catalog::read(layout)?;
    if let Some(entry) = entries.into_iter().find(|e| e.name == reference) {
        return Ok(Some(entry));
    }
    if deploy_record::projects(layout)?
        .iter()
        .any(|p| p == reference)
    {
        return Ok(Some(remote_catalog::RemoteReference {
            name: reference.into(),
            kind: "firestore".into(),
            url: String::new(),
            project: Some(reference.into()),
        }));
    }
    Err(missing(reference))
}
/// Default selection may name a public reference without claiming local ownership.
pub fn validate_reference(layout: &Layout, reference: &str) -> Result<(), RemoteError> {
    resolve(layout, reference).map(|_| ())
}
/// Owner-host commands disregard the configured targeting default.
pub fn local(layout: &Layout, explicit: Option<&str>, command: &str) -> Result<(), RemoteError> {
    let Some(reference) = explicit else {
        return Ok(());
    };
    let Some(entry) = resolve(layout, reference)? else {
        return Ok(());
    };
    let (code, message) = if entry.kind == "local" {
        (
            "REMOTE_SELECTOR_LOCAL_REQUIRED",
            format!(
                "tmt remote {command} only runs on this machine; run it on the machine serving that remote instead."
            ),
        )
    } else {
        (
            "REMOTE_SELECTOR_KIND_UNSUPPORTED",
            format!(
                "tmt remote {command} cannot manage Firebase remote '{}'",
                entry.name
            ),
        )
    };
    Err(RemoteError::new(code, &message).with_hint(&format!(
        "Run tmt remote {command} without --remote to manage this machine"
    )))
}
/// Explicit create input overrides a targeting default, but must agree with an explicit remote.
pub fn owned(
    layout: &Layout,
    explicit: Option<&str>,
    project: Option<&str>,
) -> Result<Option<String>, RemoteError> {
    deploy_record::refuse_legacy(layout)?;
    if let Some(project) = project {
        if !crate::deploy_run::project_ok(project) {
            return Err(RemoteError::new(
                "REMOTE_SELECTOR_INVALID",
                "Invalid Firebase project id.",
            )
            .with_hint("Run tmt remote ls"));
        }
        if let Some(reference) = explicit.filter(|r| *r != "local") {
            let entry = resolve(layout, reference)?;
            if entry.as_ref().and_then(|entry| entry.project.as_deref()) != Some(project) {
                return Err(RemoteError::new(
                    "REMOTE_SELECTOR_INVALID",
                    &format!("The --project and --remote options name different projects: {project} and {}", entry.as_ref().and_then(|entry| entry.project.as_deref()).unwrap_or(reference)),
                )
                .with_hint("Run the same command with only one of --project or --remote"));
            }
            select(layout, reference)?;
        }
        if remote_catalog::read(layout)?
            .iter()
            .any(|e| e.name == project && e.project.as_deref() != Some(project))
        {
            return Err(RemoteError::new(
                "REMOTE_SELECTOR_INVALID",
                "This project id is already a different remote's name.",
            )
            .with_hint("Run tmt remote ls"));
        }
        return Ok(Some(project.into()));
    }
    if let Some(reference) = explicit {
        return select(layout, reference);
    }
    if let Some(reference) = default_reference(layout)? {
        return select(layout, &reference).map_err(default_error);
    }
    fallback(layout)
}
fn select(layout: &Layout, reference: &str) -> Result<Option<String>, RemoteError> {
    let Some(entry) = resolve(layout, reference)? else {
        return fallback(layout);
    };
    if entry.kind == "local" {
        return Err(RemoteError::new("REMOTE_SELECTOR_LOCAL_REQUIRED", "This command only reads projects from this machine; run it on the machine serving that remote instead.").with_hint("Run the same command without --remote to read projects from this machine"));
    }
    let project = entry.project.expect("validated Firebase entry");
    if entry.url.is_empty() {
        return Ok(Some(project));
    }
    let record = deploy_record::read_project(layout, &project)?;
    let owned = record.as_ref().is_some_and(|record| {
        entry.url.is_empty() || crate::remote_link::matches_record(&entry.url, record)
    });
    if !owned {
        return Err(not_owned(layout, &project)?);
    }
    Ok(Some(project))
}
fn not_owned(layout: &Layout, project: &str) -> Result<RemoteError, RemoteError> {
    Ok(RemoteError::new("REMOTE_SELECTOR_NOT_OWNED", &format!("This machine did not deploy project {project}"))
        .with_hint(&match deploy_record::projects(layout)?.iter().find(|owned| owned.as_str() != project) {
            Some(owned) => format!("Run the same command with --project {owned}"),
            None => format!("Plan it here with tmt remote deploy firestore --project {project} --region <region> --sign-in anonymous"),
        }))
}
/// Read-only targeting accepts an owned project id, never an implicit creation input.
pub fn existing(
    layout: &Layout,
    explicit: Option<&str>,
    project: Option<&str>,
) -> Result<Option<String>, RemoteError> {
    let selected = owned(layout, explicit, project)?;
    if let Some(project) = selected.as_ref()
        && !deploy_record::projects(layout)?.contains(project)
    {
        return Err(not_owned(layout, project)?);
    }
    Ok(selected)
}
fn fallback(layout: &Layout) -> Result<Option<String>, RemoteError> {
    let projects = deploy_record::projects(layout)?;
    match projects.as_slice() {
        [] => Ok(None),
        [project] => Ok(Some(project.clone())),
        [first, ..] => Err(RemoteError::new("REMOTE_SELECTOR_REQUIRED", &format!("Several projects are deployed from this machine: {}", projects.join(", "))).with_hint(&format!("Run the same command with --project {first}\nRun tmt remote settings default-remote {first}"))),
    }
}

fn default_reference(layout: &Layout) -> Result<Option<String>, RemoteError> {
    let root = layout.directory.parent().expect("Remote subtree");
    let settings = settings::read(root)?;
    if settings.malformed || !settings.default_valid() {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_UNAVAILABLE",
            "The default remote could not be read.",
        )
        .with_hint("Run tmt remote settings default-remote local"));
    }
    Ok(settings.default_remote().map(str::to_owned))
}
fn default_error(error: RemoteError) -> RemoteError {
    if error.code == "REMOTE_SELECTOR_NOT_FOUND" {
        RemoteError::new(
            "REMOTE_SELECTOR_STALE_DEFAULT",
            "The default points to a removed remote.",
        )
        .with_hint(
            "Run tmt remote settings default-remote local\nRun tmt remote ls to see the remotes",
        )
    } else {
        error
    }
}
/// Provenance for presentation only; this grants no authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientKind {
    Owned,
    Imported,
}
/// Selected public config gives an ordinary client no local deployment authority.
pub struct ClientSelection {
    pub kind: ClientKind,
    pub project: String,
    pub public_config: serde_json::Value,
}
fn client_local(layout: &Layout, project: Option<&str>) -> Result<ClientSelection, RemoteError> {
    let project = match project {
        Some(project) => project.to_owned(),
        None => fallback(layout)?.ok_or_else(crate::remote_link::unavailable)?,
    };
    let record = deploy_record::read_project(layout, &project)?
        .ok_or_else(crate::remote_link::unavailable)?;
    let link = crate::remote_link::from_record(&record)?;
    Ok(ClientSelection {
        kind: ClientKind::Owned,
        project,
        public_config: crate::remote_link::public_config(&link)?,
    })
}
fn client_reference(
    layout: &Layout,
    reference: &str,
    project: Option<&str>,
) -> Result<ClientSelection, RemoteError> {
    let Some(entry) = resolve(layout, reference)? else {
        return client_local(layout, project);
    };
    if entry.kind != "firestore" {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_KIND_UNSUPPORTED",
            "This remote has no Firestore sign-in configuration.",
        )
        .with_hint("Run tmt remote ls"));
    }
    let selected = entry
        .project
        .as_deref()
        .ok_or_else(crate::remote_link::unavailable)?;
    if project.is_some_and(|project| project != selected) {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "The --project and --remote options name different projects.",
        )
        .with_hint("Run the same command with matching --project and --remote options"));
    }
    if entry.url.is_empty() {
        return client_local(layout, Some(selected));
    }
    let public_config = crate::remote_link::public_config(&entry.url)?;
    if public_config["projectId"].as_str() != Some(selected) {
        return Err(crate::remote_link::unavailable());
    }
    Ok(ClientSelection {
        kind: ClientKind::Imported,
        project: selected.into(),
        public_config,
    })
}
/// Ordinary-user targeting: explicit, configured default, then this host. Failed
/// explicit/default resolution never becomes another remote or deployment input.
pub fn client(
    layout: &Layout,
    explicit: Option<&str>,
    project: Option<&str>,
) -> Result<ClientSelection, RemoteError> {
    client_selection(layout, explicit, project, false)?.ok_or_else(crate::remote_link::unavailable)
}
/// An unconfigured host can report no sign-in. Broken explicit/default targeting
/// still refuses instead of masquerading as an unconfigured host.
pub fn client_status(
    layout: &Layout,
    explicit: Option<&str>,
    project: Option<&str>,
) -> Result<Option<ClientSelection>, RemoteError> {
    client_selection(layout, explicit, project, true)
}
fn client_selection(
    layout: &Layout,
    explicit: Option<&str>,
    project: Option<&str>,
    optional: bool,
) -> Result<Option<ClientSelection>, RemoteError> {
    if project.is_some_and(|p| !crate::deploy_run::project_ok(p)) {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "Invalid Firestore project id.",
        ));
    }
    if let Some(reference) = explicit {
        return client_reference(layout, reference, project).map(Some);
    }
    if let Some(reference) = default_reference(layout)? {
        return client_reference(layout, &reference, project)
            .map(Some)
            .map_err(default_error);
    }
    if optional && project.is_none() && deploy_record::projects(layout)?.len() != 1 {
        return Ok(None);
    }
    match client_local(layout, project) {
        Err(error) if optional && project.is_none() && error.code == "REMOTE_LINK_UNAVAILABLE" => {
            Ok(None)
        }
        result => result.map(Some),
    }
}
