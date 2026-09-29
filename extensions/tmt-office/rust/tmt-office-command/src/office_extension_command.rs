//! File acquisition and presentation only; companion admission owns semantics.

use crate::office_companion::validate_office_extension;
use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::bounded_file;
use tmt_office_model::codec::office_extension::preflight::ValidationInput;
use tmt_office_model::codec::office_extension::preflight::ValidationReport;

pub fn run(executable: &Path, file: &str, instance: &str, mode: OutputMode) -> Result<u8, Failure> {
    let input = ValidationInput {
        definition: read(file)?,
        instance: read(instance)?,
    };
    let report =
        validate_office_extension(executable, &input, Instant::now() + Duration::from_secs(30))
            .map_err(unavailable)?;
    if let ValidationReport::Invalid { problem } = report {
        return Err(Failure::new(
            "OFFICE_EXTENSION_INVALID",
            problem.to_string(),
            1,
        ));
    }
    let text = if mode.json {
        serde_json::to_string(&report).map_err(unavailable)?
    } else {
        let ValidationReport::Valid {
            definition,
            instance,
            ..
        } = report
        else {
            unreachable!()
        };
        format!(
            "Valid structure: {instance} -> {definition}\nArtwork availability and host capabilities were not checked. Nothing was installed."
        )
    };
    writeln!(io::stdout().lock(), "{text}").map_err(unavailable)?;
    Ok(0)
}

fn read(path: &str) -> Result<String, Failure> {
    let bytes =
        bounded_file::read_no_follow(Path::new(path), tmt_office_model::office_block::INPUT_LIMIT)
            .map_err(|error| {
                Failure::new("OFFICE_EXTENSION_INPUT_INVALID", error.to_string(), 1)
                    .caused_by(error)
            })?;
    String::from_utf8(bytes).map_err(|error| {
        Failure::new(
            "OFFICE_EXTENSION_INPUT_INVALID",
            "Expected a UTF-8 JSON file.",
            1,
        )
        .caused_by(error)
    })
}

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "OFFICE_IO_ERROR",
        "Could not validate the extension with the installed Office companion.",
        1,
    )
    .caused_by(error)
}
