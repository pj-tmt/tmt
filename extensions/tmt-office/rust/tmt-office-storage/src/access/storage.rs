//! The consented Office storage migration behind the companion protocol:
//! `storage-plan` reports what would happen, and `storage-migrate` runs only
//! for the plan digest the user saw.

use crate::{StorageLayout, migration};
use serde::Deserialize;
use tmt_adapters::{config::ConfigPaths, office_service};
use tmt_office_model::office_protocol::OfficeInvocation;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrateInput {
    consent: String,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    let document = match ConfigPaths::discover() {
        Ok(paths) => execute_at(operation, input, &paths),
        Err(_) => invalid("Cannot locate the tmt configuration."),
    };
    serde_json::to_vec(&document).expect("storage documents serialize")
}

pub fn execute_at(
    operation: OfficeInvocation,
    input: &[u8],
    paths: &ConfigPaths,
) -> serde_json::Value {
    let layout = StorageLayout::new(paths);
    // Reading the service receipt and health changes nothing.
    let running = office_service::status(paths, "")
        .ok()
        .map(|status| status.running);
    let result = match operation {
        OfficeInvocation::StoragePlan if input == b"{}" => {
            migration::plan(&layout, running).map(|plan| plan.json())
        }
        OfficeInvocation::StorageMigrate => {
            let Ok(input) = serde_json::from_slice::<MigrateInput>(input) else {
                return invalid("Invalid storage migration input.");
            };
            migration::migrate(
                &layout,
                &input.consent,
                running,
                &migration::OfficeService(paths),
            )
            .map(|switched| switched.json())
        }
        _ => return invalid("Invalid storage operation input."),
    };
    result.unwrap_or_else(|error| error.json())
}

fn invalid(message: &str) -> serde_json::Value {
    serde_json::json!({"error": {"code": "OFFICE_STORAGE_INVALID", "message": message}})
}
