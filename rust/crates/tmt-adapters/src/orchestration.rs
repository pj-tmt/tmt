//! Shared read/wire boundary. No admission, delivery, timer or job effects.
mod calendar;
mod read;
pub(crate) mod wire;

pub use calendar::OrchZone;
pub use read::{OrchReadFailure, orchestration_read, orchestration_read_with_time};
pub use wire::{OrchReadQuery, OrchSubjectDocument, decode_orch_query, orch_candidate_file};

use tmt_core::orchestration::OrchError;
pub fn orch_error_code(error: OrchError) -> &'static str {
    match error {
        OrchError::Invalid => "ORCH_INPUT_INVALID",
        OrchError::Limit => "ORCH_LIMIT",
        OrchError::TimeZone => "ORCH_TIME_ZONE",
        OrchError::NotFound => "ORCH_PLAN_NOT_FOUND",
        OrchError::Ambiguous => "ORCH_PLAN_AMBIGUOUS",
    }
}
pub fn orch_error_message(error: OrchError) -> &'static str {
    match error {
        OrchError::Invalid => "Invalid orchestration input or calendar selection.",
        OrchError::Limit => "Orchestration query exceeds its bounded expansion.",
        OrchError::TimeZone => "A named IANA time zone could not be resolved.",
        OrchError::NotFound => "Orchestration plan was not found.",
        OrchError::Ambiguous => "Plan name is ambiguous; select its UUID.",
    }
}
