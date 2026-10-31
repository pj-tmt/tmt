//! Job wire/calendar conversion and public projection.
mod read;
pub(crate) use read::read;

use serde::{Deserialize, Serialize};
use tmt_core::orchestration::jobs::{JobError, JobSchedule};

// This persisted definition is independent of hook-queue payloads. Calendar
// admission validates the rule before saving; decoding never chooses a new zone.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum JobScheduleDocument {
    Every {
        zone: String,
        seconds: i64,
        #[serde(rename = "anchorMs")]
        anchor_ms: i64,
        from: Option<String>,
    },
    At {
        zone: String,
        at: String,
        on: Option<String>,
    },
    Cron {
        zone: String,
        expression: String,
    },
}
impl From<&JobSchedule> for JobScheduleDocument {
    fn from(rule: &JobSchedule) -> Self {
        match rule {
            JobSchedule::Every {
                zone,
                seconds,
                anchor_ms,
                from,
            } => Self::Every {
                zone: zone.clone(),
                seconds: *seconds,
                anchor_ms: *anchor_ms,
                from: from.clone(),
            },
            JobSchedule::At { zone, at, on } => Self::At {
                zone: zone.clone(),
                at: at.clone(),
                on: on.clone(),
            },
            JobSchedule::Cron { zone, expression } => Self::Cron {
                zone: zone.clone(),
                expression: expression.clone(),
            },
        }
    }
}
impl From<JobScheduleDocument> for JobSchedule {
    fn from(rule: JobScheduleDocument) -> Self {
        match rule {
            JobScheduleDocument::Every {
                zone,
                seconds,
                anchor_ms,
                from,
            } => Self::Every {
                zone,
                seconds,
                anchor_ms,
                from,
            },
            JobScheduleDocument::At { zone, at, on } => Self::At { zone, at, on },
            JobScheduleDocument::Cron { zone, expression } => Self::Cron { zone, expression },
        }
    }
}

pub(crate) fn calendar(schedule: &JobSchedule) -> Result<tmt_job_calendar::Schedule, JobError> {
    let value =
        serde_json::to_value(JobScheduleDocument::from(schedule)).map_err(|_| JobError::Invalid)?;
    if serde_json::to_vec(&value)
        .map_err(|_| JobError::Invalid)?
        .len()
        > 4096
    {
        return Err(JobError::Limit);
    }
    tmt_job_calendar::Schedule::from_document(&value).map_err(|_| JobError::Invalid)
}

pub(crate) fn error_code(error: &crate::storage::JobStoreError) -> &'static str {
    use crate::storage::JobStoreError;
    match error {
        JobStoreError::Job(JobError::NotFound) => "ORCH_JOB_NOT_FOUND",
        JobStoreError::Job(JobError::Conflict) => "ORCH_CONFLICT",
        JobStoreError::Job(JobError::NoOwner) => "ORCH_JOB_NO_OWNER",
        JobStoreError::Job(JobError::NotAllowed) => "ORCH_JOB_NOT_PERMITTED",
        JobStoreError::Job(JobError::Limit) => "ORCH_LIMIT",
        JobStoreError::Job(JobError::Invalid) => "ORCH_INPUT_INVALID",
        JobStoreError::Dispatch(error) => error.code(),
        JobStoreError::Storage(_) => "ORCH_STORAGE_UNAVAILABLE",
    }
}
pub(crate) fn error_message(error: &crate::storage::JobStoreError) -> &'static str {
    use crate::storage::JobStoreError;
    match error {
        JobStoreError::Job(JobError::NotFound) => "Job was not found.",
        JobStoreError::Job(JobError::Conflict) => {
            "Job or policy changed. Preview again before saving."
        }
        JobStoreError::Job(JobError::NoOwner) => "Job has no current owner.",
        JobStoreError::Job(JobError::NotAllowed) => "This job operation is not permitted.",
        JobStoreError::Job(JobError::Limit) => "Job operation exceeds its bounded expansion.",
        JobStoreError::Job(JobError::Invalid) => "Invalid job input, calendar or page selection.",
        JobStoreError::Dispatch(_) => {
            "Job acceptance could not be confirmed. Recover its canonical dispatch receipt before retrying."
        }
        JobStoreError::Storage(_) => "Could not read or change job storage.",
    }
}
