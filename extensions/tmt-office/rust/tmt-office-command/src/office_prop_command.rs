//! Bounded local prop-pack commands through the verified Office companion.

use serde_json::json;
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::office_companion::invoke_local_office_prop;
use tmt_adapters::office_prop::read_pack_file;
use tmt_office_model::codec::office_prop::PropPackError;
use tmt_office_model::codec::office_prop::command_pack_input;
use tmt_office_model::codec::office_prop::quality_warnings;
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};

use crate::{
    invocation::{OfficeOperation, OfficePropOperation, OutputMode},
    output::Failure,
};

pub fn run(executable: &Path, operation: OfficeOperation, mode: OutputMode) -> Result<u8, Failure> {
    let OfficeOperation::Prop(operation) = operation else {
        return Err(Failure::new(
            "USAGE_ERROR",
            "Expected an Office prop operation.",
            1,
        ));
    };
    if matches!(operation, OfficePropOperation::Preview { .. }) {
        return preview(executable, operation, mode);
    }
    let mut warnings = None;
    let (invocation, input, mutating) = match operation {
        OfficePropOperation::Validate { file } => {
            let pack = read_pack_file(Path::new(&file)).map_err(prop_file_error)?;
            warnings = Some(quality_warnings(&pack));
            (
                OfficeInvocation::LocalPropValidate,
                command_pack_input(&pack),
                false,
            )
        }
        OfficePropOperation::Install { file, if_revision } => {
            let pack = read_pack_file(Path::new(&file)).map_err(prop_file_error)?;
            let mut input = command_pack_input(&pack);
            input["expectedRevision"] = json!(if_revision);
            (OfficeInvocation::LocalPropInstall, input, true)
        }
        OfficePropOperation::Remove {
            digest,
            if_revision,
        } => (
            OfficeInvocation::LocalPropRemove,
            json!({"digest":digest,"expectedRevision":if_revision}),
            true,
        ),
        OfficePropOperation::List { limit, cursor } => (
            OfficeInvocation::LocalPropList,
            json!({"limit":limit,"cursor":cursor}),
            false,
        ),
        OfficePropOperation::Show { digest } => (
            OfficeInvocation::LocalPropShow,
            json!({"digest":digest}),
            false,
        ),
        OfficePropOperation::Preview { .. } => unreachable!(),
    };
    let mut result = invoke_local_office_prop(
        executable,
        invocation,
        &input,
        Instant::now() + Duration::from_secs(30),
    )
    .map_err(|error| unavailable(error, mutating))?
    .map_err(prop_error)?;
    if let Some(warnings) = warnings {
        result["warnings"] = json!(warnings);
    }
    publish(result, mode)
}

fn preview(
    executable: &Path,
    operation: OfficePropOperation,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let OfficePropOperation::Preview { file } = operation else {
        unreachable!()
    };
    let pack = read_pack_file(Path::new(&file)).map_err(prop_file_error)?;
    let version =
        tmt_adapters::office_companion::probe_office_companion(executable).map_err(|error| {
            Failure::new("OFFICE_INCOMPATIBLE", error.to_string(), 1).caused_by(error)
        })?;
    let paths = tmt_adapters::config::ConfigPaths::discover().map_err(|error| {
        Failure::new("OFFICE_LOCATION_INVALID", error.to_string(), 1).caused_by(error)
    })?;
    let value = tmt_adapters::office_service::preview(&paths, &version.to_string(), &pack)
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

fn prop_invalid(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("OFFICE_PROP_INVALID", error.to_string(), 1).caused_by(error)
}

fn prop_file_error(error: PropPackError) -> Failure {
    if matches!(error, PropPackError::Io(_)) {
        Failure::new("OFFICE_IO_ERROR", error.to_string(), 1).caused_by(error)
    } else {
        prop_invalid(error)
    }
}

fn prop_error(error: OfficeError) -> Failure {
    Failure::new(error.code(), error.to_string(), 1)
}

fn unavailable(error: impl std::error::Error + 'static, mutating: bool) -> Failure {
    Failure::new(
        if mutating { "OFFICE_LOCAL_UNCERTAIN" } else { "OFFICE_IO_ERROR" },
        if mutating {
            "Local Office did not confirm the prop catalog mutation. Read the current catalog revision before retrying."
        } else {
            "Could not read local Office prop state."
        },
        1,
    )
    .caused_by(error)
}

fn preview_error(error: tmt_adapters::office_service::PreviewError) -> Failure {
    use tmt_adapters::office_service::PreviewError;
    let code = match error {
        PreviewError::NotRunning | PreviewError::RestartRequired => "OFFICE_SERVICE_NOT_RUNNING",
        PreviewError::Limit => "OFFICE_PROP_PREVIEW_LIMIT",
        PreviewError::Invalid => "OFFICE_PROP_INVALID",
        PreviewError::Unavailable(_) => "OFFICE_IO_ERROR",
    };
    Failure::new(code, error.to_string(), 1).caused_by(error)
}
