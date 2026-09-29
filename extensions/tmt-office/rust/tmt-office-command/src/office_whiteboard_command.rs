//! Snapshot presentation and explicit file export; no database or browser token access.

use crate::office_companion::SnapshotResource;
use crate::office_companion::read_office_snapshot;
use crate::office_whiteboard::export::export_snapshot_image;
use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_office_model::codec::office_whiteboard::snapshot::encode_snapshot;

pub fn run(
    executable: &Path,
    reference: &str,
    output: Option<&str>,
    mode: OutputMode,
) -> Result<u8, Failure> {
    let resource = read_office_snapshot(
        executable,
        reference,
        output.is_some(),
        Instant::now() + Duration::from_secs(30),
    )
    .map_err(unavailable)?
    .map_err(|error| Failure::new(error.code(), error.to_string(), 1))?;
    match resource {
        SnapshotResource::Scene(snapshot) => {
            let bytes = encode_snapshot(&snapshot).map_err(unavailable)?;
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(&bytes)
                .and_then(|()| stdout.write_all(b"\n"))
                .map_err(unavailable)?;
        }
        SnapshotResource::Image(bytes) => {
            let path = output.expect("image requested only for export");
            export_snapshot_image(Path::new(path), &bytes).map_err(|error| {
                Failure::new(
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        "OUTPUT_EXISTS"
                    } else {
                        "OUTPUT_UNAVAILABLE"
                    },
                    format!("Could not publish snapshot to {path}: {error}"),
                    1,
                )
                .caused_by(error)
            })?;
            let value = if mode.json {
                serde_json::json!({"path":path,"bytes":bytes.len(),"snapshotId":tmt_office_model::office_whiteboard::snapshot::resolve_snapshot_reference(reference).expect("validated reference")}).to_string()
            } else {
                format!("Saved snapshot to {path}")
            };
            writeln!(io::stdout().lock(), "{value}").map_err(unavailable)?;
        }
    }
    Ok(0)
}

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new(
        "OFFICE_IO_ERROR",
        "Could not read or present the stored snapshot. Check that Office is up to date.",
        1,
    )
    .caused_by(error)
}
