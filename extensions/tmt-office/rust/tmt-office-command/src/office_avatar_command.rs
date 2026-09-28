//! Bounded local avatar-pack commands through the verified Office companion.

use crate::{
    invocation::{OfficeAvatarOperation, OfficeOperation, OutputMode},
    output::Failure,
};
use serde_json::json;
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::office_avatar::read_pack_file;
use tmt_adapters::office_companion::invoke_local_office_avatar;
use tmt_office_model::codec::office_avatar::AvatarPackError;
use tmt_office_model::codec::office_avatar::command_pack_input;
use tmt_office_model::codec::office_avatar::quality_warnings;
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};

pub fn run(executable: &Path, operation: OfficeOperation, mode: OutputMode) -> Result<u8, Failure> {
    let OfficeOperation::Avatar(operation) = operation else {
        return Err(Failure::new(
            "USAGE_ERROR",
            "Expected an Office avatar operation.",
            1,
        ));
    };
    if matches!(operation, OfficeAvatarOperation::Preview { .. }) {
        return preview(executable, operation, mode);
    }
    let mut warnings = None;
    let (invocation, input, mutating) = match operation {
        OfficeAvatarOperation::Validate { file } => {
            let pack = read_pack_file(Path::new(&file)).map_err(avatar_file_error)?;
            warnings = Some(quality_warnings(&pack));
            (
                OfficeInvocation::LocalAvatarValidate,
                command_pack_input(&pack),
                false,
            )
        }
        OfficeAvatarOperation::Install { file, if_revision } => {
            let pack = read_pack_file(Path::new(&file)).map_err(avatar_file_error)?;
            let mut input = command_pack_input(&pack);
            input["expectedRevision"] = json!(if_revision);
            (OfficeInvocation::LocalAvatarInstall, input, true)
        }
        OfficeAvatarOperation::Remove {
            digest,
            if_revision,
        } => (
            OfficeInvocation::LocalAvatarRemove,
            json!({"digest":digest,"expectedRevision":if_revision}),
            true,
        ),
        OfficeAvatarOperation::List { limit, cursor } => (
            OfficeInvocation::LocalAvatarList,
            json!({"limit":limit,"cursor":cursor}),
            false,
        ),
        OfficeAvatarOperation::Show { digest } => (
            OfficeInvocation::LocalAvatarShow,
            json!({"digest":digest}),
            false,
        ),
        OfficeAvatarOperation::Preview { .. } => unreachable!(),
    };
    let mut result = invoke_local_office_avatar(
        executable,
        invocation,
        &input,
        Instant::now() + Duration::from_secs(30),
    )
    .map_err(|error| unavailable(error, mutating))?
    .map_err(avatar_error)?;
    if let Some(warnings) = warnings {
        result["warnings"] = json!(warnings);
    }
    publish(result, mode)
}

fn preview(
    executable: &Path,
    operation: OfficeAvatarOperation,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let OfficeAvatarOperation::Preview { file } = operation else {
        unreachable!()
    };
    let pack = read_pack_file(Path::new(&file)).map_err(avatar_file_error)?;
    let version =
        tmt_adapters::office_companion::probe_office_companion(executable).map_err(|error| {
            Failure::new("OFFICE_INCOMPATIBLE", error.to_string(), 1).caused_by(error)
        })?;
    let paths = tmt_adapters::config::ConfigPaths::discover().map_err(|error| {
        Failure::new("OFFICE_LOCATION_INVALID", error.to_string(), 1).caused_by(error)
    })?;
    let value = tmt_adapters::office_service::preview_avatar(&paths, &version.to_string(), &pack)
        .map_err(preview_error)?;
    publish(value, mode)
}

fn publish(value: serde_json::Value, mode: OutputMode) -> Result<u8, Failure> {
    let output = if mode.json {
        value.to_string()
    } else {
        serde_json::to_string_pretty(&value).map_err(|error| unavailable(error, false))?
    };
    writeln!(io::stdout().lock(), "{output}").map_err(|error| unavailable(error, false))?;
    Ok(0)
}
fn avatar_file_error(error: AvatarPackError) -> Failure {
    let code = if matches!(error, AvatarPackError::Io(_)) {
        "OFFICE_IO_ERROR"
    } else {
        "OFFICE_AVATAR_INVALID"
    };
    Failure::new(code, error.to_string(), 1).caused_by(error)
}
fn avatar_error(error: OfficeError) -> Failure {
    Failure::new(error.code(), error.to_string(), 1)
}
fn unavailable(error: impl std::error::Error + 'static, mutating: bool) -> Failure {
    Failure::new(
        if mutating { "OFFICE_LOCAL_UNCERTAIN" } else { "OFFICE_IO_ERROR" },
        if mutating { "Local Office did not confirm the avatar catalog mutation. Read the current catalog revision before retrying." } else { "Could not read local Office avatar state." },
        1,
    ).caused_by(error)
}
fn preview_error(error: tmt_adapters::office_service::PreviewError) -> Failure {
    use tmt_adapters::office_service::PreviewError;
    let code = match error {
        PreviewError::NotRunning | PreviewError::RestartRequired => "OFFICE_SERVICE_NOT_RUNNING",
        PreviewError::Limit => "OFFICE_AVATAR_PREVIEW_LIMIT",
        PreviewError::Invalid => "OFFICE_AVATAR_INVALID",
        PreviewError::Unavailable(_) => "OFFICE_IO_ERROR",
    };
    Failure::new(code, error.to_string(), 1).caused_by(error)
}
