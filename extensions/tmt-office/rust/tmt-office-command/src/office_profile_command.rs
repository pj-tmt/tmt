//! One-shot local Office profile composition using the verified identity resolver.

use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    interrupt::Interrupt, office_companion::invoke_local_office_profile,
    office_profile::read_profile_file,
};
use tmt_office_model::office_protocol::OfficeError;

use crate::{
    core_access::CoreAccess,
    invocation::{OfficeOperation, OfficeProfileOperation, OutputMode},
    office_pairing_command::pairing_error,
    output::Failure,
};

pub fn run(
    executable: &Path,
    operation: OfficeOperation,
    mode: OutputMode,
    core_access: &dyn CoreAccess,
) -> Result<u8, Failure> {
    let OfficeOperation::Profile {
        identity: selector,
        operation,
    } = operation
    else {
        return Err(Failure::new(
            "USAGE_ERROR",
            "Expected an Office profile operation.",
            1,
        ));
    };
    let edit = match operation {
        OfficeProfileOperation::Show => None,
        OfficeProfileOperation::Apply { file, if_revision } => {
            let profile = read_profile_file(Path::new(&file)).map_err(pairing_error)?;
            Some((profile, if_revision))
        }
    };
    let identity = core_access.identity(selector.as_deref())?;
    let interrupt = Interrupt::install().map_err(unavailable)?;
    if interrupt.is_interrupted() {
        return Err(crate::office_pairing_command::interrupted());
    }
    let editing = edit.is_some();
    let result = match edit {
        Some((profile, revision)) => invoke_local_office_profile(
            executable,
            &identity.id,
            Some((&profile, revision)),
            Instant::now() + Duration::from_secs(30),
        ),
        None => invoke_local_office_profile(
            executable,
            &identity.id,
            None,
            Instant::now() + Duration::from_secs(30),
        ),
    };
    let value = result
        .map_err(|error| unavailable_for(error, editing))?
        .map_err(|error| profile_error(error, &identity.name))?;
    if interrupt.is_interrupted() {
        return Err(crate::office_pairing_command::interrupted());
    }
    let output = if mode.json {
        value.to_string()
    } else {
        human(&value)
    };
    writeln!(io::stdout().lock(), "{output}").map_err(unavailable)?;
    Ok(0)
}

fn human(value: &serde_json::Value) -> String {
    let state = if value["exists"].as_bool() == Some(true) {
        "saved"
    } else {
        "default"
    };
    let outcome = match value["changed"].as_bool() {
        Some(true) if value["revision"].as_u64() == Some(1) => " · created",
        Some(true) => " · updated",
        Some(false) => " · unchanged",
        None => "",
    };
    format!(
        "{} ({}) — revision {} {}{}\nLabel: {}\nDescription: {}\nAvatar: {}\nHair: {} / {}\nSkin: {}\nShirt: {} / {}",
        value["identityName"].as_str().unwrap_or("identity"),
        value["identityId"].as_str().unwrap_or(""),
        value["revision"].as_u64().unwrap_or(0),
        state,
        outcome,
        value["profile"]["displayLabel"].as_str().unwrap_or(""),
        value["profile"]["description"].as_str().unwrap_or(""),
        value["profile"]["avatarRef"].as_str().unwrap_or("default"),
        value["profile"]["appearance"]["hairStyle"]
            .as_str()
            .unwrap_or(""),
        value["profile"]["appearance"]["hairColor"]
            .as_str()
            .unwrap_or(""),
        value["profile"]["appearance"]["skinTone"]
            .as_str()
            .unwrap_or(""),
        value["profile"]["appearance"]["shirtColor"]
            .as_str()
            .unwrap_or(""),
        value["profile"]["appearance"]["shirtMark"]
            .as_str()
            .unwrap_or(""),
    )
}

fn profile_error(error: OfficeError, name: &str) -> Failure {
    if error == OfficeError::IdentityInactive {
        return crate::output::identity_missing(name);
    }
    if error == OfficeError::RevisionConflict {
        return Failure::new(
            error.code(),
            "The Office profile changed after it was read. Read the current profile and reconcile your preserved draft before submitting a new revision. No profile was changed by this attempt.",
            1,
        );
    }
    if error == OfficeError::AvatarNotFound {
        return Failure::new(
            error.code(),
            "The selected avatar pack or key is not installed. The profile was not changed.",
            1,
        );
    }
    pairing_error(error)
}
fn unavailable_for(error: impl std::error::Error + 'static, editing: bool) -> Failure {
    if editing {
        Failure::new("OFFICE_LOCAL_UNCERTAIN", "Local Office did not confirm the profile edit. Read the current revision before retrying.", 1).caused_by(error)
    } else {
        unavailable(error)
    }
}
fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "OFFICE_IO_ERROR",
        "Could not access local Office profile state or output.",
        1,
    )
    .caused_by(error)
}
