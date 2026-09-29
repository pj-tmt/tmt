//! Office's side of the consented extension hook protocol:
//! `tmt-office __tmt-hooks 1 capabilities|observe`.
//!
//! Office runs only after the user enabled it with `tmt extension hooks
//! enable office`. `context` returns a short read-only summary of the
//! identity's desk (never opening storage for writing). An observation
//! triggers the full, idempotent reconciliation
//! of Office's stored references; the event list only says that something
//! changed, so dropped, duplicate or reordered observations converge the same
//! way. Output is always empty, and core ignores the exit status.

use serde_json::Value;
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::ExitCode,
};
use tmt_adapters::{config::ConfigPaths, extension_hooks};
use tmt_office_storage::{OfficeStore, StorageLayout};

pub const PREFIX: &str = "__tmt-hooks";
const INPUT_LIMIT: u64 = 64 * 1024;

pub fn run(arguments: &[OsString]) -> ExitCode {
    let operation = match arguments {
        [prefix, version, operation]
            if prefix == PREFIX && version == extension_hooks::PROTOCOL_VERSION =>
        {
            operation.to_string_lossy().into_owned()
        }
        _ => return ExitCode::from(2),
    };
    match operation.as_str() {
        "capabilities" => {
            let reply = format!(
                "TMT-HOOKS/{}\n{}\n{}\n",
                extension_hooks::PROTOCOL_VERSION,
                extension_hooks::LIFECYCLE_CAPABILITY,
                extension_hooks::CONTEXT_CAPABILITY
            );
            match io::stdout().lock().write_all(reply.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        "observe" => {
            let mut input = Vec::new();
            let read = io::stdin()
                .lock()
                .take(INPUT_LIMIT + 1)
                .read_to_end(&mut input);
            if read.is_err() || input.len() as u64 > INPUT_LIMIT || !valid(&input) {
                return ExitCode::from(2);
            }
            if reconcile() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "context" => {
            let mut input = Vec::new();
            let read = io::stdin()
                .lock()
                .take(CONTEXT_INPUT_LIMIT + 1)
                .read_to_end(&mut input);
            let Some(identity_id) = read
                .ok()
                .filter(|_| input.len() as u64 <= CONTEXT_INPUT_LIMIT)
                .and_then(|_| context_identity(&input))
            else {
                return ExitCode::from(2);
            };
            // Strictly read-only: no reconciliation, migration or file creation.
            let summary = ConfigPaths::discover().ok().and_then(|paths| {
                tmt_office_storage::context::summary(&StorageLayout::new(&paths), &identity_id)
            });
            let reply = serde_json::json!({"summary": summary}).to_string();
            match io::stdout().lock().write_all(reply.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        _ => ExitCode::from(2),
    }
}

const CONTEXT_INPUT_LIMIT: u64 = 4096;

/// `{"version":1,"identityId":"<uuid>"}`, nothing else.
fn context_identity(input: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(input).ok()?;
    let object = value.as_object()?;
    let identity = object.get("identityId")?.as_str()?;
    (object.len() == 2 && value["version"] == 1 && uuid_like(identity)).then(|| identity.to_owned())
}

fn uuid_like(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn valid(input: &[u8]) -> bool {
    serde_json::from_slice::<Value>(input)
        .is_ok_and(|value| value["version"] == 1 && value["events"].as_array().is_some())
}

fn reconcile() -> bool {
    let Ok(paths) = ConfigPaths::discover() else {
        return false;
    };
    OfficeStore::open_configured(&StorageLayout::new(&paths))
        .and_then(|mut store| {
            let outcome = store.reconcile(tmt_adapters::request_runtime::wall_time_ms() as i64);
            let _ = store.close();
            outcome
        })
        .is_ok()
}
