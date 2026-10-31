//! Shared read/wire boundary. No admission, delivery, timer or job effects.
mod calendar;
mod read;
mod receiving;
pub(crate) mod wire;
mod write;

pub use calendar::OrchZone;
pub use read::{OrchReadFailure, orchestration_read, orchestration_read_with_time};
pub use receiving::{DigestReceiving, DigestReceivingMember, DigestReceivingSettings};
pub use wire::{OrchReadQuery, OrchSubjectDocument, decode_orch_query, orch_candidate_file};
pub use write::{OrchMutationDocument, OrchWriteQuery, decode_orch_write, orchestration_write};

use tmt_core::orchestration::OrchError;
pub fn orch_error_code(error: OrchError) -> &'static str {
    match error {
        OrchError::Invalid => "ORCH_INPUT_INVALID",
        OrchError::Conflict => "ORCH_CONFLICT",
        OrchError::Limit => "ORCH_LIMIT",
        OrchError::TimeZone => "ORCH_TIME_ZONE",
        OrchError::NotFound => "ORCH_PLAN_NOT_FOUND",
        OrchError::MemberNotFound => "ORCH_MEMBER_NOT_FOUND",
        OrchError::ReceivingUnavailable => "ORCH_RECEIVING_UNAVAILABLE",
        OrchError::Ambiguous => "ORCH_PLAN_AMBIGUOUS",
        OrchError::NotPermitted => "ORCH_NOT_PERMITTED",
    }
}
pub fn orch_error_message(error: OrchError) -> &'static str {
    match error {
        OrchError::Invalid => "Invalid orchestration input or calendar selection.",
        OrchError::Conflict => {
            "Orchestration changed or a plan name or priority conflicts. Preview again before saving."
        }
        OrchError::Limit => "Orchestration query exceeds its bounded expansion.",
        OrchError::TimeZone => "A named IANA time zone could not be resolved.",
        OrchError::NotFound => "Orchestration plan was not found.",
        OrchError::MemberNotFound => "Orchestration member was not found or is retired.",
        OrchError::ReceivingUnavailable => {
            "The Digest tick has not published its settings yet; run tmt digest tick or install Digest."
        }
        OrchError::Ambiguous => "Plan name is ambiguous; select its UUID.",
        OrchError::NotPermitted => {
            "Only a person or a lead may do this; the calling agent is not a lead."
        }
    }
}
