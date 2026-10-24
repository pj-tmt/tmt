//! Permission set admission and presentation over the manifest owner's metadata.
//! No permission inventory, provider I/O, cache or deployment authority lives here.
use crate::{deploy_cli::inputs::PermissionRequirement, error::RemoteError, limits};

pub fn requested(requirements: &[PermissionRequirement]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for requirement in requirements {
        if !names.contains(&requirement.permission) {
            names.push(requirement.permission);
        }
    }
    names
}

pub(crate) fn valid_names(names: &[&str]) -> bool {
    !names.is_empty()
        && names.len() <= limits::DEPLOY_PERMISSION_COUNT
        && names.iter().enumerate().all(|(index, name)| {
            name.len() <= limits::DEPLOY_PERMISSION_BYTES
                && !names[..index].contains(name)
                && name.split('.').count() >= 3
                && name.split('.').enumerate().all(|(part, value)| {
                    !value.is_empty()
                        && value.as_bytes()[0].is_ascii_alphabetic()
                        && (part != 0 || value.bytes().all(|b| !b.is_ascii_uppercase()))
                        && value.bytes().all(|b| b.is_ascii_alphanumeric())
                })
        })
}

pub fn check(
    requirements: &[PermissionRequirement],
    granted: &[String],
) -> Result<(), RemoteError> {
    let names = requested(requirements);
    if granted
        .iter()
        .enumerate()
        .any(|(index, name)| !names.contains(&name.as_str()) || granted[..index].contains(name))
    {
        return Err(RemoteError::new(
            "REMOTE_DEPLOY_UNAVAILABLE",
            "Firebase permissions could not be read. Nothing changed in your Firebase project.",
        ));
    }
    let missing: Vec<_> = requirements
        .iter()
        .filter(|requirement| !granted.iter().any(|name| name == requirement.permission))
        .map(|requirement| format!("{} ({})", requirement.permission, requirement.step))
        .collect();
    if !missing.is_empty() {
        return Err(RemoteError::new(
            "REMOTE_DEPLOY_PERMISSION_DENIED",
            &format!(
                "Missing deployment permissions: {}. Nothing changed in your Firebase project.",
                missing.join(", ")
            ),
        ));
    }
    Ok(())
}

fn details(requirements: &[PermissionRequirement]) -> String {
    let mut text = String::from("Permissions required:\n");
    for requirement in requirements {
        text.push_str(&format!(
            "  {} ({})\n",
            requirement.permission, requirement.step
        ));
    }
    text
}

/// Called only by the CLI after its successful probe, never by the pure runner.
pub(crate) fn annotate(
    output: &mut crate::deploy_command::DeployCommandOutput,
    requirements: &[PermissionRequirement],
    details_requested: bool,
) {
    output.json["permissions"] = serde_json::json!({"checked":true,"required":requirements.iter().map(|requirement| serde_json::json!({"permission":requirement.permission,"step":requirement.step})).collect::<Vec<_>>()});
    if output.json["declined"] != true {
        output.human.push_str("Permissions: checked\n");
        if details_requested {
            output.human.push_str(&details(requirements));
        }
    }
}
