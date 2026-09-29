//! One local world operation; no identity selection, implicit service or retry.

use crate::office_companion::invoke_office_world;
use crate::office_world::read_world_file;
use crate::{
    invocation::{OfficeLayoutOperation, OutputMode},
    output::Failure,
};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::interrupt::Interrupt;
use tmt_office_model::codec::office_world::SaveWorld;
use tmt_office_model::codec::office_world::reply::WorldFailure;
use tmt_office_model::codec::office_world::snapshot_value;

pub fn run(
    executable: &Path,
    operation: OfficeLayoutOperation,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let edit = match operation {
        OfficeLayoutOperation::Show => None,
        OfficeLayoutOperation::Apply {
            file,
            if_revision,
            legacy_basis,
        } => Some(SaveWorld {
            expected_revision: if_revision,
            legacy_basis,
            layout: read_world_file(Path::new(&file)).map_err(|error| failure(error.into()))?,
        }),
    };
    let interrupt = Interrupt::install().map_err(|error| unavailable(error, false))?;
    if interrupt.is_interrupted() {
        return Err(crate::office_pairing_command::interrupted());
    }
    let snapshot = invoke_office_world(
        executable,
        edit.as_ref(),
        Instant::now() + Duration::from_secs(30),
    )
    .map_err(|error| unavailable(error, edit.is_some()))?
    .map_err(failure)?;
    if interrupt.is_interrupted() {
        return Err(crate::office_pairing_command::interrupted());
    }
    let value = snapshot_value(&snapshot);
    let output = if mode.json {
        value.to_string()
    } else {
        serde_json::to_string_pretty(&value).map_err(|error| unavailable(error, edit.is_some()))?
    };
    writeln!(io::stdout().lock(), "{output}")
        .map_err(|error| unavailable(error, edit.is_some()))?;
    Ok(0)
}

fn failure(error: WorldFailure) -> Failure {
    Failure::new(error.code.code(), error.value().to_string(), 1)
        .suggestion("Run `tmt office layout show --json` to inspect the current layout and revision. Keep your draft; never retry at a newer revision without reviewing it.".into())
}
fn unavailable(error: impl std::error::Error + 'static, editing: bool) -> Failure {
    Failure::new(if editing { "OFFICE_LOCAL_UNCERTAIN" } else { "OFFICE_IO_ERROR" },
        if editing { "Office did not confirm the layout save. Keep your draft and inspect the saved revision before retrying; the save may have completed." }
        else { "Could not read the local Office layout." }, 1).caused_by(error)
}
